//! Live check for the phone's controls, captures and display settings.
//!
//! Every assertion here is about Android's own behaviour, not about DroidWin's
//! bookkeeping: Home has to bring the launcher forward, Back has to leave
//! Settings, a screenshot has to be a decodable PNG at the device's size, a
//! recording has to be a real MP4 that survives a second `screenrecord` running
//! beside it (the live view's pipeline), and a display override has to change
//! what Android actually captures.
//!
//! It boots or reuses the emulator, so it is ignored by default:
//!
//! ```text
//! cargo test --test capture_live -- --ignored --nocapture
//! ```

mod support;

use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::capture::{is_mp4, Capture};
use droidwin_lib::diagnose::frame_facts;
use droidwin_lib::runtime::display_override;
use droidwin_lib::runtime::hardware::{self, Preset};
use support::Harness;

const HOME: i32 = 3;
const BACK: i32 = 4;
const APP_SWITCH: i32 = 187;

fn artifacts() -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".droidwin")
        .join("live-capture");
    std::fs::create_dir_all(&dir).expect("create the artifact folder");
    dir
}

async fn foreground(adb: &droidwin_lib::runtime::adb::Adb, serial: &str) -> String {
    adb.foreground_on(serial, droidwin_lib::runtime::display::DEVICE_DISPLAY)
        .await
        .and_then(|focus| focus.package_id)
        .unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots or reuses the Android emulator"]
async fn the_phone_answers_its_controls_saves_captures_and_obeys_display_settings() {
    let harness = Harness::start("capture").await;
    let runtime = Arc::clone(&harness.runtime);
    let store = Arc::clone(harness.store());
    let started = std::time::Instant::now();
    runtime
        .ensure_ready(&store, "capture-live")
        .await
        .expect("the runtime becomes ready");
    eprintln!("runtime ready in {} ms", started.elapsed().as_millis());

    let adb = runtime.adb().expect("the SDK is configured");
    let serial = runtime.serial();

    // A known starting state: an earlier interrupted run may have left the
    // device rotated or resized, and a test that inherits that measures the
    // wrong thing.
    let _ = display_override::reset(&adb, &serial).await;
    // A known starting orientation: an earlier interrupted run may have left the
    // device rotated, and a test that inherits that measures the wrong thing.
    for setting in [
        "settings put system accelerometer_rotation 0",
        "settings put system user_rotation 0",
    ] {
        let _ = adb
            .run(&["-s", &serial, "shell", setting], Duration::from_secs(15))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    // ---------------------------------------------------------------- Home ---
    runtime.device_key(HOME).await.expect("Home is dispatched");
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let after_home = foreground(&adb, &serial).await;
    let (home_shot, width, height) = runtime
        .device_screen()
        .await
        .expect("the launcher is captured");
    eprintln!("HOME foreground: {after_home}; display: {width}x{height}");
    assert!(
        after_home.to_lowercase().contains("launcher"),
        "Home should bring Android's launcher forward, got {after_home}"
    );
    assert!(width > 0 && height > 0, "the device reports its own size");

    // ---------------------------------------------------------------- Back ---
    // A fresh task at Settings' root. Android restores the page it last showed,
    // and a sub-screen left open by an earlier run would make Back go up one
    // level inside Settings instead of out — a wrong assertion about Back.
    let _ = adb
        .run(
            &[
                "-s",
                &serial,
                "shell",
                "am",
                "force-stop",
                "com.android.settings",
            ],
            Duration::from_secs(30),
        )
        .await;
    let out = adb
        .run(
            &[
                "-s",
                &serial,
                "shell",
                "am",
                "start",
                "-a",
                "android.settings.SETTINGS",
            ],
            Duration::from_secs(30),
        )
        .await
        .expect("Settings can be opened");
    assert!(out.ok(), "Settings launch failed: {}", out.combined());
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let in_settings = foreground(&adb, &serial).await;
    eprintln!("Settings foreground: {in_settings}");
    assert!(
        in_settings.contains("settings"),
        "Android Settings should be foreground, got {in_settings}"
    );
    let (settings_shot, _, _) = runtime.device_screen().await.expect("Settings is captured");

    runtime.device_key(BACK).await.expect("Back is dispatched");
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    let after_back = foreground(&adb, &serial).await;
    let (back_shot, _, _) = runtime.device_screen().await.expect("captured again");
    eprintln!("BACK foreground: {after_back}");
    assert_ne!(
        after_back, in_settings,
        "Back should leave Settings, it is still {after_back}"
    );
    assert_ne!(
        settings_shot, back_shot,
        "the screen Android draws should change after Back"
    );

    // -------------------------------------------------------------- Recents ---
    runtime
        .device_key(APP_SWITCH)
        .await
        .expect("recent apps is dispatched");
    tokio::time::sleep(Duration::from_millis(1_800)).await;
    let (recents_shot, _, _) = runtime.device_screen().await.expect("recents captured");
    let recents_focus = foreground(&adb, &serial).await;
    // Android's overview is a real view with real ids; its panel is the evidence
    // that the key opened recents rather than merely repainting the launcher.
    let hierarchy = adb.ui_hierarchy_xml(&serial).await.unwrap_or_default();
    let overview = hierarchy.contains("overview_panel")
        || hierarchy.contains("task_view_single")
        || hierarchy.contains("overview_actions_view");
    eprintln!(
        "recents foreground: {recents_focus}; overview in the hierarchy: {overview}; screen changed: {}",
        recents_shot != back_shot
    );
    assert!(
        overview,
        "recent apps should open Android's overview (hierarchy was {} bytes)",
        hierarchy.len()
    );
    runtime.device_key(HOME).await.expect("Home is dispatched");
    tokio::time::sleep(Duration::from_millis(1_000)).await;

    // ------------------------------------------------------------ Screenshot ---
    let shots = artifacts().join("shots");
    let shot = Capture::save_screenshot(&runtime, &shots)
        .await
        .expect("a screenshot is saved");
    let bytes = std::fs::read(&shot.path).expect("the saved file is readable");
    let facts = frame_facts(&bytes).expect("the saved file decodes as 8-bit colour");
    eprintln!(
        "screenshot: {} ({} bytes) {}x{}",
        shot.path, shot.bytes, shot.width, shot.height
    );
    assert_eq!((facts.0, facts.1), (shot.width, shot.height));
    assert_eq!((shot.width, shot.height), (width, height));
    assert!(!home_shot.is_empty());
    assert_eq!(bytes.len() as u64, shot.bytes);
    assert!(bytes.starts_with(b"\x89PNG"), "the file is a PNG");

    // ------------------------------------------------- Landscape orientation ---
    // Settings rotates with the device; the launcher on this image does not.
    let _ = adb
        .run(
            &[
                "-s",
                &serial,
                "shell",
                "am",
                "start",
                "-a",
                "android.settings.SETTINGS",
            ],
            Duration::from_secs(30),
        )
        .await;
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    for setting in ["settings put system user_rotation 1"] {
        let _ = adb
            .run(&["-s", &serial, "shell", setting], Duration::from_secs(15))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(2_000)).await;
    let landscape = Capture::save_screenshot(&runtime, &shots)
        .await
        .expect("a landscape screenshot is saved");
    eprintln!(
        "landscape screenshot: {} {}x{}",
        landscape.path, landscape.width, landscape.height
    );
    assert!(
        landscape.width > landscape.height,
        "the landscape capture should be wider than it is tall, got {}x{}",
        landscape.width,
        landscape.height
    );
    for setting in ["settings put system user_rotation 0"] {
        let _ = adb
            .run(&["-s", &serial, "shell", setting], Duration::from_secs(15))
            .await;
    }
    tokio::time::sleep(Duration::from_millis(1_500)).await;
    runtime.device_key(HOME).await.expect("Home is dispatched");
    tokio::time::sleep(Duration::from_millis(1_000)).await;

    // ------------------------------------------------------ Display settings ---
    let before = display_override::read(&adb, &serial)
        .await
        .expect("Android reports its display");
    eprintln!("display: {before:?}");
    assert_eq!((before.width, before.height), (Some(width), Some(height)));
    assert!(!before.overridden, "the AVD starts with no override");

    let applied = display_override::apply(&adb, &serial, 720, 1600, Some(280))
        .await
        .expect("the smaller display applies");
    eprintln!("after apply: {applied:?}");
    assert_eq!((applied.width, applied.height), (Some(720), Some(1600)));
    assert_eq!(applied.density, Some(280));
    assert!(applied.overridden);

    let (_, small_width, small_height) = runtime
        .device_screen()
        .await
        .expect("the smaller display is captured");
    assert_eq!((small_width, small_height), (720, 1600));
    let small_shot = Capture::save_screenshot(&runtime, &shots)
        .await
        .expect("a screenshot at the new size");
    assert_eq!((small_shot.width, small_shot.height), (720, 1600));

    // A size Android could not use is refused before it reaches the device.
    assert!(display_override::apply(&adb, &serial, 12, 12, None)
        .await
        .is_err());
    assert!(
        display_override::apply(&adb, &serial, 720, 1600, Some(9_000))
            .await
            .is_err()
    );

    let reset = display_override::reset(&adb, &serial)
        .await
        .expect("the AVD's own display comes back");
    eprintln!("after reset: {reset:?}");
    assert_eq!((reset.width, reset.height), (before.width, before.height));
    let (_, restored_width, restored_height) = runtime
        .device_screen()
        .await
        .expect("the restored display is captured");
    assert_eq!((restored_width, restored_height), (width, height));

    // ------------------------------------------------------------- Recording ---
    let videos = artifacts().join("videos");
    let capture = Capture::default();
    let started = capture
        .start_recording(&runtime, &videos)
        .await
        .expect("recording starts");
    assert!(started.active);
    eprintln!("recording into {:?}", started.directory);

    // Input keeps working while recording, and so does a second screenrecord:
    // that is the live view's own process, so the two have to coexist. The
    // swipes are also what gives the encoder frames to write — SurfaceFlinger
    // sends none while nothing on screen changes.
    for _ in 0..3 {
        runtime
            .device_swipe(
                (restored_width / 2, restored_height * 4 / 5),
                (restored_width / 2, restored_height / 4),
                Duration::from_millis(180),
            )
            .await
            .expect("a swipe during recording");
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let live = adb
        .run_bytes(
            &[
                "-s",
                &serial,
                "exec-out",
                "screenrecord --output-format=h264 --time-limit 2 -",
            ],
            Duration::from_secs(30),
        )
        .await
        .expect("the live pipeline still runs");
    eprintln!("live-view bytes while recording: {}", live.len());
    assert!(
        live.len() > 1_000,
        "the live view should still produce video while recording: {} bytes",
        live.len()
    );
    tokio::time::sleep(Duration::from_millis(1_500)).await;

    let state = capture.recording_state().await;
    eprintln!("recording elapsed: {} ms", state.elapsed_ms);
    assert!(state.active && state.elapsed_ms > 1_000);

    let file = capture
        .stop_recording(&runtime)
        .await
        .expect("the recording is saved");
    let video = std::fs::read(&file.path).expect("the saved video is readable");
    eprintln!(
        "recording: {} ({} bytes, {} ms wall clock)",
        file.path, file.bytes, file.duration_ms
    );
    assert!(is_mp4(&video), "the saved file is an MP4");
    assert_eq!(video.len() as u64, file.bytes);
    assert!(file.bytes > 10_000, "the recording has real content");

    // Stopping twice is a request Android was never asked to satisfy.
    assert!(capture.stop_recording(&runtime).await.is_err());

    // The remote file is not left behind on the device.
    let leftovers = adb
        .run(
            &[
                "-s",
                &serial,
                "shell",
                "ls /data/local/tmp/ | grep -c droidwin-rec || true",
            ],
            Duration::from_secs(15),
        )
        .await
        .expect("the device answers about leftovers");
    eprintln!(
        "recording files left on the device: {}",
        leftovers.combined().trim()
    );
    assert_eq!(leftovers.combined().trim(), "0");

    // -------------------------------------------------- Performance presets ---
    // A preset is real state: it persists and comes back as the plan.
    let host = hardware::host_facts();
    let plan = hardware::preset_plan(&host, Preset::Performance);
    hardware::persist(&store, &plan).expect("the preset is saved");
    let report = runtime.hardware_report(&store).await;
    eprintln!(
        "preset plan: {} cores, {} MB, {}; Android reports {:?}",
        report.plan.cores,
        report.plan.ram_mb,
        report.plan.gpu.as_label(),
        report.effective
    );
    assert_eq!(report.plan.preset.as_deref(), Some("performance"));
    assert!(
        report.plan.cores <= host.logical_cpus.max(2) / 2,
        "{} vCPUs out of {} is more than half the machine",
        report.plan.cores,
        host.logical_cpus
    );
    assert!(report.plan.cores < host.logical_cpus, "never every thread");
    assert_eq!(
        report.plan.mode,
        droidwin_lib::runtime::hardware::PlanMode::Custom
    );

    // Leave the device as it was found for whatever runs next.
    let _ = adb
        .run(
            &[
                "-s",
                &serial,
                "shell",
                "settings put system accelerometer_rotation 1",
            ],
            Duration::from_secs(15),
        )
        .await;

    harness.shutdown().await;
}
