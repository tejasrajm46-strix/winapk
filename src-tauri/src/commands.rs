//! Command layer. Parses arguments, calls one service, records the operation and
//! returns a typed result. No backend command strings and no business logic.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::apk;
use crate::domain::{
    now_ms, ApkInspection, AppWindow, DwError, EnvReport, ErrorCode, InstalledApp, Stage,
    StageEvent,
};
use crate::runtime::emulator_runtime::{AVD_NAME, SYSTEM_IMAGE};
use crate::runtime::sdk::Sdk;
use crate::runtime::{EmulatorRuntime, InstallOutcome, RuntimeEvents};
use crate::store::{OperationRecord, Store};

pub struct AppState {
    pub runtime: Arc<EmulatorRuntime>,
    pub store: Arc<Store>,
    pub data_dir: PathBuf,
    /// Keeps the non-blocking log writer alive for the process lifetime.
    pub log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

/// Bridges runtime notifications into webview events.
pub struct TauriEvents(pub AppHandle);

impl RuntimeEvents for TauriEvents {
    fn stage(&self, event: &StageEvent) {
        let _ = self.0.emit("op://stage", event);
    }

    fn status(&self, status: &crate::domain::RuntimeStatus) {
        let _ = self.0.emit("runtime://status", status);
    }

    fn apps_changed(&self) {
        let _ = self.0.emit("apps://changed", ());
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
    Ok(state.runtime.probe())
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
        env: state.runtime.probe(),
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

#[tauri::command]
pub async fn runtime_stop(app: AppHandle) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    state.runtime.shutdown().await;
    Ok(())
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
    let outcome = match runtime.install(&inspection, &store, &op).await {
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
    let window = state
        .runtime
        .launch(&package_id, label.as_deref(), &state.store, "launch")
        .await?;
    let _ = state.store.record_launch(&package_id, now_ms());
    Ok(window)
}

#[tauri::command]
pub async fn close_app_window(app: AppHandle, package_id: String) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    state.runtime.close_window(&package_id).await
}

#[tauri::command]
pub async fn stop_app(app: AppHandle, package_id: String) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    state.runtime.close_window(&package_id).await?;
    state.runtime.stop_app(&package_id).await
}

#[tauri::command]
pub async fn uninstall_app(
    app: AppHandle,
    package_id: String,
    delete_data: bool,
) -> Result<(), DwError> {
    let state = app.state::<AppState>();
    let started = now_ms();
    let op = op_id();
    match state
        .runtime
        .uninstall(&package_id, delete_data, &state.store)
        .await
    {
        Ok(()) => {
            record(
                &state.store,
                OperationRecord::finished(
                    &op,
                    "uninstall",
                    "completed",
                    started,
                    if delete_data {
                        "Uninstalled and deleted app data"
                    } else {
                        "Uninstalled, app data kept"
                    },
                    Some(package_id),
                    None,
                ),
            );
            Ok(())
        }
        Err(err) => Err(fail(&app, &state.store, &op, "uninstall", started, err)),
    }
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
    std::process::Command::new("explorer.exe")
        .arg(&dir)
        .spawn()
        .map_err(|e| {
            DwError::io(
                ErrorCode::BackendFailure,
                "Could not open the log folder.",
                &e,
            )
        })?;
    Ok(())
}

fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{} KB", (bytes as f64 / 1024.0).round() as u64)
    }
}
