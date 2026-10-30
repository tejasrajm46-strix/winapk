//! Triggers: an action or a whole workflow that fires for one named app.
//!
//! Three kinds of trigger, one execution path:
//!
//! ```text
//! explicit      fire_trigger ─┐
//! app lifecycle opened/closed/crashed ─┤──▶ same workflow runner ──▶ same tools
//! state based   element appears, app is foreground, runtime fails ─┘
//! ```
//!
//! Only lifecycle facts WINAPK itself observes are used: the window host
//! starting, exiting and exiting with a failure code, plus the runtime's own
//! status. Nothing here guesses at an app's state from a picture or a timer.
//!
//! Every fire runs through `ControlService::dispatch`, so a trigger cannot reach
//! Android by a path an agent call could not. Firing is single-flight per
//! trigger, and the engine stops with the agent endpoint: no trigger keeps a
//! worker alive after automation is switched off.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::control::{Cancel, ControlService};
use super::workflow::{self, Step, Workflow};
use super::ActionLog;
use crate::domain::{now_ms, DwError, DwResult, ErrorCode, RuntimeState};
use crate::runtime::{AppEvent, AppEventKind, EmulatorRuntime, EventBus};
use crate::store::{Store, StoredTrigger};

/// What makes a trigger fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenKind {
    /// Only a `fire_trigger` call. The trigger exists to be reused on demand.
    Manual,
    AppOpened,
    AppClosed,
    /// The window host exited with a failure code.
    AppCrashed,
    /// The runtime was ready and then stopped or became unhealthy.
    RuntimeFailed,
    /// State based: an element matching the selector is on the display.
    ElementPresent,
    /// State based: no such element is on the display.
    ElementAbsent,
    /// State based: the package owns the focused window on its display.
    AppForeground,
}

impl WhenKind {
    pub fn as_str(self) -> &'static str {
        match self {
            WhenKind::Manual => "manual",
            WhenKind::AppOpened => "app_opened",
            WhenKind::AppClosed => "app_closed",
            WhenKind::AppCrashed => "app_crashed",
            WhenKind::RuntimeFailed => "runtime_failed",
            WhenKind::ElementPresent => "element_present",
            WhenKind::ElementAbsent => "element_absent",
            WhenKind::AppForeground => "app_foreground",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value.trim().to_ascii_lowercase().as_str() {
            "manual" => WhenKind::Manual,
            "app_opened" => WhenKind::AppOpened,
            "app_closed" => WhenKind::AppClosed,
            "app_crashed" => WhenKind::AppCrashed,
            "runtime_failed" => WhenKind::RuntimeFailed,
            "element_present" => WhenKind::ElementPresent,
            "element_absent" => WhenKind::ElementAbsent,
            "app_foreground" => WhenKind::AppForeground,
            _ => return None,
        })
    }

    /// Watched by polling the display, as opposed to pushed by the runtime.
    pub fn is_state_based(self) -> bool {
        matches!(
            self,
            WhenKind::ElementPresent | WhenKind::ElementAbsent | WhenKind::AppForeground
        )
    }

    /// Whether a trigger of this kind watches one package's lifecycle.
    fn needs_package(self) -> bool {
        !matches!(self, WhenKind::RuntimeFailed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    /// What a state-based trigger looks for. Ignored by the others.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<super::ui::Selector>,
    /// How often a state-based trigger is checked.
    #[serde(default = "default_interval")]
    pub interval_ms: u64,
    /// A display to check on, instead of the one the package owns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<u32>,
}

fn default_interval() -> u64 {
    1_000
}

impl Default for Condition {
    fn default() -> Self {
        Self {
            selector: None,
            interval_ms: default_interval(),
            display_id: None,
        }
    }
}

/// A trigger as a client sets it and reads it back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trigger {
    pub id: String,
    pub name: String,
    /// The app this trigger is about. Required except for `runtime_failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    pub when: WhenKind,
    #[serde(default)]
    pub condition: Condition,
    /// What to do when it fires. One step is an action trigger; several are a
    /// workflow.
    pub steps: Vec<Step>,
    pub enabled: bool,
    #[serde(default)]
    pub fire_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<i64>,
    #[serde(default)]
    pub created_at: i64,
}

impl Trigger {
    fn workflow(&self) -> Workflow {
        Workflow {
            name: Some(self.name.clone()),
            package_id: self.package_id.clone(),
            steps: self.steps.clone(),
        }
    }

