//! `adb` wrapper. Every call passes an argument list — nothing is ever handed to
//! a shell — and every call has a timeout.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::domain::{DwError, DwResult, ErrorCode};
use crate::proc;

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// The device-shell command one `input` call runs.
///
/// Key events are scoped only on the device's own display: Android delivers an
/// injected key to the display that holds the focused window, and drops it when
/// `-d` names a different one. An app window's mapping therefore sends an
/// unscoped key (it only fires while that window is focused), and the phone keeps
/// its scope so clicking the phone cannot type into an app window.
fn input_line(display_id: u32, args: &[String]) -> String {
    let scoped = display_id == super::display::DEVICE_DISPLAY
        || args.first().map(String::as_str) != Some("keyevent");
    let mut line = if scoped {
        format!("input -d {display_id}")
    } else {
        "input".to_string()
    };
    for arg in args {
        line.push(' ');
        line.push_str(&quote_for_device_shell(arg));
    }
    line
}

impl CmdOutput {
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }

    pub fn combined(&self) -> String {
        let mut text = self.stdout.trim().to_string();
        let err = self.stderr.trim();
        if !err.is_empty() {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(err);
        }
        text
    }
}

#[derive(Debug, Clone)]
pub struct Device {
    pub serial: String,
    pub state: String,
}

/// What the guest was denied, and whether the denial was observed.
///
/// The status text is built from this, so an installation that did not happen (a
/// production image that refuses `adb root`, a rule the kernel rejected) is
/// reported as a warning rather than a guarantee.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostIsolation {
    /// `iptables` refuses 10.0.2.2 — the emulator's IPv4 host gateway.
    pub ipv4: bool,
    /// `ip6tables` refuses fec0::2 — the same gateway over IPv6. A v4-only rule
    /// is not containment: measured here, the guest reached a host listener bound
    /// to `::1` through `fec0::2` while it was open.
    pub ipv6: bool,
    /// A real connection from the guest to the adb port was refused afterwards.
    pub probe_blocked: Option<bool>,
    /// Why the rules are not in place, when they are not.
    pub detail: Option<String>,
}

impl HostIsolation {
    /// Both families denied, and a refused connection actually observed.
    pub fn active(&self) -> bool {
        self.ipv4 && self.ipv6 && self.probe_blocked == Some(true)
    }
}

/// Check and insert for `iptables -C`/`-I`. The check half is what the caller
/// reads: `-C` exits 0 only when the rule is in the kernel.
const IPV4_RULE: (&str, &str) = (
    "iptables -C OUTPUT -d 10.0.2.2 -j REJECT",
    "iptables -I OUTPUT 1 -d 10.0.2.2 -j REJECT",
);
const IPV6_RULE: (&str, &str) = (
    "ip6tables -C OUTPUT -d fec0::2 -j REJECT",
    "ip6tables -I OUTPUT 1 -d fec0::2 -j REJECT",
);
/// The adb smart socket, asked the way a guest app would. `2>&1` keeps nc's
/// refusal text, which is the only difference between "blocked" and "nothing
/// was listening anyway".
const HOST_PROBE: &str = "printf '000chost:version' | toybox nc -w 2 10.0.2.2 5037 2>&1";

/// `Some(false)` the socket answered, `Some(true)` the connection was refused,
/// `None` the guest could not be asked — never guess containment from silence.
pub fn classify_host_probe(out: &CmdOutput) -> Option<bool> {
    if out.stdout.contains("OKAY") {
        Some(false)
    } else if out.combined().to_ascii_lowercase().contains("refused") {
        Some(true)
    } else {
        None
    }
}

/// Extract the first non-protocol line from `adb emu avd name`.
pub fn parse_emulator_avd_name(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && *line != "OK")
        .map(|line| line.trim_end_matches('\r').to_string())
}

/// What `getprop` said about the runtime in one round trip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Readiness {
    pub booted: bool,
    pub api: Option<u32>,
}

/// `getprop` prints every property as `[name]: [value]`. Reading the whole dump
/// once replaces the three separate adb calls a status poll used to make.
pub fn parse_props(text: &str) -> Readiness {
    fn prop<'a>(text: &'a str, name: &str) -> Option<&'a str> {
        text.lines().find_map(|line| {
            let rest = line.trim().strip_prefix('[')?;
            let (key, rest) = rest.split_once("]: [")?;
            (key == name).then(|| rest.strip_suffix(']').unwrap_or(rest))
        })
    }
    Readiness {
        booted: prop(text, "sys.boot_completed") == Some("1"),
        api: prop(text, "ro.build.version.sdk").and_then(|v| v.trim().parse().ok()),
    }
}

#[derive(Debug, Clone)]
pub struct RemotePackage {
    pub package_id: String,
    pub version_code: Option<u64>,
}

/// One Android display and the component Android says is focused on it. A
/// DroidWin app window is a virtual display, so this is the only honest way to
/// map a package to the display its input must target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayFocus {
    pub display_id: u32,
    pub package_id: Option<String>,
    pub activity: Option<String>,
    /// Geometry Android reports for the display, when the dump includes it.
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Clone)]
pub struct Adb {
    exe: PathBuf,
}

impl Adb {
    pub fn new(exe: PathBuf) -> Self {
        Self { exe }
    }

    pub fn exe(&self) -> &Path {
        &self.exe
    }

