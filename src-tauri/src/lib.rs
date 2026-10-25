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
/// Copy the bundled window host into the writable data directory on first run.
/// A failure is logged and left to the runtime probe, which reports the missing
/// piece as a blocking item instead of pretending windows will open.
fn provision_window_host(data_dir: &std::path::Path, resources: Option<std::path::PathBuf>) {
    let target = data_dir.join("runtime").join("bin").join("scrcpy");
    let exe = if cfg!(windows) {
        "scrcpy.exe"
    } else {
        "scrcpy"
    };
    if target.join(exe).is_file() {
        return;
    }
    let Some(source) = resources.map(|dir| dir.join("resources").join("scrcpy")) else {
        return;
    };
    if !source.is_dir() {
        tracing::warn!(
            "bundled window host not found at {}; scrcpy on PATH will be used",
            source.display()
        );
        return;
    }
    match copy_tree(&source, &target) {
        Ok(()) => tracing::info!(
            "provisioned the bundled window host into {}",
            target.display()
        ),
        Err(err) => tracing::warn!("could not provision the window host: {err}"),
    }
}

fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

pub fn run() {
    let builder = tauri::Builder::default();

    builder
        // Must be registered first: a second launch forwards its arguments instead
        // of opening a second manager window.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let paths = commands::apk_paths_from_args(argv);
            let forwarded = paths.len();
            if let Some(first) = paths.first().cloned() {
                // Queue the full list for the webview, then nudge the one it
                // already has a listener for.
                if let Some(state) = app.try_state::<commands::AppState>() {
                    if let Ok(mut queued) = state.pending_activation.lock() {
                        queued.extend(paths);
                    }
                }
                let _ = app.emit("activation://apk", first);
                tracing::info!("forwarded {forwarded} package path(s) to the running window");
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

            // The installed app carries the Apache-2.0 scrcpy build; copy it into
            // the data directory once so every later launch finds one host.
            provision_window_host(&data_dir, app.path().resource_dir().ok());

            let store = Arc::new(
                store::Store::open(&data_dir.join("droidwin.db"))
                    .map_err(|e| format!("{} {:?}", e.summary, e.detail))?,
            );
            let events = Arc::new(commands::TauriEvents(app.handle().clone()));
            let runtime = Arc::new(runtime::EmulatorRuntime::new(data_dir.clone(), events));

            // A cold start from Explorer carries the package as argv[1]. The
            // webview is not listening yet, so hold the paths until it asks.
            let startup_paths = commands::apk_paths_from_args(
                std::env::args_os()
                    .skip(1)
                    .map(|arg| arg.to_string_lossy().into_owned()),
            );
            if !startup_paths.is_empty() {
                tracing::info!(
                    "started with {} package path(s) on the command line",
                    startup_paths.len()
                );
            }

            app.manage(commands::AppState {
                runtime,
                store,
                data_dir,
                log_guard: guard,
                pending_activation: std::sync::Mutex::new(startup_paths),
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
            commands::take_pending_activation,
        ])
        .run(tauri::generate_context!())
        .expect("DroidWin failed to start");
}
