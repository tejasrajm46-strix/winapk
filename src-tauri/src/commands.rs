//! Command layer. Parses arguments, calls one service, records the operation and
//! returns a typed result. No backend command strings and no business logic.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::agent::{self, skills, ActionRecord, AgentStatus, Permission};
use crate::apk;
use crate::branding;
use crate::capture::{Capture, RecordingState, SavedRecording, SavedShot};
use crate::diagnose::{Diagnoser, Recovery, RecoveryOutcome, Report};
use crate::domain::{
    now_ms, ApkInspection, AppWindow, DwError, DwResult, EnvReport, ErrorCode, InstalledApp,
    PhoneScreen, RuntimeState, SetupReport, Stage, StageEvent,
};
use crate::input::{InputAction, InputState};
use crate::runtime::display::Orientation;
use crate::runtime::display_override::{self, DisplayInfo};
use crate::runtime::emulator_runtime::{AVD_NAME, SYSTEM_IMAGE};
use crate::runtime::hardware::{self, GpuMode, HardwareReport, Preset};
use crate::runtime::install::Setup;
use crate::runtime::scrcpy::KeyboardMode;
use crate::runtime::sdk::Sdk;
use crate::runtime::{EmulatorRuntime, InstallOutcome, RuntimeEvents};
use crate::store::{OperationRecord, Store};

pub struct AppState {
    pub runtime: Arc<EmulatorRuntime>,
    pub phone_stream: Arc<crate::runtime::phone_stream::PhoneStream>,
    /// Saved stills and recordings. It records the phone's own display through
    /// the same adb as everything else, and holds no second emulator.
    pub capture: Arc<Capture>,
    pub store: Arc<Store>,
    /// The agent control layer. It drives `runtime`; it never owns an emulator.
    pub agent: Arc<agent::AgentState>,
    /// Per-app keyboard mapping. Off for every package until its profile is
    /// switched on, and it dispatches through the same runtime and displays.
    pub input: Arc<crate::input::InputController>,
    /// The nav bar and mapping labels drawn over the app window in front.
    pub overlay: Arc<crate::overlay::Overlay>,
    /// "Attention Needed": diagnostics built from the same runtime and event
    /// feed, with targeted recovery. It owns no process and no poller.
    pub diagnose: Arc<Diagnoser>,
    /// The one-time Android runtime installation. It writes only inside the SDK
    /// root the setup screen names, and it never starts a runtime itself.
    pub setup: Arc<Setup>,
    pub data_dir: PathBuf,
    /// Keeps the non-blocking log writer alive for the process lifetime.
    pub log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
    /// Packages requested by the shell before the webview was listening.
    pub pending_activation: Mutex<Vec<String>>,
}

/// Package paths in a process argument list. Explorer hands Windows one argument
/// per file — spaces, Unicode and all — so the only cleanup needed is a quote
/// pair that survived a hand-typed command line.
pub fn apk_paths_from_args<I, S>(args: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter()
        .filter_map(|arg| {
            let path = arg.as_ref().trim().trim_matches('"');
            (path.len() > 4
                && path.to_ascii_lowercase().ends_with(".apk")
                && !path.starts_with('-'))
            .then(|| path.to_string())
        })
        .collect()
}

/// Bridges runtime notifications into webview events, and app lifecycle into the
/// in-process bus the trigger engine listens on.
pub struct TauriEvents {
    pub app: AppHandle,
    pub bus: crate::runtime::EventBus,
}

impl TauriEvents {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            bus: crate::runtime::EventBus::new(),
        }
    }
}

impl RuntimeEvents for TauriEvents {
    fn stage(&self, event: &StageEvent) {
        let _ = self.app.emit("op://stage", event);
    }

    fn status(&self, status: &crate::domain::RuntimeStatus) {
        let _ = self.app.emit("runtime://status", status);
    }

    fn apps_changed(&self) {
        let _ = self.app.emit("apps://changed", ());
    }

    fn app_event(&self, event: &crate::runtime::AppEvent) {
        self.bus.publish(event.clone());
        let _ = self.app.emit("app://event", event);
    }
}

fn op_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    format!(
        "op-{}-{}",
        now_ms(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )
}

fn emit(app: &AppHandle, op: &str, stage: Stage, message: &str, package_id: Option<String>) {
    let _ = app.emit(
        "op://stage",
        StageEvent {
            op: op.to_string(),
            package_id,
            stage,
            message: message.to_string(),
            at_ms: now_ms(),
        },
    );
}

fn record(store: &Store, entry: OperationRecord) {
    let _ = store.record_operation(&entry);
}

fn fail(
    app: &AppHandle,
    store: &Store,
    op: &str,
    kind: &str,
    started_at: i64,
    err: DwError,
) -> DwError {
    emit(app, op, Stage::Failed, &err.summary, None);
    record(
        store,
        OperationRecord::finished(
            op,
            kind,
            "failed",
            started_at,
            &err.summary,
            None,
            err.detail.clone(),
        ),
    );
    err
}

#[derive(Serialize)]
pub struct Paths {
    pub data_dir: String,
    pub log_dir: String,
    pub avd: String,
    pub system_image: String,
}

#[derive(Serialize)]
pub struct RuntimeControl {
    pub status: crate::domain::RuntimeStatus,
    pub env: EnvReport,
}

#[tauri::command]
pub async fn env_probe(app: AppHandle) -> Result<EnvReport, DwError> {
    let state = app.state::<AppState>();
    // An explicit ask, so every check really runs.
    Ok(state.runtime.probe_now())
}

#[tauri::command]
pub async fn runtime_status(app: AppHandle) -> Result<crate::domain::RuntimeStatus, DwError> {
    let state = app.state::<AppState>();
    Ok(state.runtime.status(&state.store).await)
}