    pub async fn run(&self, args: &[&str], timeout: Duration) -> DwResult<CmdOutput> {
        let mut cmd = Command::new(&self.exe);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        // Never allocate a console window for an adb invocation.
        proc::no_console_async(&mut cmd);

        let child = cmd.spawn().map_err(|e| {
            DwError::io(
                ErrorCode::RuntimeUnavailable,
                "adb could not be started.",
                &e,
            )
        })?;

        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(DwError::io(
                    ErrorCode::RuntimeUnavailable,
                    "adb exited unexpectedly.",
                    &e,
                ))
            }
            Err(_) => {
                return Err(DwError::new(
                    ErrorCode::Timeout,
                    format!("adb did not answer within {}s.", timeout.as_secs()),
                ))
            }
        };

        Ok(CmdOutput {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        })
    }

    async fn shell(
        &self,
        serial: &str,
        command: &[&str],
        timeout: Duration,
    ) -> DwResult<CmdOutput> {
        let line = device_shell_command(command);
        self.shell_line(serial, &line, timeout).await
    }

    /// Run a binary-safe adb command and keep stdout as bytes. Screenshots are
    /// PNG streams: decoding them as text corrupts them.
    pub async fn run_bytes(&self, args: &[&str], timeout: Duration) -> DwResult<Vec<u8>> {
        let mut cmd = Command::new(&self.exe);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        proc::no_console_async(&mut cmd);
        let child = cmd.spawn().map_err(|e| {
            DwError::io(
                ErrorCode::RuntimeUnavailable,
                "adb could not be started.",
                &e,
            )
        })?;
        let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(DwError::io(
                    ErrorCode::RuntimeUnavailable,
                    "adb exited unexpectedly.",
                    &e,
                ))
            }
            Err(_) => {
                return Err(DwError::new(
                    ErrorCode::Timeout,
                    format!("adb did not answer within {}s.", timeout.as_secs()),
                ))
            }
        };
        if output.status.code() != Some(0) {
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                "adb could not capture that stream.",
            )
            .detail(String::from_utf8_lossy(&output.stderr).trim().to_string()));
        }
        Ok(output.stdout)
    }

    /// Execute one already-quoted command line on the device.
    async fn shell_line(&self, serial: &str, line: &str, timeout: Duration) -> DwResult<CmdOutput> {
        self.run(&["-s", serial, "shell", line], timeout).await
    }

    /// A PNG screenshot of one display.
    ///
    /// `screencap -d` does **not** take the logical display id the window
    /// manager uses; it takes a SurfaceFlinger display token. The two are
    /// bridged through the display's `uniqueId`. Passing the logical id there
    /// fails with "Display Id '3' is not valid" — verified on Android 36.
    pub async fn screenshot_png(&self, serial: &str, display_id: u32) -> DwResult<Vec<u8>> {
        let mut args: Vec<String> = vec![
            "-s".into(),
            serial.into(),
            "exec-out".into(),
            "screencap".into(),
            "-p".into(),
        ];
        if display_id != 0 {
            let token = self.surface_token(serial, display_id).await?;
            args.push("-d".into());
            args.push(token.to_string());
        }
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let png = self.run_bytes(&borrowed, Duration::from_secs(25)).await?;
        if !png.starts_with(b"\x89PNG") {
            let detail = String::from_utf8_lossy(&png[..png.len().min(200)]).to_string();
            if detail.contains("not valid") {
                return Err(DwError::new(
                    ErrorCode::RuntimeUnavailable,
                    format!("Android no longer has display {display_id}."),
                )
                .detail(detail));
            }
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                format!("Android did not return an image for display {display_id}."),
            )
            .detail(detail));
        }
        Ok(png)
    }

    /// Map a logical display id to the SurfaceFlinger display token that
    /// `screencap -d` accepts.
    async fn surface_token(&self, serial: &str, display_id: u32) -> DwResult<u64> {
        let display = self
            .shell(serial, &["dumpsys", "display"], Duration::from_secs(20))
            .await?;
        let unique = parse_display_unique_ids(&display.stdout)
            .into_iter()
            .find(|(id, _)| *id == display_id)
            .map(|(_, unique)| unique)
            .ok_or_else(|| {
                DwError::new(
                    ErrorCode::RuntimeUnavailable,
                    format!("Android does not list display {display_id}."),
                )
                .detail("The app window may have just closed.")
            })?;

        let surface = self
            .shell(
                serial,
                &["dumpsys", "SurfaceFlinger", "--display-id"],
                Duration::from_secs(20),
            )
            .await?;
        parse_sf_display_tokens(&surface.stdout)
            .into_iter()
            .find(|(candidate, _)| candidate == &unique)
            .map(|(_, token)| token)
            .ok_or_else(|| {
                DwError::new(
                    ErrorCode::BackendFailure,
                    format!("SurfaceFlinger has no token for display {display_id}."),
                )
                .detail(format!("uniqueId {unique}"))
            })
    }

    /// Send one `input` command to a specific display. Arguments are quoted for
    /// the device shell, which is what `adb shell` hands the command line to.
    ///
    /// A key event is scoped only for the device's own display. Android delivers
    /// injected keys to whichever display holds the focused window and **drops**
    /// them when `-d` names another one, logging
    /// `Focused display #0 does not have a focused window. But another display has
    /// a focused window`. An app window's mapped key therefore leaves `-d` off and
    /// lands on its own display, because a mapping only fires while that window is
    /// the focused one. The phone keeps its scope, so clicking the phone can never
    /// type into an app window. Motion events are display-scoped on every display.
    pub async fn input(
        &self,
        serial: &str,
        display_id: u32,
        args: &[String],
    ) -> DwResult<CmdOutput> {
        let line = input_line(display_id, args);
        let out = self
            .shell_line(serial, &line, Duration::from_secs(20))
            .await?;
        if out.ok() {
            Ok(out)
        } else {
            Err(
                DwError::new(ErrorCode::BackendFailure, "Android rejected that input.")
                    .detail(out.combined()),
            )
        }
    }

    /// Every display Android knows about, with the component focused on it.
    pub async fn display_focus(&self, serial: &str) -> DwResult<Vec<DisplayFocus>> {
        let out = self
            .shell(
                serial,
                &["dumpsys", "window", "displays"],
                Duration::from_secs(20),
            )
            .await?;
        if !out.ok() {
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Android did not list its displays.",
            )
            .detail(out.combined()));
        }
        let displays = parse_display_focus(&out.stdout);
        if displays.is_empty() {
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                "Android's window dump did not contain a display.",
            )
            .detail(truncate(&out.stdout, 400)));
        }
        Ok(displays)
    }

    /// The display id running `package_id`, when Android put it on one.
    pub async fn display_for_package(&self, serial: &str, package_id: &str) -> Option<u32> {
        self.display_focus_for_package(serial, package_id)
            .await
            .map(|d| d.display_id)
    }

    /// The display running `package_id` together with the geometry Android is
    /// composing there. A rotated virtual display reports the geometry it is
    /// drawn in, which is what input coordinates and screenshots are measured in.
    ///
    /// Usually the display's focused window is the app. It is not on a fresh
    /// image: the first launch of almost any app puts a system permission dialog
    /// (the permission controller, not the app) in front of it, and on that one
    /// display the app itself never holds focus. The app's own task is still
    /// there, so a display that hosts the package's task counts as its display
    /// instead of being reported as "nothing drawn".
    pub async fn display_focus_for_package(
        &self,
        serial: &str,
        package_id: &str,
    ) -> Option<DisplayFocus> {
        let displays = self.display_focus(serial).await.ok()?;
        if let Some(focused) = displays
            .iter()
            .find(|d| d.package_id.as_deref() == Some(package_id))
        {
            return Some(focused.clone());
        }
        let display_id = self.task_display(serial, package_id).await?;
        let mut found = displays.into_iter().find(|d| d.display_id == display_id)?;
        found.package_id = Some(package_id.to_string());
        Some(found)
    }

    /// The display hosting `package_id`'s tasks, read from the activity dump.
    ///
    /// `dumpsys window displays` only names the focused component per display,
    /// which a system dialog displaces; the activity dump keeps the task itself.
    async fn task_display(&self, serial: &str, package_id: &str) -> Option<u32> {
        let out = self
            .shell(
                serial,
                &["dumpsys", "activity", "activities"],
                Duration::from_secs(20),
            )
            .await
            .ok()?;
        if !out.ok() {
            return None;
        }
        parse_task_display(&out.stdout, package_id)
    }

    pub async fn foreground_on(&self, serial: &str, display_id: u32) -> Option<DisplayFocus> {
        self.display_focus(serial)
            .await
            .ok()?
            .into_iter()
            .find(|d| d.display_id == display_id)
    }

    /// Dump the accessibility hierarchy. Android writes it to a file, so this is
    /// dump → read → remove; a leftover file from an interrupted call is
    /// overwritten by the next one rather than accumulating.
    pub async fn ui_hierarchy_xml(&self, serial: &str) -> DwResult<String> {
        const REMOTE: &str = "/data/local/tmp/droidwin-ui.xml";
        let dump = self
            .shell(
                serial,
                &["uiautomator", "dump", REMOTE],
                Duration::from_secs(25),
            )
            .await?;
        if !dump.ok() || !dump.combined().contains("dumped to") {
            let _ = self
                .shell(serial, &["rm", "-f", REMOTE], Duration::from_secs(10))
                .await;
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                "Android could not dump its UI hierarchy.",
            )
            .detail(dump.combined()));
        }
        let xml = self
            .run_bytes(
                &["-s", serial, "exec-out", "cat", REMOTE],
                Duration::from_secs(25),
            )
            .await;
        let _ = self
            .shell(serial, &["rm", "-f", REMOTE], Duration::from_secs(10))
            .await;
        let xml = xml?;
        Ok(String::from_utf8_lossy(&xml).to_string())
    }

    pub async fn devices(&self) -> DwResult<Vec<Device>> {
        let out = self.run(&["devices"], Duration::from_secs(15)).await?;
        if !out.ok() {
            return Err(
                DwError::new(ErrorCode::RuntimeUnavailable, "adb could not list devices.")
                    .detail(out.combined()),
            );
        }
        let mut devices = Vec::new();
        for line in out.stdout.lines().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let mut parts = line.split_whitespace();
            if let (Some(serial), Some(state)) = (parts.next(), parts.next()) {
                devices.push(Device {
                    serial: serial.to_string(),
                    state: state.to_string(),
                });
            }
        }
        Ok(devices)
    }

    /// One adb call answers "is the device there and has Android finished
    /// booting" — the readiness check every poll and wait loop relies on.
    pub async fn readiness(&self, serial: &str) -> DwResult<Readiness> {
        let out = self
            .shell(serial, &["getprop"], Duration::from_secs(10))
            .await?;
        if !out.ok() {
            return Err(
                DwError::new(ErrorCode::RuntimeUnavailable, "The runtime did not answer.")
                    .detail(out.combined()),
            );
        }
        Ok(parse_props(&out.stdout))
    }

    /// Ask the emulator console which AVD owns this transport. The serial is a
    /// port-based label, so it alone is not enough to adopt a device that is
    /// already listening on the same port.
    pub async fn emulator_avd_name(&self, serial: &str) -> DwResult<String> {
        let out = self
            .run(
                &["-s", serial, "emu", "avd", "name"],
                Duration::from_secs(10),
            )
            .await?;
        if !out.ok() {
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "ADB could not identify the emulator attached to its managed serial.",
            )
            .detail(out.combined()));
        }
        parse_emulator_avd_name(&out.stdout).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeUnavailable,
                "The emulator console returned no AVD name for its managed serial.",
            )
            .detail(out.combined())
        })
    }

    /// Deny the guest every route to the host's loopback interface.
    ///
    /// `10.0.2.2` is the emulator's IPv4 host gateway and `fec0::2` is the same
    /// gateway over IPv6; both are forwarded to the Windows loopback interface by
    /// the emulator's user-mode networking, so without a rule a guest app with
    /// Android's Internet permission can open the adb server's smart socket (port
    /// 5037) or any other local listener. Measured on this machine: an unblocked
    /// guest reached both, over both families.
    ///
    /// Changing netfilter rules needs root, and on Android that means restarting
    /// adbd (`adb root`). The rules live in the guest kernel, so the shell user is
    /// restored before returning: the block outlives the elevated adbd, which does
    /// not stay behind. DNS and the internet are untouched — only the two host
    /// gateway addresses are refused.
    pub async fn isolate_host(&self, serial: &str) -> HostIsolation {
        let mut state = HostIsolation::default();
        match self
            .run(&["-s", serial, "root"], Duration::from_secs(20))
            .await
        {
            Ok(out) if out.combined().contains("cannot run as root") => {
                // A production (Play) image refuses this. Say so instead of
                // reporting a containment that was never installed.
                state.detail = Some(out.combined().trim().to_string());
                return state;
            }
            Ok(_) => {}
            Err(e) => {
                state.detail = Some(format!("adb could not raise privileges: {e}"));
                return state;
            }
        }
        // `adb root` restarts adbd; the next command must wait for the transport
        // rather than racing the restart.
        let _ = self
            .run(&["-s", serial, "wait-for-device"], Duration::from_secs(30))
            .await;

        state.ipv4 = self.ensure_rule(serial, IPV4_RULE).await;
        state.ipv6 = self.ensure_rule(serial, IPV6_RULE).await;
        state.probe_blocked = self.host_probe(serial).await;

        let _ = self
            .run(&["-s", serial, "unroot"], Duration::from_secs(20))
            .await;
        let _ = self
            .run(&["-s", serial, "wait-for-device"], Duration::from_secs(30))
            .await;
        state
    }

    /// Install one egress rule idempotently, then ask netfilter whether it is
    /// there. `iptables -C` exits 0 only when the rule exists, so the answer is
    /// the kernel's, not the insert command's.
    async fn ensure_rule(&self, serial: &str, rule: (&str, &str)) -> bool {
        let (check, insert) = rule;
        let line = format!("{check} 2>/dev/null || {insert}");
        let _ = self
            .shell_line(serial, &line, Duration::from_secs(15))
            .await;
        matches!(
            self.shell_line(serial, check, Duration::from_secs(15)).await,
            Ok(out) if out.ok()
        )
    }

    /// Android's identity for this boot of the kernel. Netfilter rules live in
    /// that kernel, so this is what tells a cached containment from a stale one:
    /// a reboot of the guest silently discards the rules, and the status must not
    /// go on claiming them.
    pub async fn boot_id(&self, serial: &str) -> Option<String> {
        let out = self
            .shell_line(
                serial,
                "cat /proc/sys/kernel/random/boot_id",
                Duration::from_secs(10),
            )
            .await
            .ok()?;
        let id = out.stdout.trim().to_string();
        (!id.is_empty() && out.ok()).then_some(id)
    }

    /// Ask the adb server from inside the guest, the way a guest app would.
    ///
    /// `Some(true)` means the connection was refused, `Some(false)` means the
    /// smart socket answered, `None` means the guest could not be asked (no
    /// `nc`, a timeout). An unanswered probe is not evidence of containment, so
    /// the caller must not treat it as one.
    pub async fn host_probe(&self, serial: &str) -> Option<bool> {
        let out = self
            .shell_line(serial, HOST_PROBE, Duration::from_secs(10))
            .await
            .ok()?;
        classify_host_probe(&out)
    }

    pub async fn connect_ready(&self, serial: &str) -> bool {
        matches!(self.devices().await, Ok(devices) if devices.iter().any(|d| d.serial == serial && d.state == "device"))
    }

    /// vCPUs Android is actually running on. `nproc` answers it directly; older
    /// images may not ship toybox's `nproc`, so `/proc/cpuinfo` is the fallback.
    pub async fn cpu_count(&self, serial: &str) -> Option<u32> {
        if let Ok(out) = self
            .shell(serial, &["nproc"], Duration::from_secs(10))
            .await
        {
            if out.ok() {
                if let Ok(count) = out.stdout.trim().parse::<u32>() {
                    if count > 0 {
                        return Some(count);
                    }
                }
            }
        }
        let out = self
            .shell(serial, &["cat", "/proc/cpuinfo"], Duration::from_secs(10))
            .await
            .ok()?;
        let count = out
            .stdout
            .lines()
            .filter(|line| line.trim_start().starts_with("processor"))
            .count();
        (count > 0).then_some(count as u32)
    }

    /// Physical memory Android sees, in megabytes.
    pub async fn mem_total_mb(&self, serial: &str) -> Option<u32> {
        let out = self
            .shell(serial, &["cat", "/proc/meminfo"], Duration::from_secs(10))
            .await
            .ok()?;
        super::hardware::parse_android_meminfo(&out.stdout)
    }

    /// Total and available memory Android reports, in megabytes.
    pub async fn guest_memory(&self, serial: &str) -> Option<(u32, u32)> {
        let out = self
            .shell(serial, &["cat", "/proc/meminfo"], Duration::from_secs(10))
            .await
            .ok()?;
        super::hardware::parse_android_memory(&out.stdout)
    }

    /// The GLES renderer SurfaceFlinger reports. This is the only honest source
    /// for "is the GPU actually accelerated" — flags are not evidence.
    pub async fn gles_renderer(&self, serial: &str) -> Option<String> {
        let out = self
            .shell(
                serial,
                &["dumpsys", "SurfaceFlinger"],
                Duration::from_secs(20),
            )
            .await
            .ok()?;
        parse_gles_renderer(&out.stdout)
    }

    /// `allow_old_target` uses adb's documented testing flag for packages whose
    /// target SDK Android 14+ refuses. It is never applied silently: the caller
    /// has to have been told why.
    pub async fn install(
        &self,
        serial: &str,
        apk: &Path,
        allow_old_target: bool,
    ) -> DwResult<CmdOutput> {
        let path = apk.to_string_lossy().to_string();
        let mut args = vec!["-s", serial, "install", "-r"];
        if allow_old_target {
            args.push("--bypass-low-target-sdk-block");
        }
        args.push(path.as_str());
        self.run(&args, Duration::from_secs(600)).await
    }

    pub async fn uninstall(
        &self,
        serial: &str,
        package_id: &str,
        keep_data: bool,
    ) -> DwResult<CmdOutput> {
        // Modern Android refuses `adb uninstall -k` and points at the package
        // manager command instead, so keeping data takes its own path.
        let args: Vec<String> = uninstall_args(serial, package_id, keep_data);
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run(&borrowed, Duration::from_secs(120)).await
    }

    pub async fn package_path(&self, serial: &str, package_id: &str) -> Option<String> {
        let out = self
            .shell(serial, &["pm", "path", package_id], Duration::from_secs(15))
            .await
            .ok()?;
        out.stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix("package:").map(str::to_string))
    }

    /// Third-party packages, because system packages are not choices the user made.
    pub async fn third_party_packages(&self, serial: &str) -> DwResult<Vec<RemotePackage>> {
        let out = self
            .shell(
                serial,
                &["pm", "list", "packages", "-3", "--show-versioncode"],
                Duration::from_secs(30),
            )
            .await?;
        if !out.ok() {
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "The runtime did not return its package list.",
            )
            .detail(out.combined()));
        }
        let mut packages = Vec::new();
        for line in out.stdout.lines() {
            let line = line.trim();
            let Some(rest) = line.strip_prefix("package:") else {
                continue;
            };
            let mut parts = rest.split_whitespace();
            let Some(package_id) = parts.next() else {
                continue;
            };
            let version_code = parts
                .find_map(|p| p.strip_prefix("versionCode:"))
                .and_then(|v| v.parse().ok());
            packages.push(RemotePackage {
                package_id: package_id.to_string(),
                version_code,
            });
        }
        Ok(packages)
    }

    /// Packages Android still knows about, including ones whose data was kept
    /// after an uninstall (`pm uninstall -k`). This is how DroidWin checks a
    /// keep-data claim instead of repeating it.
    pub async fn packages_including_uninstalled(&self, serial: &str) -> DwResult<Vec<String>> {
        let out = self
            .shell(
                serial,
                &["pm", "list", "packages", "-u", "-3"],
                Duration::from_secs(30),
            )
            .await?;
        if !out.ok() {
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "The runtime did not return its retained-data list.",
            )
            .detail(out.combined()));
        }
        Ok(out
            .stdout
            .lines()
            .filter_map(|line| line.trim().strip_prefix("package:"))
            .map(str::to_string)
            .collect())
    }

    /// Whether the package currently has a process. Used after opening a window
    /// so "scrcpy started" is not mistaken for "Android started the app".
    pub async fn process_running(&self, serial: &str, package_id: &str) -> bool {
        !self.process_pids(serial, package_id).await.is_empty()
    }

    /// Every pid `pidof` reports for a package. Android restarts a crashed app
    /// under a new pid, so ``exists'' is not the same answer as ``the process we
    /// started is still there''.
    pub async fn process_pids(&self, serial: &str, package_id: &str) -> Vec<u32> {
        match self
            .shell(serial, &["pidof", package_id], Duration::from_secs(10))
            .await
        {
            Ok(out) => out
                .stdout
                .split_whitespace()
                .filter_map(|pid| pid.parse().ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    /// The last crash block Android recorded for a package.
    ///
    /// `pidof` only says a process exists; Android prints `Displayed …` even
    /// when the app's GL thread throws immediately afterwards. The crash buffer
    /// is separate from the main log, so this stays cheap and specific.
    pub async fn crash_excerpt(&self, serial: &str, package_id: &str) -> Option<String> {
        let out = self
            .shell(
                serial,
                &["logcat", "-b", "crash", "-d", "-t", "400"],
                Duration::from_secs(20),
            )
            .await
            .ok()?;
        crash_block(&out.stdout, package_id)
    }

    /// The last few log lines that mention a package, or the graphics stack it
    /// would fail in. Used as evidence when a running app draws nothing: the
    /// screenshot says it is blank, the log says why it might be.
    pub async fn log_excerpt(
        &self,
        serial: &str,
        package_id: &str,
        limit: usize,
    ) -> Option<String> {
        let out = self
            .shell(
                serial,
                &["logcat", "-d", "-t", "1500"],
                Duration::from_secs(20),
            )
            .await
            .ok()?;
        log_lines_for(&out.stdout, package_id, limit)
    }

    /// Re-register an installed package for the current user, keeping its data.
    /// This is Android's own targeted repair for a package the runtime can see
    /// but no longer starts; it never touches another package.
    pub async fn install_existing(&self, serial: &str, package_id: &str) -> DwResult<CmdOutput> {
        self.shell(
            serial,
            &["cmd", "package", "install-existing", package_id],
            Duration::from_secs(60),
        )
        .await
    }

    pub async fn force_stop(&self, serial: &str, package_id: &str) -> DwResult<CmdOutput> {
        self.shell(
            serial,
            &["am", "force-stop", package_id],
            Duration::from_secs(30),
        )
        .await
    }

    pub async fn launch_activity(&self, serial: &str, package_id: &str) -> DwResult<CmdOutput> {
        // Monkey resolves the launcher activity for us; no activity name guessing.
        self.shell(
            serial,
            &[
                "monkey",
                "-p",
                package_id,
                "-c",
                "android.intent.category.LAUNCHER",
                "1",
            ],
            Duration::from_secs(30),
        )
        .await
    }

    /// Ask the emulator console to shut down. The emulator runs as a child
    /// process (qemu), so killing the launcher alone does not stop it.
    pub async fn emu_kill(&self, serial: &str) -> DwResult<CmdOutput> {
        self.run(&["-s", serial, "emu", "kill"], Duration::from_secs(20))
            .await
    }

    pub async fn start(&self) -> DwResult<()> {
        let out = self.run(&["start-server"], Duration::from_secs(30)).await?;
        if out.ok() {
            Ok(())
        } else {
            Err(
                DwError::new(ErrorCode::RuntimeUnavailable, "adb server did not start.")
                    .detail(out.combined()),
            )
        }
    }
}

/// The log lines that are actually about a package: its own tag (`E pkg: …` or
/// `E/pkg(pid): …`) or the graphics stack it would fail in. A line that merely
/// mentions the package id in passing — a system component naming it — is noise,
/// and quoting it as the app's evidence would be misleading.
pub fn log_lines_for(text: &str, package_id: &str, limit: usize) -> Option<String> {
    // The tag has to be in the tag position, not merely mentioned somewhere in
    // the message: `E org.droidwin.blank: …` or `E/org.droidwin.blank( … ): …`.
    // WindowManager prints `ComponentInfo{org.droidwin.blank/…}` in its dumps,
    // and quoting one of those as the app's log would be wrong.
    let named = |line: &str| {
        line.contains(&format!(" {package_id}: "))
            || line.contains(&format!("/{package_id}("))
            || line.contains(&format!(" {package_id}("))
    };
    let graphics = |line: &str| {
        line.contains("eglChooseConfig")
            || line.contains("libEGL")
            || line.contains("OpenGLRenderer")
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    // The app's own lines win. Graphics warnings are a fallback only when the
    // app said nothing, because a failing driver usually explains a blank frame
    // but DroidWin's own dumpsys/screencap chatter must never be quoted as the
    // app's evidence.
    let mut named_hits: Vec<String> = lines
        .iter()
        .filter(|line| named(line))
        .map(|line| truncate(line, 280))
        .collect();
    let mut hits = if named_hits.is_empty() {
        lines
            .iter()
            .filter(|line| graphics(line))
            .map(|line| truncate(line, 280))
            .collect()
    } else {
        std::mem::take(&mut named_hits)
    };
    if hits.is_empty() {
        return None;
    }
    let start = hits.len().saturating_sub(limit);
    Some(hits.split_off(start).join("\n"))
}

fn truncate(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }
    let cut: String = trimmed.chars().take(limit).collect();
    format!("{cut}…")
}

/// `dumpsys display` → the uniqueId each logical display reports. That string
/// is the only stable bridge to a SurfaceFlinger token.
pub fn parse_display_unique_ids(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("mBaseDisplayInfo=DisplayInfo{") else {
            continue;
        };
        let Some(after_id) = rest.split_once("displayId ").map(|(_, tail)| tail) else {
            continue;
        };
        let digits: String = after_id.chars().take_while(char::is_ascii_digit).collect();
        let Ok(id) = digits.parse::<u32>() else {
            continue;
        };
        let Some(unique) = rest
            .split_once("uniqueId \"")
            .and_then(|(_, tail)| tail.split_once('"'))
            .map(|(value, _)| value.to_string())
        else {
            continue;
        };
        out.push((id, unique));
    }
    out
}

/// `dumpsys SurfaceFlinger --display-id` → (uniqueId, display token).
///
/// ```text
/// Display 4619827259835644672 (HWC display 0): port=0 pnpId=GGL displayName="EMU_display_0"
/// Display 11529215049505111569 (Virtual display): displayName="scrcpy" uniqueId="virtual:com.android.shell,2000,scrcpy,1"
/// ```
pub fn parse_sf_display_tokens(text: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("Display ") else {
            continue;
        };
        let Some(token) = rest
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u64>().ok())
        else {
            continue;
        };
        let Some(unique) = line
            .split_once("uniqueId=\"")
            .and_then(|(_, tail)| tail.split_once('"'))
            .map(|(value, _)| value.to_string())
        else {
            continue;
        };
        out.push((unique, token));
    }
    out
}

