//! The Android Emulator + scrcpy adapter.
//!
//! One headless emulator hosts every app; each launched app gets its own scrcpy
//! window on its own virtual display. The emulator is either one already on the
//! machine or the one DroidWin installed for itself; it never ships inside the
//! installer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::process::Child;

use super::adb::{Adb, HostIsolation};
use super::display::{DisplayGeometry, Orientation};
use super::emulator::{self, AvdConfig};
use super::hardware::{self, GpuMode, HardwareReport, PlanMode};
use super::scrcpy::{self, KeyboardMode, WindowRequest};
use super::sdk::{self, Sdk};
use super::{InstallOutcome, RuntimeEvents};
use crate::domain::{
    now_ms, ApkInspection, AppOrientation, AppWindow, Capabilities, DwError, DwResult, EnvReport,
    ErrorCode, InstalledApp, RuntimeState, RuntimeStatus, Stage, StageEvent,
};
use crate::store::{CachedApp, Store};

// The runtime installer owns these names: the AVD and image it creates are the
// ones this runtime starts, and two spellings of them would be one bug too many.
pub use super::install::{AVD_NAME, SYSTEM_IMAGE};
const EMULATOR_PORT: u16 = 5554;
const BOOT_BUDGET: Duration = Duration::from_secs(420);
// Display geometry is not set here: each app gets the surface its own
// orientation calls for, from one policy in `super::display`.
/// Environment discovery launches the emulator once to read its acceleration
/// report. That answer only changes when the SDK does, so it is cached.
const ENV_TTL: Duration = Duration::from_secs(30);
/// How long a booted/not-booted answer may be reused. The status poll and a
/// launch/install can ask within the same second; without this, each question
/// cost its own `adb` process.
const READINESS_TTL: Duration = Duration::from_millis(2_000);
/// What Android reports about its own hardware (cores, RAM, renderer) only
/// changes when the runtime restarts, so the settings and performance views can
/// reuse one answer for a while instead of spawning four adb calls per poll.
const EFFECTIVE_TTL: Duration = Duration::from_secs(20);

struct WindowEntry {
    child: Arc<tokio::sync::Mutex<Child>>,
    title: String,
    pid: u32,
    /// The geometry this window's Android display was created with. Recorded here
    /// so it is answerable for exactly as long as the window exists.
    geometry: DisplayGeometry,
    /// The Android display scrcpy created for this window. Input and screenshots
    /// are display-scoped, so an unknown display (0) means "not resolved yet".
    display: std::sync::atomic::AtomicU32,
    /// What Android says it is composing, packed as `width << 32 | height`. Zero
    /// until it answers. This is the coordinate space input is measured in, which
    /// can differ from the size the display was created with (a rotation).
    composed: std::sync::atomic::AtomicU64,
    /// Set before DroidWin kills the window host, so its non-zero exit is
    /// reported as a close and not as the app crashing. Shared with the watcher,
    /// which outlives the map entry that owns it.
    closing: Arc<std::sync::atomic::AtomicBool>,
}

/// What an exited window means.
///
/// The classification is the event detector's whole basis, so it is one pure
/// function: a window DroidWin closed is a close even though its killed host
/// returns a failure code, and a host that died on its own with a failure code
/// is a crash.
/// The executables the emulator runs under: the launcher, and the VM it spawns.
pub fn is_emulator_process(image: &str) -> bool {
    let name = image.to_ascii_lowercase();
    name.contains("emulator") || name.contains("qemu-system")
}

/// Whether a recorded pid is still the emulator. A recycled pid belongs to
/// something else, and killing that would be a bug of its own.
fn is_emulator_pid(pid: u32) -> bool {
    crate::proc::image_name(pid)
        .map(|name| is_emulator_process(&name))
        .unwrap_or(false)
}

pub fn classify_exit(
    package_id: &str,
    closing: bool,
    exit_code: Option<i32>,
    app_gone: bool,
) -> super::AppEvent {
    if closing {
        return super::AppEvent::new(
            package_id,
            super::AppEventKind::Closed,
            "its window was closed in DroidWin",
        )
        .exited(exit_code);
    }
    if app_gone {
        return super::AppEvent::new(
            package_id,
            super::AppEventKind::Closed,
            "its process disappeared while its window was open",
        )
        .exited(exit_code);
    }
    match exit_code {
        Some(code) if code != 0 => super::AppEvent::new(
            package_id,
            super::AppEventKind::Crashed,
            format!("its window host exited with code {code}"),
        )
        .exited(Some(code)),
        code => super::AppEvent::new(package_id, super::AppEventKind::Closed, "its window closed")
            .exited(code),
    }
}

/// A window as the control layer sees it: which package, which window process,
/// and which Android display its pixels and input belong to.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OpenWindow {
    pub package_id: String,
    pub title: String,
    pub pid: u32,
    pub display_id: Option<u32>,
    /// The surface this window is drawn on, for coordinate conversion.
    pub geometry: Option<DisplayGeometry>,
}

/// One app window's surface as DroidWin created it.
///
/// Everything that maps a point in a window to a moment on the Android screen —
/// keyboard-to-touch mappings, MCP taps, screenshot sizing — needs exactly these
/// numbers, so they are read from here instead of being repeated as constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct AppSurface {
    /// Android's display for this window, once it has reported one.
    pub display_id: Option<u32>,
    pub geometry: DisplayGeometry,
    /// What Android reported composing, when that differs from what was asked.
    pub composed: Option<(u32, u32)>,
}

impl AppSurface {
    /// The pixel size input and screenshots are measured in: the composed size
    /// when Android reported one, otherwise the size the display was created with.
    pub fn screen_size(&self) -> (u32, u32) {
        self.composed
            .unwrap_or((self.geometry.width, self.geometry.height))
    }
}

/// Android's install failures mean very different things to a user. Keep the
/// mapping here, next to the command that produces the text, and test it.
pub fn install_failure_summary(text: &str) -> &'static str {
    if text.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
        "The installed app has a different signature; Android will not let this package replace it."
    } else if text.contains("INSTALL_FAILED_INSUFFICIENT_STORAGE") {
        "The runtime has run out of storage for this package."
    } else if text.contains("INSTALL_FAILED_DEPRECATED_SDK_VERSION") {
        "This package targets an Android version old enough that the runtime refuses it (target SDK below 23)."
    } else if text.contains("INSTALL_FAILED_OLDER_SDK") {
        "This package requires a newer Android version than the runtime provides."
    } else if text.contains("INSTALL_FAILED_NO_MATCHING_ABIS") {
        "This package has no native code for this runtime's CPU architecture."
    } else if text.contains("INSTALL_PARSE_FAILED") {
        "Android could not parse this package."
    } else if text.contains("INSTALL_FAILED_ALREADY_EXISTS") {
        "Android reports a conflicting copy of this package in the runtime."
    } else {
        "Android rejected the installation."
    }
}

fn pack_size(width: u32, height: u32) -> u64 {
    ((width as u64) << 32) | height as u64
}

fn unpack_size(packed: u64) -> Option<(u32, u32)> {
    (packed != 0).then_some(((packed >> 32) as u32, packed as u32))
}

/// The status sentence for a containment state, split out so every branch —
/// installed and observed, installed but unprobed, not installed, not attempted —
/// can be read and tested without a device. An unanswered probe is never
/// reported as containment.
pub fn isolation_note(state: Option<&HostIsolation>) -> String {
    match state {
        Some(state) if state.active() => "Apps cannot reach Windows services listening on loopback: egress from the guest to the emulator's host gateway (10.0.2.2 and its IPv6 twin fec0::2) is refused by rules installed when the runtime booted, and the refusal was measured from inside Android. DNS and the internet still work. The guest can still reach the host by its LAN address, so untrusted packages belong in a separate Windows account or VM.".to_string(),
        Some(state) => {
            let why = if !state.ipv4 || !state.ipv6 {
                state.detail.clone().unwrap_or_else(|| {
                    "the egress rules could not be installed; a production image that refuses `adb root` cannot be contained this way"
                        .to_string()
                })
            } else if state.probe_blocked == Some(false) {
                "the rules are installed, but a real connection from the guest still reached the adb port".to_string()
            } else {
                "the rules are installed, but the guest could not be probed, so containment is unverified".to_string()
            };
            format!(
                "Guest access to Windows loopback is NOT contained: {why}. Do not run untrusted packages without a separate Windows account or a VM."
            )
        }
        None => "An app with Android's Internet permission can reach Windows services listening on loopback through 10.0.2.2. Measured here: the adb server on port 5037 answered the guest's smart-socket requests. DroidWin denies that at boot, but the runtime has not booted in this session, so nothing is installed yet. Do not run untrusted packages without a separate Windows account or a VM.".to_string(),
    }
}