#[tauri::command]
pub async fn runtime_probe(app: AppHandle) -> Result<RuntimeControl, DwError> {
    let state = app.state::<AppState>();
    let status = state.runtime.status(&state.store).await;
    Ok(RuntimeControl {
        status,
        env: state.runtime.probe_now(),
    })
}

#[tauri::command]
pub async fn runtime_start(app: AppHandle) -> Result<crate::domain::RuntimeStatus, DwError> {
    let state = app.state::<AppState>();
    state
        .runtime
        .ensure_ready(&state.store, "runtime-start")
        .await
}

/// Host facts and the emulator plan, with Android's own numbers when it is up.
#[tauri::command]
pub async fn hardware_report(app: AppHandle) -> Result<HardwareReport, DwError> {
    let state = app.state::<AppState>();
    Ok(state.runtime.hardware_report(&state.store).await)
}

/// Save an automatic or custom plan. Values are validated before anything is
/// written, and the plan only applies on the next emulator start.
#[tauri::command]
pub async fn hardware_apply(
    app: AppHandle,
    mode: String,
    cores: Option<u32>,
    ram_mb: Option<u32>,
    gpu: Option<String>,
) -> Result<HardwareReport, DwError> {
    let state = app.state::<AppState>();
    let host = hardware::host_facts();
    let plan = match mode.as_str() {
        "auto" => hardware::recommend(&host),
        "custom" => {
            let gpu = gpu.as_deref().and_then(GpuMode::parse).ok_or_else(|| {
                DwError::new(ErrorCode::InvalidConfig, "Choose a supported GPU mode.")
            })?;
            hardware::custom_plan(
                &host,
                cores.unwrap_or(hardware::recommend(&host).cores),
                ram_mb.unwrap_or(hardware::recommend(&host).ram_mb),
                gpu,
            )?
        }
        _ => {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                "Choose the automatic or custom hardware plan.",
            ))
        }
    };
    hardware::persist(&state.store, &plan)?;
    Ok(state.runtime.hardware_report(&state.store).await)
}

/// The saved keyboard mode, in the values the UI uses.
#[tauri::command]
pub async fn keyboard_mode(app: AppHandle) -> Result<String, DwError> {
    let state = app.state::<AppState>();
    Ok(state
        .store
        .setting("runtime.keyboard_mode")
        .and_then(|v| KeyboardMode::parse(&v))
        .unwrap_or(KeyboardMode::Auto)
        .as_str()
        .to_string())
}

/// Persist a keyboard mode. `uhid` needs Android 9+; `sdk` is always available.
#[tauri::command]
pub async fn set_keyboard_mode(app: AppHandle, mode: String) -> Result<String, DwError> {
    let state = app.state::<AppState>();
    let parsed = KeyboardMode::parse(&mode).ok_or_else(|| {
        DwError::new(
            ErrorCode::InvalidConfig,
            "Choose the automatic, hardware (uhid) or simulated (sdk) keyboard.",
        )
    })?;
    state
        .store
        .set_setting("runtime.keyboard_mode", parsed.as_str())?;
    Ok(parsed.as_str().to_string())
}

#[tauri::command]
pub async fn runtime_stop(app: AppHandle) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    state.input.release_all();
    state.input.flush().await;
    state.phone_stream.stop(None).await;
    // A recorder left running would hold the display and write into a runtime
    // that is going away.
    state.capture.abort_recording(&state.runtime).await;
    state.capture.abort_window_recording().await;
    state.runtime.stop_now().await;
    Ok(())
}

/// The setup report, with "ready" reserved for a boot the runtime verified.
/// Everything the setup screen shows about files arrives from `Setup::report`;
/// this adds the one fact the files cannot carry.
async fn setup_report(state: &AppState) -> SetupReport {
    let booted = matches!(
        state.runtime.status(&state.store).await.state,
        RuntimeState::Ready
    );
    state.setup.report(&state.store, booted)
}

#[tauri::command]
pub async fn runtime_setup_state(app: AppHandle) -> Result<SetupReport, DwError> {
    let state = app.state::<AppState>();
    Ok(setup_report(&state).await)
}

/// Install the runtime, then boot it once. "Ready" has to mean a device
/// answered adb, not that files exist on disk.
#[tauri::command]
pub async fn runtime_setup_start(
    app: AppHandle,
    accept_licenses: bool,
) -> Result<SetupReport, DwError> {
    let state = app.state::<AppState>();
    let store = Arc::clone(&state.store);
    state.setup.run(&store, accept_licenses).await?;
    state.runtime.probe_now();
    let status = state.runtime.ensure_ready(&store, "setup").await?;
    Ok(state
        .setup
        .report(&store, matches!(status.state, RuntimeState::Ready)))
}

#[tauri::command]
pub async fn runtime_setup_cancel(app: AppHandle) -> Result<(), DwError> {
    app.state::<AppState>().setup.cancel();
    Ok(())
}

/// Point WINAPK at an SDK directory, or at its own managed one when `path` is
/// null. The choice is stored and survives a restart.
#[tauri::command]
pub async fn runtime_setup_location(
    app: AppHandle,
    path: Option<String>,
) -> Result<SetupReport, DwError> {
    let state = app.state::<AppState>();
    match path
        .map(|value| value.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        Some(value) => {
            let root = PathBuf::from(&value);
            crate::runtime::install::ensure_writable_root(&root)?;
            crate::runtime::sdk::set_configured_root(Some(root));
            state.store.set_setting("runtime.sdk_root", &value)?;
        }
        None => {
            crate::runtime::sdk::set_configured_root(None);
            state.store.set_setting("runtime.sdk_root", "")?;
        }
    }
    state.runtime.probe_now();
    Ok(setup_report(&state).await)
}

