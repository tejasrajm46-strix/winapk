//! Domain types shared by the APK inspector, the runtime adapter, the store and
//! the command layer. No process, filesystem or Tauri access lives here.

pub mod apk;
pub mod app;
pub mod error;
pub mod operation;
pub mod runtime;

pub use apk::{ApkInspection, SignatureInfo};
pub use app::{AppWindow, InstalledApp};
pub use error::{DwError, DwResult, ErrorCode};
pub use operation::{now_ms, Stage, StageEvent, StageSink};
pub use runtime::{Capabilities, EnvReport, RuntimeState, RuntimeStatus};
