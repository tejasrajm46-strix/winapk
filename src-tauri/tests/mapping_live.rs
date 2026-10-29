//! Live check for keyboard mapping: one physical key press must reach Android
//! exactly once, and every release must actually release.
//!
//! The decision layer is driven directly (`InputController::on_key_for`) because
//! a harness cannot own Windows' keyboard focus, but nothing below it is mocked:
//! real adb, the real running emulator, a real app window on its own display.
//! Every assertion is read back out of Android:
//!
//! * a mapped `KEYCODE_BRIGHTNESS_UP` moves `screen_brightness`, an integer
//!   Android writes down, so a press that fires twice, or an auto-repeat that
//!   fires again, shows up as the wrong number instead of as a passing test;
//! * a held touch is read from `dumpsys input`, which reports the pointers a
//!   display currently has down — a finger DroidWin never lifted stays visible
//!   there, which is exactly the stuck-key failure this test exists to catch.
//!
//! It boots or reuses the emulator and opens a real window, so it is ignored by
//! default:
//!
//! ```text
//! cargo test --test mapping_live -- --ignored --nocapture
//! ```

mod support;

use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::input::{InputAction, InputController};
use droidwin_lib::runtime::adb::Adb;
use support::{touching_windows, Harness};

/// Settings ships on every AVD and runs on a secondary display, so the test
/// never depends on a package the machine happens to have installed.
const PACKAGE: &str = "com.android.settings";

const VK_J: u32 = 0x4A;
const VK_K: u32 = 0x4B;
const VK_CTRL: u32 = 0x11;

/// Where a mapped brightness key has to take Android's own brightness value,
/// measured from the reset value below. The step Android uses is not linear, so
/// these are the numbers this platform actually writes, not a formula.
const BRIGHTNESS_START: i32 = 10;
const AFTER_ONE_PUSH: i32 = 16;
const AFTER_TWO_PUSHES: i32 = 25;

async fn shell(adb: &Adb, serial: &str, line: &str) -> String {
    let out = adb
        .run(&["-s", serial, "shell", line], Duration::from_secs(40))
        .await
        .unwrap_or_else(|err| panic!("`{line}` could not be run: {}", err.summary));
    assert!(out.ok(), "`{line}` failed: {}", out.combined());
    out.stdout.trim().to_string()
}

async fn brightness(adb: &Adb, serial: &str) -> i32 {
    shell(adb, serial, "settings get system screen_brightness")
        .await
        .parse()
        .expect("Android reports its brightness as a number")
}

/// Put the brightness back to a known value, then let the display settle.
async fn reset_brightness(adb: &Adb, serial: &str) {
    shell(
        adb,
        serial,
        &format!("settings put system screen_brightness {BRIGHTNESS_START}"),
    )
    .await;
    // Android applies the change asynchronously; a key pressed too early is
    // applied to the old level and the count would be off by a step.
    tokio::time::sleep(Duration::from_millis(600)).await;
}