#[tauri::command]
pub fn runtime_stop_on_exit(app: AppHandle) -> bool {
    app.state::<AppState>().runtime.stop_on_exit()
}

#[tauri::command]
pub fn set_runtime_stop_on_exit(app: AppHandle, enabled: bool) -> Result<bool, DwError> {
    let state = app.state::<AppState>();
    state
        .store
        .set_setting("runtime.stop_on_exit", if enabled { "1" } else { "0" })?;
    state.runtime.set_stop_on_exit(enabled);
    Ok(enabled)
}

/// The name every surface shows. One stored setting is the authority, so the
/// title bar and the UI cannot disagree.
#[tauri::command]
pub fn app_display_name(app: AppHandle) -> String {
    branding::display_name(&app.state::<AppState>().store)
}

/// Change the display name. Only the branding changes: the executable, the
/// installer's product name and the bundle identifier stay as they were built,
/// which is what keeps shortcuts, updates and the single-instance lock working.
#[tauri::command]
pub fn set_app_display_name(app: AppHandle, name: String) -> Result<String, DwError> {
    let state = app.state::<AppState>();
    let saved = branding::set_display_name(&state.store, &name)?;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&saved);
    }
    Ok(saved)
}

/// The version this executable was built with. Read from the bundle rather than
/// written into the UI, so the About panel cannot drift from the manifests.
#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Return to the shipped name. An empty stored value is what the default is.
#[tauri::command]
pub fn reset_app_display_name(app: AppHandle) -> Result<String, DwError> {
    let state = app.state::<AppState>();
    let saved = branding::reset_display_name(&state.store)?;
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.set_title(&saved);
    }
    Ok(saved)
}

#[tauri::command]
pub async fn phone_stream_start(
    app: AppHandle,
    id: String,
    output: tauri::ipc::Channel<tauri::ipc::Response>,
) -> Result<(), DwError> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err(DwError::new(
            ErrorCode::InvalidRequest,
            "Invalid phone stream identifier.",
        ));
    }
    let state = app.state::<AppState>();
    let adb = state.runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
    })?;
    let serial = state.runtime.serial();
    // The mirror is a preview, not a recording master. Asking screenrecord for
    // fewer pixels cuts both the guest's encoder and this WebView's decoder
    // without changing what a touch means: points stay percentages of the
    // display, not pixels of the stream.
    let size = crate::runtime::display_override::read_size(&adb, &serial)
        .await
        .ok()
        .and_then(|(width, height)| crate::runtime::phone_stream::capture_size(width, height));
    state
        .phone_stream
        .start(adb, serial, id, size, output)
        .await
}

#[tauri::command]
pub async fn phone_dimensions(app: AppHandle) -> Result<(u32, u32), DwError> {
    let state = app.state::<AppState>();
    let adb = state.runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
    })?;
    let display = adb
        .display_focus(&state.runtime.serial())
        .await?
        .into_iter()
        .find(|display| display.display_id == crate::runtime::display::DEVICE_DISPLAY)
        .ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Android did not report the phone display.",
            )
        })?;
    match (display.width, display.height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => Ok((width, height)),
        _ => Err(DwError::new(
            ErrorCode::RuntimeUnavailable,
            "Android did not report the phone's dimensions.",
        )),
    }
}

#[tauri::command]
pub async fn phone_stream_stop(app: AppHandle, id: String) -> Result<(), DwError> {
    app.state::<AppState>().phone_stream.stop(Some(&id)).await;
    Ok(())
}

/// One still of the device's own screen: what the virtual phone is looking at.
///
/// It is deliberately a still, not a stream. WINAPK's app windows are already a
/// live view of their own Android displays; this is the phone's home screen, and
/// a caller decides how often to refresh it. Nothing is cached here, so an image
/// the UI shows is always one Android just produced.
#[tauri::command]
pub async fn phone_screen(app: AppHandle) -> Result<PhoneScreen, DwError> {
    let state = app.state::<AppState>();
    let (png, width, height) = state.runtime.device_screen().await?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    Ok(PhoneScreen {
        display_id: crate::runtime::display::DEVICE_DISPLAY,
        width,
        height,
        image: format!("data:image/png;base64,{encoded}"),
        captured_at: now_ms(),
    })
}

/// Tap the device's own display at a point of the capture the UI is showing.
#[tauri::command]
pub async fn phone_tap(app: AppHandle, x: u32, y: u32) -> Result<(), DwError> {
    check_touch(x, y)?;
    let state = app.state::<AppState>();
    state.runtime.device_tap(x, y).await
}

/// One drag on the device's own display — a swipe, for scrolling a list or
/// turning a launcher page.
#[tauri::command]
pub async fn phone_swipe(
    app: AppHandle,
    x1: u32,
    y1: u32,
    x2: u32,
    y2: u32,
    duration_ms: Option<u32>,
) -> Result<(), DwError> {
    check_touch(x1, y1)?;
    check_touch(x2, y2)?;
    // Android needs a duration for a swipe, and a slow one becomes a drag that
    // selects instead of scrolling.
    let duration = Duration::from_millis(u64::from(duration_ms.unwrap_or(220).clamp(20, 2_000)));
    let state = app.state::<AppState>();
    state
        .runtime
        .device_swipe((x1, y1), (x2, y2), duration)
        .await
}

/// Send one key to the device's own display: Enter, Tab, Back, Home, an arrow,
/// or any Android keycode name or number.
#[tauri::command]
pub async fn phone_key(app: AppHandle, key: String) -> Result<(), DwError> {
    let keycode = crate::agent::control::parse_key(&key)?;
    let state = app.state::<AppState>();
    state.runtime.device_key(keycode).await
}

