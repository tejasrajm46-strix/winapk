//! The Android runtime boundary. Everything that knows how apps are actually
//! executed lives behind this module; the UI and command layer never build a
//! backend command themselves.

pub mod adb;
pub mod display;
pub mod display_override;
pub mod emulator;
pub mod emulator_runtime;
pub mod hardware;
pub mod install;
pub mod phone_stream;
pub mod scrcpy;
pub mod sdk;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::domain::{RuntimeStatus, StageEvent};

pub use emulator_runtime::EmulatorRuntime;

/// What happened to one app window. These are DroidWin's own observations of a
/// window it owns: the window host started, exited, or exited badly. Nothing
/// here is inferred from a screenshot or invented by a timer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppEventKind {
    /// The window opened and its Android display was resolved.
    Opened,
    /// The window closed with a clean exit.
    Closed,
    /// The window host exited with a failure code, so the app died under it.
    Crashed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppEvent {
    pub package_id: String,
    pub kind: AppEventKind,
    /// Wall clock, so a consumer can order events from different sources.
    pub at_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// One sentence: why this is the event it is.
    pub reason: String,
}

impl AppEvent {
    pub fn new(
        package_id: impl Into<String>,
        kind: AppEventKind,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            package_id: package_id.into(),
            kind,
            at_ms: crate::domain::now_ms(),
            display_id: None,
            pid: None,
            exit_code: None,
            reason: reason.into(),
        }
    }

    pub fn at(mut self, display_id: Option<u32>, pid: Option<u32>) -> Self {
        self.display_id = display_id;
        self.pid = pid;
        self
    }

    pub fn exited(mut self, code: Option<i32>) -> Self {
        self.exit_code = code;
        self
    }
}

/// Notifications the runtime produces as it works. The desktop app forwards
/// these to the webview; the proof harness prints them.
pub trait RuntimeEvents: Send + Sync {
    fn stage(&self, event: &StageEvent);
    fn status(&self, status: &RuntimeStatus);
    fn apps_changed(&self);
    /// App lifecycle. Defaulted so an implementation that only cares about
    /// stages (a proof harness) does not have to write it.
    fn app_event(&self, _event: &AppEvent) {}
}

/// One ordered feed of app lifecycle events for anything inside the process that
/// needs to react to them (the agent trigger engine and diagnostics today).
/// Broadcast rather than a queue so a slow subscriber cannot stall the runtime.
///
/// A bounded history is kept alongside: diagnostics must be able to say what the
/// runtime observed *before* it was asked, and subscribing only at the moment of
/// a check would miss the crash that prompted it.
#[derive(Clone)]
pub struct EventBus {
    sender: tokio::sync::broadcast::Sender<AppEvent>,
    history: Arc<Mutex<VecDeque<AppEvent>>>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = tokio::sync::broadcast::channel(256);
        Self {
            sender,
            history: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn publish(&self, event: AppEvent) {
        if let Ok(mut history) = self.history.lock() {
            history.push_front(event.clone());
            history.truncate(100);
        }
        // A send with no subscriber is not an error: the bus is optional.
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<AppEvent> {
        self.sender.subscribe()
    }

    /// The most recent events, newest first.
    pub fn recent(&self, limit: usize) -> Vec<AppEvent> {
        self.history
            .lock()
            .map(|history| history.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    /// The last thing the runtime observed about one package.
    pub fn last_for(&self, package_id: &str) -> Option<AppEvent> {
        self.history
            .lock()
            .ok()?
            .iter()
            .find(|event| event.package_id == package_id)
            .cloned()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Publishes app lifecycle onto a bus and nothing else. The desktop's event
/// bridge also forwards to its webview; a headless harness only needs the bus.
pub struct BusEvents {
    pub bus: EventBus,
}

impl BusEvents {
    pub fn new(bus: EventBus) -> Self {
        Self { bus }
    }
}

impl RuntimeEvents for BusEvents {
    fn stage(&self, _event: &StageEvent) {}
    fn status(&self, _status: &RuntimeStatus) {}
    fn apps_changed(&self) {}
    fn app_event(&self, event: &AppEvent) {
        self.bus.publish(event.clone());
    }
}

pub struct NoopEvents;

impl RuntimeEvents for NoopEvents {
    fn stage(&self, _event: &StageEvent) {}
    fn status(&self, _status: &RuntimeStatus) {}
    fn apps_changed(&self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(package_id: &str, kind: AppEventKind) -> AppEvent {
        AppEvent::new(package_id, kind, "observed")
    }

    #[test]
    fn the_bus_keeps_a_bounded_newest_first_history() {
        let bus = EventBus::new();
        for index in 0..120 {
            bus.publish(event(&format!("org.example.{index}"), AppEventKind::Opened));
        }
        let recent = bus.recent(5);
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].package_id, "org.example.119");
        assert!(bus.recent(500).len() <= 100);
    }

    #[test]
    fn last_for_finds_the_latest_event_of_one_package() {
        let bus = EventBus::new();
        bus.publish(event("com.a", AppEventKind::Opened));
        bus.publish(event("com.b", AppEventKind::Opened));
        bus.publish(event("com.a", AppEventKind::Crashed));
        assert_eq!(
            bus.last_for("com.a").map(|event| event.kind),
            Some(AppEventKind::Crashed)
        );
        assert!(bus.last_for("com.missing").is_none());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallOutcome {
    pub package_id: String,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub updated: bool,
    pub message: String,
}