async fn pointers(adb: &Adb, serial: &str, display: u32) -> Vec<String> {
    let dump = shell(adb, serial, "dumpsys input").await;
    touching_windows(&dump, display)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots or reuses the Android emulator and opens a real app window"]
async fn a_held_key_reaches_android_once_and_every_release_lands() {
    let harness = Harness::start("mapping").await;
    let runtime = Arc::clone(&harness.runtime);
    let store = Arc::clone(harness.store());
    runtime
        .ensure_ready(&store, "mapping-live")
        .await
        .expect("the runtime becomes ready");

    let adb = runtime.adb().expect("the SDK is configured");
    let serial = runtime.serial();

    // A mapping needs a window: its targets are percentages of the display
    // Android composed that app on, and Android's own screen is the phone.
    let window = runtime
        .launch(PACKAGE, Some("Settings"), &store, "mapping")
        .await
        .expect("Settings opens in a window");
    assert_eq!(
        runtime.package_for_pid(window.pid).as_deref(),
        Some(PACKAGE),
        "the process the hook sees resolves to the package that owns it"
    );
    let display = runtime
        .geometry_of(PACKAGE)
        .and_then(|surface| surface.display_id)
        .expect("the window reports the display Android composed it on");
    eprintln!("{PACKAGE} window pid {} on display {display}", window.pid);

    // The brightness counter is only readable while Android is not choosing the
    // level itself.
    let original_mode = shell(&adb, &serial, "settings get system screen_brightness_mode").await;
    let original_brightness = brightness(&adb, &serial).await;
    shell(
        &adb,
        &serial,
        "settings put system screen_brightness_mode 0",
    )
    .await;

    let controller = InputController::new(Arc::clone(&runtime), Arc::clone(&store));
    // Enabled through the store rather than `set_enabled`, which also installs
    // the process-wide keyboard hook: the poll it starts would release held keys
    // whenever this harness window is not the foreground one. This test is about
    // the release rules, so it drives them itself.
    store
        .set_input_mapping_enabled(PACKAGE, true)
        .expect("mapping switches on");
    controller
        .save(
            PACKAGE,
            VK_K,
            InputAction::KeyEvent {
                keycode: "KEYCODE_BRIGHTNESS_UP".into(),
            },
        )
        .expect("K is mapped");

    // ------------------------------------------------- one press, one event ---
    reset_brightness(&adb, &serial).await;
    assert_eq!(brightness(&adb, &serial).await, BRIGHTNESS_START);
    assert!(
        controller.on_key_for(PACKAGE, VK_K, true, 0),
        "a mapped key press is consumed, so scrcpy cannot send it a second time"
    );
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        controller.on_key_for(PACKAGE, VK_K, false, 0),
        "the release belongs to the press that was consumed"
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        brightness(&adb, &serial).await,
        AFTER_ONE_PUSH,
        "one press of a mapped key moves Android's brightness by exactly one step"
    );

    // ------------------------------------------------ held key, no repeating ---
    reset_brightness(&adb, &serial).await;
    for repeat in 0..4 {
        assert!(
            controller.on_key_for(PACKAGE, VK_K, true, 0),
            "auto-repeat {repeat} is swallowed, not forwarded"
        );
    }
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        controller.on_key_for(PACKAGE, VK_K, false, 0),
        "the release after a held key is consumed too"
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        brightness(&adb, &serial).await,
        AFTER_ONE_PUSH,
        "a key held down (five key-downs) fires once, not once per auto-repeat"
    );

    // ------------------------------------------------------ two quick presses ---
    reset_brightness(&adb, &serial).await;
    for _ in 0..2 {
        assert!(controller.on_key_for(PACKAGE, VK_K, true, 0));
        assert!(controller.on_key_for(PACKAGE, VK_K, false, 0));
    }
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        brightness(&adb, &serial).await,
        AFTER_TWO_PUSHES,
        "two quick presses are both delivered, in order and neither lost"
    );

    // -------------------------------- an unmapped combo must not fire the plain key ---
    reset_brightness(&adb, &serial).await;
    assert!(
        !controller.on_key_for(PACKAGE, VK_CTRL, true, 0),
        "an unmapped Ctrl belongs to the window, so scrcpy still sees it"
    );
    assert!(
        !controller.on_key_for(PACKAGE, VK_K, true, 1),
        "Ctrl+K has no mapping of its own, so it must not trigger plain K"
    );
    controller.on_key_for(PACKAGE, VK_K, false, 2);
    assert!(
        !controller.on_key_for(PACKAGE, VK_CTRL, false, 0),
        "Ctrl's release follows its press to the window"
    );
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        brightness(&adb, &serial).await,
        BRIGHTNESS_START,
        "holding Ctrl while pressing K left Android's brightness alone"
    );

    // ------------------------------------- a modifier released before its key ---
    // Ctrl+K presses a point, so the key-up has to find that same point after
    // Ctrl is already up. A release resolved against the wrong binding would
    // leave the finger on the app — which `dumpsys input` would show.
    controller
        .save(
            PACKAGE,
            0x100 | VK_K,
            InputAction::TouchHold { x: 50.0, y: 50.0 },
        )
        .expect("Ctrl+K is mapped to a hold");
    assert!(!controller.on_key_for(PACKAGE, VK_CTRL, true, 0));
    assert!(
        controller.on_key_for(PACKAGE, VK_K, true, 1),
        "Ctrl+K is mapped and consumed"
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        !pointers(&adb, &serial, display).await.is_empty(),
        "the Ctrl+K hold reaches display {display}"
    );
    assert!(!controller.on_key_for(PACKAGE, VK_CTRL, false, 0));
    assert!(
        controller.on_key_for(PACKAGE, VK_K, false, 0),
        "K's release still belongs to the Ctrl+K press, even after Ctrl is up"
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    let after_combo = pointers(&adb, &serial, display).await;
    eprintln!("pointers on display {display} after the combo released: {after_combo:?}");
    assert!(
        after_combo.is_empty(),
        "releasing Ctrl first did not leave Ctrl+K's finger pressed on the app"
    );

    // -------------------------------------- a held touch, and focus leaving it ---
    controller
        .save(PACKAGE, VK_J, InputAction::TouchHold { x: 50.0, y: 50.0 })
        .expect("J is mapped to a hold");
    assert!(
        controller.on_key_for(PACKAGE, VK_J, true, 0),
        "a hold mapping presses once"
    );
    tokio::time::sleep(Duration::from_millis(900)).await;
    let held = pointers(&adb, &serial, display).await;
    eprintln!("pointers on display {display} while J is held: {held:?}");
    assert!(
        !held.is_empty(),
        "Android reports the finger a held key pressed on display {display}"
    );

    // Alt-tab away with the key still down: the poll sees no foreground package.
    controller.reconcile_focus(None);
    tokio::time::sleep(Duration::from_millis(900)).await;
    let after_blur = pointers(&adb, &serial, display).await;
    eprintln!("pointers on display {display} after focus left: {after_blur:?}");
    assert!(
        after_blur.is_empty(),
        "focus leaving the window lifts the held finger instead of leaving it on the app"
    );
    assert!(
        !controller.on_key_for(PACKAGE, VK_J, false, 0),
        "the key-up that arrives after the focus release is not a second release"
    );

    // ------------------------------- stopping DroidWin while a key is held ---
    assert!(controller.on_key_for(PACKAGE, VK_J, true, 0));
    tokio::time::sleep(Duration::from_millis(900)).await;
    assert!(
        !pointers(&adb, &serial, display).await.is_empty(),
        "the second hold is down before shutdown"
    );
    // Exactly what the app does on exit and on a runtime stop.
    controller.release_all();
    controller.flush().await;
    tokio::time::sleep(Duration::from_millis(900)).await;
    let after_stop = pointers(&adb, &serial, display).await;
    eprintln!("pointers on display {display} after release_all: {after_stop:?}");
    assert!(
        after_stop.is_empty(),
        "shutting down lifts every finger this profile held"
    );

    // --------------------------------------------- cleaning up after the test ---
    shell(
        &adb,
        &serial,
        &format!("settings put system screen_brightness {original_brightness}"),
    )
    .await;
    shell(
        &adb,
        &serial,
        &format!("settings put system screen_brightness_mode {original_mode}"),
    )
    .await;
    controller.reset(PACKAGE).expect("the profile resets");
    runtime
        .close_window(PACKAGE)
        .await
        .expect("the window closes");
    assert!(
        runtime.geometry_of(PACKAGE).is_none(),
        "a closed window leaves no display behind"
    );
    harness.shutdown().await;
}