/// Type text into whatever field the device's own display has focused.
#[tauri::command]
pub async fn phone_text(app: AppHandle, text: String) -> Result<(), DwError> {
    if text.chars().count() > 512 {
        return Err(DwError::new(
            ErrorCode::InvalidRequest,
            "That is more text than the phone can be given at once.",
        )
        .detail(format!("{} characters were sent.", text.chars().count())));
    }
    let state = app.state::<AppState>();
    state.runtime.device_text(&text).await
}

/// Where saved captures go: the user's own Pictures or Videos folder, in a
/// WINAPK subfolder. A machine that does not report those folders gets the
/// data directory instead, so a save never fails for lack of a destination —
/// and never lands in a system folder.
#[derive(Clone, Copy)]
enum CaptureKind {
    Picture,
    Video,
}

fn capture_directory(state: &AppState, kind: CaptureKind) -> PathBuf {
    let base = match kind {
        CaptureKind::Picture => dirs::picture_dir(),
        CaptureKind::Video => dirs::video_dir(),
    };
    base.unwrap_or_else(|| state.data_dir.clone())
        .join(crate::capture::FOLDER)
}

/// Capture the phone's screen into the user's Pictures folder.
#[tauri::command]
pub async fn phone_screenshot(app: AppHandle) -> Result<SavedShot, DwError> {
    let state = app.state::<AppState>();
    let directory = capture_directory(&state, CaptureKind::Picture);
    Capture::save_screenshot(&state.runtime, &directory).await
}

#[tauri::command]
pub async fn recording_start(app: AppHandle) -> Result<RecordingState, DwError> {
    let state = app.state::<AppState>();
    let directory = capture_directory(&state, CaptureKind::Video);
    state
        .capture
        .start_recording(&state.runtime, &directory)
        .await
}

#[tauri::command]
pub async fn recording_stop(app: AppHandle) -> Result<SavedRecording, DwError> {
    let state = app.state::<AppState>();
    state.capture.stop_recording(&state.runtime).await
}

#[tauri::command]
pub async fn recording_status(app: AppHandle) -> Result<RecordingState, DwError> {
    Ok(app.state::<AppState>().capture.recording_state().await)
}

/// What Android's accessibility tree says this app's controls are called.
///
/// Names only: the coordinates a virtual display reports are not in the
/// display's own space, so the panel offers the names and the user clicks the
/// real control to place the point.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DetectedControls {
    pub package_id: String,
    pub controls: Vec<String>,
    pub note: String,
}

#[tauri::command]
pub async fn detect_controls(
    app: AppHandle,
    package_id: String,
) -> Result<DetectedControls, DwError> {
    let state = app.state::<AppState>();
    let display = state.runtime.display_of(&package_id).ok_or_else(|| {
        DwError::new(
            ErrorCode::LaunchFailed,
            "Android has not reported a display for that window yet.",
        )
        .detail("Open the app, wait for its picture, then detect its controls.")
    })?;
    let adb = state.runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
    })?;
    let xml = adb.dump_hierarchy(&state.runtime.serial(), display).await?;
    let controls = crate::agent::ui::candidate_controls(&xml, &package_id, 24);
    let note = if controls.is_empty() {
        "Android exposed no named controls for this app — games that draw their own controls never do. Use Place on the game to click one.".to_string()
    } else {
        "Android names these controls but not in a coordinate space WINAPK can trust on a virtual display. Pick one, then click that control on the game to place its point.".to_string()
    };
    Ok(DetectedControls {
        package_id,
        controls,
        note,
    })
}

/// Record one app window's own Android display.
#[tauri::command]
pub async fn window_recording_start(
    app: AppHandle,
    package_id: String,
) -> Result<RecordingState, DwError> {
    let state = app.state::<AppState>();
    let display = state.runtime.display_of(&package_id).ok_or_else(|| {
        DwError::new(
            ErrorCode::LaunchFailed,
            "Android has not reported a display for that window yet.",
        )
        .detail("Wait for the window to show its picture, then record it.")
    })?;
    let directory = capture_directory(&state, CaptureKind::Video);
    state
        .capture
        .start_window_recording(&state.runtime, &package_id, display, &directory)
        .await
}

#[tauri::command]
pub async fn window_recording_stop(app: AppHandle) -> Result<SavedRecording, DwError> {
    app.state::<AppState>()
        .capture
        .stop_window_recording()
        .await
}

#[tauri::command]
pub async fn window_recording_status(app: AppHandle) -> Result<RecordingState, DwError> {
    Ok(app
        .state::<AppState>()
        .capture
        .window_recording_state()
        .await)
}

/// The phone display's own size and density, as Android reports them.
#[tauri::command]
pub async fn phone_display(app: AppHandle) -> Result<DisplayInfo, DwError> {
    let state = app.state::<AppState>();
    let adb = state.runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
    })?;
    display_override::read(&adb, &state.runtime.serial()).await
}

/// Resize the phone's display without restarting the emulator.
#[tauri::command]
pub async fn set_phone_display(
    app: AppHandle,
    width: u32,
    height: u32,
    density: Option<u32>,
) -> Result<DisplayInfo, DwError> {
    let state = app.state::<AppState>();
    let adb = state.runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
    })?;
    display_override::apply(&adb, &state.runtime.serial(), width, height, density).await
}

#[tauri::command]
pub async fn reset_phone_display(app: AppHandle) -> Result<DisplayInfo, DwError> {
    let state = app.state::<AppState>();
    let adb = state.runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
    })?;
    display_override::reset(&adb, &state.runtime.serial()).await
}

