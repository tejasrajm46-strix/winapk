//! State of a package as the runtime reports it, plus DroidWin's own window state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledApp {
    pub package_id: String,
    /// `None` when the label could not be resolved without inventing one.
    pub label: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    /// A DroidWin-launched window for this package is currently open.
    pub running: bool,
    pub window_title: Option<String>,
    pub icon: Option<String>,
    /// Third-party (non-system) package as reported by the runtime.
    pub user_installed: bool,
    /// Target SDK from the manifest at install time. None when DroidWin did not
    /// install the package and therefore never saw its manifest.
    pub target_sdk: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppWindow {
    pub package_id: String,
    pub title: String,
    pub pid: u32,
}