/// Quote every argument before joining a command for Android's `sh`. This is
/// important for package names read from untrusted APK manifests as well as
/// values supplied to the developer-control API.
fn device_shell_command(args: &[&str]) -> String {
    args.iter()
        .map(|arg| quote_for_device_shell(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Quote one argument for the device's `sh`. `adb shell` joins its arguments
/// with spaces and hands the result to that shell, so a value with a space, a
/// quote or a `$` must survive it untouched.
pub fn quote_for_device_shell(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "@%_+=:,./-".contains(c))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Read `Display: mDisplayId=N` blocks and the component each one focuses.
/// `mCurrentFocus` is preferred; `mFocusedApp` is the fallback because a display
/// that has not drawn yet reports `mCurrentFocus=null`.
pub fn parse_display_focus(text: &str) -> Vec<DisplayFocus> {
    fn component(value: &str) -> Option<(String, Option<String>)> {
        // `Window{9ab u0 pkg/act}` has the component as its last token, still
        // wrapped in the closing brace; a hash token has no slash at all.
        let token = value
            .split_whitespace()
            .find(|part| part.contains('/'))
            .map(|part| part.trim_matches(|c| c == '{' || c == '}'))?;
        let (package, activity) = token.split_once('/')?;
        if package.is_empty() {
            return None;
        }
        Some((
            package.to_string(),
            (!activity.is_empty()).then(|| activity.to_string()),
        ))
    }

    let mut displays: Vec<DisplayFocus> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("Display: mDisplayId=") {
            let id: String = rest.chars().take_while(char::is_ascii_digit).collect();
            if let Ok(id) = id.parse::<u32>() {
                displays.push(DisplayFocus {
                    display_id: id,
                    package_id: None,
                    activity: None,
                    width: None,
                    height: None,
                });
            }
            continue;
        }
        let Some(current) = displays.last_mut() else {
            continue;
        };
        if current.width.is_none() {
            // `cur=` is the geometry Android is composing right now, which is
            // what input coordinates are measured in. `init=` is only the size
            // the display was created with, so a display that rotated (a
            // landscape game on a portrait virtual display) would otherwise be
            // reported with the wrong width and height.
            let mut parts = trimmed.split_whitespace();
            let size = parts
                .find_map(|part| part.strip_prefix("cur="))
                .and_then(parse_size)
                .or_else(|| {
                    trimmed
                        .split_whitespace()
                        .find_map(|part| part.strip_prefix("init="))
                        .and_then(parse_size)
                });
            if let Some(size) = size {
                current.width = Some(size.0);
                current.height = Some(size.1);
            }
        }
        let focused = trimmed
            .strip_prefix("mCurrentFocus=")
            .filter(|v| *v != "null");
        if let Some(value) = focused {
            if let Some((package, activity)) = component(value) {
                current.package_id = Some(package);
                current.activity = activity;
            }
            continue;
        }
        if current.package_id.is_none() {
            if let Some(value) = trimmed.strip_prefix("mFocusedApp=") {
                if let Some((package, activity)) = component(value) {
                    current.package_id = Some(package);
                    current.activity = activity;
                }
            }
        }
    }
    displays
}

/// The display whose activity list contains `package_id`.
///
/// The activity dump groups tasks under `Display #N (activities from top to
/// bottom):` headings, so the display is the last heading seen before a line
/// naming the package. Top-to-bottom order means the first match is the display
/// the app is actually on.
pub fn parse_task_display(text: &str, package_id: &str) -> Option<u32> {
    let mut current: Option<u32> = None;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("Display #") {
            current = rest
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
                .parse()
                .ok()
                .or(current);
            continue;
        }
        let Some(display) = current else {
            continue;
        };
        // A task, activity record or window naming this package: `A=10216:pkg`,
        // `packageName=pkg`, or `pkg/Activity` inside a component.
        let names_package = trimmed.contains(&format!(":{package_id}"))
            || trimmed.contains(&format!("={package_id}"))
            || trimmed.contains(&format!("packageName={package_id}"))
            || trimmed.contains(&format!("{package_id}/"));
        if names_package {
            return Some(display);
        }
    }
    None
}

/// `540x1170` → (540, 1170).
fn parse_size(value: &str) -> Option<(u32, u32)> {
    let (width, height) = value.split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}

/// Pull the renderer out of SurfaceFlinger's `GLES:` line, e.g.
/// `GLES: Google (NVIDIA), Android Emulator ... (NVIDIA GeForce RTX 5060), OpenGL ES 3.1`.
pub fn parse_gles_renderer(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix("GLES:")?.trim();
        (!rest.is_empty()).then(|| rest.to_string())
    })
}