/// Save one of the named performance presets. It is a real plan, validated the
/// same way a hand-written one is.
#[tauri::command]
pub async fn hardware_preset(app: AppHandle, preset: String) -> Result<HardwareReport, DwError> {
    let state = app.state::<AppState>();
    let parsed = Preset::parse(&preset).ok_or_else(|| {
        DwError::new(
            ErrorCode::InvalidConfig,
            "Choose the balanced, performance or compatibility preset.",
        )
        .detail(format!("\"{preset}\" is not a preset WINAPK knows."))
    })?;
    let host = hardware::host_facts();
    let plan = hardware::preset_plan(&host, parsed);
    hardware::persist(&state.store, &plan)?;
    Ok(state.runtime.hardware_report(&state.store).await)
}

/// Show a file WINAPK saved in Explorer. Only files inside WINAPK's own
/// capture folders (or its data directory) can be revealed, so a stray path from
/// a page cannot be used to open something arbitrary.
#[tauri::command]
pub async fn reveal_path(app: AppHandle, path: String) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    let target = PathBuf::from(&path);
    let canonical = target.canonicalize().map_err(|e| {
        DwError::new(ErrorCode::InvalidPath, "That file is no longer there.")
            .detail(format!("{}: {e}", target.display()))
    })?;
    if !canonical.is_file() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "That path is not a file WINAPK can open.",
        ));
    }
    let allowed = [
        capture_directory(&state, CaptureKind::Picture),
        capture_directory(&state, CaptureKind::Video),
        state.data_dir.clone(),
    ];
    let inside = allowed
        .iter()
        .filter_map(|dir| dir.canonicalize().ok())
        .any(|dir| canonical.starts_with(&dir));
    if !inside {
        return Err(DwError::new(
            ErrorCode::PermissionDenied,
            "WINAPK only opens the files it saved itself.",
        )
        .detail("Choose a capture from the phone screen; other paths are left to Explorer."));
    }
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.arg(format!("/select,{}", canonical.display()));
    crate::proc::no_console(&mut cmd);
    cmd.spawn().map_err(|e| {
        DwError::io(
            ErrorCode::BackendFailure,
            "Could not open the file in Explorer.",
            &e,
        )
    })?;
    Ok(())
}

/// A phone touch is a pixel pair on a real display. The UI converts a click with
/// the capture size it is displaying, so a value beyond any display means the
/// caller sent something that is not a point — refuse it rather than pass it to
/// Android.
fn check_touch(x: u32, y: u32) -> DwResult<()> {
    const MAX_EDGE: u32 = 20_000;
    if x < MAX_EDGE && y < MAX_EDGE {
        Ok(())
    } else {
        Err(DwError::new(
            ErrorCode::InvalidRequest,
            "That point is not on the phone's screen.",
        )
        .detail(format!(
            "({x}, {y}) is beyond any display WINAPK can drive."
        )))
    }
}

#[tauri::command]
pub async fn inspect_apk(app: AppHandle, path: String) -> Result<ApkInspection, DwError> {
    let state = app.state::<AppState>();
    let aapt2 = state.runtime.probe().aapt2.map(PathBuf::from);
    let apk_path = PathBuf::from(path);
    let result = tokio::task::spawn_blocking(move || apk::inspect(&apk_path, aapt2.as_deref()))
        .await
        .map_err(|e| {
            DwError::new(
                ErrorCode::BackendFailure,
                "The inspector stopped unexpectedly.",
            )
            .detail(e.to_string())
        })??;
    Ok(result)
}

#[tauri::command]
pub async fn install_apk(
    app: AppHandle,
    path: String,
    launch: bool,
    allow_old_target: Option<bool>,
) -> Result<InstallOutcome, DwError> {
    let state = app.state::<AppState>();
    let store = Arc::clone(&state.store);
    let runtime = Arc::clone(&state.runtime);
    let op = op_id();
    let started = now_ms();

    emit(&app, &op, Stage::Received, "Received the package.", None);

    let apk_path = PathBuf::from(&path);
    let size = match apk::validate(&apk_path) {
        Ok(size) => size,
        Err(err) => return Err(fail(&app, &store, &op, "install", started, err)),
    };
    emit(
        &app,
        &op,
        Stage::Validating,
        &format!("Validated a {} package.", human_size(size)),
        None,
    );

    let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
    let inspect_path = apk_path.clone();
    let inspection =
        match tokio::task::spawn_blocking(move || apk::inspect(&inspect_path, aapt2.as_deref()))
            .await
        {
            Ok(Ok(inspection)) => inspection,
            Ok(Err(err)) => return Err(fail(&app, &store, &op, "install", started, err)),
            Err(err) => {
                let err = DwError::new(
                    ErrorCode::BackendFailure,
                    "The inspector stopped unexpectedly.",
                )
                .detail(err.to_string());
                return Err(fail(&app, &store, &op, "install", started, err));
            }
        };
    emit(
        &app,
        &op,
        Stage::Inspecting,
        &format!(
            "{} {}{}",
            inspection
                .label
                .as_deref()
                .unwrap_or(&inspection.package_id),
            inspection.version_name.as_deref().unwrap_or(""),
            inspection
                .min_sdk
                .map(|s| format!(" · min SDK {s}"))
                .unwrap_or_default()
        ),
        Some(inspection.package_id.clone()),
    );

    emit(
        &app,
        &op,
        Stage::CheckingRuntime,
        "Checking the runtime.",
        Some(inspection.package_id.clone()),
    );
    let outcome = match runtime
        .install(&inspection, &store, &op, allow_old_target.unwrap_or(false))
        .await
    {
        Ok(outcome) => outcome,
        Err(err) => return Err(fail(&app, &store, &op, "install", started, err)),
    };

    record(
        &store,
        OperationRecord::finished(
            &op,
            "install",
            "completed",
            started,
            &outcome.message,
            Some(outcome.package_id.clone()),
            None,
        ),
    );

    if launch {
        match runtime
            .launch(
                &outcome.package_id,
                inspection.label.as_deref(),
                &store,
                &op,
            )
            .await
        {
            Ok(_) => {}
            Err(err) => {
                emit(
                    &app,
                    &op,
                    Stage::Failed,
                    &err.summary,
                    Some(outcome.package_id.clone()),
                );
                record(
                    &store,
                    OperationRecord::finished(
                        &op,
                        "install",
                        "launch_failed",
                        started,
                        &err.summary,
                        Some(outcome.package_id.clone()),
                        err.detail.clone(),
                    ),
                );
                return Err(err);
            }
        }
    }

    emit(
        &app,
        &op,
        Stage::Completed,
        "Done.",
        Some(outcome.package_id.clone()),
    );
    state.diagnose.invalidate();
    Ok(outcome)
}

