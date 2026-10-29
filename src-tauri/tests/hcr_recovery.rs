//! Live check for a game Android refuses to put on its own display.
//!
//! Hill Climb Racing asks for EGL configs the host GPU path does not expose, and
//! its activity is non-resizable and landscape-only, so Android keeps it on the
//! default display instead of the secondary display DroidWin sends it to. Its
//! scrcpy window then has nothing to draw (`mCurrentFocus=null` on that display).
//!
//! The contract this asserts: DroidWin does not leave that window open and does
//! not call it an open app. It reports that the window has no picture, closes it,
//! and keeps the runtime it did not start running.
//!
//! This boots the Android emulator and needs `com.fingersoft.hillclimb` installed
//! in the AVD, so it is ignored by default:
//!
//! ```text
//! cargo test --test hcr_recovery -- --ignored --nocapture
//! ```

mod support;

use serde_json::json;
use support::Harness;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots the Android emulator and needs the game installed in the AVD"]
async fn a_game_android_keeps_off_its_own_display_is_reported_not_left_black() {
    let harness = Harness::start("hcr").await;

    let started = std::time::Instant::now();
    let opened = harness.call(
        "open_app",
        json!({ "package_id": "com.fingersoft.hillclimb" }),
    );
    eprintln!("open took {} ms", started.elapsed().as_millis());
    eprintln!(
        "result: {}",
        serde_json::to_string_pretty(&opened["structuredContent"]).unwrap_or_default()
    );

    if opened["isError"] == true
        && opened["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .contains("not installed")
    {
        eprintln!("Hill Climb Racing is not installed in this AVD; skipping");
        harness.shutdown().await;
        return;
    }

    // Either outcome is honest, and both are accepted here:
    //  - the app reached a display of its own (a newer image or driver allowed
    //    it), in which case a screenshot must come back; or
    //  - Android kept it off its own display, in which case the launch must say
    //    so and must not leave a window behind.
    let display = opened["structuredContent"]["display_id"].as_u64();
    if opened["isError"] == false || display.is_some() {
        eprintln!("the app did land on its own display {display:?}");
        let screenshot = harness.call(
            "get_screenshot",
            json!({ "package_id": "com.fingersoft.hillclimb" }),
        );
        assert_eq!(
            screenshot["isError"], false,
            "a window with a display has pixels"
        );
        harness.call(
            "close_app_window",
            json!({ "package_id": "com.fingersoft.hillclimb" }),
        );
        harness.shutdown().await;
        return;
    }

    let text = opened["content"][0]["text"].as_str().unwrap_or_default();
    eprintln!("reported: {text}");
    assert!(
        text.contains("drew nothing on its window"),
        "the launch must name the empty window, not just fail: {text}"
    );

    // The empty window is closed, so nothing is left holding a black surface.
    let windows = harness.call_ok(
        "get_active_window",
        json!({ "package_id": "com.fingersoft.hillclimb" }),
    );
    assert_eq!(windows["count"], 0, "{windows}");

    // A refusal must not restart the runtime: the emulator stays the same process.
    let status = harness.call_ok("get_runtime_status", json!({}));
    assert_eq!(status["state"], "ready", "{status}");

    harness.shutdown().await;
}