#[test]
fn the_pointer_reader_follows_android_own_dump_shape() {
    // Trimmed from a real `dumpsys input` while a finger was held on display 0,
    // beside display 2's empty block, which must not be attributed to 0.
    let dump = "\
Input Dispatcher State:
  TouchStatesByDisplay:
    0 :     Windows:
        0 : name='aaee7c9 com.android.settings/.Settings', targetFlags=FOREGROUND | SPLIT, forwardingWindowToken=0x0, mDeviceStates=-1:[touchingPointers=[Pointer(id=0, FINGER)], downTimeInTarget=2082887000000, hoveringPointers=[], pilferingPointerIds=<none>]
        1 : name='[Gesture Monitor] swipe-up', targetFlags=SPLIT, forwardingWindowToken=0x0, mDeviceStates=-1:[touchingPointers=[Pointer(id=0, FINGER)], downTimeInTarget=2082887000000, hoveringPointers=[], pilferingPointerIds=<none>]
    2 :     Windows: <none>
  CursorStatesByDisplay: <no displays touched by cursor>
";
    assert_eq!(
        touching_windows(dump, 0),
        vec![
            "aaee7c9 com.android.settings/.Settings".to_string(),
            "[Gesture Monitor] swipe-up".to_string()
        ]
    );
    assert!(touching_windows(dump, 2).is_empty());
    assert!(touching_windows(dump, 7).is_empty());
    // Released: the block is still printed, with no pointers in it.
    let released = dump.replace(
        "[touchingPointers=[Pointer(id=0, FINGER)]",
        "[touchingPointers=[]",
    );
    assert!(touching_windows(&released, 0).is_empty());
}