#[tauri::command]
pub async fn list_apps(app: AppHandle) -> Result<Vec<InstalledApp>, DwError> {
    let state = app.state::<AppState>();
    state.runtime.list_apps(&state.store).await
}

#[tauri::command]
pub async fn launch_app(app: AppHandle, package_id: String) -> Result<AppWindow, DwError> {
    let state = app.state::<AppState>();
    let label = state
        .store
        .cached_apps()
        .into_iter()
        .find(|c| c.package_id == package_id)
        .and_then(|c| c.label);
    // A launch changes exactly what a report measured a moment ago.
    state.diagnose.invalidate();
    let window = state
        .runtime
        .launch(&package_id, label.as_deref(), &state.store, "launch")
        .await?;
    let _ = state.store.record_launch(&package_id, now_ms());
    Ok(window)
}

/// One still of the app window's own display, for the rail's Screenshot button.
#[tauri::command]
pub async fn app_screenshot(app: AppHandle, package_id: String) -> Result<SavedShot, DwError> {
    let state = app.state::<AppState>();
    let display = state.runtime.display_of(&package_id).ok_or_else(|| {
        DwError::new(
            ErrorCode::LaunchFailed,
            "Android has not reported a display for that window yet.",
        )
        .detail("Give the window a moment, then take the screenshot again.")
    })?;
    let directory = capture_directory(&state, CaptureKind::Picture);
    Capture::save_display_screenshot(&state.runtime, &directory, display).await
}

/// Open the app in the other orientation.
///
/// A window's Android display is created when the window opens, and nothing can
/// rotate it afterwards, so this records the choice and opens the window again
/// on a surface of the other shape. The stored choice is what makes it stick.
#[tauri::command]
pub async fn rotate_app_window(app: AppHandle, package_id: String) -> Result<AppWindow, DwError> {
    let state = app.state::<AppState>();
    let current = state
        .runtime
        .geometry_of(&package_id)
        .map(|surface| surface.geometry.orientation)
        .or_else(|| {
            state
                .store
                .setting(&Orientation::setting_key(&package_id))
                .and_then(|value| Orientation::parse(&value))
        })
        .unwrap_or(Orientation::Portrait);
    let next = current.flipped();
    // An app that declares one orientation does not adapt to the other surface;
    // Android gives it a letterboxed portrait (or landscape) window inside a
    // frame of the wrong shape. Refusing here is why the rotated frame is the
    // shape it claims to be, instead of a strip with bars around it.
    if let Some(fixed) = state.runtime.fixed_orientation(&package_id).await {
        if fixed != next {
            let label = state
                .store
                .cached_app(&package_id)
                .and_then(|cached| cached.label)
                .unwrap_or_else(|| package_id.clone());
            return Err(DwError::new(
                ErrorCode::NotSupported,
                format!(
                    "{} only opens in {}.",
                    label,
                    fixed.as_str()
                ),
            )
            .detail(
                "Its Android activity declares that orientation, so a window of the other shape would letterbox it. The window was left as it is.",
            ));
        }
    }
    state
        .store
        .set_setting(&Orientation::setting_key(&package_id), next.as_str())?;
    state.diagnose.invalidate();
    state.input.release_package(&package_id);
    state.input.flush().await;
    state.runtime.close_window(&package_id).await?;
    let label = state
        .store
        .cached_apps()
        .into_iter()
        .find(|c| c.package_id == package_id)
        .and_then(|c| c.label);
    state
        .runtime
        .launch(&package_id, label.as_deref(), &state.store, "rotate")
        .await
}

#[tauri::command]
pub async fn close_app_window(app: AppHandle, package_id: String) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    state.diagnose.invalidate();
    state.input.release_package(&package_id);
    state.input.flush().await;
    state.runtime.close_window(&package_id).await
}

#[tauri::command]
pub async fn stop_app(app: AppHandle, package_id: String) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    state.diagnose.invalidate();
    state.input.release_package(&package_id);
    state.input.flush().await;
    state.runtime.close_window(&package_id).await?;
    state.runtime.stop_app(&package_id).await
}

#[tauri::command]
pub async fn uninstall_app(
    app: AppHandle,
    package_id: String,
    delete_data: bool,
) -> Result<String, DwError> {
    let state = app.state::<AppState>();
    let started = now_ms();
    let op = op_id();
    state.input.release_package(&package_id);
    state.input.flush().await;
    match state
        .runtime
        .uninstall(&package_id, delete_data, &state.store)
        .await
    {
        // The runtime's sentence is recorded verbatim: it says what Android
        // reported about retained data, not what we hoped happened.
        Ok(message) => {
            state.diagnose.invalidate();
            record(
                &state.store,
                OperationRecord::finished(
                    &op,
                    "uninstall",
                    "completed",
                    started,
                    &message,
                    Some(package_id),
                    None,
                ),
            );
            Ok(message)
        }
        Err(err) => Err(fail(&app, &state.store, &op, "uninstall", started, err)),
    }
}

