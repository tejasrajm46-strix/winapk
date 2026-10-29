//! Runtime status and environment report. Capabilities describe what this
//! backend has actually been verified to do — not what the backend class implies.

use serde::{Deserialize, Serialize};

/// One still of the device's own screen, as the virtual phone shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhoneScreen {
    /// The display this still came from — the device's own, not an app's.
    pub display_id: u32,
    /// Capture size in device pixels. The phone frame only scales this image, so
    /// a click is turned back into a device coordinate with these numbers.
    pub width: u32,
    pub height: u32,
    /// `data:image/png;base64,…`, the shape app icons already travel in.
    pub image: String,
    pub captured_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    /// Required pieces are missing (SDK, emulator, system image, AVD, scrcpy).
    NotConfigured,
    NotRunning,
    Starting,
    Ready,
    /// Present but a probe failed (adb unreachable, boot never completed, ...).
    Unhealthy,
}

impl RuntimeState {
    pub fn as_label(self) -> &'static str {
        match self {
            RuntimeState::NotConfigured => "Not configured",
            RuntimeState::NotRunning => "Stopped",
            RuntimeState::Starting => "Starting",
            RuntimeState::Ready => "Ready",
            RuntimeState::Unhealthy => "Unhealthy",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub install: bool,
    pub uninstall: bool,
    pub enumerate: bool,
    pub launch: bool,
    pub independent_windows: bool,
    pub concurrent_apps: bool,
    pub clipboard: bool,
    pub file_drop: bool,
    pub audio: bool,
    pub hardware_accel: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub backend: String,
    pub state: RuntimeState,
    /// Human sentence explaining the state. Empty when there is nothing to add.
    pub detail: String,
    pub avd: Option<String>,
    pub serial: Option<String>,
    pub android_api: Option<u32>,
    pub capabilities: Capabilities,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvReport {
    pub sdk_root: Option<String>,
    pub adb: Option<String>,
    pub emulator: Option<String>,
    pub avdmanager: Option<String>,
    pub sdkmanager: Option<String>,
    pub aapt2: Option<String>,
    pub scrcpy: Option<String>,
    pub system_image: Option<String>,
    pub avds: Vec<String>,
    pub accel: Option<String>,
    /// Concrete missing pieces, each naming the package that provides it.
    pub blocking: Vec<String>,
}

impl EnvReport {
    pub fn ready(&self) -> bool {
        self.blocking.is_empty()
    }
}

/// One Android runtime component as the setup screen lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetupComponent {
    pub id: String,
    pub label: String,
    pub present: bool,
    /// Where it is when present, or what is missing when it is not.
    pub detail: String,
    /// Download size for a missing component, when the source states one.
    pub download_mb: Option<u32>,
    pub required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    /// Every required component is present and adb has seen Android boot.
    Ready,
    /// Every required component is present, but no boot has been verified yet.
    Starting,
    /// Components are missing; they can be installed into the managed SDK.
    NeedsInstall,
    /// An installation is running right now.
    Installing,
    /// The last installation failed. The error is in the report.
    Failed,
}

/// What the setup screen renders. Every field is something that was checked, not
/// an intention: `present` is a filesystem answer and `state` follows from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupReport {
    pub state: SetupState,
    /// The SDK root DroidWin is using, or the one it would install into.
    pub sdk_root: String,
    /// True when that root is DroidWin's own directory, not a user SDK.
    pub managed: bool,
    /// True when the user chose this location instead of the default.
    pub configured: bool,
    pub components: Vec<SetupComponent>,
    /// Epoch-ms of the last completed installation, when one ran.
    pub installed_at: Option<i64>,
    /// The last failure this process saw, verbatim.
    pub error: Option<String>,
    /// Free space on the volume the SDK is (or would be) installed on.
    pub disk_free_mb: u64,
    /// What a full installation downloads, in MB. Known sizes only.
    pub download_mb: u64,
    pub licenses_accepted: bool,
    pub system_image: String,
    pub avd: String,
    pub message: String,
}

impl SetupReport {
    /// The components still needed to run an app, so an install can name them.
    pub fn missing(&self) -> Vec<&SetupComponent> {
        self.components
            .iter()
            .filter(|component| component.required && !component.present)
            .collect()
    }
}

/// Progress of a running installation, forwarded to the setup screen as it
/// happens. `percent` is only set where a real total is known (a download whose
/// length came from the server); everything else stays indeterminate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SetupProgress {
    /// Stable step id: `tools`, `licenses`, `packages`, `avd`, `validate`, `start`.
    pub step: String,
    pub message: String,
    pub percent: Option<f64>,
    pub done: bool,
    /// Set on the final event when the installation failed.
    pub failed: Option<String>,
}