    pub fn validate(&self) -> DwResult<()> {
        if self.name.trim().is_empty() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "A trigger needs a name.",
            ));
        }
        if self.when.needs_package()
            && self
                .package_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                format!(
                    "A {} trigger must name the package it watches.",
                    self.when.as_str()
                ),
            ));
        }
        if self.when == WhenKind::RuntimeFailed && self.package_id.is_some() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "runtime_failed is about the runtime, not one package, so it must not name a package.",
            )
            .detail("Use app_crashed for one app's failure."));
        }
        if self.when.is_state_based() {
            let Some(selector) = &self.condition.selector else {
                return Err(DwError::new(
                    ErrorCode::InvalidRequest,
                    format!("A {} trigger needs a selector.", self.when.as_str()),
                ));
            };
            if selector.is_empty() {
                return Err(DwError::new(
                    ErrorCode::InvalidRequest,
                    "The selector must have at least one field.",
                ));
            }
            if !(250..=60_000).contains(&self.condition.interval_ms) {
                return Err(DwError::new(
                    ErrorCode::InvalidConfig,
                    "interval_ms must be between 250 and 60000.",
                ));
            }
        }
        self.workflow().validate()
    }
}

/// One entry in the detector's feed: something WINAPK observed, whether or not
/// a trigger matched it. This is how a client checks the detector instead of
/// trusting that it works.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedEvent {
    pub at_ms: i64,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    pub detail: String,
    /// Trigger ids that fired because of this event.
    pub fired: Vec<String>,
}

pub struct TriggerEngine {
    runtime: Arc<EmulatorRuntime>,
    control: Arc<ControlService>,
    store: Arc<Store>,
    log: Arc<ActionLog>,
    bus: EventBus,
    events: Mutex<VecDeque<DetectedEvent>>,
    /// One fire at a time per trigger.
    in_flight: Mutex<HashMap<String, tokio::task::JoinHandle<()>>>,
    /// Edge detection for state-based triggers: what the last check saw.
    last_state: Mutex<HashMap<String, bool>>,
    last_checked: Mutex<HashMap<String, std::time::Instant>>,
    last_runtime: Mutex<Option<RuntimeState>>,
    worker: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    counter: AtomicU64,
}