/// The last crash block for `package_id` in a `logcat -b crash` dump.
///
/// The buffer holds every process, so the block is matched by its
/// `Process: <package>` line; the header line is kept because it names the
/// thread and the exception, which is what a person needs to see.
pub fn crash_block(dump: &str, package_id: &str) -> Option<String> {
    let mut blocks: Vec<Vec<&str>> = Vec::new();
    for line in dump.lines() {
        if line.contains("FATAL EXCEPTION") {
            blocks.push(vec![line]);
        } else if let Some(block) = blocks.last_mut() {
            if block.len() < 12 {
                block.push(line);
            }
        }
    }
    blocks
        .into_iter()
        .rev()
        .find(|block| {
            block
                .iter()
                .any(|line| line.contains("Process:") && line.contains(package_id))
        })
        .map(|block| {
            block
                .into_iter()
                .take(8)
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_string()
        })
}

/// The crash signature of an app that refuses the renderer's EGL config set.
/// This is a renderer problem, not a package problem: switching the emulator to
/// SwiftShader is what makes these apps start.
pub fn is_egl_config_crash(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Logs spell the same call `eglChooseConfig`, `EGL_ChooseConfig` and
    // `egl_choose_config`; compare with the separators removed once.
    let compact: String = lower.chars().filter(|c| *c != '_').collect();
    text.contains("No config chosen")
        || compact.contains("eglchooseconfig")
        || lower.contains("failed to choose config")
        || lower.contains("no acceptable egl config")
}

