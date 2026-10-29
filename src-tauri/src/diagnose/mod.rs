//! "Attention Needed": what DroidWin actually observed going wrong, separated
//! into confirmed failures and suspicions, each with the evidence behind it and
//! one recovery that touches as little as possible.
//!
//! Nothing here invents a problem. Every diagnosis is built from a signal the
//! runtime already produces:
//!
//! - the environment probe (`EnvReport`) for missing SDK pieces,
//! - the runtime state and its detail sentence,
//! - the windows this process owns, their recorded geometry and display ids,
//! - Android's own `pidof` and display dump for those windows,
//! - the app lifecycle events the runtime published (including the
//!   close-versus-crash classification),
//! - the operation history for a launch that already failed,
//! - and two screenshots of a live window, decoded only to answer one question:
//!   is every pixel the same colour?
//!
//! A *confirmed* problem is backed by a direct observation: a process that is
//! gone, an event with an exit code, a probe's output, two screenshots whose
//! pixels are all one colour. A *suspected* problem is something DroidWin could
//! not observe well enough to be sure — a capture that failed, a log that stayed
//! silent — and it says so instead of presenting a guess as a fact.
//!
//! Recovery is targeted by construction: reopening one window, retrying one
//! launch or reinstalling one package never restarts the emulator and never
//! touches another running app. Restarting the whole runtime exists, is offered
//! only for a failure that cannot be narrowed to an app, and is labelled as the
//! last resort in the UI.
//!
//! Probes are single-flight (one report at a time) and cached, so a screen that
//! refreshes cannot turn diagnostics into the polling problem the runtime just
//! removed. Screenshots are only taken on an explicit deep check, at most once
//! per package per [`RENDER_TTL`].

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::domain::{now_ms, DwError, DwResult, EnvReport, ErrorCode, RuntimeState, RuntimeStatus};
use crate::runtime::{AppEvent, AppEventKind, EmulatorRuntime, EventBus};
use crate::store::{CachedApp, OperationRecord, Store};

/// How long a report may be reused. Long enough that a re-render of the UI does
/// not probe the device again, short enough that the section is not stale.
const CACHE_TTL: Duration = Duration::from_secs(5);
/// A screenshot pair is expensive (a display dump and two captures), so it is
/// kept per package far longer than the rest of the report.
const RENDER_TTL: Duration = Duration::from_secs(120);
/// The gap between the two captures that decide whether a window is drawing.
const SCREENSHOT_GAP: Duration = Duration::from_millis(700);
/// How many of the newest operations are considered for "the last attempt".
const OPERATION_WINDOW: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// Directly observed.
    Confirmed,
    /// A clue DroidWin could not confirm.
    Suspected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Nothing is broken; the user may want to know.
    Notice,
    /// It works, but worse than it should.
    Degraded,
    /// The thing cannot be used until this is resolved.
    Blocking,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Environment,
    Runtime,
    App,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// `probe`, `log`, `event`, `screenshot` or `state`.
    pub source: String,
    pub detail: String,
}

impl Evidence {
    fn new(source: &str, detail: impl Into<String>) -> Self {
        Self {
            source: source.to_string(),
            detail: detail.into(),
        }
    }
}

/// One recovery a user (or an agent) can run for a problem. Each variant names
/// exactly what it touches; none of the per-app ones restart the emulator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recovery {
    /// Boot the Android runtime. Does nothing to a runtime that is already up.
    StartRuntime,
    /// Close and reopen one app's window, recreating its display.
    ReopenWindow { package_id: String },
    /// Stop one app and open it again in a fresh window.
    RetryLaunch { package_id: String },
    /// Re-register one package with the runtime, keeping its data.
    ReinstallPackage { package_id: String },
    /// The last resort: stops every app window and the emulator, then boots.
    RestartRuntime,
    /// No state change; run the check again.
    Recheck,
    /// Nothing can be done automatically.
    None,
}

impl Recovery {
    pub fn package_id(&self) -> Option<&str> {
        match self {
            Recovery::ReopenWindow { package_id }
            | Recovery::RetryLaunch { package_id }
            | Recovery::ReinstallPackage { package_id } => Some(package_id),
            _ => None,
        }
    }

    /// The command name recorded in the operation history.
    fn operation_kind(&self) -> &'static str {
        match self {
            Recovery::StartRuntime => "recovery-start",
            Recovery::ReopenWindow { .. } => "recovery-reopen",
            Recovery::RetryLaunch { .. } => "recovery-retry-launch",
            Recovery::ReinstallPackage { .. } => "recovery-reinstall",
            Recovery::RestartRuntime => "recovery-restart",
            Recovery::Recheck | Recovery::None => "recovery-check",
        }
    }
}

/// Something that needs attention, with the observation behind it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Problem {
    /// Stable across refreshes, so the UI can key on it.
    pub id: String,
    pub scope: Scope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
    pub severity: Severity,
    pub confidence: Confidence,
    pub title: String,
    /// What DroidWin measured, in one or two sentences.
    pub observed: String,
    /// The probe output, log lines or event that backs the observation.
    pub evidence: Vec<Evidence>,
    pub recommendation: String,
    pub recovery: Recovery,
    /// Further targeted steps, offered next to the primary one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Recovery>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub checked_at_ms: i64,
    /// Whether screenshots were taken. A cheap report never claims a render
    /// observation it did not make.
    pub deep: bool,
    pub problems: Vec<Problem>,
    pub confirmed: usize,
    pub suspected: usize,
    /// What this pass looked at, newest first, for the "what was checked" line.
    pub checked: Vec<String>,
}

