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
        let mut args = vec!["-s", serial, "shell"];
        args.extend_from_slice(command);
        self.run(&args, timeout).await
    }

    pub async fn devices(&self) -> DwResult<Vec<Device>> {
        let out = self.run(&["devices"], Duration::from_secs(15)).await?;
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

    pub async fn connect_ready(&self, serial: &str) -> bool {
        matches!(self.devices().await, Ok(devices) if devices.iter().any(|d| d.serial == serial && d.state == "device"))
    }

    pub async fn boot_completed(&self, serial: &str) -> bool {
        let Ok(out) = self
            .shell(
                serial,
                &["getprop", "sys.boot_completed"],
                Duration::from_secs(10),
            )
            .await
        else {
            return false;
        };
        out.stdout.trim() == "1"
    }

    pub async fn api_level(&self, serial: &str) -> Option<u32> {
        let out = self
            .shell(
                serial,
                &["getprop", "ro.build.version.sdk"],
                Duration::from_secs(10),
            )
            .await
            .ok()?;
        out.stdout.trim().parse().ok()
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
        match self
            .shell(serial, &["pidof", package_id], Duration::from_secs(10))
            .await
        {
            Ok(out) => !out.stdout.trim().is_empty(),
            Err(_) => false,
        }
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

/// Argument list for an uninstall. Split out so the keep-data path is testable
/// without a device.
pub fn uninstall_args(serial: &str, package_id: &str, keep_data: bool) -> Vec<String> {
    let mut args: Vec<String> = if keep_data {
        vec![
            "-s".into(),
            serial.into(),
            "shell".into(),
            "cmd".into(),
            "package".into(),
            "uninstall".into(),
            "-k".into(),
        ]
    } else {
        vec!["-s".into(), serial.into(), "uninstall".into()]
    };
    args.push(package_id.to_string());
    args
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn keeping_data_uses_the_package_manager_command() {
        let args = uninstall_args("emulator-5554", "org.fdroid.fdroid", true);
        assert_eq!(
            args,
            vec![
                "-s",
                "emulator-5554",
                "shell",
                "cmd",
                "package",
                "uninstall",
                "-k",
                "org.fdroid.fdroid"
            ]
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