pub struct EmulatorRuntime {
    data_dir: PathBuf,
    events: Arc<dyn RuntimeEvents>,
    emulator_child: tokio::sync::Mutex<Option<Child>>,
    windows: Arc<Mutex<HashMap<String, WindowEntry>>>,
    /// Collapses concurrent "make the runtime ready" requests into one boot.
    /// Without it, a start request racing an install could boot two emulators
    /// against the same port.
    boot: tokio::sync::Mutex<()>,
    /// Serialises the check-then-spawn of a window so a double click cannot
    /// produce two scrcpy processes for the same package.
    launch: tokio::sync::Mutex<()>,
    /// One adb install at a time. Two packages dropped together would otherwise
    /// race the package manager and make each other's verify step unreliable.
    install_gate: tokio::sync::Mutex<()>,
    env_cache: Mutex<Option<(Instant, EnvReport)>>,
    /// Last answer to "is the runtime booted, and at which API level".
    readiness_cache: Mutex<Option<(Instant, Option<u32>)>>,
    effective_cache: Mutex<Option<(Instant, hardware::EffectiveHardware)>>,
    /// What the guest was denied, and which boot of Android's kernel it was
    /// denied under. `None` until a boot attempt has run: the status text must not
    /// claim containment that has not been installed, and `Some` with
    /// `active() == false` must say why rather than fall back to a reassuring
    /// sentence. The boot id is what keeps a guest reboot from leaving a stale
    /// claim behind — the rules go with the kernel that held them.
    isolation: Mutex<Option<(String, HostIsolation)>>,
    /// A pid written by an earlier DroidWin run that started this emulator and
    /// exited without stopping it. Together with `emulator_child` this is the
    /// whole ownership answer: one of the two is set only when DroidWin started
    /// the emulator, and both stay empty for one the user started.
    owned_pid: Mutex<Option<u32>>,
    /// An emulator a previous run started that is still alive, but whose pid is
    /// gone (`emulator.exe` exits once it has spawned the VM) or has been
    /// recycled. Claimed only after adb confirms the AVD on our port is
    /// DroidWin's own, and stopped through `adb emu kill` rather than by pid.
    owned_orphan: std::sync::atomic::AtomicBool,
    /// Whether closing DroidWin stops an emulator DroidWin started. The user
    /// chooses this on the Runtime screen; the default is to clean up.
    stop_on_exit: std::sync::atomic::AtomicBool,
    /// Closing runs the stop twice — once from the manager window's close, once
    /// from the process `Exit` event — and a double click on the title bar can
    /// ask a third time. The gate makes the later calls wait for the first and
    /// then find nothing left to do, instead of exiting the process while the
    /// emulator is still going down.
    shutdown_gate: tokio::sync::Mutex<()>,
    stopped: std::sync::atomic::AtomicBool,
}