/// Paths an earlier launch queued (cold start or second instance). The webview
/// asks once it is ready, so nothing is emitted before it can listen.
#[tauri::command]
pub async fn take_pending_activation(app: AppHandle) -> Result<Vec<String>, DwError> {
    let state = app.state::<AppState>();
    let mut queued = state.pending_activation.lock().map_err(|_| {
        DwError::new(
            ErrorCode::BackendFailure,
            "The activation queue is unavailable.",
        )
    })?;
    let paths = std::mem::take(&mut *queued);
    if !paths.is_empty() {
        tracing::info!("activation: {} package path(s) queued", paths.len());
    }
    Ok(paths)
}

#[tauri::command]
pub async fn activity(
    app: AppHandle,
    limit: Option<usize>,
) -> Result<Vec<OperationRecord>, DwError> {
    let state = app.state::<AppState>();
    Ok(state.store.operations(limit.unwrap_or(20)))
}

#[tauri::command]
pub async fn paths(app: AppHandle) -> Result<Paths, DwError> {
    let state = app.state::<AppState>();
    Ok(Paths {
        data_dir: state.data_dir.display().to_string(),
        log_dir: crate::diag::log_dir(&state.data_dir).display().to_string(),
        avd: AVD_NAME.to_string(),
        system_image: SYSTEM_IMAGE.to_string(),
    })
}

#[tauri::command]
pub async fn open_logs(app: AppHandle) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    let dir = crate::diag::log_dir(&state.data_dir);
    let mut cmd = std::process::Command::new("explorer.exe");
    cmd.arg(&dir);
    crate::proc::no_console(&mut cmd);
    cmd.spawn().map_err(|e| {
        DwError::io(
            ErrorCode::BackendFailure,
            "Could not open the log folder.",
            &e,
        )
    })?;
    Ok(())
}

#[tauri::command]
pub async fn agent_status(app: AppHandle) -> Result<AgentStatus, DwError> {
    let state = app.state::<AppState>();
    Ok(state.agent.status().await)
}

#[tauri::command]
pub async fn agent_start(app: AppHandle) -> Result<AgentStatus, DwError> {
    let state = app.state::<AppState>();
    Arc::clone(&state.agent).start().await?;
    Ok(state.agent.status().await)
}

#[tauri::command]
pub async fn agent_stop(app: AppHandle) -> Result<AgentStatus, DwError> {
    let state = app.state::<AppState>();
    state.agent.stop().await;
    Ok(state.agent.status().await)
}

/// Pausing stops waits that are already in flight, not just the next call.
#[tauri::command]
pub async fn agent_set_paused(app: AppHandle, paused: bool) -> Result<AgentStatus, DwError> {
    let state = app.state::<AppState>();
    state.agent.set_paused(paused)?;
    Ok(state.agent.status().await)
}

#[tauri::command]
pub async fn agent_set_permission(
    app: AppHandle,
    permission: String,
) -> Result<AgentStatus, DwError> {
    let state = app.state::<AppState>();
    let parsed = Permission::parse(&permission).ok_or_else(|| {
        DwError::new(
            ErrorCode::InvalidConfig,
            "Choose read-only or control for agent access.",
        )
    })?;
    state.agent.set_permission(parsed)?;
    Ok(state.agent.status().await)
}

#[tauri::command]
pub async fn agent_actions(
    app: AppHandle,
    limit: Option<usize>,
) -> Result<Vec<ActionRecord>, DwError> {
    let state = app.state::<AppState>();
    Ok(state.agent.log().recent(limit.unwrap_or(30)))
}

#[tauri::command]
pub async fn agent_skills(app: AppHandle) -> Result<Vec<skills::SkillInfo>, DwError> {
    let state = app.state::<AppState>();
    skills::discover(&state.data_dir, &state.store)
}

#[tauri::command]
pub async fn agent_set_skill(
    app: AppHandle,
    id: String,
    enabled: bool,
) -> Result<Vec<skills::SkillInfo>, DwError> {
    let state = app.state::<AppState>();
    skills::set_enabled(&state.data_dir, &state.store, &id, enabled)?;
    skills::discover(&state.data_dir, &state.store)
}

#[tauri::command]
pub async fn agent_install_skill(
    app: AppHandle,
    path: String,
) -> Result<Vec<skills::SkillInfo>, DwError> {
    let state = app.state::<AppState>();
    let source = PathBuf::from(path);
    skills::install(&state.data_dir, &source)?;
    skills::discover(&state.data_dir, &state.store)
}

#[tauri::command]
pub async fn agent_remove_skill(
    app: AppHandle,
    id: String,
) -> Result<Vec<skills::SkillInfo>, DwError> {
    let state = app.state::<AppState>();
    skills::remove(&state.data_dir, &state.store, &id)?;
    skills::discover(&state.data_dir, &state.store)
}

/// The exact tool surface an agent sees, for the UI's capability list.
#[tauri::command]
pub async fn input_state(app: AppHandle, package_id: String) -> Result<InputState, DwError> {
    let state = app.state::<AppState>();
    Ok(state.input.state(&package_id))
}

/// Add or replace one key's mapping. Saving does not switch mapping on: the
/// profile stays off until the user enables it.
#[tauri::command]
pub async fn save_input_mapping(
    app: AppHandle,
    package_id: String,
    key: u32,
    action: InputAction,
) -> Result<InputState, DwError> {
    let state = app.state::<AppState>();
    let next = state.input.save(&package_id, key, action)?;
    // A point picked on the game belongs to the mapping that was just saved;
    // the panel must not offer it again without a fresh click.
    state.overlay.clear_pending();
    Ok(next)
}