impl TriggerEngine {
    pub fn new(
        runtime: Arc<EmulatorRuntime>,
        control: Arc<ControlService>,
        store: Arc<Store>,
        log: Arc<ActionLog>,
        bus: EventBus,
    ) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            control,
            store,
            log,
            bus,
            events: Mutex::new(VecDeque::new()),
            in_flight: Mutex::new(HashMap::new()),
            last_state: Mutex::new(HashMap::new()),
            last_checked: Mutex::new(HashMap::new()),
            last_runtime: Mutex::new(None),
            worker: tokio::sync::Mutex::new(None),
            counter: AtomicU64::new(0),
        })
    }

    fn new_id(&self) -> String {
        format!(
            "trg-{}-{}",
            now_ms(),
            self.counter.fetch_add(1, Ordering::Relaxed)
        )
    }

    // ---- persistence -----------------------------------------------------

    pub fn list(&self) -> Vec<Trigger> {
        self.store.triggers().into_iter().map(from_stored).collect()
    }

    pub fn get(&self, id: &str) -> Option<Trigger> {
        self.store.trigger(id).map(from_stored)
    }

    /// Create or replace. An update keeps the id and therefore the fire history.
    pub fn save(&self, mut trigger: Trigger) -> DwResult<Trigger> {
        trigger.name = trigger.name.trim().to_string();
        trigger.validate()?;
        let existing = if trigger.id.is_empty() {
            None
        } else {
            self.store.trigger(&trigger.id)
        };
        match existing {
            Some(stored) => {
                trigger.created_at = stored.created_at;
                trigger.fire_count = stored.fire_count;
                trigger.last_fired_at = stored.last_fired_at;
            }
            None => {
                trigger.id = self.new_id();
                trigger.created_at = now_ms();
                trigger.fire_count = 0;
                trigger.last_fired_at = None;
            }
        }
        self.store.save_trigger(&to_stored(&trigger))?;
        Ok(trigger)
    }

    pub fn remove(&self, id: &str) -> DwResult<bool> {
        self.abort_fire(id);
        let removed = self.store.remove_trigger(id)?;
        self.forget(id);
        Ok(removed)
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> DwResult<Trigger> {
        let mut trigger = self.get(id).ok_or_else(|| unknown_trigger(id))?;
        trigger.enabled = enabled;
        if !enabled {
            self.abort_fire(id);
            self.forget(id);
        }
        self.store.save_trigger(&to_stored(&trigger))?;
        Ok(trigger)
    }

    fn forget(&self, id: &str) {
        if let Ok(mut state) = self.last_state.lock() {
            state.remove(id);
        }
        if let Ok(mut checked) = self.last_checked.lock() {
            checked.remove(id);
        }
    }

    // ---- worker ----------------------------------------------------------

    /// Start the detector. Called when the agent endpoint starts and once for a
    /// trigger fired by hand, so state triggers work whether or not automation
    /// was restored.
    pub async fn start(self: &Arc<Self>) {
        let mut worker = self.worker.lock().await;
        if worker.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        let mut receiver = self.bus.subscribe();
        let engine = Arc::clone(self);
        let task = tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(1_000));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    event = receiver.recv() => match event {
                        Ok(event) => engine.on_app_event(event).await,
                        // A slow consumer is not a reason to stop detecting; the
                        // feed is bounded and the newest events are what matter.
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(missed)) => {
                            tracing::debug!("trigger detector missed {missed} app event(s)");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    _ = tick.tick() => engine.on_tick().await,
                }
            }
        });
        *worker = Some(task);
    }

    /// Stop the detector and any fire still running. Called when the endpoint
    /// stops, so nothing outlives the automation it belongs to.
    pub async fn stop(&self) {
        let mut worker = self.worker.lock().await;
        if let Some(task) = worker.take() {
            task.abort();
        }
        let handles: Vec<tokio::task::JoinHandle<()>> = self
            .in_flight
            .lock()
            .map(|mut in_flight| in_flight.drain().map(|(_, handle)| handle).collect())
            .unwrap_or_default();
        for handle in handles {
            handle.abort();
        }
    }

    async fn on_app_event(self: &Arc<Self>, event: AppEvent) {
        let when = match event.kind {
            AppEventKind::Opened => WhenKind::AppOpened,
            AppEventKind::Closed => WhenKind::AppClosed,
            AppEventKind::Crashed => WhenKind::AppCrashed,
        };
        let mut fired = Vec::new();
        for trigger in self.matching(when, Some(&event.package_id)) {
            if self.fire_in_background(&trigger.id, &event.reason) {
                fired.push(trigger.id);
            }
        }
        self.record(DetectedEvent {
            at_ms: event.at_ms,
            kind: when.as_str().to_string(),
            package_id: Some(event.package_id.clone()),
            detail: format!(
                "{} ({})",
                event.reason,
                event
                    .exit_code
                    .map(|code| format!("exit {code}"))
                    .unwrap_or_else(|| "still running".into())
            ),
            fired,
        });
    }

    async fn on_tick(self: &Arc<Self>) {
        let triggers = self.list();
        self.check_runtime(&triggers).await;
        for trigger in triggers
            .iter()
            .filter(|trigger| trigger.enabled && trigger.when.is_state_based())
        {
            if !self.due(&trigger.id, trigger.condition.interval_ms) {
                continue;
            }
            let reached = self.evaluate(trigger).await;
            let previous = self
                .last_state
                .lock()
                .ok()
                .and_then(|mut state| state.insert(trigger.id.clone(), reached))
                .unwrap_or(false);
            if reached && !previous {
                let mut fired = Vec::new();
                if self.fire_in_background(&trigger.id, "its condition became true") {
                    fired.push(trigger.id.clone());
                }
                self.record(DetectedEvent {
                    at_ms: now_ms(),
                    kind: trigger.when.as_str().to_string(),
                    package_id: trigger.package_id.clone(),
                    detail: format!("{} became true on {}", trigger.when.as_str(), trigger.name),
                    fired,
                });
            }
        }
    }

    /// Runtime failure is a transition, not a state: an emulator that was ready
    /// and is now stopped or unhealthy is a failure worth telling a client about.
    async fn check_runtime(self: &Arc<Self>, triggers: &[Trigger]) {
        let mut watched: Vec<&Trigger> = triggers
            .iter()
            .filter(|trigger| trigger.enabled && trigger.when == WhenKind::RuntimeFailed)
            .collect();
        if watched.is_empty() {
            return;
        }
        let status = self.runtime.status(&self.store).await;
        let previous = self
            .last_runtime
            .lock()
            .map(|mut last| last.replace(status.state))
            .unwrap_or(None);
        let failed = previous == Some(RuntimeState::Ready)
            && matches!(
                status.state,
                RuntimeState::NotRunning | RuntimeState::Unhealthy
            );
        if !failed {
            return;
        }
        watched.retain(|trigger| trigger.enabled);
        let mut fired = Vec::new();
        for trigger in &watched {
            if self.fire_in_background(&trigger.id, "the runtime stopped being ready") {
                fired.push(trigger.id.clone());
            }
        }
        self.record(DetectedEvent {
            at_ms: now_ms(),
            kind: WhenKind::RuntimeFailed.as_str().to_string(),
            package_id: None,
            detail: format!(
                "the runtime was ready and is now {} ({})",
                status.state.as_label().to_lowercase(),
                status.detail
            ),
            fired,
        });
    }

    fn due(&self, id: &str, interval_ms: u64) -> bool {
        let Ok(mut checked) = self.last_checked.lock() else {
            return false;
        };
        let now = std::time::Instant::now();
        match checked.get(id) {
            Some(last) if now.duration_since(*last) < Duration::from_millis(interval_ms) => false,
            _ => {
                checked.insert(id.to_string(), now);
                true
            }
        }
    }

    /// One state check. A failure to look (adb unreachable, no display) counts as
    /// "not true": a trigger must not fire on an error it cannot see.
    async fn evaluate(&self, trigger: &Trigger) -> bool {
        let Some(package_id) = trigger.package_id.as_deref() else {
            return false;
        };
        match trigger.when {
            WhenKind::ElementPresent | WhenKind::ElementAbsent => {
                let Some(selector) = &trigger.condition.selector else {
                    return false;
                };
                match self
                    .control
                    .has_element(package_id, trigger.condition.display_id, selector)
                    .await
                {
                    Ok(present) => present == (trigger.when == WhenKind::ElementPresent),
                    Err(err) => {
                        tracing::debug!(
                            "trigger {} could not check its element: {}",
                            trigger.id,
                            err.summary
                        );
                        false
                    }
                }
            }
            WhenKind::AppForeground => {
                let foreground = self
                    .control
                    .observe(Some(package_id), trigger.condition.display_id, "trigger")
                    .await
                    .ok()
                    .and_then(|value| {
                        value
                            .get("foreground_package")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });
                foreground.as_deref() == Some(package_id)
            }
            _ => false,
        }
    }

    fn matching(&self, when: WhenKind, package_id: Option<&str>) -> Vec<Trigger> {
        self.list()
            .into_iter()
            .filter(|trigger| trigger.enabled && trigger.when == when)
            .filter(
                |trigger| match (package_id, trigger.package_id.as_deref()) {
                    (Some(event_package), Some(watched)) => event_package == watched,
                    (Some(_), None) => true,
                    (None, _) => false,
                },
            )
            .collect()
    }

    fn record(&self, event: DetectedEvent) {
        if let Ok(mut events) = self.events.lock() {
            events.push_front(event);
            while events.len() > 200 {
                events.pop_back();
            }
        }
    }

    pub fn events(&self, limit: usize) -> Vec<DetectedEvent> {
        self.events
            .lock()
            .map(|events| events.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    // ---- firing ----------------------------------------------------------

    /// Fire one trigger and wait for its workflow. This is the explicit path and
    /// the one clients use to test a trigger.
    pub async fn fire(self: &Arc<Self>, id: &str, cancel: &Cancel) -> DwResult<Value> {
        let trigger = self.get(id).ok_or_else(|| unknown_trigger(id))?;
        self.start().await;
        self.run(&trigger, "fired by an explicit request", cancel)
            .await
    }

    fn fire_in_background(self: &Arc<Self>, id: &str, reason: &str) -> bool {
        let engine = Arc::clone(self);
        let id = id.to_string();
        let reason = reason.to_string();
        {
            let Ok(mut in_flight) = self.in_flight.lock() else {
                return false;
            };
            // Single flight: a trigger already running does not start a second
            // copy of its workflow because a second event arrived.
            if in_flight.contains_key(&id) {
                return false;
            }
            let key = id.clone();
            let handle = tokio::spawn(async move {
                let Some(trigger) = engine.get(&key) else {
                    return;
                };
                let result = engine.run(&trigger, &reason, &Cancel::new()).await;
                if let Ok(mut in_flight) = engine.in_flight.lock() {
                    in_flight.remove(&key);
                }
                match result {
                    Ok(value) => tracing::info!(
                        "trigger {key} fired: dispatched {} step(s), verified={}",
                        value
                            .get("dispatched_count")
                            .and_then(|v| v.as_u64())
                            .unwrap_or(0),
                        value
                            .get("verified")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false)
                    ),
                    Err(err) => tracing::warn!("trigger {key} failed: {}", err.summary),
                }
            });
            in_flight.insert(id, handle);
        }
        true
    }

    fn abort_fire(&self, id: &str) {
        if let Ok(mut in_flight) = self.in_flight.lock() {
            if let Some(handle) = in_flight.remove(id) {
                handle.abort();
            }
        }
    }

    /// Run a trigger's steps. Automation that is paused or read-only never gets
    /// this far: the caller checks first, and the check is repeated here so a
    /// background fire cannot bypass it.
    async fn run(
        self: &Arc<Self>,
        trigger: &Trigger,
        reason: &str,
        cancel: &Cancel,
    ) -> DwResult<Value> {
        let started = now_ms();
        let result = workflow::run(&self.control, &trigger.workflow(), cancel).await;
        let (ok, summary) = match &result {
            Ok(value) => (
                true,
                format!(
                    "trigger “{}”: {} of {} step(s) dispatched",
                    trigger.name,
                    value
                        .get("dispatched_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    trigger.steps.len()
                ),
            ),
            Err(err) => (false, err.summary.clone()),
        };
        self.log.record(super::ActionRecord {
            id: self.log.next_id(),
            at_ms: started,
            tool: format!("trigger:{}", trigger.name),
            ok,
            summary,
            duration_ms: (now_ms() - started).max(0) as u64,
            display_id: None,
            package_id: trigger.package_id.clone(),
        });
        let _ = self.store.record_trigger_fired(&trigger.id, now_ms());
        let mut value = result?;
        value["trigger_id"] = json!(trigger.id);
        value["reason"] = json!(reason);
        Ok(value)
    }
}

fn unknown_trigger(id: &str) -> DwError {
    DwError::new(
        ErrorCode::InvalidRequest,
        format!("There is no trigger with id {id}."),
    )
}

fn to_stored(trigger: &Trigger) -> StoredTrigger {
    StoredTrigger {
        id: trigger.id.clone(),
        name: trigger.name.clone(),
        package_id: trigger.package_id.clone(),
        when_kind: trigger.when.as_str().to_string(),
        condition: serde_json::to_string(&trigger.condition).unwrap_or_else(|_| "{}".into()),
        steps: serde_json::to_string(&trigger.steps).unwrap_or_else(|_| "[]".into()),
        enabled: trigger.enabled,
        fire_count: trigger.fire_count,
        last_fired_at: trigger.last_fired_at,
        created_at: trigger.created_at,
    }
}

fn from_stored(stored: StoredTrigger) -> Trigger {
    Trigger {
        id: stored.id,
        name: stored.name,
        package_id: stored.package_id,
        when: WhenKind::parse(&stored.when_kind).unwrap_or(WhenKind::Manual),
        condition: serde_json::from_str(&stored.condition).unwrap_or_default(),
        steps: serde_json::from_str(&stored.steps).unwrap_or_default(),
        enabled: stored.enabled,
        fire_count: stored.fire_count,
        last_fired_at: stored.last_fired_at,
        created_at: stored.created_at,
    }
}

/// Build a trigger from a tool call. Steps are parsed from the same shape the
/// workflow runner takes, so a client writes one kind of step.
pub fn trigger_from_args(arguments: &Value) -> DwResult<Trigger> {
    let steps: Vec<Step> = match arguments.get("steps") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|e| {
            DwError::new(
                ErrorCode::InvalidRequest,
                "Every step needs a tool name and an object of arguments.",
            )
            .detail(e.to_string())
        })?,
        None => {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "A trigger needs at least one step.",
            ))
        }
    };
    let when = arguments
        .get("when")
        .and_then(Value::as_str)
        .and_then(WhenKind::parse)
        .ok_or_else(|| {
            DwError::new(
                ErrorCode::InvalidRequest,
                "when must be manual, app_opened, app_closed, app_crashed, runtime_failed, element_present, element_absent or app_foreground.",
            )
        })?;
    let condition = arguments
        .get("condition")
        .map(|value| serde_json::from_value::<Condition>(value.clone()))
        .transpose()
        .map_err(|e| {
            DwError::new(ErrorCode::InvalidRequest, "condition is not valid.").detail(e.to_string())
        })?
        .unwrap_or_default();
    Ok(Trigger {
        id: arguments
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
        name: arguments
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        package_id: arguments
            .get("package_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        when,
        condition,
        steps,
        enabled: arguments
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        fire_count: 0,
        last_fired_at: None,
        created_at: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_trigger(when: WhenKind) -> Trigger {
        Trigger {
            id: String::new(),
            name: "watch the start button".into(),
            package_id: Some("org.asafonov.blockbuster".into()),
            when,
            condition: Condition {
                selector: serde_json::from_value(json!({ "text": "Start" })).unwrap(),
                interval_ms: 500,
                display_id: None,
            },
            steps: vec![Step {
                tool: "tap".into(),
                arguments: json!({ "x": 1200, "y": 870 }),
                verify: None,
            }],
            enabled: true,
            fire_count: 0,
            last_fired_at: None,
            created_at: 0,
        }
    }

    #[test]
    fn a_lifecycle_trigger_must_name_the_package_it_watches() {
        let mut trigger = state_trigger(WhenKind::AppOpened);
        trigger.condition = Condition::default();
        trigger.package_id = None;
        assert_eq!(
            trigger.validate().unwrap_err().code,
            ErrorCode::InvalidRequest
        );

        trigger.package_id = Some("org.example".into());
        assert!(trigger.validate().is_ok());
    }

    #[test]
    fn a_state_trigger_needs_a_selector_and_a_sane_interval() {
        let mut trigger = state_trigger(WhenKind::ElementPresent);
        trigger.condition.selector = None;
        assert!(trigger.validate().is_err());

        trigger.condition.selector = Some(serde_json::from_value(json!({})).unwrap());
        assert!(trigger.validate().is_err());

        trigger.condition.selector = serde_json::from_value(json!({ "text": "Start" })).unwrap();
        trigger.condition.interval_ms = 10;
        assert!(trigger.validate().is_err());

        trigger.condition.interval_ms = 250;
        assert!(trigger.validate().is_ok());
    }

    #[test]
    fn runtime_failure_is_about_the_runtime_not_a_package() {
        let mut trigger = state_trigger(WhenKind::RuntimeFailed);
        trigger.condition = Condition::default();
        trigger.package_id = Some("org.example".into());
        assert!(trigger.validate().is_err());

        trigger.package_id = None;
        assert!(trigger.validate().is_ok());
    }

    #[test]
    fn a_trigger_without_steps_is_not_accepted() {
        let mut trigger = state_trigger(WhenKind::AppOpened);
        trigger.condition = Condition::default();
        trigger.steps.clear();
        assert!(trigger.validate().is_err());
    }

    #[test]
    fn stored_rows_round_trip_through_the_same_shape_a_client_sends() {
        let trigger = state_trigger(WhenKind::ElementPresent);
        let stored = to_stored(&trigger);
        assert_eq!(stored.when_kind, "element_present");
        let back = from_stored(stored);
        assert_eq!(back.when, WhenKind::ElementPresent);
        assert_eq!(back.package_id, trigger.package_id);
        assert_eq!(back.steps.len(), 1);
        assert_eq!(back.condition.interval_ms, 500);
    }

    #[test]
    fn an_unknown_when_kind_is_not_silently_manual() {
        assert_eq!(
            WhenKind::parse("element_present"),
            Some(WhenKind::ElementPresent)
        );
        assert_eq!(WhenKind::parse("vibes"), None);
        assert_eq!(WhenKind::parse("APP_OPENED"), Some(WhenKind::AppOpened));
    }

    #[test]
    fn arguments_become_a_trigger_or_an_actionable_error() {
        let args = json!({
            "name": "start the game when it opens",
            "package_id": "org.asafonov.blockbuster",
            "when": "app_opened",
            "steps": [{ "tool": "tap", "arguments": { "x": 1200, "y": 870 } }]
        });
        let trigger = trigger_from_args(&args).unwrap();
        assert_eq!(trigger.when, WhenKind::AppOpened);
        assert!(trigger.enabled);
        assert!(trigger.validate().is_ok());

        let missing = trigger_from_args(&json!({ "when": "app_opened", "steps": [] })).unwrap();
        assert!(missing.validate().is_err());

        assert!(trigger_from_args(&json!({ "name": "x", "when": "soon" })).is_err());
    }
}
