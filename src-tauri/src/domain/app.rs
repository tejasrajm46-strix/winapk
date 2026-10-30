//! State of a package as the runtime reports it, plus WINAPK's own window state.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledApp {
    pub package_id: String,
    /// `None` when the label could not be resolved without inventing one.
    pub label: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    /// A WINAPK-launched window for this package is currently open.
    pub running: bool,
    pub window_title: Option<String>,
    pub icon: Option<String>,
    /// Third-party (non-system) package as reported by the runtime.
    pub user_installed: bool,
    /// Target SDK from the manifest at install time. None when WINAPK did not
    /// install the package and therefore never saw its manifest.
    pub target_sdk: Option<u32>,
}

/// The orientation an app's launchable activity locks itself to.
///
/// Read from `android:screenOrientation` in the manifest. No value means the
/// manifest does not restrict the orientation — a portrait surface is the default
/// then, and the runtime records what Android actually did after a launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppOrientation {
    Portrait,
    Landscape,
}

impl AppOrientation {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "portrait" => Some(AppOrientation::Portrait),
            "landscape" => Some(AppOrientation::Landscape),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AppOrientation::Portrait => "portrait",
            AppOrientation::Landscape => "landscape",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppWindow {
    pub package_id: String,
    pub title: String,
    pub pid: u32,
}