impl Report {
    fn empty(deep: bool) -> Self {
        Self {
            checked_at_ms: now_ms(),
            deep,
            problems: Vec::new(),
            confirmed: 0,
            suspected: 0,
            checked: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RecoveryOutcome {
    pub ok: bool,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The state immediately after the attempt, so the UI does not need a
    /// second round trip to learn whether the problem is gone.
    pub report: Report,
}

/// What two captures of one window's display showed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderObservation {
    pub display_id: u32,
    pub width: u32,
    pub height: u32,
    pub first_hash: u64,
    pub second_hash: u64,
    pub first_bytes: usize,
    pub second_bytes: usize,
    /// The single colour every pixel had, when the frame was uniform.
    pub uniform: Option<[u8; 3]>,
    pub unchanged: bool,
    /// When the pair was captured.
    pub at_ms: i64,
}

pub struct Diagnoser {
    runtime: Arc<EmulatorRuntime>,
    store: Arc<Store>,
    bus: EventBus,
    /// One report at a time. Two callers asking at once share the first answer
    /// rather than each probing the device.
    gate: tokio::sync::Mutex<()>,
    /// The token records what the answer depended on (the open windows and the
    /// newest lifecycle event) so a cached report cannot outlive them. A window
    /// lost a moment ago must never be reported as still there.
    cache: Mutex<Option<(Instant, bool, String, Report)>>,
    renders: Mutex<HashMap<String, (u32, Instant, RenderObservation)>>,
}

impl Diagnoser {
    pub fn new(runtime: Arc<EmulatorRuntime>, store: Arc<Store>, bus: EventBus) -> Arc<Self> {
        Arc::new(Self {
            runtime,
            store,
            bus,
            gate: tokio::sync::Mutex::new(()),
            cache: Mutex::new(None),
            renders: Mutex::new(HashMap::new()),
        })
    }

    /// Build the current picture. `deep` additionally captures each live
    /// window's display twice to decide whether it is drawing at all.
    pub async fn report(&self, deep: bool) -> Report {
        let _single = self.gate.lock().await;
        let token = self.state_token();
        if let Some(report) = self.cached(deep, &token) {
            return report;
        }

        let mut problems: Vec<Problem> = Vec::new();
        let mut checked: Vec<String> = Vec::new();

        let env = self.runtime.probe();
        checked.push(describe_env(&env));
        problems.extend(environment_problems(&env));

        let status = self.runtime.status(&self.store).await;
        checked.push(format!(
            "runtime: {} — {}",
            status.state.as_label(),
            status.detail
        ));
        // One read of the app list this process already holds: it answers both
        // "does DroidWin know any apps" and "is the package from an old failure
        // still installed", without touching the device again.
        let apps = self.store.cached_apps();
        let apps_known = !apps.is_empty();
        let known_apps = if apps.is_empty() { None } else { Some(apps) };
        if let Some(problem) = status_problem(&status, apps_known) {
            problems.push(problem);
        }

        let windows = self.runtime.open_windows();
        checked.push(format!(
            "windows: {} open{}",
            windows.len(),
            if windows.is_empty() {
                String::new()
            } else {
                format!(
                    " ({})",
                    windows
                        .iter()
                        .map(|window| window.package_id.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
        ));

        let adb = self.runtime.adb();
        let serial = self.runtime.serial();
        let mut per_package: HashMap<String, Problem> = HashMap::new();

        for window in &windows {
            let package = window.package_id.clone();
            if let Some(problem) = self.window_problem(adb.as_ref(), &serial, window).await {
                upsert(&mut per_package, problem);
            }

            if !deep {
                continue;
            }
            let Some(display_id) = window.display_id else {
                continue;
            };
            if adb.is_none() {
                continue;
            }
            match self.observe_render(&package, display_id).await {
                Ok(observation) => {
                    checked.push(describe_render(&package, &observation));
                    if observation.uniform.is_some() {
                        // Only a uniform frame is worth asking the log about: one
                        // bounded read, and only when there is something to
                        // explain.
                        let log = match adb.as_ref() {
                            Some(adb) => adb.log_excerpt(&serial, &package, 6).await,
                            None => None,
                        };
                        if let Some(problem) =
                            render_problem(&package, &observation, log.as_deref())
                        {
                            upsert(&mut per_package, problem);
                        }
                    }
                }
                Err(err) => {
                    let confirmed = is_missing_display(&err);
                    upsert(
                        &mut per_package,
                        capture_problem(&package, display_id, &err, confirmed),
                    );
                }
            }
        }

        // What the runtime already observed. A crash is the strongest app
        // signal, so it outranks a failed operation record for the same package.
        let open: HashSet<&str> = windows
            .iter()
            .map(|window| window.package_id.as_str())
            .collect();
        let mut seen_events: HashSet<String> = HashSet::new();
        for event in self.bus.recent(40) {
            if !seen_events.insert(event.package_id.clone()) {
                continue;
            }
            if open.contains(event.package_id.as_str()) || event.kind != AppEventKind::Crashed {
                continue;
            }
            let excerpt = match adb.as_ref() {
                Some(adb) => adb.crash_excerpt(&serial, &event.package_id).await,
                None => None,
            };
            let process_alive = match adb.as_ref() {
                Some(adb) => Some(adb.process_running(&serial, &event.package_id).await),
                None => None,
            };
            checked.push(format!(
                "event: {} {:?} at {}",
                event.package_id,
                event.kind,
                ago(event.at_ms)
            ));
            upsert(
                &mut per_package,
                crash_problem(&event, excerpt.as_deref(), process_alive),
            );
        }

        let mut seen_operations: HashSet<String> = HashSet::new();
        for record in self.store.operations(OPERATION_WINDOW) {
            let Some(package) = record.package_id.clone() else {
                continue;
            };
            if !seen_operations.insert(package.clone()) {
                continue;
            }
            if open.contains(package.as_str()) || seen_events.contains(&package) {
                continue;
            }
            checked.push(format!(
                "last operation for {package}: {} ({})",
                record.summary, record.status
            ));
            if let Some(problem) = operation_problem(&record, known_apps.as_deref()) {
                upsert(&mut per_package, problem);
            }
        }

        problems.extend(per_package.into_values());
        let mut report = Report::empty(deep);
        report.checked = checked;
        report.problems = problems;
        sort_problems(&mut report.problems);
        report.confirmed = report
            .problems
            .iter()
            .filter(|problem| problem.confidence == Confidence::Confirmed)
            .count();
        report.suspected = report.problems.len() - report.confirmed;
        report.checked.truncate(16);

        if let Ok(mut cache) = self.cache.lock() {
            *cache = Some((Instant::now(), deep, token, report.clone()));
        }
        report
    }

    /// What the cached answer depended on. Both parts are read from memory; no
    /// process or device is touched.
    fn state_token(&self) -> String {
        let windows = self.runtime.open_windows().len();
        let last = self
            .bus
            .recent(1)
            .first()
            .map(|event| format!("{}@{}:{:?}", event.package_id, event.at_ms, event.kind))
            .unwrap_or_default();
        format!("{windows}|{last}")
    }

    /// Run one recovery and report what happened, then show the state it left.
    pub async fn recover(&self, recovery: &Recovery) -> RecoveryOutcome {
        let started = now_ms();
        let result: DwResult<String> = match recovery {
            Recovery::StartRuntime => self
                .runtime
                .ensure_ready(&self.store, "recover-start")
                .await
                .map(|status| format!("The Android runtime is {}.", status.state.as_label())),
            Recovery::RestartRuntime => {
                self.runtime.shutdown().await;
                self.runtime
                    .ensure_ready(&self.store, "recover-restart")
                    .await
                    .map(|status| {
                        format!(
                            "The runtime was restarted and is {}.",
                            status.state.as_label()
                        )
                    })
            }
            Recovery::ReopenWindow { package_id } => self.reopen(package_id, false).await,
            Recovery::RetryLaunch { package_id } => self.reopen(package_id, true).await,
            Recovery::ReinstallPackage { package_id } => self.reinstall(package_id).await,
            Recovery::Recheck | Recovery::None => Ok("Checked again.".to_string()),
        };

        let (ok, summary, detail) = match result {
            Ok(summary) => (true, summary, None),
            Err(err) => (false, err.summary, err.detail),
        };
        let _ = self.store.record_operation(&OperationRecord::finished(
            &format!("{}-{started}", recovery.operation_kind()),
            "recovery",
            if ok { "completed" } else { "failed" },
            started,
            summary.clone(),
            recovery.package_id().map(str::to_string),
            detail.clone(),
        ));
        self.invalidate();

        RecoveryOutcome {
            ok,
            summary,
            detail,
            report: self.report(false).await,
        }
    }

    /// Forget cached answers: after a recovery, a launch or a close, the old
    /// picture is exactly what must not be shown.
    pub fn invalidate(&self) {
        if let Ok(mut cache) = self.cache.lock() {
            *cache = None;
        }
        if let Ok(mut renders) = self.renders.lock() {
            renders.clear();
        }
    }

    fn cached(&self, deep: bool, token: &str) -> Option<Report> {
        let cache = self.cache.lock().ok()?;
        let (at, cached_deep, cached_token, report) = cache.as_ref()?;
        if at.elapsed() < CACHE_TTL && (*cached_deep || !deep) && cached_token == token {
            let mut report = report.clone();
            report
                .checked
                .push("cached from the check a moment ago".to_string());
            return Some(report);
        }
        None
    }

    /// One window's cheap checks: does its display exist, and does its package
    /// still have a process?
    async fn window_problem(
        &self,
        adb: Option<&crate::runtime::adb::Adb>,
        serial: &str,
        window: &crate::runtime::emulator_runtime::OpenWindow,
    ) -> Option<Problem> {
        let package = window.package_id.clone();
        let mut evidence = vec![
            Evidence::new(
                "state",
                format!(
                    "DroidWin owns this window: process {}, geometry {:?}.",
                    window.pid,
                    window
                        .geometry
                        .map(|geometry| (geometry.width, geometry.height, geometry.dpi))
                ),
            ),
            Evidence::new(
                "state",
                format!("Display id recorded: {:?}.", window.display_id),
            ),
        ];
        let adb = adb?;

        if window.display_id.is_none() {
            let focus = adb.display_focus_for_package(serial, &package).await;
            evidence.push(Evidence::new(
                "probe",
                match &focus {
                    Some(focus) => format!(
                        "Android's window dump now lists {} on display {} ({:?}).",
                        package, focus.display_id, focus.activity
                    ),
                    None => format!("Android's window dump lists no display focused on {package}."),
                },
            ));
            return Some(Problem {
                id: format!("app.display-unresolved:{package}"),
                scope: Scope::App,
                package_id: Some(package.clone()),
                severity: Severity::Blocking,
                confidence: Confidence::Confirmed,
                title: format!("{package} has a window but no Android display"),
                observed: "Its window is open, but DroidWin never resolved the display it is drawn on, so screenshots and input cannot be sent to it.".to_string(),
                evidence,
                recommendation: format!(
                    "Reopen this app's window. DroidWin recreates the display for {package}; other windows are left alone."
                ),
                recovery: Recovery::ReopenWindow {
                    package_id: package.clone(),
                },
                alternatives: Vec::new(),
            });
        }

        let pids = adb.process_pids(serial, &package).await;
        evidence.push(Evidence::new(
            "probe",
            format!("pidof {package} → {} pid(s).", pids.len()),
        ));
        if pids.is_empty() {
            return Some(Problem {
                id: format!("app.process-gone:{package}"),
                scope: Scope::App,
                package_id: Some(package.clone()),
                severity: Severity::Blocking,
                confidence: Confidence::Confirmed,
                title: format!("{package} is no longer running"),
                observed: format!(
                    "Its window is still open, but Android reports no process for {package}. The window is showing a frozen or empty frame."
                ),
                evidence,
                recommendation: format!(
                    "Retry the launch. DroidWin opens {package} again in a new window without restarting the runtime."
                ),
                recovery: Recovery::RetryLaunch {
                    package_id: package.clone(),
                },
                alternatives: vec![Recovery::ReinstallPackage {
                    package_id: package.clone(),
                }],
            });
        }
        None
    }

    /// Capture one display twice and decode the first frame. The observation is
    /// cached per package so a refresh does not capture again.
    async fn observe_render(&self, package: &str, display_id: u32) -> DwResult<RenderObservation> {
        if let Ok(renders) = self.renders.lock() {
            if let Some((cached_display, at, observation)) = renders.get(package) {
                if *cached_display == display_id && at.elapsed() < RENDER_TTL {
                    return Ok(observation.clone());
                }
            }
        }
        let adb = self
            .runtime
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = self.runtime.serial();
        let first = adb.screenshot_png(&serial, display_id).await?;
        tokio::time::sleep(SCREENSHOT_GAP).await;
        let second = adb.screenshot_png(&serial, display_id).await?;
        let (width, height, uniform) = frame_facts(&first).ok_or_else(|| {
            DwError::new(
                ErrorCode::BackendFailure,
                "Android returned a screenshot DroidWin could not read.",
            )
            .detail(format!(
                "The first capture was {} bytes and did not decode as an 8-bit RGB image.",
                first.len()
            ))
        })?;
        let observation = RenderObservation {
            display_id,
            width,
            height,
            first_hash: fingerprint(&first),
            second_hash: fingerprint(&second),
            first_bytes: first.len(),
            second_bytes: second.len(),
            uniform,
            unchanged: first == second,
            at_ms: now_ms(),
        };
        if let Ok(mut renders) = self.renders.lock() {
            renders.insert(
                package.to_string(),
                (display_id, Instant::now(), observation.clone()),
            );
        }
        Ok(observation)
    }

    async fn reopen(&self, package: &str, force_stop: bool) -> DwResult<String> {
        self.runtime.close_window(package).await?;
        if force_stop {
            if let Err(err) = self.runtime.stop_app(package).await {
                tracing::warn!(
                    "could not stop {package} before retrying its launch: {}",
                    err.summary
                );
            }
        }
        let label = self.store.cached_app(package).and_then(|app| app.label);
        self.runtime
            .launch(package, label.as_deref(), &self.store, "recover")
            .await?;
        Ok(format!("Reopened {package} in its own window."))
    }

    async fn reinstall(&self, package: &str) -> DwResult<String> {
        let adb = self
            .runtime
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let out = adb
            .install_existing(&self.runtime.serial(), package)
            .await?;
        if !out.ok() {
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                format!("Android could not reinstall {package}."),
            )
            .detail(out.combined()));
        }
        Ok(format!(
            "Reinstalled {package} for this runtime; its data was kept."
        ))
    }
}

/// Missing SDK pieces, as the probe reported them. Each blocking sentence names
/// the package that provides it, so the recommendation can be specific.
pub fn environment_problems(env: &EnvReport) -> Vec<Problem> {
    if env.blocking.is_empty() {
        return Vec::new();
    }
    let mut evidence = vec![
        Evidence::new(
            "probe",
            format!(
                "SDK root: {}",
                env.sdk_root.as_deref().unwrap_or("not found")
            ),
        ),
        Evidence::new(
            "probe",
            format!(
                "System image: {}",
                env.system_image.as_deref().unwrap_or("not installed")
            ),
        ),
        Evidence::new(
            "probe",
            format!(
                "adb: {} · emulator: {}",
                env.adb.as_deref().unwrap_or("missing"),
                env.emulator.as_deref().unwrap_or("missing")
            ),
        ),
        Evidence::new(
            "probe",
            format!("scrcpy: {}", env.scrcpy.as_deref().unwrap_or("missing")),
        ),
    ];
    for item in &env.blocking {
        evidence.push(Evidence::new("probe", item.clone()));
    }
    vec![Problem {
        id: "environment.missing-pieces".to_string(),
        scope: Scope::Environment,
        package_id: None,
        severity: Severity::Blocking,
        confidence: Confidence::Confirmed,
        title: "The Android runtime is missing pieces DroidWin needs".to_string(),
        observed: format!(
            "The environment probe found {} missing piece(s): {}",
            env.blocking.len(),
            env.blocking.join(" ")
        ),
        evidence,
        recommendation: "Install the missing SDK package(s) with the Android SDK manager, then choose Re-check. DroidWin uses an SDK you install yourself and does not download or redistribute it.".to_string(),
        recovery: Recovery::Recheck,
        alternatives: Vec::new(),
    }]
}

/// The runtime's own state, when it is a problem rather than simply stopped.
pub fn status_problem(status: &RuntimeStatus, apps_known: bool) -> Option<Problem> {
    match status.state {
        RuntimeState::Unhealthy => Some(Problem {
            id: "runtime.unhealthy".to_string(),
            scope: Scope::Runtime,
            package_id: None,
            severity: Severity::Blocking,
            confidence: Confidence::Confirmed,
            title: "The runtime answered its probe with a failure".to_string(),
            observed: status.detail.clone(),
            evidence: vec![
                Evidence::new("state", format!("Runtime state: {}", status.state.as_label())),
                Evidence::new("probe", status.detail.clone()),
                Evidence::new(
                    "state",
                    format!(
                        "Android API: {} · serial: {}",
                        status
                            .android_api
                            .map(|api| api.to_string())
                            .unwrap_or_else(|| "not reported".into()),
                        status.serial.as_deref().unwrap_or("not connected")
                    ),
                ),
            ],
            recommendation:
                "Restart the runtime. This is the last resort: it closes every app window, because a runtime-level failure cannot be repaired for one app.".to_string(),
            recovery: Recovery::RestartRuntime,
            alternatives: vec![Recovery::StartRuntime],
        }),
        RuntimeState::NotRunning if apps_known => Some(Problem {
            id: "runtime.stopped".to_string(),
            scope: Scope::Runtime,
            package_id: None,
            severity: Severity::Notice,
            confidence: Confidence::Confirmed,
            title: "The Android runtime is stopped".to_string(),
            observed: status.detail.clone(),
            evidence: vec![
                Evidence::new("state", format!("Runtime state: {}", status.state.as_label())),
                Evidence::new(
                    "probe",
                    format!(
                        "Virtual device: {} · serial: {}",
                        status.avd.as_deref().unwrap_or("not configured"),
                        status.serial.as_deref().unwrap_or("not connected")
                    ),
                ),
            ],
            recommendation:
                "Start the runtime before opening an app. Starting it does not change any app's data."
                    .to_string(),
            recovery: Recovery::StartRuntime,
            alternatives: Vec::new(),
        }),
        _ => None,
    }
}

/// A window host that died, as the runtime observed and reported it.
///
/// `process_alive` separates two very different failures: the app itself died
/// (its Android process is gone), or only its window host did (the app is still
/// running, with nowhere to be drawn). The recovery differs — retry the launch
/// versus reopening a window — so the diagnosis must not guess.
pub fn crash_problem(
    event: &AppEvent,
    excerpt: Option<&str>,
    process_alive: Option<bool>,
) -> Problem {
    let package = event.package_id.clone();
    let host_only = process_alive == Some(true);
    let mut evidence = vec![Evidence::new(
        "event",
        format!(
            "app_crashed at {}: {} (exit code {}, display {:?}).",
            ago(event.at_ms),
            event.reason,
            event
                .exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "unknown".to_string()),
            event.display_id
        ),
    )];
    evidence.push(Evidence::new(
        "log",
        excerpt.map(str::to_string).unwrap_or_else(|| {
            "Android's crash buffer holds no block for this package.".to_string()
        }),
    ));
    if host_only {
        evidence.push(Evidence::new(
            "probe",
            format!("pidof {package} still reports a process, so only the window was lost."),
        ));
    } else if process_alive.is_none() {
        evidence.push(Evidence::new(
            "probe",
            "DroidWin could not probe the package's process, so whether the app itself survived is unknown.".to_string(),
        ));
    }
    let exit = event
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    Problem {
        id: if host_only {
            format!("app.window-lost:{package}")
        } else {
            format!("app.crashed:{package}")
        },
        scope: Scope::App,
        package_id: Some(package.clone()),
        severity: Severity::Blocking,
        confidence: Confidence::Confirmed,
        title: if host_only {
            format!("{package} lost its window while still running")
        } else {
            format!("{package} crashed while its window was open")
        },
        observed: if host_only {
            format!(
                "DroidWin's window host for {package} exited with code {exit}, but its Android process is still alive. The app was not killed; it has no window on which to be seen."
            )
        } else if process_alive == Some(false) {
            format!(
                "DroidWin's window host for {package} exited with code {exit}, and its process is gone."
            )
        } else {
            format!(
                "DroidWin's window host for {package} exited with code {exit}. DroidWin could not confirm whether the app's process survived."
            )
        },
        evidence,
        recommendation: if host_only {
            format!(
                "Reopen this app's window. DroidWin reattaches {package} on a fresh display without restarting the runtime or touching other apps."
            )
        } else {
            format!(
                "Retry the launch. If it crashes again at the same point, reinstall {package} (its data is kept) or read the crash block above."
            )
        },
        recovery: if host_only {
            Recovery::ReopenWindow {
                package_id: package.clone(),
            }
        } else {
            Recovery::RetryLaunch {
                package_id: package.clone(),
            }
        },
        alternatives: vec![Recovery::ReinstallPackage {
            package_id: package,
        }],
    }
}

/// A launch or install that ended in failure, from DroidWin's own history.
///
/// A record is history, not a present-tense fact: it keeps its own sentence even
/// after the app was installed or opened again. So the package is re-checked
/// against the app list the runtime already holds, the age is part of the title,
/// and the observation says which part is recorded and which part is measured
/// now. `installed` is that list, or `None` when DroidWin has not enumerated apps
/// yet — in which case it says so instead of repeating the record as if it were
/// true today.
pub fn operation_problem(
    record: &OperationRecord,
    installed: Option<&[CachedApp]>,
) -> Option<Problem> {
    if record.status != "failed" && record.status != "launch_failed" {
        return None;
    }
    let package = record.package_id.clone()?;
    let launch = record.kind.contains("launch") || record.status == "launch_failed";
    let present = installed.and_then(|apps| apps.iter().find(|app| app.package_id == package));
    let (now_sentence, probe) = match installed {
        None => (
            "DroidWin had no installed-app list to re-check this package against, so the sentence above is recorded history, not a current observation."
                .to_string(),
            Evidence::new(
                "state",
                "No installed-app list was cached, so the package was not re-checked.",
            ),
        ),
        Some(_) => match present {
            Some(app) => (
                format!(
                    "Re-checked now: {package} is installed{} and has no window open.",
                    app.version_name
                        .as_deref()
                        .map(|version| format!(" ({version})"))
                        .unwrap_or_default()
                ),
                Evidence::new(
                    "state",
                    format!("The runtime's app list reports {package} installed right now."),
                ),
            ),
            None => (
                format!("Re-checked now: {package} is still not installed."),
                Evidence::new(
                    "state",
                    format!("The runtime's app list does not contain {package} right now."),
                ),
            ),
        },
    };
    let mut evidence = vec![
        Evidence::new(
            "state",
            format!(
                "{} ({}) at {}: {}",
                record.kind,
                record.status,
                ago(record.started_at),
                record.summary
            ),
        ),
        Evidence::new(
            "log",
            record
                .detail
                .clone()
                .unwrap_or_else(|| "The operation recorded no further detail.".to_string()),
        ),
    ];
    evidence.push(Evidence::new(
        "state",
        "No window is open for this package now.",
    ));
    evidence.push(probe);
    Some(Problem {
        id: format!("app.last-operation:{package}"),
        scope: Scope::App,
        package_id: Some(package.clone()),
        severity: if launch {
            Severity::Degraded
        } else {
            Severity::Notice
        },
        confidence: Confidence::Confirmed,
        title: if launch {
            format!(
                "The last launch of {package} failed ({})",
                ago(record.started_at)
            )
        } else {
            format!(
                "The last {} of {package} failed ({})",
                record.kind,
                ago(record.started_at)
            )
        },
        observed: format!(
            "Recorded {}: {} {now_sentence}",
            ago(record.started_at),
            record.summary
        ),
        evidence,
        recommendation: if launch {
            format!("Retry the launch of {package}. Its window is recreated on its own display, without touching other apps.")
        } else {
            "Read the recorded detail, fix what it names (a signature conflict needs the old copy removed first), then install the package again from its file.".to_string()
        },
        recovery: if launch {
            Recovery::RetryLaunch {
                package_id: package.clone(),
            }
        } else {
            Recovery::None
        },
        alternatives: Vec::new(),
    })
}

/// A live window whose visible area is one solid colour.
///
/// `log` is the app's own last few lines (or the graphics stack's), so the blank
/// frame is confirmed from the screenshot and the log together — never from the
/// screenshot alone with a guess about why.
pub fn render_problem(
    package: &str,
    observation: &RenderObservation,
    log: Option<&str>,
) -> Option<Problem> {
    let colour = observation.uniform?;
    let hex = format!("#{:02x}{:02x}{:02x}", colour[0], colour[1], colour[2]);
    Some(Problem {
        id: format!("app.blank-render:{package}"),
        scope: Scope::App,
        package_id: Some(package.to_string()),
        severity: Severity::Blocking,
        confidence: Confidence::Confirmed,
        title: format!("{package} is drawing a blank window"),
        observed: format!(
            "Two screenshots of display {} taken {} ms apart are both {}x{} and every pixel is {}.",
            observation.display_id,
            SCREENSHOT_GAP.as_millis(),
            observation.width,
            observation.height,
            hex
        ),
        evidence: vec![
            Evidence::new(
                "screenshot",
                format!(
                    "first capture {} bytes, hash {:016x}; second capture {} bytes, hash {:016x}{}.",
                    observation.first_bytes,
                    observation.first_hash,
                    observation.second_bytes,
                    observation.second_hash,
                    if observation.unchanged {
                        " — identical"
                    } else {
                        " — different bytes, same solid colour"
                    }
                ),
            ),
            Evidence::new(
                "probe",
                format!(
                    "The package has a live process and the window is open on display {}.",
                    observation.display_id
                ),
            ),
            Evidence::new(
                "log",
                log.unwrap_or(
                    "No line in the last 400 log entries mentions this package or its graphics stack.",
                ),
            ),
        ],
        recommendation: format!(
            "Retry the launch first: DroidWin closes this window and opens {package} again on a fresh display. If it is blank again, reinstall it (data is kept) and read the log evidence above."
        ),
        recovery: Recovery::RetryLaunch {
            package_id: package.to_string(),
        },
        alternatives: vec![Recovery::ReinstallPackage {
            package_id: package.to_string(),
        }],
    })
}

fn capture_problem(
    package: &str,
    display_id: u32,
    err: &DwError,
    missing_display: bool,
) -> Problem {
    Problem {
        id: format!("app.capture-failed:{package}"),
        scope: Scope::App,
        package_id: Some(package.to_string()),
        severity: if missing_display {
            Severity::Blocking
        } else {
            Severity::Degraded
        },
        confidence: if missing_display {
            Confidence::Confirmed
        } else {
            Confidence::Suspected
        },
        title: if missing_display {
            format!("Android no longer has display {display_id} for {package}")
        } else {
            format!("DroidWin could not capture {package}'s display")
        },
        observed: if missing_display {
            format!(
                "The window is still open in DroidWin, but Android's display list no longer contains display {display_id}, so input and screenshots have nowhere to go."
            )
        } else {
            format!(
                "The window is open on display {display_id}, but its screenshot failed, so whether it renders correctly is unknown — this is a suspicion, not a confirmed blank window."
            )
        },
        evidence: vec![
            Evidence::new("probe", err.summary.clone()),
            Evidence::new(
                "log",
                err.detail
                    .clone()
                    .unwrap_or_else(|| "The failure carried no device output.".to_string()),
            ),
        ],
        recommendation: format!(
            "Reopen {package}'s window so DroidWin recreates its display; other windows are unaffected."
        ),
        recovery: Recovery::ReopenWindow {
            package_id: package.to_string(),
        },
        alternatives: Vec::new(),
    }
}

fn is_missing_display(err: &DwError) -> bool {
    let text = format!("{} {}", err.summary, err.detail.as_deref().unwrap_or(""));
    text.contains("no longer has display")
        || text.contains("does not list display")
        || text.contains("no token for display")
}

fn upsert(problems: &mut HashMap<String, Problem>, problem: Problem) {
    let key = problem.package_id.clone().unwrap_or_default();
    match problems.get(&key) {
        Some(existing) if rank(existing) >= rank(&problem) => {}
        _ => {
            problems.insert(key, problem);
        }
    }
}

/// Which problem to show when one package has several. A lost display or a dead
/// process explains the rest; a blank render is still worth more than an old
/// operation record.
fn rank(problem: &Problem) -> u8 {
    match problem.id.split(':').next().unwrap_or_default() {
        "app.display-unresolved" => 6,
        "app.process-gone" => 5,
        "app.window-lost" => 4,
        "app.crashed" => 4,
        "app.blank-render" => 4,
        "app.capture-failed" => 3,
        _ => 2,
    }
}

fn sort_problems(problems: &mut [Problem]) {
    fn key(problem: &Problem) -> (bool, std::cmp::Reverse<Severity>, String, String) {
        (
            problem.confidence != Confidence::Confirmed,
            std::cmp::Reverse(problem.severity),
            problem.scope_label(),
            problem.title.clone(),
        )
    }
    problems.sort_by_key(key);
}

impl Problem {
    fn scope_label(&self) -> String {
        match self.scope {
            Scope::Environment => "0-environment".to_string(),
            Scope::Runtime => "1-runtime".to_string(),
            Scope::App => "2-app".to_string(),
        }
    }
}

fn describe_env(env: &EnvReport) -> String {
    format!(
        "environment: SDK {}, adb {}, emulator {}, scrcpy {}, {} AVD(s)",
        env.sdk_root.as_deref().unwrap_or("missing"),
        if env.adb.is_some() {
            "found"
        } else {
            "missing"
        },
        if env.emulator.is_some() {
            "found"
        } else {
            "missing"
        },
        if env.scrcpy.is_some() {
            "found"
        } else {
            "missing"
        },
        env.avds.len()
    )
}

fn describe_render(package: &str, observation: &RenderObservation) -> String {
    format!(
        "render {package}: {}x{} on display {}, hash {:016x} → {:016x} ({}), {}",
        observation.width,
        observation.height,
        observation.display_id,
        observation.first_hash,
        observation.second_hash,
        if observation.unchanged {
            "unchanged"
        } else {
            "changed"
        },
        match observation.uniform {
            Some(colour) => format!(
                "confirmed solid #{:02x}{:02x}{:02x}",
                colour[0], colour[1], colour[2]
            ),
            None => "not a solid colour".to_string(),
        }
    )
}

fn ago(at_ms: i64) -> String {
    let delta = (now_ms() - at_ms).max(0);
    if delta < 1_000 {
        format!("{delta} ms ago")
    } else if delta < 60_000 {
        format!("{} s ago", delta / 1_000)
    } else if delta < 3_600_000 {
        format!("{} min ago", delta / 60_000)
    } else {
        format!("{} h ago", delta / 3_600_000)
    }
}

/// FNV-1a over the encoded PNG. Any two different captures give different
/// values; the same bytes mean the screen did not change between them.
fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    hash
}

/// Decode a screenshot far enough to answer two questions: how big is it, and is
/// every pixel the same colour? A palette or 16-bit image is refused rather than
/// guessed at; every Android screencap is 8-bit RGBA.
pub fn frame_facts(png: &[u8]) -> Option<(u32, u32, Option<[u8; 3]>)> {
    let decoder = png::Decoder::new(std::io::Cursor::new(png));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let samples = info.color_type.samples();
    if info.bit_depth != png::BitDepth::Eight || samples < 3 {
        return None;
    }
    let data = &buffer[..info.buffer_size()];
    let pixels = info.width as usize * info.height as usize;
    if pixels == 0 {
        return None;
    }
    let at = |index: usize| -> [u8; 3] {
        let offset = index * samples;
        [data[offset], data[offset + 1], data[offset + 2]]
    };
    let first = at(0);
    for index in 1..pixels {
        if at(index) != first {
            return Some((info.width, info.height, None));
        }
    }
    Some((info.width, info.height, Some(first)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::AppOrientation;
    use crate::runtime::{AppEvent, AppEventKind};
    use crate::store::OperationRecord;

    fn solid_png(rgb: [u8; 3], width: u32, height: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let mut data = Vec::with_capacity((width * height * 4) as usize);
            for _ in 0..width * height {
                data.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
            writer.write_image_data(&data).unwrap();
        }
        out
    }

    #[test]
    fn a_solid_frame_is_recognised_and_a_drawn_one_is_not() {
        let blank = solid_png([0, 0, 0], 16, 9);
        let (width, height, uniform) = frame_facts(&blank).unwrap();
        assert_eq!((width, height), (16, 9));
        assert_eq!(uniform, Some([0, 0, 0]));

        let mut drawn = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut drawn, 16, 9);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let mut data = Vec::new();
            for x in 0..16 {
                for _ in 0..9 {
                    data.extend_from_slice(&[x as u8 * 15, 0, 0, 255]);
                }
            }
            writer.write_image_data(&data).unwrap();
        }
        let (_, _, uniform) = frame_facts(&drawn).unwrap();
        assert_eq!(uniform, None);
    }

    #[test]
    fn the_fingerprint_is_stable_and_changes_with_the_frame() {
        assert_eq!(fingerprint(b"same"), fingerprint(b"same"));
        assert_ne!(fingerprint(b"one"), fingerprint(b"two"));
    }

    #[test]
    fn a_confirmed_render_problem_names_the_colour_and_both_captures() {
        let observation = RenderObservation {
            display_id: 4,
            width: 1080,
            height: 2400,
            first_hash: 0xaa,
            second_hash: 0xaa,
            first_bytes: 3763,
            second_bytes: 3763,
            uniform: Some([0, 0, 0]),
            unchanged: true,
            at_ms: now_ms(),
        };
        let problem = render_problem(
            "org.example.game",
            &observation,
            Some("E/org.example.game: blank surface drawn"),
        )
        .unwrap();
        assert_eq!(problem.confidence, Confidence::Confirmed);
        assert_eq!(problem.severity, Severity::Blocking);
        assert!(problem.observed.contains("#000000"));
        assert!(problem.evidence.iter().any(|e| e.detail.contains("hash")));
        assert!(problem
            .evidence
            .iter()
            .any(|e| e.source == "log" && e.detail.contains("blank surface drawn")));
        assert_eq!(
            problem.recovery,
            Recovery::RetryLaunch {
                package_id: "org.example.game".into()
            }
        );
        assert_eq!(problem.alternatives.len(), 1);

        let drawn = RenderObservation {
            uniform: None,
            ..observation
        };
        assert!(render_problem("org.example.game", &drawn, None).is_none());

        let silent = render_problem("org.example.game", &observation, None).unwrap();
        assert!(silent
            .evidence
            .iter()
            .any(|e| e.source == "log" && e.detail.contains("No line")));
    }
    #[test]
    fn a_crash_is_reported_as_confirmed_with_its_crash_block() {
        let event = AppEvent::new(
            "org.example.game",
            AppEventKind::Crashed,
            "its window host exited with code 1",
        )
        .exited(Some(1))
        .at(Some(3), Some(1200));
        let problem = crash_problem(&event, Some("FATAL EXCEPTION: main"), Some(false));
        assert_eq!(problem.confidence, Confidence::Confirmed);
        assert!(problem.observed.contains("code 1"));
        assert!(problem.title.contains("crashed"));
        assert!(problem
            .evidence
            .iter()
            .any(|e| e.detail.contains("FATAL EXCEPTION")));
        assert!(matches!(problem.recovery, Recovery::RetryLaunch { .. }));

        let quiet = crash_problem(&event, None, Some(false));
        assert!(quiet.evidence.iter().any(|e| e.detail.contains("no block")));
    }

    #[test]
    fn a_lost_window_on_a_living_app_is_not_reported_as_a_crash() {
        // Killing scrcpy leaves the Android app running. Reopening a window is
        // the targeted repair; force-stopping a healthy app would not be.
        let event = AppEvent::new(
            "org.example.game",
            AppEventKind::Crashed,
            "its window host exited with code 1",
        )
        .exited(Some(1))
        .at(Some(3), Some(1200));
        let problem = crash_problem(&event, None, Some(true));
        assert_eq!(problem.confidence, Confidence::Confirmed);
        assert!(problem.title.contains("lost its window"));
        assert!(problem.observed.contains("still alive"));
        assert!(matches!(problem.recovery, Recovery::ReopenWindow { .. }));

        let unknown = crash_problem(&event, None, None);
        assert_eq!(unknown.confidence, Confidence::Confirmed);
        assert!(unknown.observed.contains("could not confirm"));
        assert!(unknown
            .evidence
            .iter()
            .any(|e| e.detail.contains("could not probe")));
    }

    #[test]
    fn missing_sdk_pieces_are_blocking_and_name_the_provider() {
        let env = EnvReport {
            blocking: vec![
                "The system image system-images;android-36;google_apis;x86_64 is missing; install it with sdkmanager.".into(),
            ],
            system_image: None,
            avds: Vec::new(),
            ..EnvReport::default()
        };
        let problems = environment_problems(&env);
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].confidence, Confidence::Confirmed);
        assert_eq!(problems[0].severity, Severity::Blocking);
        assert!(problems[0]
            .evidence
            .iter()
            .any(|e| e.detail.contains("sdkmanager")));
        assert_eq!(problems[0].recovery, Recovery::Recheck);
        assert!(environment_problems(&EnvReport::default()).is_empty());
    }

    #[test]
    fn a_stopped_runtime_is_a_notice_only_when_there_is_something_to_open() {
        let status = RuntimeStatus {
            backend: "test".into(),
            state: RuntimeState::NotRunning,
            detail: "The Android runtime is stopped.".into(),
            avd: Some("DroidWin_API36".into()),
            serial: None,
            android_api: None,
            capabilities: Default::default(),
            limitations: Vec::new(),
        };
        assert!(status_problem(&status, false).is_none());
        let problem = status_problem(&status, true).unwrap();
        assert_eq!(problem.severity, Severity::Notice);
        assert_eq!(problem.recovery, Recovery::StartRuntime);

        let unhealthy = RuntimeStatus {
            state: RuntimeState::Unhealthy,
            detail: "adb did not answer.".into(),
            ..status
        };
        let problem = status_problem(&unhealthy, false).unwrap();
        assert_eq!(problem.recovery, Recovery::RestartRuntime);
        assert!(problem.recommendation.contains("last resort"));
    }

    #[test]
    fn a_failed_launch_offers_a_targeted_retry_but_an_install_failure_does_not_pretend_to() {
        let launch = OperationRecord::finished(
            "op-1",
            "launch",
            "failed",
            now_ms() - 4_000,
            "org.example.game started and then crashed.",
            Some("org.example.game".into()),
            Some("FATAL EXCEPTION".into()),
        );
        let problem = operation_problem(&launch, None).unwrap();
        assert!(matches!(problem.recovery, Recovery::RetryLaunch { .. }));
        assert!(problem.title.contains("failed ("), "{}", problem.title);

        let install = OperationRecord::finished(
            "op-2",
            "install",
            "failed",
            now_ms() - 4_000,
            "Android rejected the installation.",
            Some("org.example.game".into()),
            Some("INSTALL_FAILED_UPDATE_INCOMPATIBLE".into()),
        );
        let problem = operation_problem(&install, None).unwrap();
        assert_eq!(problem.recovery, Recovery::None);
        assert!(problem.recommendation.contains("signature conflict"));
    }

    #[test]
    fn an_old_failure_is_reported_as_history_against_what_is_installed_now() {
        // A launch that failed hours ago said the package was not installed. It
        // is installed now, and repeating that sentence as a present-tense fact
        // would contradict the app list one screen away.
        let record = OperationRecord::finished(
            "op-9",
            "install",
            "launch_failed",
            now_ms() - 9 * 60 * 60 * 1000,
            "org.example.game is not installed in the runtime.",
            Some("org.example.game".into()),
            None,
        );
        let installed = vec![CachedApp {
            package_id: "org.example.game".into(),
            label: Some("Game".into()),
            version_name: Some("1.61.0".into()),
            version_code: Some(161),
            icon: None,
            target_sdk: Some(36),
            orientation: None,
        }];
        let problem = operation_problem(&record, Some(&installed)).unwrap();
        assert_eq!(problem.confidence, Confidence::Confirmed);
        assert!(problem.title.contains("9 h ago"), "{}", problem.title);
        assert!(problem.observed.starts_with("Recorded 9 h ago:"));
        assert!(
            problem
                .observed
                .contains("is installed (1.61.0) and has no window open"),
            "{}",
            problem.observed
        );
        assert!(problem
            .evidence
            .iter()
            .any(|item| item.detail.contains("installed right now")));
    }

    #[test]
    fn a_completed_operation_is_not_a_problem() {
        let done = OperationRecord::finished(
            "op-3",
            "launch",
            "completed",
            now_ms(),
            "Opened the window.",
            Some("org.example.game".into()),
            None,
        );
        assert!(operation_problem(&done, None).is_none());
    }

    #[test]
    fn only_the_most_explanatory_problem_per_app_is_kept() {
        let base = |id: &str, title: &str| Problem {
            id: id.to_string(),
            scope: Scope::App,
            package_id: Some("org.example.game".into()),
            severity: Severity::Blocking,
            confidence: Confidence::Confirmed,
            title: title.into(),
            observed: String::new(),
            evidence: Vec::new(),
            recommendation: String::new(),
            recovery: Recovery::None,
            alternatives: Vec::new(),
        };
        let mut problems = HashMap::new();
        upsert(
            &mut problems,
            base("app.last-operation:org.example.game", "old"),
        );
        upsert(
            &mut problems,
            base("app.blank-render:org.example.game", "blank"),
        );
        upsert(&mut problems, base("app.crashed:org.example.game", "crash"));
        assert_eq!(problems.len(), 1);
        assert_eq!(problems.values().next().unwrap().title, "blank");
    }

    #[test]
    fn confirmed_problems_sort_before_suspected_ones() {
        let mut problems = vec![
            Problem {
                id: "b".into(),
                scope: Scope::App,
                package_id: Some("b".into()),
                severity: Severity::Blocking,
                confidence: Confidence::Suspected,
                title: "suspected".into(),
                observed: String::new(),
                evidence: Vec::new(),
                recommendation: String::new(),
                recovery: Recovery::None,
                alternatives: Vec::new(),
            },
            Problem {
                id: "a".into(),
                scope: Scope::App,
                package_id: Some("a".into()),
                severity: Severity::Degraded,
                confidence: Confidence::Confirmed,
                title: "confirmed".into(),
                observed: String::new(),
                evidence: Vec::new(),
                recommendation: String::new(),
                recovery: Recovery::None,
                alternatives: Vec::new(),
            },
        ];
        sort_problems(&mut problems);
        assert_eq!(problems[0].title, "confirmed");
    }

    #[test]
    fn a_display_that_disappeared_is_confirmed_but_a_failed_capture_is_only_suspected() {
        let gone = DwError::new(
            ErrorCode::RuntimeUnavailable,
            "Android no longer has display 4.",
        );
        let problem = capture_problem("org.example.game", 4, &gone, true);
        assert_eq!(problem.confidence, Confidence::Confirmed);
        assert!(matches!(problem.recovery, Recovery::ReopenWindow { .. }));

        let unknown = DwError::new(
            ErrorCode::BackendFailure,
            "Android did not return an image for display 4.",
        );
        let problem = capture_problem("org.example.game", 4, &unknown, false);
        assert_eq!(problem.confidence, Confidence::Suspected);
        assert!(problem.observed.contains("suspicion"));
    }

    #[test]
    fn the_serialized_recovery_names_what_it_touches() {
        let json = serde_json::to_value(Recovery::ReopenWindow {
            package_id: "org.example.game".into(),
        })
        .unwrap();
        assert_eq!(json["kind"], "reopen_window");
        assert_eq!(json["package_id"], "org.example.game");
        assert_eq!(Recovery::RestartRuntime.package_id(), None);
        assert_eq!(
            Recovery::RetryLaunch {
                package_id: "x".into()
            }
            .operation_kind(),
            "recovery-retry-launch"
        );
    }

    #[test]
    fn an_app_with_a_declared_orientation_still_has_a_geometry_on_its_window() {
        // Guards the shape diagnostics relies on: the window record carries the
        // geometry, so a problem can name the surface it was created with.
        let geometry =
            crate::runtime::display::DisplayGeometry::for_app(Some(AppOrientation::Landscape));
        assert_eq!(geometry.orientation.as_str(), "landscape");
        assert!(geometry.window_width > geometry.window_height);
    }
}
