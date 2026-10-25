//! The Android runtime boundary. Everything that knows how apps are actually
//! executed lives behind this module; the UI and command layer never build a
//! backend command themselves.

pub mod adb;
pub mod emulator;
pub mod emulator_runtime;
pub mod scrcpy;
pub mod sdk;

use serde::{Deserialize, Serialize};

use crate::domain::{RuntimeStatus, StageEvent};

pub use emulator_runtime::EmulatorRuntime;

/// Notifications the runtime produces as it works. The desktop app forwards
/// these to the webview; the proof harness prints them.
pub trait RuntimeEvents: Send + Sync {
    fn stage(&self, event: &StageEvent);
    fn status(&self, status: &RuntimeStatus);
    fn apps_changed(&self);
}

pub struct NoopEvents;

impl RuntimeEvents for NoopEvents {
    fn stage(&self, _event: &StageEvent) {}
    fn status(&self, _status: &RuntimeStatus) {}
    fn apps_changed(&self) {}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub package_id: String,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub updated: bool,
    pub message: String,
}
