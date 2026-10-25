//! DroidWin desktop shell wiring: one window, one runtime, one store.

pub mod apk;
pub mod commands;
pub mod diag;
pub mod domain;
pub mod runtime;
pub mod store;

use std::sync::Arc;

use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();

    builder
        // Must be registered first: a second launch forwards its arguments instead
        // of opening a second manager window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(path) = argv
                .iter()
                .find(|arg| arg.to_ascii_lowercase().ends_with(".apk"))
            {
                let _ = app.emit("activation://apk", path.clone());
            }
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // DROIDWIN_HOME lets a development run keep its database, logs and
            // bundled window host inside the checkout instead of %APPDATA%.
            let data_dir = match std::env::var("DROIDWIN_HOME") {
                Ok(value) if !value.trim().is_empty() => std::path::PathBuf::from(value),
                _ => app.path().app_data_dir().map_err(|e| e.to_string())?,
            };
            std::fs::create_dir_all(&data_dir)?;

            let guard = diag::init(&diag::log_dir(&data_dir));
            tracing::info!("DroidWin starting, data dir {}", data_dir.display());

            let store = Arc::new(
                store::Store::open(&data_dir.join("droidwin.db"))
                    .map_err(|e| format!("{} {:?}", e.summary, e.detail))?,
            );
            let events = Arc::new(commands::TauriEvents(app.handle().clone()));
            let runtime = Arc::new(runtime::EmulatorRuntime::new(data_dir.clone(), events));

            app.manage(commands::AppState {
                runtime,
                store,
                data_dir,
                log_guard: guard,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::env_probe,
            commands::runtime_status,
            commands::runtime_probe,
            commands::runtime_start,
            commands::runtime_stop,
            commands::inspect_apk,
            commands::install_apk,
            commands::list_apps,
            commands::launch_app,
            commands::close_app_window,
            commands::stop_app,
            commands::uninstall_app,
            commands::activity,
            commands::paths,
            commands::open_logs,
        ])
        .run(tauri::generate_context!())
        .expect("DroidWin failed to start");
}
