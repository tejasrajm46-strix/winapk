//! DroidWin desktop shell wiring: one window, one runtime, one store.

pub mod agent;
pub mod apk;
pub mod capture;
pub mod cli;
pub mod commands;
pub mod diag;
pub mod diagnose;
pub mod domain;
pub mod input;
pub mod overlay;
pub mod proc;
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
            // The SDK locator needs the data directory before anything probes
            // it: DroidWin's own Android SDK lives there. A location the user
            // chose on the setup screen outranks every discovered one.
            runtime::sdk::configure(&data_dir);
            if let Some(root) = store
                .setting("runtime.sdk_root")
                .filter(|value| !value.trim().is_empty())
            {
                runtime::sdk::set_configured_root(Some(std::path::PathBuf::from(root)));
            }

            let bridge = commands::TauriEvents::new(app.handle().clone());
            let bus = bridge.bus.clone();
            let runtime = Arc::new(runtime::EmulatorRuntime::new(
                data_dir.clone(),
                Arc::new(bridge),
            ));
            if store.setting("runtime.stop_on_exit").as_deref() == Some("0") {
                runtime.set_stop_on_exit(false);
            }

            // The runtime installer reports as it works; the setup screen follows
            // the same events a boot does.
            let setup = Arc::new(runtime::install::Setup::new(data_dir.clone(), {
                let handle = app.handle().clone();
                Arc::new(move |progress| {
                    let _ = handle.emit("runtime://setup", &progress);
                })
            }));

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

            // Keyboard mapping shares the runtime and is also used by the
            // agent so both paths release held touches before closing a window.
            let input = input::InputController::new(Arc::clone(&runtime), Arc::clone(&store));
            input.restore();

            // One capture session for the window and the agent: a recording
            // started from the command line is the one the phone screen stops.
            let capture = Arc::new(capture::Capture::default());

            let agent = agent::AgentState::with_events_input_and_capture(
                Arc::clone(&runtime),
                Arc::clone(&store),
                data_dir.clone(),
                bus.clone(),
                Arc::clone(&input),
                Arc::clone(&capture),
            );

            // Diagnostics observe the same runtime and the same lifecycle feed;
            // they never start a process or a poller of their own.
            let diagnose =
                diagnose::Diagnoser::new(Arc::clone(&runtime), Arc::clone(&store), bus.clone());

            // The overlay draws over whichever app window is in front. It owns
            // no input path of its own: a nav click is the same adb key event a
            // mapping sends. `Ctrl+Shift+R` arrives through the keyboard hook
            // and is handed to this thread, which never runs on the hook.
            let overlay = overlay::Overlay::new(
                app.handle().clone(),
                Arc::clone(&runtime),
                Arc::clone(&store),
                Arc::clone(&input),
            );
            let (toggle_tx, toggle_rx) = std::sync::mpsc::channel::<String>();
            input.set_overlay_toggle(toggle_tx);
            {
                let overlay = Arc::clone(&overlay);
                std::thread::Builder::new()
                    .name("droidwin-overlay-toggle".into())
                    .spawn(move || {
                        while let Ok(package) = toggle_rx.recv() {
                            if let Err(err) = overlay.toggle_labels(package) {
                                tracing::warn!(
                                    "Ctrl+Shift+R could not toggle the overlay: {}",
                                    err.summary
                                );
                            }
                        }
                    })
                    .ok();
            }

            // Shift+P and Shift+O are Android controls too, but they act on the
            // window rather than sending a key, so they run through the same
            // commands the action rail calls.
            {
                let (sink_tx, sink_rx) =
                    std::sync::mpsc::channel::<(String, input::UniversalAction)>();
                input.set_universal_sink(sink_tx);
                let handle = app.handle().clone();
                std::thread::Builder::new()
                    .name("droidwin-universal".into())
                    .spawn(move || {
                        while let Ok((package, action)) = sink_rx.recv() {
                            let app = handle.clone();
                            tauri::async_runtime::spawn(async move {
                                match action {
                                    input::UniversalAction::Screenshot => {
                                        if let Err(err) =
                                            commands::app_screenshot(app, package).await
                                        {
                                            tracing::warn!("Shift+P failed: {}", err.summary);
                                        }
                                    }
                                    input::UniversalAction::Rotate => {
                                        if let Err(err) =
                                            commands::rotate_app_window(app, package).await
                                        {
                                            tracing::warn!("Shift+O failed: {}", err.summary);
                                        }
                                    }
                                    _ => {}
                                }
                            });
                        }
                    })
                    .ok();
            }

            // An endpoint the user left running comes back with the app, so a
            // client's saved configuration keeps working. A failure is only
            // logged: the Agent screen shows the stopped state.
            if agent.should_restore() {
                let restored = Arc::clone(&agent);
                if let Err(err) =
                    tauri::async_runtime::block_on(async move { restored.start().await })
                {
                    tracing::warn!("agent endpoint did not restart: {}", err.summary);
                }
            }

            // The Android runtime comes up with the shell: the window opens on a
            // machine where the emulator may be cold, and the first thing anyone
            // does is use an app, not press start. `ensure_ready` is single
            // flight, so this is a no-op when the emulator is already running,
            // and a failure is logged for the doctor screen to explain.
            let startup_runtime = Arc::clone(&runtime);
            let startup_store = Arc::clone(&store);
            let startup_setup = Arc::clone(&setup);
            let overlay_windows = Arc::clone(&overlay);

            app.manage(commands::AppState {
                runtime,
                phone_stream: Arc::new(runtime::phone_stream::PhoneStream::default()),
                capture,
                store,
                agent,
                input,
                overlay,
                diagnose,
                setup,
                data_dir,
                log_guard: guard,
                pending_activation: std::sync::Mutex::new(startup_paths),
            });

            // The manager window is created here, not from the config: Tauri
            // builds config windows before this hook runs, and a bundled
            // release loads its embedded frontend in milliseconds, so the first
            // commands arrived before `manage` and panicked the runtime thread
            // with "state() called before manage()". Building it after the
            // state is registered makes every command able to read it.
            if let Some(config) = app.config().app.windows.first().cloned() {
                let (asked_width, asked_height) = (config.width, config.height);
                let window = tauri::WebviewWindowBuilder::from_config(app.handle(), &config)?.build()?;
                // A window taller or wider than the desktop cannot be used: the
                // bottom of every screen, including its buttons, ends up off the
                // monitor. The configured size is a preference, so fit it to the
                // screen it opened on and keep the proportions when it does not
                // fit — a 150% laptop is smaller in logical pixels than the
                // 1280x820 the manager asks for.
                if let Some(monitor) = window
                    .current_monitor()
                    .ok()
                    .flatten()
                    .or_else(|| app.primary_monitor().ok().flatten())
                {
                    let scale = monitor.scale_factor();
                    let area = monitor.work_area();
                    let room = |physical: u32| physical as f64 / scale - 24.0;
                    let (max_width, max_height) = (room(area.size.width), room(area.size.height));
                    let factor = (max_width / asked_width)
                        .min(max_height / asked_height)
                        .min(1.0);
                    if factor < 1.0 {
                        let size = tauri::LogicalSize::new(
                            (asked_width * factor).floor(),
                            (asked_height * factor).floor(),
                        );
                        tracing::info!(
                            "fitting the manager window to {:.0}x{:.0} on this screen (scale {scale})",
                            size.width,
                            size.height
                        );
                        let _ = window.set_size(size);
                        let _ = window.center();
                    }
                }
                // Closing the manager ends DroidWin. The overlay windows are
                // separate and would otherwise keep the process alive with no
                // window to see, leaving the runtime it started running.
                let handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        // Closing the manager means quitting. The window waits
                        // for the runtime this process started to stop, then the
                        // app exits: the runtime's `Exit` cleanup is not reached
                        // once the last window is gone, and an emulator left
                        // running is not a clean shutdown.
                        api.prevent_close();
                        let Some(state) = handle.try_state::<commands::AppState>() else {
                            return;
                        };
                        let runtime = std::sync::Arc::clone(&state.runtime);
                        let overlay = std::sync::Arc::clone(&state.overlay);
                        let exit = handle.clone();
                        std::thread::spawn(move || {
                            overlay.close_windows();
                            tauri::async_runtime::block_on(async move {
                                let _ = tokio::time::timeout(
                                    std::time::Duration::from_secs(10),
                                    runtime.shutdown(),
                                )
                                .await;
                            });
                            exit.exit(0);
                        });
                    }
                });
            }
            // Their webviews fetch overlay data as soon as they load, so they can
            // only exist once the state above does.
            overlay_windows.ensure_windows();

            // A runtime that is not installed yet is the setup screen's job: it
            // must not also start an emulator that cannot boot.
            // The boot below is what turns the report from `starting` to
            // `ready`, so the report is asked without a verified boot here.
            if startup_setup
                .report(&startup_store, false)
                .missing()
                .is_empty()
            {
                tauri::async_runtime::spawn(async move {
                    match startup_runtime.ensure_ready(&startup_store, "startup").await {
                        Ok(status) => tracing::info!(
                            "runtime available at startup: {}",
                            status.state.as_label()
                        ),
                        Err(err) => tracing::warn!(
                            "the runtime did not start with the app: {}",
                            err.summary
                        ),
                    }
                });
            } else {
                tracing::info!(
                    "the Android runtime is not installed yet; the setup screen will offer to install it"
                );
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::env_probe,
            commands::runtime_status,
            commands::runtime_probe,
            commands::runtime_start,
            commands::runtime_stop,
            commands::runtime_setup_state,
            commands::runtime_setup_start,
            commands::runtime_setup_cancel,
            commands::runtime_setup_location,
            commands::runtime_stop_on_exit,
            commands::set_runtime_stop_on_exit,
            commands::phone_screen,
            commands::phone_screenshot,
            commands::app_screenshot,
            commands::rotate_app_window,
            commands::recording_start,
            commands::recording_stop,
            commands::recording_status,
            commands::phone_display,
            commands::set_phone_display,
            commands::reset_phone_display,
            commands::reveal_path,
            commands::phone_stream_start,
            commands::phone_dimensions,
            commands::phone_stream_stop,
            commands::phone_tap,
            commands::phone_swipe,
            commands::phone_key,
            commands::phone_text,
            commands::hardware_report,
            commands::hardware_apply,
            commands::hardware_preset,
            commands::keyboard_mode,
            commands::set_keyboard_mode,
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
            commands::agent_status,
            commands::agent_start,
            commands::agent_stop,
            commands::agent_set_paused,
            commands::agent_set_permission,
            commands::agent_actions,
            commands::agent_skills,
            commands::agent_set_skill,
            commands::agent_install_skill,
            commands::agent_remove_skill,
            commands::agent_tool_catalog,
            commands::input_state,
            commands::universal_bindings,
            commands::set_universal_binding,
            commands::save_input_mapping,
            commands::remove_input_mapping,
            commands::set_input_mapping_enabled,
            commands::set_input_profile_enabled,
            commands::reset_input_mappings,
            overlay::overlay_current,
            overlay::overlay_status,
            overlay::overlay_toggle_labels,
            overlay::overlay_hide,
            overlay::overlay_nav,
            overlay::overlay_toggle_mapper,
            overlay::overlay_open_controls,
            overlay::overlay_begin_place,
            overlay::overlay_place_at,
            overlay::overlay_cancel_place,
            overlay::overlay_clear_pending,
            commands::attention_report,
            commands::attention_recover,
        ])
        .build(tauri::generate_context!())
        .expect("DroidWin failed to start")
        .run(|app, event| {
            // Closing the manager must not leave app windows and an emulator
            // running behind it. Only what this process started is stopped,
            // and the wait is bounded so closing never appears to hang.
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<commands::AppState>() {
                    let runtime = std::sync::Arc::clone(&state.runtime);
                    let agent = std::sync::Arc::clone(&state.agent);
                    let phone_stream = std::sync::Arc::clone(&state.phone_stream);
                    let input = std::sync::Arc::clone(&state.input);
                    // A recording is finalised (or discarded) before the runtime
                    // it records disappears.
                    let capture = std::sync::Arc::clone(&state.capture);
                    // A touch a mapped key is holding must be lifted before the
                    // android display it belongs to goes away.
                    input.release_all();
                    let _ = tauri::async_runtime::block_on(async move {
                        input.flush().await;
                        agent.stop().await;
                        phone_stream.stop(None).await;
                        capture.abort_recording(&runtime).await;
                        tokio::time::timeout(std::time::Duration::from_secs(8), runtime.shutdown())
                            .await
                    });
                }
            }
        });
}
