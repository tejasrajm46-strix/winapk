//! Runtime status and environment report. Capabilities describe what this
//! backend has actually been verified to do — not what the backend class implies.

use serde::{Deserialize, Serialize};

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