#[tauri::command]
pub async fn remove_input_mapping(
    app: AppHandle,
    package_id: String,
    key: u32,
) -> Result<InputState, DwError> {
    let state = app.state::<AppState>();
    state.input.remove(&package_id, key)
}

#[tauri::command]
pub async fn set_input_mapping_enabled(
    app: AppHandle,
    package_id: String,
    key: u32,
    enabled: bool,
) -> Result<InputState, DwError> {
    let state = app.state::<AppState>();
    state.input.set_mapping_enabled(&package_id, key, enabled)
}

/// The per-app switch. Enabling arms the keyboard hook; disabling releases any
/// key this app's profile was holding, so the app keeps no pressed touch.
#[tauri::command]
pub async fn set_input_profile_enabled(
    app: AppHandle,
    package_id: String,
    enabled: bool,
) -> Result<InputState, DwError> {
    let state = app.state::<AppState>();
    state.input.set_enabled(&package_id, enabled)
}

#[tauri::command]
pub async fn reset_input_mappings(
    app: AppHandle,
    package_id: String,
) -> Result<InputState, DwError> {
    let state = app.state::<AppState>();
    state.input.reset(&package_id)
}

/// The universal Android shortcuts, as they are bound right now.
fn universal_list(controller: &crate::input::InputController) -> Vec<serde_json::Value> {
    controller
        .universal_bindings()
        .into_iter()
        .map(|(action, chord)| {
            serde_json::json!({
                "action": action.id(),
                "label": action.label(),
                "chord": chord.label(),
                "android_key": action.android_key(),
            })
        })
        .collect()
}

#[tauri::command]
pub async fn universal_bindings(app: AppHandle) -> Result<Vec<serde_json::Value>, DwError> {
    let state = app.state::<AppState>();
    Ok(universal_list(&state.input))
}

#[tauri::command]
pub async fn set_universal_binding(
    app: AppHandle,
    action: String,
    chord: String,
) -> Result<Vec<serde_json::Value>, DwError> {
    let state = app.state::<AppState>();
    let action = crate::input::UniversalAction::from_id(&action).ok_or_else(|| {
        DwError::new(ErrorCode::InvalidConfig, "That is not a universal control.")
            .detail("Use back, home, recents, menu, screenshot, rotate or show_hide.")
    })?;
    let parsed = crate::input::Chord::parse(&chord).ok_or_else(|| {
        DwError::new(
            ErrorCode::InvalidConfig,
            "That is not a keyboard combination.",
        )
        .detail("Use something like Shift+A — a modifier plus one key.")
    })?;
    state.input.set_universal_binding(action, parsed)?;
    Ok(universal_list(&state.input))
}

#[tauri::command]
pub async fn agent_tool_catalog(app: AppHandle) -> Result<Vec<serde_json::Value>, DwError> {
    let _ = app;
    Ok(agent::mcp::descriptors())
}

/// What needs attention right now. `deep` also captures each live window's
/// display, which costs two screenshots per window and is therefore only run
/// when the user asks for a full check.
#[tauri::command]
pub async fn attention_report(app: AppHandle, deep: Option<bool>) -> Result<Report, DwError> {
    let state = app.state::<AppState>();
    Ok(state.diagnose.report(deep.unwrap_or(false)).await)
}

/// Run one targeted recovery. The returned report is the state it left behind.
#[tauri::command]
pub async fn attention_recover(
    app: AppHandle,
    recovery: Recovery,
) -> Result<RecoveryOutcome, DwError> {
    let state = app.state::<AppState>();
    Ok(state.diagnose.recover(&recovery).await)
}

fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{} KB", (bytes as f64 / 1024.0).round() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::{apk_paths_from_args, check_touch};
    use crate::domain::ErrorCode;

    #[test]
    fn finds_one_package_among_shell_arguments() {
        let args = [
            "WINAPK.exe",
            "--flag",
            "C:\\Users\\tejas\\Downloads\\FDroid.apk",
            "notes.txt",
        ];
        assert_eq!(
            apk_paths_from_args(args),
            vec!["C:\\Users\\tejas\\Downloads\\FDroid.apk".to_string()]
        );
    }

    #[test]
    fn keeps_spaces_and_unicode_in_the_path() {
        let args = ["C:\\Users\\tejas\\Downloads\\音楽 アプリ.apk"];
        assert_eq!(apk_paths_from_args(args), args);
    }

    #[test]
    fn strips_a_surviving_quote_pair_without_splitting_the_path() {
        let args = ["\"C:\\My Downloads\\F Droid.apk\""];
        assert_eq!(
            apk_paths_from_args(args),
            vec!["C:\\My Downloads\\F Droid.apk".to_string()]
        );
    }

    #[test]
    fn extension_match_is_case_insensitive_and_ignores_flags() {
        let args = ["app.APK", "--apk", ".apk"];
        assert_eq!(apk_paths_from_args(args), vec!["app.APK".to_string()]);
    }

    #[test]
    fn keeps_every_package_when_several_are_requested() {
        let args = ["a.apk", "b.apk"];
        assert_eq!(apk_paths_from_args(args).len(), 2);
    }

    #[test]
    fn a_phone_touch_must_be_a_point_on_a_real_display() {
        // A 1080x2400 capture is the ordinary case, and its last pixel is valid.
        assert!(check_touch(0, 0).is_ok());
        assert!(check_touch(1079, 2399).is_ok());
        // Values a broken converter could produce are refused, not sent to Android.
        for (x, y) in [(u32::MAX, 0), (0, u32::MAX), (20_000, 20_000)] {
            let err = check_touch(x, y).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidRequest);
            assert!(
                err.summary.contains("not on the phone's screen"),
                "{}",
                err.summary
            );
        }
    }
}