impl EmulatorRuntime {
    pub fn new(data_dir: PathBuf, events: Arc<dyn RuntimeEvents>) -> Self {
        Self {
            data_dir,
            events,
            emulator_child: tokio::sync::Mutex::new(None),
            windows: Arc::new(Mutex::new(HashMap::new())),
            boot: tokio::sync::Mutex::new(()),
            launch: tokio::sync::Mutex::new(()),
            install_gate: tokio::sync::Mutex::new(()),
            env_cache: Mutex::new(None),
            readiness_cache: Mutex::new(None),
            effective_cache: Mutex::new(None),
            isolation: Mutex::new(None),
            owned_pid: Mutex::new(None),
            owned_orphan: std::sync::atomic::AtomicBool::new(false),
            stop_on_exit: std::sync::atomic::AtomicBool::new(true),
            shutdown_gate: tokio::sync::Mutex::new(()),
            stopped: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// The file that makes ownership survive a crash: written when DroidWin
    /// starts the emulator, removed when it stops it.
    fn ownership_file(&self) -> PathBuf {
        self.data_dir.join("runtime").join("emulator.pid")
    }

    fn record_ownership(&self, pid: Option<u32>) {
        if let Some(pid) = pid {
            if let Some(parent) = self.ownership_file().parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(self.ownership_file(), pid.to_string());
            if let Ok(mut slot) = self.owned_pid.lock() {
                *slot = Some(pid);
            }
        }
    }

    /// Claim an emulator this install started before a previous exit.
    ///
    /// The caller has already verified through adb that the device on DroidWin's
    /// port is DroidWin's own AVD, so this only decides *how* to stop it later: by
    /// pid when that pid is still an emulator process, otherwise through the
    /// emulator console. `emulator.exe` exits after spawning
    /// `qemu-system-x86_64-headless`, so a dead launcher pid is the normal case
    /// after a crash, not evidence that the emulator is somebody else's.
    fn adopt_ownership(&self) {
        if self
            .emulator_child
            .try_lock()
            .map(|child| child.is_some())
            .unwrap_or(false)
        {
            return;
        }
        let Ok(text) = std::fs::read_to_string(self.ownership_file()) else {
            return;
        };
        let pid = text.trim().parse::<u32>().ok();
        let alive = pid.filter(|pid| {
            crate::proc::image_name(*pid)
                .map(|name| is_emulator_process(&name))
                .unwrap_or(false)
        });
        match alive {
            Some(pid) => {
                tracing::info!("adopted the emulator DroidWin started earlier (pid {pid})");
                if let Ok(mut slot) = self.owned_pid.lock() {
                    *slot = Some(pid);
                }
            }
            None => {
                tracing::info!(
                    "adopted the emulator DroidWin started earlier (pid no longer live)"
                );
                self.owned_orphan
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }

    pub fn stop_on_exit(&self) -> bool {
        self.stop_on_exit.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub fn set_stop_on_exit(&self, enabled: bool) {
        self.stop_on_exit
            .store(enabled, std::sync::atomic::Ordering::SeqCst);
    }

    fn config() -> AvdConfig {
        AvdConfig {
            name: AVD_NAME.to_string(),
            system_image: SYSTEM_IMAGE.to_string(),
            port: EMULATOR_PORT,
        }
    }

    pub fn adb(&self) -> Option<Adb> {
        Sdk::locate(None).map(|sdk| Adb::new(sdk.adb()))
    }

    /// The serial every operation is scoped to. Exposed so the control layer
    /// narrates the same device the runtime is driving.
    pub fn serial(&self) -> String {
        Self::config().serial()
    }

    /// The surface a running app window was created on, together with the Android
    /// display it resolved to. This is the single source of truth for turning a
    /// window coordinate into a display coordinate.
    pub fn geometry_of(&self, package_id: &str) -> Option<AppSurface> {
        self.windows.lock().ok().and_then(|windows| {
            windows.get(package_id).map(|entry| AppSurface {
                display_id: Some(entry.display.load(std::sync::atomic::Ordering::Relaxed))
                    .filter(|id| *id != 0),
                geometry: entry.geometry,
                composed: unpack_size(entry.composed.load(std::sync::atomic::Ordering::Relaxed)),
            })
        })
    }

    /// The package whose window process is `pid`, when it is one of ours. The
    /// keyboard hook sees a foreground process id, not a window title.
    pub fn package_for_pid(&self, pid: u32) -> Option<String> {
        self.windows.lock().ok().and_then(|windows| {
            windows
                .iter()
                .find(|(_, entry)| entry.pid == pid)
                .map(|(package_id, _)| package_id.clone())
        })
    }

    /// One still of the device's own screen, with its pixel size.
    ///
    /// This is the virtual phone: Android's real launcher on the device's own
    /// display, captured through the same adb and the same running emulator every
    /// app window uses. Nothing here starts a second emulator, and nothing here
    /// decides how often to look — the caller owns that.
    pub async fn device_screen(&self) -> DwResult<(Vec<u8>, u32, u32)> {
        let adb = self.adb().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK is not configured.",
            )
            .detail("Point DroidWin at an SDK with platform-tools before opening the phone.")
        })?;
        let png = adb
            .screenshot_png(&self.serial(), super::display::DEVICE_DISPLAY)
            .await?;
        // A capture whose header cannot be read is not shown at a guessed size.
        let (width, height) = super::display::png_size(&png).ok_or_else(|| {
            DwError::new(
                ErrorCode::BackendFailure,
                "Android returned a capture with no readable size.",
            )
        })?;
        Ok((png, width, height))
    }

    /// Tap one point on the device's own display, in its own pixels.
    ///
    /// The virtual phone forwards what the user clicked, so the coordinate is
    /// already resolved from the capture's size — no percentage and no geometry
    /// lookup is involved, unlike an app window's mapped touch target.
    pub async fn device_tap(&self, x: u32, y: u32) -> DwResult<()> {
        self.device_input(vec!["tap".into(), x.to_string(), y.to_string()])
            .await
    }

    /// A drag on the device's own display — one swipe, which is what scrolled
    /// content and the launcher's pages need.
    pub async fn device_swipe(
        &self,
        from: (u32, u32),
        to: (u32, u32),
        duration: Duration,
    ) -> DwResult<()> {
        self.device_input(vec![
            "swipe".into(),
            from.0.to_string(),
            from.1.to_string(),
            to.0.to_string(),
            to.1.to_string(),
            duration.as_millis().to_string(),
        ])
        .await
    }

    /// One Android key event on the device's own display, by numeric keycode.
    ///
    /// This is how the virtual phone is typed into and how its navigation keys
    /// (Back, Home, Enter, Tab, the arrows) work. It is the same `input` path an
    /// app window's mapped key uses, aimed at the phone's display instead.
    pub async fn device_key(&self, keycode: i32) -> DwResult<()> {
        self.key_on_display(super::display::DEVICE_DISPLAY, keycode)
            .await
    }

    /// One Android key event on one app window's display.
    ///
    /// This is what the overlay's Back/Home/Recents buttons send, so a nav click
    /// is the same real `input keyevent` a mapped key uses — not a synthesized
    /// UI state. The keycode is validated by the caller.
    pub async fn key_on_display(&self, display_id: u32, keycode: i32) -> DwResult<()> {
        let adb = self.adb().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK is not configured.",
            )
        })?;
        adb.input(
            &self.serial(),
            display_id,
            &["keyevent".to_string(), keycode.to_string()],
        )
        .await?;
        Ok(())
    }

    /// Type text on the device's own display.
    ///
    /// Android's own `input text` requires a space to be written as `%s`; a
    /// literal space ends the argument there and silently drops the rest of the
    /// line. Newlines are not typed: a caller that wants Enter wants a key event.
    pub async fn device_text(&self, text: &str) -> DwResult<()> {
        let argument = device_text_argument(text);
        if argument.is_empty() {
            return Ok(());
        }
        self.device_input(vec!["text".into(), argument]).await
    }

    async fn device_input(&self, args: Vec<String>) -> DwResult<()> {
        let adb = self.adb().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK is not configured.",
            )
        })?;
        adb.input(&self.serial(), super::display::DEVICE_DISPLAY, &args)
            .await?;
        Ok(())
    }

    /// Displays are only known after scrcpy creates one, so a window can exist
    /// briefly without a display id. Callers must handle that instead of
    /// assuming display 0.
    pub fn display_of(&self, package_id: &str) -> Option<u32> {
        self.windows.lock().ok().and_then(|windows| {
            windows
                .get(package_id)
                .map(|entry| entry.display.load(std::sync::atomic::Ordering::Relaxed))
                .filter(|id| *id != 0)
        })
    }

    pub fn open_windows(&self) -> Vec<OpenWindow> {
        let Ok(windows) = self.windows.lock() else {
            return Vec::new();
        };
        let mut open: Vec<OpenWindow> = windows
            .iter()
            .map(|(package_id, entry)| OpenWindow {
                package_id: package_id.clone(),
                title: entry.title.clone(),
                pid: entry.pid,
                display_id: Some(entry.display.load(std::sync::atomic::Ordering::Relaxed))
                    .filter(|id| *id != 0),
                geometry: Some(entry.geometry),
            })
            .collect();
        open.sort_by(|a, b| a.package_id.cmp(&b.package_id));
        open
    }

    /// The emulator process this app started, if any. Host CPU and memory are
    /// read from it for real, not inferred from the plan.
    pub async fn emulator_pid(&self) -> Option<u32> {
        self.emulator_child
            .lock()
            .await
            .as_ref()
            .and_then(|child| child.id())
    }

    /// Cached environment report. Discovery stats the SDK and asks the emulator
    /// for its acceleration status (a process launch), and the answer cannot
    /// change between polls, so it is kept until something invalidates it.
    pub fn probe(&self) -> EnvReport {
        if let Ok(cache) = self.env_cache.lock() {
            if let Some((at, report)) = cache.as_ref() {
                if at.elapsed() < ENV_TTL {
                    return report.clone();
                }
            }
        }
        self.probe_now()
    }

    /// Fresh discovery: every filesystem check and the acceleration probe run
    /// again. Used by an explicit refresh and after the SDK could have changed.
    pub fn probe_now(&self) -> EnvReport {
        let report = self.discover_env();
        if let Ok(mut cache) = self.env_cache.lock() {
            *cache = Some((Instant::now(), report.clone()));
        }
        report
    }

    fn discover_env(&self) -> EnvReport {
        let mut report = EnvReport::default();
        // The same root the setup screen and the installer resolve, so the
        // runtime cannot report a different SDK than the one DroidWin installed.
        let Some(root) = sdk::active_root() else {
            report.blocking.push(
                "No Android SDK found. Install the Android command-line tools and set ANDROID_SDK_ROOT."
                    .to_string(),
            );
            return report;
        };
        let sdk = Sdk::at(root);
        report.sdk_root = Some(sdk.root().display().to_string());
        if sdk.adb().is_file() {
            report.adb = Some(sdk.adb().display().to_string());
        } else {
            report
                .blocking
                .push("Platform tools are missing: sdkmanager \"platform-tools\"".to_string());
        }
        if sdk.has_emulator() {
            report.emulator = Some(sdk.emulator().display().to_string());
            report.accel = sdk::accel_report(&sdk.emulator());
        } else {
            report
                .blocking
                .push("The Android Emulator is not installed: sdkmanager \"emulator\"".to_string());
        }
        if sdk.avdmanager().is_file() {
            report.avdmanager = Some(sdk.avdmanager().display().to_string());
        }
        if sdk.sdkmanager().is_file() {
            report.sdkmanager = Some(sdk.sdkmanager().display().to_string());
        }
        report.aapt2 = sdk.aapt2().map(|p| p.display().to_string());

        let images = sdk.system_images();
        report.system_image = images
            .iter()
            .find(|i| i.as_str() == SYSTEM_IMAGE)
            .or_else(|| {
                images
                    .iter()
                    .find(|i| i.contains("android-36") && i.ends_with("x86_64"))
            })
            .cloned();
        if report.system_image.is_none() {
            report.blocking.push(format!(
                "A system image is missing: sdkmanager \"{SYSTEM_IMAGE}\""
            ));
        }

        report.avds = std::fs::read_dir(sdk::avd_home())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        name.strip_suffix(".avd").map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        match sdk::scrcpy(&self.data_dir) {
            Some(path) => report.scrcpy = Some(path.display().to_string()),
            None => report.blocking.push(
                "The window host (scrcpy) is missing from this installation; reinstall DroidWin."
                    .to_string(),
            ),
        }

        report
    }

    fn capabilities(
        &self,
        _api: Option<u32>,
        accel_ok: bool,
        verified_windows: bool,
    ) -> Capabilities {
        Capabilities {
            install: true,
            uninstall: true,
            enumerate: true,
            launch: true,
            independent_windows: verified_windows,
            concurrent_apps: verified_windows,
            // Provided by the window host, but not verified on this machine yet,
            // so the product does not claim them.
            clipboard: false,
            file_drop: false,
            // The emulator plays Android's audio on the host's output device.
            // App windows stay silent so N open windows cannot play N copies;
            // the sound belongs to the runtime, not to the window.
            audio: true,
            hardware_accel: accel_ok,
        }
    }

    /// The host-isolation line of the status report, which says what was actually
    /// installed rather than what was intended.
    fn host_isolation_note(&self) -> String {
        isolation_note(self.host_isolation().as_ref())
    }

    fn limitations(&self, verified_windows: bool) -> Vec<String> {
        let mut items = vec![
            "All apps share one Android user, so they are not isolated from each other.".to_string(),
            "Each app window is a streamed virtual display: Android's back key works, but there is no host-side back button (Alt+B, Alt+H and Alt+S are the window host's own Back, Home and recents shortcuts).".to_string(),
            "Android's audio is played once by the emulator on the Windows output device; app windows do not forward it, and screen recordings are video-only because `screenrecord` has no audio option on this image.".to_string(),
            self.host_isolation_note(),
            "Clipboard sharing and dropping files onto an app window come from the window host and have not been verified in this build."
                .to_string(),
            "ARM-only packages cannot run on the x86_64 system image.".to_string(),
            "The Android SDK and emulator are never redistributed with DroidWin: they are downloaded from Google at setup, or an SDK already on the PC is used.".to_string(),
        ];
        if !verified_windows {
            items.insert(
                0,
                "Independent app windows have not been verified on this machine yet.".to_string(),
            );
        }
        items
    }

    pub async fn status(&self, store: &Store) -> RuntimeStatus {
        let api = self.adb_api().await;
        self.status_with_api(store, api).await
    }

    /// Build the status from an API level the caller already observed. A boot
    /// that just spent a `getprop` call does not need a second one.
    async fn status_with_api(&self, store: &Store, api: Option<u32>) -> RuntimeStatus {
        let env = self.probe();
        let verified = store
            .setting("runtime.independent_windows_verified")
            .as_deref()
            == Some("1");
        let accel_ok = env
            .accel
            .as_deref()
            .map(hardware::accel_usable)
            .unwrap_or(false);

        let (state, detail) = if !env.ready() {
            (RuntimeState::NotConfigured, env.blocking.join(" "))
        } else if let Some(api) = api {
            (
                RuntimeState::Ready,
                format!(
                    "Android API {api} on {} ({}).",
                    Self::config().serial(),
                    AVD_NAME
                ),
            )
        } else {
            let child_alive = self.emulator_child.lock().await.is_some();
            if child_alive {
                (
                    RuntimeState::Starting,
                    "The Android runtime is booting.".to_string(),
                )
            } else {
                (
                    RuntimeState::NotRunning,
                    "The Android runtime is stopped. Starting it takes a moment; the first boot is slower."
                        .to_string(),
                )
            }
        };

        let blocking = if matches!(state, RuntimeState::NotConfigured) {
            env.blocking.clone()
        } else {
            Vec::new()
        };

        RuntimeStatus {
            backend: "Android Emulator + scrcpy".to_string(),
            state,
            detail,
            avd: Some(AVD_NAME.to_string()),
            serial: if api.is_some() {
                Some(Self::config().serial())
            } else {
                None
            },
            android_api: api,
            capabilities: self.capabilities(api, accel_ok, verified),
            limitations: {
                let mut l = self.limitations(verified);
                l.extend(blocking);
                l
            },
        }
    }

    /// A short-lived cache: a status poll, a launch and a library refresh can
    /// all ask within the same second, and each answer used to cost its own
    /// `adb` process.
    async fn adb_api(&self) -> Option<u32> {
        if let Ok(cache) = self.readiness_cache.lock() {
            if let Some((at, api)) = cache.as_ref() {
                if at.elapsed() < READINESS_TTL {
                    return *api;
                }
            }
        }
        let adb = self.adb()?;
        let api = match adb.readiness(&Self::config().serial()).await {
            Ok(ready) if ready.booted => ready.api,
            _ => None,
        };
        self.remember_readiness(api);
        api
    }

    fn remember_readiness(&self, api: Option<u32>) {
        if let Ok(mut cache) = self.readiness_cache.lock() {
            *cache = Some((Instant::now(), api));
        }
    }

    /// Deny the guest access to the host's loopback, once per boot.
    ///
    /// The rules live in the guest kernel, so they are lost when Android reboots
    /// and this runs again on the next boot. It is idempotent either way: the
    /// `iptables -C` check makes an already-containered guest a no-op. A failure
    /// is logged and reported through the status limitations, never swallowed into
    /// a claim of isolation.
    async fn ensure_isolation(&self, adb: &Adb, serial: &str) {
        let boot = adb.boot_id(serial).await;
        if let (Some(boot), Ok(slot)) = (&boot, self.isolation.lock()) {
            if slot
                .as_ref()
                .map(|(installed_under, _)| installed_under == boot)
                .unwrap_or(false)
            {
                return;
            }
        }
        let state = adb.isolate_host(serial).await;
        if state.active() {
            tracing::info!("guest egress to the host loopback is denied (IPv4 and IPv6)");
        } else if state.ipv4 && state.ipv6 {
            // The rules are in the kernel; only the confirming connection has not
            // been observed. Saying "not contained" here would be as wrong as
            // claiming containment: this is unverified.
            tracing::warn!(
                "guest egress rules are installed but unverified; the probe did not answer"
            );
        } else {
            tracing::warn!(
                "guest egress to the host loopback is NOT contained: {}",
                state
                    .detail
                    .clone()
                    .unwrap_or_else(|| format!("probe {state:?}"))
            );
        }
        // An unreadable boot id is recorded as an empty one: the rules were
        // installed, and the next call will not repeat the privilege round trip
        // just because the guest would not name its boot.
        let id = boot.unwrap_or_default();
        if let Ok(mut slot) = self.isolation.lock() {
            *slot = Some((id, state));
        }
    }

    /// The containment state of the running guest, as last installed. `None`
    /// means no boot attempt has run in this process.
    pub fn host_isolation(&self) -> Option<HostIsolation> {
        self.isolation
            .lock()
            .ok()
            .and_then(|slot| slot.as_ref().map(|(_, state)| state.clone()))
    }

    fn invalidate_readiness(&self) {
        if let Ok(mut cache) = self.readiness_cache.lock() {
            *cache = None;
        }
    }

    fn emit(&self, op: &str, stage: Stage, message: impl Into<String>, package_id: Option<String>) {
        self.events.stage(&StageEvent {
            op: op.to_string(),
            package_id,
            stage,
            message: message.into(),
            at_ms: now_ms(),
        });
    }

    /// Bring the runtime to a point where adb install can run.
    pub async fn ensure_ready(&self, store: &Store, op: &str) -> DwResult<RuntimeStatus> {
        // Single flight: a second caller waits for the boot already in progress
        // instead of launching a competing emulator on the same port.
        let _booting = self.boot.lock().await;

        let env = self.probe();
        if !env.ready() {
            return Err(DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android runtime is not ready on this machine.",
            )
            .detail(env.blocking.join("\n")));
        }

        let sdk = Sdk::locate(None).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK could not be located.",
            )
        })?;
        let adb = Adb::new(sdk.adb());
        let config = Self::config();
        let serial = config.serial();

        // A device that is online but still booting must be awaited, not
        // duplicated. Before adopting an already-running emulator, verify its
        // console-reported AVD name: `emulator-5554` is only a port label.
        let mut present = false;
        if let Ok(ready) = adb.readiness(&serial).await {
            let avd_name = adb.emulator_avd_name(&serial).await?;
            if avd_name != AVD_NAME {
                return Err(DwError::new(
                    ErrorCode::RuntimeUnavailable,
                    "A different Android virtual device is using DroidWin's managed emulator port.",
                )
                .detail(format!(
                    "Expected {AVD_NAME}, found {avd_name}; DroidWin will not install apps into or control another AVD."
                )));
            }
            present = true;
            // The device on our port is DroidWin's AVD and a pid file exists: it
            // was started by an earlier run of this app.
            self.adopt_ownership();
            if ready.booted {
                self.remember_readiness(ready.api);
                self.ensure_isolation(&adb, &serial).await;
                return Ok(self.status_with_api(store, ready.api).await);
            }
        }
        if !present && adb.connect_ready(&serial).await {
            let avd_name = adb.emulator_avd_name(&serial).await?;
            if avd_name != AVD_NAME {
                return Err(DwError::new(
                    ErrorCode::RuntimeUnavailable,
                    "A different Android virtual device is using DroidWin's managed emulator port.",
                )
                .detail(format!(
                    "Expected {AVD_NAME}, found {avd_name}; DroidWin will not install apps into or control another AVD."
                )));
            }
            self.adopt_ownership();
            present = true;
        }

        self.emit(
            op,
            Stage::PreparingRuntime,
            "Preparing the virtual device.",
            None,
        );
        if !present {
            let created = emulator::ensure_avd(&sdk, &config).await?;
            if created {
                tracing::info!("created AVD {AVD_NAME}");
            }
            let log = crate::diag::log_dir(&self.data_dir).join("emulator.log");
            let plan = hardware::load(store, &hardware::host_facts());
            let child = emulator::spawn(&sdk, &config, &log, &plan)?;
            self.record_ownership(child.id());
            *self.emulator_child.lock().await = Some(child);
            adb.start().await.ok();
            self.invalidate_readiness();
        }

        let api = emulator::wait_boot(&adb, &serial, BOOT_BUDGET).await?;
        self.remember_readiness(Some(api));
        self.ensure_isolation(&adb, &serial).await;
        Ok(self.status_with_api(store, Some(api)).await)
    }

    pub async fn install(
        &self,
        inspection: &ApkInspection,
        store: &Store,
        op: &str,
        allow_old_target: bool,
    ) -> DwResult<InstallOutcome> {
        // Serialise installs: two packages requested together must reach the
        // package manager one at a time, or each one's verify step can observe
        // the other's half-finished state.
        let _installing = self.install_gate.lock().await;
        self.ensure_ready(store, op).await?;
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = Self::config().serial();
        let package_id = inspection.package_id.clone();

        let was_installed = adb.package_path(&serial, &package_id).await.is_some();

        self.emit(
            op,
            Stage::Installing,
            format!("Installing {package_id}."),
            Some(package_id.clone()),
        );
        let output = adb
            .install(&serial, Path::new(&inspection.path), allow_old_target)
            .await?;
        if !output.ok() || !output.stdout.contains("Success") {
            let text = output.combined();
            return Err(
                DwError::new(ErrorCode::InstallFailed, install_failure_summary(&text)).detail(text),
            );
        }

        self.emit(
            op,
            Stage::Verifying,
            "Confirming the runtime sees the package.",
            Some(package_id.clone()),
        );
        if adb.package_path(&serial, &package_id).await.is_none() {
            return Err(DwError::new(
                ErrorCode::InstallFailed,
                "The installer reported success, but the runtime does not list the package.",
            ));
        }

        let version_code = adb
            .third_party_packages(&serial)
            .await
            .ok()
            .and_then(|list| list.into_iter().find(|p| p.package_id == package_id))
            .and_then(|p| p.version_code);

        let _ = store.cache_app(
            &CachedApp {
                package_id: package_id.clone(),
                label: inspection.label.clone(),
                version_name: inspection.version_name.clone(),
                version_code: version_code.or(inspection.version_code),
                icon: inspection.icon.clone(),
                target_sdk: inspection.target_sdk,
                orientation: inspection.orientation,
            },
            now_ms(),
        );
        self.events.apps_changed();

        Ok(InstallOutcome {
            package_id: package_id.clone(),
            version_name: inspection.version_name.clone(),
            version_code: version_code.or(inspection.version_code),
            updated: was_installed,
            message: if was_installed {
                format!("{package_id} updated")
            } else {
                format!("{package_id} installed")
            },
        })
    }

    pub async fn list_apps(&self, store: &Store) -> DwResult<Vec<InstalledApp>> {
        let adb = self.adb().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK could not be located.",
            )
        })?;
        let serial = Self::config().serial();
        match adb.readiness(&serial).await {
            Ok(ready) if ready.booted => {}
            _ => {
                return Err(DwError::new(
                    ErrorCode::RuntimeUnavailable,
                    "The Android runtime is not running.",
                ))
            }
        }

        let packages = adb.third_party_packages(&serial).await?;
        let cache: HashMap<String, CachedApp> = store
            .cached_apps()
            .into_iter()
            .map(|c| (c.package_id.clone(), c))
            .collect();
        let open_windows: HashMap<String, String> = self
            .windows
            .lock()
            .map(|w| {
                w.iter()
                    .map(|(package, entry)| (package.clone(), entry.title.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let mut apps = Vec::new();
        for pkg in packages {
            let cached = cache.get(&pkg.package_id);
            apps.push(InstalledApp {
                label: cached.and_then(|c| c.label.clone()),
                version_name: cached.and_then(|c| c.version_name.clone()),
                version_code: pkg.version_code.or(cached.and_then(|c| c.version_code)),
                running: open_windows.contains_key(&pkg.package_id),
                window_title: open_windows.get(&pkg.package_id).cloned(),
                icon: cached.and_then(|c| c.icon.clone()),
                user_installed: true,
                target_sdk: cached.and_then(|c| c.target_sdk),
                package_id: pkg.package_id,
            });
        }
        apps.sort_by(|a, b| {
            a.label
                .as_deref()
                .unwrap_or(&a.package_id)
                .to_lowercase()
                .cmp(&b.label.as_deref().unwrap_or(&b.package_id).to_lowercase())
        });
        Ok(apps)
    }

    pub async fn launch(
        &self,
        package_id: &str,
        label: Option<&str>,
        store: &Store,
        op: &str,
    ) -> DwResult<AppWindow> {
        // One window per package: launching an app that is already open brings
        // the existing window's facts back rather than opening a duplicate.
        if let Some(existing) = self.existing_window(package_id) {
            return Ok(existing);
        }

        let ready = self.ensure_ready(store, op).await?;
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = Self::config().serial();
        if adb.package_path(&serial, package_id).await.is_none() {
            return Err(DwError::new(
                ErrorCode::NotInstalled,
                format!("{package_id} is not installed in the runtime."),
            ));
        }

        let exe = sdk::scrcpy(&self.data_dir).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The window host (scrcpy) is missing.",
            )
        })?;

        // The check above and the spawn below must be one step: two clicks in
        // quick succession would otherwise both find no window and both open one.
        let gate = self.launch.lock().await;
        if let Some(existing) = self.existing_window(package_id) {
            return Ok(existing);
        }

        let index = self.windows.lock().map(|w| w.len()).unwrap_or(0) as i32;
        let title = format!("DroidWin — {}", label.unwrap_or(package_id));
        // uhid is the mode where Shift and every shifted symbol reach the app as
        // real key events; sdk is kept as a per-install switch and as the
        // automatic fallback when a runtime refuses a HID keyboard.
        let requested = store
            .setting("runtime.keyboard_mode")
            .and_then(|v| KeyboardMode::parse(&v))
            .unwrap_or(KeyboardMode::Auto);
        // The app's own orientation decides the surface. A landscape-locked game
        // drawn on a portrait surface gets rotated by Android and clipped by its
        // own display, which is the defect this geometry exists to prevent.
        let declared = store
            .cached_app(package_id)
            .and_then(|cached| cached.orientation);
        // A rotation the user asked for wins over the app's manifest: the
        // surface is fixed when the window opens, so "rotate" is a remembered
        // choice for the next open, not a property of the running display.
        let chosen = store
            .setting(&Orientation::setting_key(package_id))
            .and_then(|value| Orientation::parse(&value));
        let geometry = match chosen {
            Some(orientation) => DisplayGeometry::for_orientation(orientation),
            None => DisplayGeometry::for_app(declared),
        };
        tracing::info!(
            "{package_id} opens on a {} surface {}x{} at {} dpi (window {}x{})",
            geometry.orientation.as_str(),
            geometry.width,
            geometry.height,
            geometry.dpi,
            geometry.window_width,
            geometry.window_height
        );
        let mut request = WindowRequest {
            serial: serial.clone(),
            package_id: package_id.to_string(),
            title: title.clone(),
            x: 80 + index * 36,
            y: 80 + index * 36,
            width: geometry.window_width,
            height: geometry.window_height,
            display_width: geometry.width,
            display_height: geometry.height,
            dpi: geometry.dpi,
            keyboard: requested.resolve(ready.android_api),
        };

        self.emit(
            op,
            Stage::Launching,
            format!("Opening a window for {package_id}."),
            Some(package_id.to_string()),
        );
        let mut child = scrcpy::spawn(&exe, &request)?;
        // A backend this runtime cannot use makes the host exit at once. Falling
        // back keeps the app usable instead of showing an empty window.
        if let Some(failure) = scrcpy::early_failure(&mut child, Duration::from_secs(3)).await {
            if request.keyboard.is_hid() {
                tracing::warn!("uhid keyboard rejected, falling back to sdk: {failure}");
                self.emit(
                    op,
                    Stage::Launching,
                    "This runtime refused the hardware keyboard; retrying with the simulated one.",
                    Some(package_id.to_string()),
                );
                request.keyboard = KeyboardMode::Sdk;
                child = scrcpy::spawn(&exe, &request)?;
                scrcpy::drain_stderr(&mut child);
            } else {
                return Err(DwError::new(
                    ErrorCode::LaunchFailed,
                    format!("The window for {package_id} closed immediately."),
                )
                .detail(failure));
            }
        } else {
            scrcpy::drain_stderr(&mut child);
        }
        let pid = child.id().unwrap_or(0);
        let child = Arc::new(tokio::sync::Mutex::new(child));
        let closing = Arc::new(std::sync::atomic::AtomicBool::new(false));

        if let Ok(mut windows) = self.windows.lock() {
            windows.insert(
                package_id.to_string(),
                WindowEntry {
                    child: Arc::clone(&child),
                    title: title.clone(),
                    pid,
                    geometry,
                    display: std::sync::atomic::AtomicU32::new(0),
                    composed: std::sync::atomic::AtomicU64::new(0),
                    closing: Arc::clone(&closing),
                },
            );
        }
        self.watch_window(package_id.to_string(), child, closing);
        self.events.apps_changed();
        // Two windows on two displays, both alive, is the evidence the capability
        // flag was waiting for. Record it once; the status build reads it back.
        let open_windows = self.windows.lock().map(|w| w.len()).unwrap_or(0);
        if open_windows >= 2
            && store
                .setting("runtime.independent_windows_verified")
                .as_deref()
                != Some("1")
        {
            let _ = store.set_setting("runtime.independent_windows_verified", "1");
            tracing::info!("{open_windows} app windows are open; independent windows are verified");
        }
        // The window is registered; the readiness wait below must not hold the
        // gate, or a second app could not be launched meanwhile.
        drop(gate);

        // A window plus a live scrcpy process is not proof that Android started
        // the app: a blocked background activity launch leaves an empty window.
        // Watch for the app's own process for up to five seconds.
        let mut started = false;
        let mut launched_pid: Option<u32> = None;
        // The wait is bounded by evidence, not by a fixed number that happens to
        // fit a warm start: a cold boot needs seconds before the package's first
        // process exists (dexopt, multidex, a cold activity start). It returns as
        // soon as the process appears, and stops at once if the window goes away.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let open = self
                .windows
                .lock()
                .map(|w| w.contains_key(package_id))
                .unwrap_or(false);
            if !open {
                break;
            }
            if let Some(pid) = adb.process_pids(&serial, package_id).await.first().copied() {
                started = true;
                launched_pid = Some(pid);
                break;
            }
        }
        let still_open = self
            .windows
            .lock()
            .map(|w| w.contains_key(package_id))
            .unwrap_or(false);
        if !still_open {
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                format!("The window for {package_id} closed before Android started it."),
            )
            .detail("The window host exited on its own. DroidWin did not close it, and no Android process for the package was observed while it was open. This app may not support a secondary display."));
        }
        if !started {
            let _ = self.close_window(package_id).await;
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                format!("Android did not start {package_id} on its new display."),
            )
            .detail("The window host opened, but the package has no process. Android can refuse to start an activity on a secondary display (background-activity-launch rules) or the app may not support one. Try again after the runtime is idle."));
        }

        // A pid is not proof that the app is running: Android prints `Displayed`
        // and reports the process before the render thread throws. Hill Climb
        // Racing, for one, dies in `eglChooseConfig` about 600 ms in and leaves a
        // black window. But the same pid is also not proof that it stopped: a
        // cold start can take seconds, and an app that restarts its own process
        // (multidex loaders do exactly that) simply has a different pid. So watch
        // the package, not the number, and only call it a crash on evidence.
        let mut pid_now = launched_pid;
        let mut surviving = false;
        for _ in 0..16 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if let Some(pid) = adb.process_pids(&serial, package_id).await.first().copied() {
                if Some(pid) != pid_now {
                    tracing::info!(
                        "{package_id} is running as pid {pid} (it started as {pid_now:?})"
                    );
                    pid_now = Some(pid);
                }
                surviving = true;
                break;
            }
            let open = self
                .windows
                .lock()
                .map(|w| w.contains_key(package_id))
                .unwrap_or(false);
            if !open {
                break;
            }
        }
        if !surviving {
            // A crash block from an earlier attempt must not restart the whole
            // runtime, so only a block naming the pid this launch started counts.
            let crash = adb.crash_excerpt(&serial, package_id).await.filter(|text| {
                pid_now
                    .map(|pid| text.contains(&format!("PID: {pid}")))
                    .unwrap_or(true)
            });
            let _ = self.close_window(package_id).await;
            if crash
                .as_deref()
                .map(super::adb::is_egl_config_crash)
                .unwrap_or(false)
                && self.switch_to_software_renderer(store, op).await
            {
                return Box::pin(self.launch(package_id, label, store, op)).await;
            }
            let summary = match crash.as_deref() {
                Some(_) => {
                    format!("{package_id} started and then crashed, so its window has no picture.")
                }
                None => format!(
                    "{package_id} has no running process any more, so its window has no picture."
                ),
            };
            return Err(DwError::new(ErrorCode::LaunchFailed, summary).detail(
                crash.unwrap_or_else(|| {
                    "Android recorded no crash block for this package, so DroidWin is not claiming a crash: the process was started and then disappeared. Check the runtime log for this package.".into()
                }),
            ));
        }

        // Resolve the display Android actually put the app on. Input and
        // screenshots are display-scoped, and guessing 0 would drive the wrong
        // screen. A cold start composes the surface later than a warm one, so the
        // retry is long enough for that and still bounded; failure leaves the
        // display unknown rather than wrong.
        // A window whose display never resolves is now closed below, so give a
        // slow first composition ten seconds rather than six.
        for attempt in 0..20 {
            if let Some(focus) = adb.display_focus_for_package(&serial, package_id).await {
                if let Ok(windows) = self.windows.lock() {
                    if let Some(entry) = windows.get(package_id) {
                        entry
                            .display
                            .store(focus.display_id, std::sync::atomic::Ordering::Relaxed);
                    }
                }
                tracing::info!("{package_id} is on Android display {}", focus.display_id);
                if let Some((width, height)) = focus.width.zip(focus.height) {
                    if let Ok(windows) = self.windows.lock() {
                        if let Some(entry) = windows.get(package_id) {
                            entry.composed.store(
                                pack_size(width, height),
                                std::sync::atomic::Ordering::Relaxed,
                            );
                        }
                    }
                }
                self.record_observed_orientation(store, package_id, declared, &focus);
                break;
            }
            if attempt < 19 {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }

        // An app can die in the graphics stack and be restarted by Android. The
        // process check above then passes (a pid exists), but Android composes
        // nothing for the window, so no display is ever reported and the window
        // shows black. Hill Climb Racing does exactly this: it throws
        // `No config chosen` in eglChooseConfig under pid N, Android restarts it
        // as pid N+1, and only the crash buffer still names the failure. Treat
        // that as the EGL failure it is, so it gets the same one-time software
        // renderer recovery instead of a window with no picture.
        if self.display_of(package_id).is_none() {
            let crash = adb.crash_excerpt(&serial, package_id).await.filter(|text| {
                launched_pid
                    .map(|pid| text.contains(&format!("PID: {pid}")))
                    .unwrap_or(false)
            });
            if crash
                .as_deref()
                .map(super::adb::is_egl_config_crash)
                .unwrap_or(false)
            {
                let _ = self.close_window(package_id).await;
                if self.switch_to_software_renderer(store, op).await {
                    return Box::pin(self.launch(package_id, label, store, op)).await;
                }
                return Err(DwError::new(
                    ErrorCode::LaunchFailed,
                    format!("{package_id} started, then died in the graphics stack."),
                )
                .detail(crash.unwrap_or_else(|| {
                    "the app's process was restarted after a graphics failure".into()
                })));
            }

            // Nothing is on the window's display: the package has a process, but
            // no display reports it, and Android's own dump shows that virtual
            // display with `mCurrentFocus=null`. A non-resizable or
            // landscape-only activity stays on the default display instead of
            // the one it was sent to (Hill Climb Racing does this), and scrcpy's
            // window then has nothing to draw. Closing it and saying so is the
            // honest outcome; a window with no picture is not an open app.
            let _ = self.close_window(package_id).await;
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                format!("{package_id} opened, but Android drew nothing on its window."),
            )
            .detail(format!(
                "The app has a running process but no display of its own, so taps and screenshots could not be aimed at it; the empty window was closed instead of being left open. This happens when Android keeps a non-resizable or landscape-only activity on the default display rather than the secondary display DroidWin sent it to. {}",
                crash.map(|text| format!("Android's crash log for this launch: {text}"))
                    .unwrap_or_else(|| "Android recorded no crash for it.".into())
            )));
        }

        self.events.app_event(
            &super::AppEvent::new(
                package_id,
                super::AppEventKind::Opened,
                "its window opened and its Android display was resolved",
            )
            .at(self.display_of(package_id), Some(pid)),
        );

        Ok(AppWindow {
            package_id: package_id.to_string(),
            title,
            pid,
        })
    }

    /// Some apps ask for an EGL config that the host GPU path does not expose
    /// (Hill Climb Racing asks for one and dies with `No config chosen`).
    /// SwiftShader exposes a wider set, so the runtime is restarted once on
    /// software rendering and the app is reopened.
    ///
    /// Refused while another window is open: that window belongs to the runtime
    /// being restarted, and killing someone else's app is not a recovery.
    async fn switch_to_software_renderer(&self, store: &Store, op: &str) -> bool {
        let plan = hardware::load(store, &hardware::host_facts());
        if plan.gpu == GpuMode::Software {
            return false;
        }
        if self.windows.lock().map(|w| !w.is_empty()).unwrap_or(true) {
            tracing::warn!("not switching renderer: another app window is open");
            return false;
        }
        self.emit(
            op,
            Stage::Launching,
            "This app rejects the host GPU's EGL configurations. Restarting the runtime with software rendering…",
            None,
        );
        tracing::warn!("restarting the runtime with SwiftShader so the app can render");
        self.shutdown().await;
        // The plan is computed after the emulator has exited: `recommend()`
        // measures available memory, and reading it while 4 GB of qemu is still
        // running would silently cut the guest to the 2 GB floor.
        let mut next = hardware::load(store, &hardware::host_facts());
        next.gpu = GpuMode::Software;
        next.mode = PlanMode::Custom;
        next.reason = format!(
            "{} Software rendering was chosen after an app rejected the host GPU's EGL configurations.",
            next.reason.trim_end()
        );
        if let Err(err) = hardware::persist(store, &next) {
            tracing::warn!("could not save the software renderer plan: {err}");
            return false;
        }
        match self.ensure_ready(store, op).await {
            Ok(_) => true,
            Err(err) => {
                tracing::warn!("SwiftShader restart failed: {}", err.summary);
                false
            }
        }
    }

    /// When a package's manifest named no orientation, the display Android
    /// actually composed is the evidence. If it had to rotate the portrait
    /// surface to draw the app, the app belongs on that rotated surface, so it is
    /// recorded and the next launch creates it directly instead of rotating
    /// again. Nothing is recorded when the created surface was already right.
    fn record_observed_orientation(
        &self,
        store: &Store,
        package_id: &str,
        declared: Option<AppOrientation>,
        focus: &super::adb::DisplayFocus,
    ) {
        if declared.is_some() {
            return;
        }
        let Some((width, height)) = focus.width.zip(focus.height) else {
            return;
        };
        let Some(observed) = Orientation::of_size(width, height) else {
            return;
        };
        if observed == DisplayGeometry::for_app(None).orientation {
            return;
        }
        let Some(mut cached) = store.cached_app(package_id) else {
            return;
        };
        let inferred = match observed {
            Orientation::Portrait => AppOrientation::Portrait,
            Orientation::Landscape => AppOrientation::Landscape,
        };
        cached.orientation = Some(inferred);
        match store.cache_app(&cached, now_ms()) {
            Ok(()) => tracing::info!(
                "{package_id} composed in {}; recorded it for the next launch",
                observed.as_str()
            ),
            Err(err) => tracing::warn!(
                "could not record the orientation Android showed for {package_id}: {}",
                err.summary
            ),
        }
    }

    fn existing_window(&self, package_id: &str) -> Option<AppWindow> {
        self.windows.lock().ok().and_then(|w| {
            w.get(package_id).map(|entry| AppWindow {
                package_id: package_id.to_string(),
                title: entry.title.clone(),
                pid: entry.pid,
            })
        })
    }

    fn watch_window(
        &self,
        package_id: String,
        child: Arc<tokio::sync::Mutex<Child>>,
        closing: Arc<std::sync::atomic::AtomicBool>,
    ) {
        let windows = Arc::clone(&self.windows);
        let events = Arc::clone(&self.events);
        let adb = self.adb();
        let serial = Self::config().serial();
        tokio::spawn(async move {
            // Why the loop stopped decides the event: a non-zero exit is the
            // window host dying with the app, not a clean close.
            enum Ended {
                Host(Option<i32>),
                AppGone,
            }
            // Two consecutive misses, not one: an app can be between processes
            // for a moment during its own restart without being dead.
            let mut misses = 0u8;
            // A window host exists before Android has a process for the app. On
            // a cold boot that gap is seconds, so "no process" only means "gone"
            // once the app has actually been seen running on this display.
            // Killing the host on the first misses is what made the first launch
            // after a cold boot fail while the app was still starting.
            let mut seen_running = false;
            let ended = loop {
                let exited = {
                    let mut guard = child.lock().await;
                    match guard.try_wait() {
                        Ok(Some(status)) => Some(status.code()),
                        Ok(None) => None,
                        Err(_) => Some(None),
                    }
                };
                if let Some(code) = exited {
                    tracing::info!("window for {package_id} closed (exit {code:?})");
                    break Ended::Host(code);
                }
                if let Some(adb) = adb.as_ref() {
                    if adb.process_running(&serial, &package_id).await {
                        seen_running = true;
                        misses = 0;
                    } else if seen_running {
                        misses += 1;
                        if misses >= 2 {
                            // The window outlived its app. Leaving it up shows a
                            // black frame that looks like a rendering defect.
                            tracing::info!("{package_id} exited; closing its window");
                            let mut guard = child.lock().await;
                            let _ = guard.start_kill();
                            break Ended::AppGone;
                        }
                    }
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            };
            let display = windows
                .lock()
                .ok()
                .and_then(|w| {
                    w.get(&package_id)
                        .map(|entry| entry.display.load(std::sync::atomic::Ordering::Relaxed))
                })
                .filter(|id| *id != 0);
            let closing = closing.load(std::sync::atomic::Ordering::SeqCst);
            if let Ok(mut map) = windows.lock() {
                // An old watcher can finish after the same package has been
                // reopened. It must not remove the replacement window.
                let is_ours = map
                    .get(&package_id)
                    .is_some_and(|entry| Arc::ptr_eq(&entry.child, &child));
                if is_ours {
                    map.remove(&package_id);
                } else if map.contains_key(&package_id) {
                    // A replacement is already running; this stale watcher
                    // must not emit a closed event for the replacement either.
                    return;
                }
            }
            let exit_code = match &ended {
                Ended::Host(code) => *code,
                Ended::AppGone => None,
            };
            let event = classify_exit(
                &package_id,
                closing,
                exit_code,
                matches!(ended, Ended::AppGone),
            )
            .at(display, None);
            events.app_event(&event);
            events.apps_changed();
        });
    }

    pub async fn close_window(&self, package_id: &str) -> DwResult<()> {
        let entry = self
            .windows
            .lock()
            .ok()
            .and_then(|mut w| w.remove(package_id));
        let Some(entry) = entry else {
            return Ok(());
        };
        // The window host dies with a failure code when it is killed. Say why
        // before that happens, or the watcher would report a crash that did not
        // occur.
        entry
            .closing
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let mut child = entry.child.lock().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        self.events.apps_changed();
        Ok(())
    }

    pub async fn stop_app(&self, package_id: &str) -> DwResult<()> {
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let out = adb.force_stop(&Self::config().serial(), package_id).await?;
        if out.ok() {
            self.events.apps_changed();
            Ok(())
        } else {
            Err(
                DwError::new(ErrorCode::BackendFailure, "Android did not stop the app.")
                    .detail(out.combined()),
            )
        }
    }

    /// Returns the sentence to show and record: it reflects what Android
    /// reported about retained data after the package was removed.
    pub async fn uninstall(
        &self,
        package_id: &str,
        delete_data: bool,
        store: &Store,
    ) -> DwResult<String> {
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = Self::config().serial();
        if adb.package_path(&serial, package_id).await.is_none() {
            return Err(DwError::new(
                ErrorCode::NotInstalled,
                format!("{package_id} is not installed."),
            ));
        }
        self.close_window(package_id).await?;
        let out = adb.uninstall(&serial, package_id, !delete_data).await?;
        if !out.ok() || !out.stdout.contains("Success") {
            return Err(DwError::new(
                ErrorCode::UninstallFailed,
                format!("Android refused to uninstall {package_id}."),
            )
            .detail(out.combined()));
        }
        if delete_data {
            let _ = store.forget_app(package_id);
        }
        self.events.apps_changed();

        // `pm uninstall -k` keeps data by contract; `pm list packages -u` is
        // where that becomes observable, so the message follows the observation.
        let retained = adb.packages_including_uninstalled(&serial).await;
        let still_listed = retained
            .as_ref()
            .map(|list| list.iter().any(|p| p == package_id))
            .unwrap_or(false);
        Ok(if delete_data {
            if still_listed {
                format!("{package_id} was uninstalled, but Android still lists data for it.")
            } else {
                format!("{package_id} was uninstalled with its data.")
            }
        } else if still_listed {
            format!("{package_id} was uninstalled; Android confirmed its data was kept.")
        } else if retained.is_ok() {
            format!("{package_id} was uninstalled; Android lists no retained data for it.")
        } else {
            format!("{package_id} was uninstalled; data retention could not be confirmed.")
        })
    }

    /// Host facts, the plan that will be applied on the next start, and — when
    /// the runtime is up — what Android actually reports. The UI shows the two
    /// sides separately so "requested" is never mistaken for "effective".
    pub async fn hardware_report(&self, store: &Store) -> HardwareReport {
        let host = hardware::host_facts();
        let plan = hardware::load(store, &host);
        let adb = self.adb();
        let serial = Self::config().serial();
        // Share the short-lived readiness answer with status and launch rather
        // than spawning another adb process each time Settings is opened.
        let booted = self.adb_api().await.is_some();

        let effective = if booted {
            match adb.as_ref() {
                Some(adb) => {
                    let cached = self.effective_cache.lock().unwrap().clone();
                    if let Some((at, facts)) = cached {
                        if at.elapsed() < EFFECTIVE_TTL {
                            return self.report(host, plan, Some(facts)).await;
                        }
                    }
                    let gpu_renderer = adb.gles_renderer(&serial).await;
                    let facts = hardware::EffectiveHardware {
                        cores: adb.cpu_count(&serial).await,
                        ram_mb: adb.mem_total_mb(&serial).await,
                        hardware_gpu: hardware::hardware_gpu(gpu_renderer.as_deref()),
                        gpu_renderer,
                    };
                    let stored = (Instant::now(), facts.clone());
                    *self.effective_cache.lock().unwrap() = Some(stored);
                    Some(facts)
                }
                None => None,
            }
        } else {
            *self.effective_cache.lock().unwrap() = None;
            None
        };
        self.report(host, plan, effective).await
    }

    async fn report(
        &self,
        host: hardware::HostFacts,
        plan: hardware::HardwarePlan,
        effective: Option<hardware::EffectiveHardware>,
    ) -> HardwareReport {
        let restart_required = match &effective {
            Some(facts) => {
                facts.cores.is_some_and(|cores| cores != plan.cores)
                    || facts
                        .ram_mb
                        .is_some_and(|mb| (mb as i64 - plan.ram_mb as i64).abs() > 128)
                    || (plan.gpu == GpuMode::Host && facts.hardware_gpu == Some(false))
            }
            None => false,
        };

        let mut notes = Vec::new();
        if let Some(facts) = &effective {
            match facts.hardware_gpu {
                Some(true) => notes.push(
                    "Android confirmed hardware rendering; this is the renderer string it reported."
                        .to_string(),
                ),
                Some(false) => notes.push(
                    "Android is rendering in software. Restart the runtime so the host GPU plan applies, or choose Software if this machine cannot provide it."
                        .to_string(),
                ),
                None => notes.push(
                    "The runtime did not report its renderer, so acceleration is unconfirmed."
                        .to_string(),
                ),
            }
        }

        // Measured, and the reason a larger plan is not offered: the emulator
        // caps the vCPU count itself instead of giving Android what was asked
        // for. Saying it here keeps "6 vCPUs" from reading as a DroidWin limit.
        if host.logical_cpus > hardware::MAX_CORES {
            notes.push(format!(
                "The Android Emulator applies at most {} vCPUs on this build; a larger -cores request is reduced by the emulator and logged as `Emulator does not support more than {} cores`.",
                hardware::MAX_CORES, hardware::MAX_CORES
            ));
        }

        let acceleration = self.probe().accel;
        HardwareReport {
            acceleration_ok: acceleration
                .as_deref()
                .map(hardware::accel_usable)
                .unwrap_or(false),
            acceleration,
            host,
            plan,
            effective,
            restart_required,
            notes,
        }
    }

    /// Closing DroidWin. App windows always close; the emulator is stopped only
    /// when DroidWin started it and the user has not asked to keep it.
    pub async fn shutdown(&self) {
        let _gate = self.shutdown_gate.lock().await;
        if self.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        self.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
        if !self.stop_on_exit() && self.is_owned().await {
            tracing::info!("keeping the emulator: the user asked DroidWin to leave it running");
            self.close_windows().await;
            // Forget the pid, so the next launch sees an emulator it did not
            // start and does not stop it later against the user's wish.
            let _ = std::fs::remove_file(self.ownership_file());
            if let Ok(mut slot) = self.owned_pid.lock() {
                *slot = None;
            }
            return;
        }
        self.stop_windows_and_runtime().await;
    }

    /// The explicit "Stop runtime" action. It ignores the close preference: a
    /// person who pressed stop meant it.
    pub async fn stop_now(&self) {
        self.stop_windows_and_runtime().await;
    }

    async fn is_owned(&self) -> bool {
        self.emulator_child.lock().await.is_some()
            || self
                .owned_pid
                .lock()
                .map(|slot| slot.is_some())
                .unwrap_or(false)
            || self.owned_orphan.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Close every app window without touching the emulator. Separated from the
    /// emulator shutdown so "keep it running" still ends the windows.
    async fn close_windows(&self) {
        let packages: Vec<String> = self
            .windows
            .lock()
            .map(|w| w.keys().cloned().collect())
            .unwrap_or_default();
        for package in packages {
            let _ = self.close_window(&package).await;
        }
    }

    async fn stop_windows_and_runtime(&self) {
        // The rules belong to the guest kernel, so stopping the emulator discards
        // them; the cache must not outlive the boot that installed them.
        if let Ok(mut slot) = self.isolation.lock() {
            *slot = None;
        }
        self.close_windows().await;
        // Only an emulator DroidWin started is ours to stop — either the child
        // this process spawned, or one a previous run spawned and left behind.
        let owned = self.emulator_child.lock().await.take();
        let adopted = self.owned_pid.lock().ok().and_then(|mut slot| slot.take());
        let orphan = self
            .owned_orphan
            .swap(false, std::sync::atomic::Ordering::SeqCst);
        if owned.is_none() && adopted.is_none() && !orphan {
            tracing::info!(
                "not stopping the runtime: DroidWin is not running an emulator it started"
            );
            return;
        }
        let mut child = owned;
        let pid = child.as_ref().and_then(|child| child.id()).or(adopted);
        tracing::info!("stopping the emulator DroidWin started (pid {:?})", pid);
        let serial = Self::config().serial();
        let adb = self.adb();
        // A launcher this process still holds is the root of the whole tree, so
        // kill the tree outright. Asking the emulator console instead is not
        // gentler in practice: it exits the launcher and leaves
        // `emulator -kill <qemu> -sleep 20` behind for twenty seconds, which
        // reads as a leaked emulator, and the VM itself runs with `-no-snapshot`
        // so a graceful stop saves nothing.
        if child.is_some() {
            if let Some(pid) = pid {
                emulator::kill_tree(pid);
            }
        } else if let Some(pid) = adopted.filter(|pid| is_emulator_pid(*pid)) {
            emulator::kill_tree(pid);
        } else if let Some(adb) = &adb {
            // Only the console can reach an emulator whose launcher pid is gone.
            let _ = adb.emu_kill(&serial).await;
        }
        if let Some(child) = child.as_mut() {
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
        // Closing DroidWin must not leave the device listed. Bounded: a stuck
        // emulator is still worth exiting from, and the next launch adopts or
        // replaces it.
        if let Some(adb) = adb {
            for _ in 0..24 {
                if adb.readiness(&serial).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        }
        let _ = std::fs::remove_file(self.ownership_file());
        self.invalidate_readiness();
    }
}

/// The device shell's spelling of typed text.
///
/// A space becomes `%s` (Android ends the argument at a literal space and drops
/// everything after it) and control characters are removed — a newline is a key
/// event, not text. Pure, so the rule can be read and tested without a device.
pub fn device_text_argument(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .replace(' ', "%s")
}

#[cfg(test)]
mod tests {
    use super::{
        classify_exit, device_text_argument, install_failure_summary, is_emulator_process,
        isolation_note, EmulatorRuntime,
    };
    use crate::runtime::adb::HostIsolation;
    use crate::runtime::{AppEventKind, NoopEvents};
    use std::sync::Arc;

    fn owner_dir(tag: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "droidwin-owner-{tag}-{}-{}",
            std::process::id(),
            crate::domain::now_ms()
        ))
    }

    /// A pid file left by a crashed run is the claim. The pid is often already
    /// gone (`emulator.exe` exits after spawning the VM), so the claim cannot
    /// depend on it being live; the caller only adopts after adb confirmed the
    /// device on our port is DroidWin's own AVD.
    #[tokio::test]
    async fn an_emulator_left_by_a_crashed_run_is_claimed_by_its_pid_file() {
        let dir = owner_dir("orphan");
        std::fs::create_dir_all(dir.join("runtime")).unwrap();
        let marker = dir.join("runtime").join("emulator.pid");
        // This test process is not an emulator, so the pid is treated as gone.
        std::fs::write(&marker, std::process::id().to_string()).unwrap();

        let runtime = EmulatorRuntime::new(dir.clone(), Arc::new(NoopEvents));
        assert!(!runtime.is_owned().await);
        runtime.adopt_ownership();
        assert!(marker.exists(), "the marker stays until the emulator stops");
        assert!(
            runtime.is_owned().await,
            "the marker is the ownership claim"
        );
        runtime.set_stop_on_exit(false);
        assert!(!runtime.stop_on_exit());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn without_a_pid_file_the_runtime_owns_nothing() {
        let dir = owner_dir("none");
        std::fs::create_dir_all(&dir).unwrap();
        let runtime = EmulatorRuntime::new(dir.clone(), Arc::new(NoopEvents));
        runtime.adopt_ownership();
        assert!(!runtime.is_owned().await);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn both_the_launcher_and_the_vm_count_as_the_emulator() {
        assert!(is_emulator_process("emulator.exe"));
        assert!(is_emulator_process("qemu-system-x86_64-headless.exe"));
        assert!(!is_emulator_process("chrome.exe"));
        assert!(!is_emulator_process(""));
    }

    fn contained(
        ipv4: bool,
        ipv6: bool,
        probe: Option<bool>,
        detail: Option<&str>,
    ) -> HostIsolation {
        HostIsolation {
            ipv4,
            ipv6,
            probe_blocked: probe,
            detail: detail.map(str::to_string),
        }
    }

    #[test]
    fn containment_is_reported_as_measured_not_as_intended() {
        // Installed on both families and observed refused: the strong claim.
        let active = isolation_note(Some(&contained(true, true, Some(true), None)));
        assert!(
            active.contains("Apps cannot reach Windows services"),
            "{active}"
        );
        assert!(active.contains("measured from inside Android"), "{active}");

        // Rules present, but the connection still got through: not contained.
        let leaked = isolation_note(Some(&contained(true, true, Some(false), None)));
        assert!(
            leaked.starts_with("Guest access to Windows loopback is NOT contained"),
            "{leaked}"
        );
        assert!(leaked.contains("still reached the adb port"), "{leaked}");

        // Rules present, probe impossible: unverified, never "contained".
        let unprobed = isolation_note(Some(&contained(true, true, None, None)));
        assert!(unprobed.contains("could not be probed"), "{unprobed}");
        assert!(!unprobed.contains("Apps cannot reach"), "{unprobed}");

        // The IPv6 gateway alone counts as a failure, and the reason is carried.
        let half = isolation_note(Some(&contained(
            true,
            false,
            Some(true),
            Some("adbd cannot run as root in production builds"),
        )));
        assert!(half.contains("NOT contained"), "{half}");
        assert!(half.contains("cannot run as root"), "{half}");

        // Nothing installed yet is said plainly, not implied to be safe.
        let before = isolation_note(None);
        assert!(before.contains("nothing is installed yet"), "{before}");
    }

    #[test]
    fn typed_text_survives_the_device_shell() {
        // A literal space would end Android's `input text` argument here.
        assert_eq!(device_text_argument("hello world"), "hello%sworld");
        assert_eq!(device_text_argument("  two  "), "%s%stwo%s%s");
        // Enter and newline are key events, never text.
        assert_eq!(device_text_argument("a\nb\tc"), "abc");
        assert_eq!(device_text_argument("fdroid"), "fdroid");
    }

    #[test]
    fn a_window_droidwin_closed_is_not_a_crash() {
        // Killing scrcpy exits it with 1; reporting that as a crash would tell
        // every client the app died when the user closed the window.
        let event = classify_exit("org.example.game", true, Some(1), false);
        assert_eq!(event.kind, AppEventKind::Closed);
        assert!(event.reason.contains("closed in DroidWin"));
        assert_eq!(event.exit_code, Some(1));
    }

    #[test]
    fn a_window_host_that_died_by_itself_is_a_crash() {
        let event = classify_exit("org.example.game", false, Some(3), false);
        assert_eq!(event.kind, AppEventKind::Crashed);
        assert!(event.reason.contains("code 3"));
        assert_eq!(event.exit_code, Some(3));
    }

    #[test]
    fn a_clean_exit_and_a_vanished_process_are_both_closes() {
        assert_eq!(
            classify_exit("org.example.game", false, Some(0), false).kind,
            AppEventKind::Closed
        );
        let gone = classify_exit("org.example.game", false, None, true);
        assert_eq!(gone.kind, AppEventKind::Closed);
        assert!(gone.reason.contains("process disappeared"));
    }

    #[test]
    fn signature_conflicts_are_named_as_such() {
        let text = "Failure [INSTALL_FAILED_UPDATE_INCOMPATIBLE: Package org.example signatures do not match]";
        assert!(install_failure_summary(text).contains("different signature"));
    }

    #[test]
    fn missing_abi_is_not_reported_as_a_generic_failure() {
        assert!(
            install_failure_summary("INSTALL_FAILED_NO_MATCHING_ABIS: arm64-v8a")
                .contains("CPU architecture")
        );
    }

    #[test]
    fn new_sdk_requirement_suggests_the_version_mismatch() {
        assert!(
            install_failure_summary("INSTALL_FAILED_OLDER_SDK").contains("newer Android version")
        );
    }

    #[test]
    fn storage_exhaustion_is_distinguished_from_a_parse_error() {
        assert!(
            install_failure_summary("INSTALL_FAILED_INSUFFICIENT_STORAGE")
                .contains("out of storage")
        );
        assert!(
            install_failure_summary("INSTALL_PARSE_FAILED_MANIFEST_MALFORMED")
                .contains("could not parse")
        );
    }

    #[test]
    fn an_old_target_sdk_is_named_instead_of_blamed_on_the_archive() {
        let summary = install_failure_summary(
            "Failure [INSTALL_FAILED_DEPRECATED_SDK_VERSION: App package must target at least SDK version 23, but found 17]",
        );
        assert!(summary.contains("target SDK below 23"));
    }

    #[test]
    fn unknown_failures_stay_generic_instead_of_inventing_a_cause() {
        let summary = install_failure_summary("something unexpected happened");
        assert_eq!(summary, "Android rejected the installation.");
    }
}