/// Argument list for an uninstall. Split out so the keep-data path is testable
/// without a device.
pub fn uninstall_args(serial: &str, package_id: &str, keep_data: bool) -> Vec<String> {
    if keep_data {
        return vec![
            "-s".into(),
            serial.into(),
            "shell".into(),
            format!(
                "cmd package uninstall -k {}",
                quote_for_device_shell(package_id)
            ),
        ];
    }
    vec![
        "-s".into(),
        serial.into(),
        "uninstall".into(),
        package_id.into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(stdout: &str, stderr: &str) -> CmdOutput {
        CmdOutput {
            code: Some(0),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
        }
    }

    #[test]
    fn a_guest_probe_distinguishes_refused_from_answered_and_from_unknown() {
        // The adb smart socket answered: the guest is not contained.
        assert_eq!(classify_host_probe(&probe("OKAY0004", "")), Some(false));
        // The rule refused it, which is what containment looks like.
        assert_eq!(
            classify_host_probe(&probe("", "nc: connect: Connection refused")),
            Some(true)
        );
        // Silence is not evidence: a timeout or a missing `nc` must not be read
        // as a blocked guest.
        assert_eq!(classify_host_probe(&probe("", "nc: Timeout")), None);
        assert_eq!(
            classify_host_probe(&probe("", "/system/bin/sh: nc: inaccessible or not found")),
            None
        );
    }

    #[test]
    fn isolation_is_only_active_when_both_families_and_the_probe_agree() {
        let both = HostIsolation {
            ipv4: true,
            ipv6: true,
            probe_blocked: Some(true),
            detail: None,
        };
        assert!(both.active());
        // IPv4 alone leaves the measured IPv6 gateway open.
        assert!(!HostIsolation {
            ipv4: true,
            ipv6: false,
            probe_blocked: Some(true),
            detail: None
        }
        .active());
        // An unanswered probe is not a contained guest.
        assert!(!HostIsolation {
            ipv4: true,
            ipv6: true,
            probe_blocked: None,
            detail: None
        }
        .active());
        // A probe that connected means the rules are not doing their job.
        assert!(!HostIsolation {
            ipv4: true,
            ipv6: true,
            probe_blocked: Some(false),
            detail: None
        }
        .active());
    }

    #[test]
    fn an_app_windows_key_is_not_scoped_to_a_display_android_ignores() {
        let args = vec!["keyevent".to_string(), "KEYCODE_SPACE".to_string()];
        // Android drops a key injected for a display that is not the focused one.
        assert_eq!(input_line(4, &args), "input keyevent KEYCODE_SPACE");
        // The phone's own display keeps its scope: a phone key never types into
        // an app window by accident.
        assert_eq!(
            input_line(super::super::display::DEVICE_DISPLAY, &args),
            "input -d 0 keyevent KEYCODE_SPACE"
        );
        // Motion is display-scoped everywhere: a tap has to land on the app's
        // own display, which Android honours.
        let tap = vec!["tap".to_string(), "540".to_string(), "1200".to_string()];
        assert_eq!(input_line(4, &tap), "input -d 4 tap 540 1200");
        let hold = vec![
            "motionevent".to_string(),
            "DOWN".to_string(),
            "10".to_string(),
            "20".to_string(),
        ];
        assert_eq!(input_line(4, &hold), "input -d 4 motionevent DOWN 10 20");
    }

    #[test]
    fn a_crash_block_is_matched_by_its_process_line() {
        let dump = "\
--------- beginning of crash
10-26 08:53:52.505 11612 11997 E AndroidRuntime: FATAL EXCEPTION: GLThread 101
10-26 08:53:52.505 11612 11997 E AndroidRuntime: Process: com.other.app, PID: 11612
10-26 08:53:52.505 11612 11997 E AndroidRuntime: java.lang.IllegalStateException: other
10-26 08:53:53.000 12332 12384 E AndroidRuntime: FATAL EXCEPTION: GLThread 91
10-26 08:53:53.000 12332 12384 E AndroidRuntime: Process: com.example.game, PID: 12332
10-26 08:53:53.000 12332 12384 E AndroidRuntime: java.lang.IllegalArgumentException: No config chosen
10-26 08:53:53.000 12332 12384 E AndroidRuntime: \tat com.example.Game$Chooser.chooseConfig(Game.java:884)";
        let block = crash_block(dump, "com.example.game").expect("block");
        assert!(block.contains("GLThread 91"));
        assert!(block.contains("No config chosen"));
        assert!(!block.contains("com.other.app"));
        assert!(crash_block(dump, "com.absent.app").is_none());
    }

    #[test]
    fn a_log_excerpt_quotes_the_app_and_not_system_chatter_about_it() {
        let text = "\
10-26 18:40:16.256   743   967 W PackageConfigPersister: App-specific configuration not found for packageName: org.droidwin.blank and userId: 0\n\
10-26 18:40:16.300   743   967 E org.droidwin.blank: blank surface drawn; no content view\n\
10-26 18:40:16.400   752  2230 V WindowManager: Transition for ComponentInfo{org.droidwin.blank/org.droidwin.blank.BlankActivity} pkg=org.droidwin.blank\n\
10-26 18:40:17.000   900   900 W libEGL: eglChooseConfig failed\n\
10-26 18:40:17.100   900   900 I chatty: unrelated";
        let excerpt = log_lines_for(text, "org.droidwin.blank", 6).unwrap();
        assert!(excerpt.contains("blank surface drawn"));
        // The app spoke, so the graphics warning is not what gets quoted.
        assert!(!excerpt.contains("eglChooseConfig"));
        assert!(!excerpt.contains("PackageConfigPersister"));
        assert!(!excerpt.contains("WindowManager"));
        assert!(!excerpt.contains("unrelated"));

        // With no app line, the graphics stack is the best available evidence.
        let silent_app = "10-26 18:40:17.000   900   900 W libEGL: eglChooseConfig failed";
        assert!(log_lines_for(silent_app, "org.droidwin.blank", 6)
            .unwrap()
            .contains("eglChooseConfig"));

        // Format with the tag in parentheses, as some logcat versions print it.
        let bracketed = "10-26 18:40:16.300  123  456 E/org.droidwin.blank( 123): drawn";
        assert!(log_lines_for(bracketed, "org.droidwin.blank", 6).is_some());
        assert!(log_lines_for("nothing here", "org.droidwin.blank", 6).is_none());
    }

    #[test]
    fn the_egl_config_signature_is_recognised() {
        assert!(is_egl_config_crash(
            "java.lang.IllegalArgumentException: No config chosen"
        ));
        assert!(is_egl_config_crash("EGL_ChooseConfig failed"));
        assert!(!is_egl_config_crash(
            "java.lang.IllegalStateException: Not attached"
        ));
    }

    #[test]
    fn emulator_console_reports_the_avd_name_without_its_status_line() {
        assert_eq!(
            parse_emulator_avd_name("DroidWin_API36\r\nOK\r\n").as_deref(),
            Some("DroidWin_API36")
        );
        assert_eq!(parse_emulator_avd_name("OK\r\n"), None);
    }

    #[test]
    fn one_prop_dump_reports_a_booted_device_and_its_api() {
        let dump = "[ro.build.version.sdk]: [36]\n[sys.boot_completed]: [1]\n[ro.product.model]: [sdk_gphone64_x86_64]\n";
        assert_eq!(
            parse_props(dump),
            Readiness {
                booted: true,
                api: Some(36)
            }
        );
    }

    #[test]
    fn a_booting_device_is_present_but_not_ready() {
        let dump = "[ro.build.version.sdk]: [36]\n[sys.boot_completed]: [0]\n";
        assert_eq!(
            parse_props(dump),
            Readiness {
                booted: false,
                api: Some(36)
            }
        );
    }

    #[test]
    fn an_empty_or_unexpected_dump_is_not_ready() {
        assert!(!parse_props("").booted);
        assert!(!parse_props("error: device 'emulator-5554' not found").booted);
        assert_eq!(parse_props("[sys.boot_completed]: [1]").api, None);
    }

    #[tokio::test]
    async fn missing_binary_reports_typed_error() {
        let adb = Adb::new(PathBuf::from("Z:/nope/adb.exe"));
        let err = adb
            .run(&["devices"], Duration::from_secs(2))
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::RuntimeUnavailable);
    }

    #[test]
    fn the_gles_line_is_read_without_the_prefix() {
        let dump = "---- GLES ----\nGLES: Google (NVIDIA), Android Emulator OpenGL ES Translator (NVIDIA GeForce RTX 5060), OpenGL ES 3.1\n";
        let renderer = parse_gles_renderer(dump).unwrap();
        assert!(renderer.contains("NVIDIA GeForce RTX 5060"));
        assert!(!renderer.starts_with("GLES:"));
    }

    #[test]
    fn a_dump_without_a_gles_line_has_no_renderer() {
        assert_eq!(parse_gles_renderer("no surface data"), None);
    }

    #[test]
    fn a_permission_dialog_on_top_still_names_the_display_hosting_the_app() {
        // What the first launch of an app on a fresh image looks like: the
        // virtual display's focus is the permission controller's dialog, while
        // the app's own task sits on that same display.
        let dump = "ACTIVITY MANAGER ACTIVITIES (dumpsys activity activities)\nDisplay #5 (activities from top to bottom):\n  * Task{6d2a1e5 #11 type=standard A=10216:org.fdroid.fdroid U=0 visible=true sz=2}\n    topResumedActivity=ActivityRecord{227 u0 com.google.android.permissioncontroller/com.android.permissioncontroller.permission.ui.GrantPermissionsActivity t11}\nDisplay #0 (activities from top to bottom):\n  * Task{cead91f #1 type=home U=0 visible=true sz=1}\n";
        assert_eq!(parse_task_display(dump, "org.fdroid.fdroid"), Some(5));
        assert_eq!(parse_task_display(dump, "com.example.missing"), None);
    }

    #[test]
    fn every_display_reports_the_component_android_focused_on_it() {
        let dump = "WINDOW MANAGER DISPLAY CONTENTS (dumpsys window displays)\n  Display: mDisplayId=0 rootTaskId=1\n    init=1080x2340 420dpi cur=1080x2340\n    mCurrentFocus=Window{3c0f9e5 u0 com.android.launcher3/com.android.launcher3.uioverrides.QuickstepLauncher}\n    mFocusedApp=ActivityRecord{e2ca3a1 u0 com.android.launcher3/.uioverrides.QuickstepLauncher t4}\n  Display: mDisplayId=2\n    init=540x1170 420dpi cur=540x1170\n    mCurrentFocus=Window{9ab u0 org.fdroid.fdroid/org.fdroid.fdroid.views.main.MainActivity}\n";
        let displays = parse_display_focus(dump);
        assert_eq!(displays.len(), 2);
        assert_eq!(displays[0].display_id, 0);
        assert_eq!(
            displays[0].package_id.as_deref(),
            Some("com.android.launcher3")
        );
        assert_eq!(displays[1].display_id, 2);
        assert_eq!(displays[1].package_id.as_deref(), Some("org.fdroid.fdroid"));
        assert_eq!(
            displays[1].activity.as_deref(),
            Some("org.fdroid.fdroid.views.main.MainActivity")
        );
        assert_eq!(displays[1].width, Some(540));
        assert_eq!(displays[1].height, Some(1170));
    }

    #[test]
    fn a_rotated_display_reports_the_geometry_it_is_composing_now() {
        // The game's virtual display was created 540x1170 and Android rotated it
        // to landscape. Input is measured in the rotated frame, so reporting the
        // creation size would put every tap in the wrong place.
        let dump = "  Display: mDisplayId=7 (organized)\n    init=540x1170 420dpi mMinSizeOfResizeableTaskDp=220 cur=1170x540 app=1170x540 rng=540x540-1170x1170\n    mCurrentFocus=Window{1 u0 com.fingersoft.hillclimb/.UnityPlayerActivity}\n";
        let displays = parse_display_focus(dump);
        assert_eq!(displays[0].width, Some(1170));
        assert_eq!(displays[0].height, Some(540));
    }

    #[test]
    fn a_dump_without_cur_falls_back_to_the_initial_size() {
        let dump = "  Display: mDisplayId=5\n    init=540x1170 420dpi\n";
        let displays = parse_display_focus(dump);
        assert_eq!(displays[0].width, Some(540));
        assert_eq!(displays[0].height, Some(1170));
    }

    #[test]
    fn a_display_without_focus_falls_back_to_its_focused_app() {
        let dump = "  Display: mDisplayId=0\n    mCurrentFocus=null\n    mFocusedApp=ActivityRecord{aa u0 com.example/.Main t1}\n";
        let displays = parse_display_focus(dump);
        assert_eq!(displays[0].package_id.as_deref(), Some("com.example"));
        assert_eq!(displays[0].activity.as_deref(), Some(".Main"));
    }

    #[test]
    fn an_unknown_dump_yields_no_displays_instead_of_a_guess() {
        assert!(parse_display_focus("error: device not found").is_empty());
    }

    #[test]
    fn a_logical_display_is_bridged_to_its_surfaceflinger_token() {
        let display = "  mBaseDisplayInfo=DisplayInfo{\"Built-in Screen\", displayId 0, displayGroupId 0, uniqueId \"local:4619827259835644672\", app 1080 x 2400, density 420}\n  mBaseDisplayInfo=DisplayInfo{\"scrcpy\", displayId 3, displayGroupId 1, uniqueId \"virtual:com.android.shell,2000,scrcpy,1\", app 540 x 1170, density 420}\n";
        let ids = parse_display_unique_ids(display);
        assert_eq!(ids.len(), 2);
        assert_eq!(
            ids[1],
            (3, "virtual:com.android.shell,2000,scrcpy,1".to_string())
        );

        let surface = "Display 4619827259835644672 (HWC display 0): port=0 pnpId=GGL displayName=\"EMU_display_0\"\nDisplay 11529215049505111569 (Virtual display): displayName=\"scrcpy\" uniqueId=\"virtual:com.android.shell,2000,scrcpy,1\"\n";
        let tokens = parse_sf_display_tokens(surface);
        // The physical display line carries no uniqueId, so only the virtual
        // display is addressable by token — which is the one app windows use.
        assert_eq!(tokens.len(), 1);
        let found = tokens
            .iter()
            .find(|(unique, _)| unique == "virtual:com.android.shell,2000,scrcpy,1")
            .unwrap();
        assert_eq!(found.1, 11529215049505111569);
    }

    #[test]
    fn an_unrelated_dump_yields_no_display_mapping() {
        assert!(parse_display_unique_ids("no displays").is_empty());
        assert!(parse_sf_display_tokens("no surface data").is_empty());
    }

    #[test]
    fn device_shell_quoting_survives_spaces_quotes_and_dollars() {
        assert_eq!(quote_for_device_shell("plain"), "plain");
        assert_eq!(quote_for_device_shell("hello world"), "'hello world'");
        assert_eq!(quote_for_device_shell("a'b"), "'a'\\''b'");
        assert_eq!(quote_for_device_shell("$HOME"), "'$HOME'");
        assert_eq!(quote_for_device_shell(""), "''");
    }

    #[test]
    fn device_shell_command_quotes_untrusted_package_arguments() {
        assert_eq!(
            device_shell_command(&["pm", "path", "com.example.app; id"]),
            "pm path 'com.example.app; id'"
        );
        assert_eq!(
            device_shell_command(&["monkey", "-p", "com.example' && id #"]),
            "monkey -p 'com.example'\\'' && id #'"
        );
    }

    #[test]
    fn keeping_data_uses_the_package_manager_command() {
        let args = uninstall_args("emulator-5554", "org.fdroid.fdroid", true);
        assert_eq!(
            args,
            vec![
                "-s",
                "emulator-5554",
                "shell",
                "cmd package uninstall -k org.fdroid.fdroid"
            ]
        );
        let malicious = uninstall_args("emulator-5554", "com.example; id", true);
        assert_eq!(
            malicious.last().map(String::as_str),
            Some("cmd package uninstall -k 'com.example; id'")
        );
    }

    #[test]
    fn deleting_data_uses_a_plain_uninstall() {
        let args = uninstall_args("emulator-5554", "org.fdroid.fdroid", false);
        assert_eq!(
            args,
            vec!["-s", "emulator-5554", "uninstall", "org.fdroid.fdroid"]
        );
    }

    #[tokio::test]
    async fn timeout_is_reported_as_timeout() {
        let adb = Adb::new(PathBuf::from("cmd.exe"));
        // `cmd /c ping` is unavailable through this API; use a command that waits.
        let out = adb
            .run(&["/c", "ping -n 6 127.0.0.1"], Duration::from_millis(300))
            .await;
        assert_eq!(out.unwrap_err().code, ErrorCode::Timeout);
    }
}
