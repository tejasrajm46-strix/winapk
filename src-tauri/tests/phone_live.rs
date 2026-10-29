//! Live check for the virtual phone: Android's own display, captured and touched.
//!
//! The phone screen is the device display DroidWin did not create — Android's own
//! launcher — so this asserts the three things the screen depends on, against a
//! real emulator rather than a mock:
//!
//! 1. a capture comes back as a PNG whose header size matches the pixels in it;
//! 2. the capture has content (a screen made of one flat colour is a black
//!    window, not a phone);
//! 3. a touch sent through DroidWin's own input path changes what Android draws.
//!
//! It boots or reuses the Android emulator and needs a running AVD, so it is
//! ignored by default:
//!
//! ```text
//! cargo test --test phone_live -- --ignored --nocapture
//! ```

mod support;

use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::diagnose::frame_facts;
use support::Harness;

/// Android's own Home key. A swipe only changes the picture from a known
/// state: Android restores whatever was left on screen, and an app drawer
/// already scrolled to its last row has nowhere to go.
const HOME: i32 = 3;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots or reuses the Android emulator"]
async fn the_phone_display_captures_pixels_and_receives_a_touch() {
    // Only the runtime is used here; the harness is what provisions one for a
    // live test, without duplicating its data directory and PATH setup.
    let harness = Harness::start("phone").await;
    let runtime = Arc::clone(&harness.runtime);
    let store = Arc::clone(harness.store());

    let started = std::time::Instant::now();
    runtime
        .ensure_ready(&store, "phone-live")
        .await
        .expect("the runtime becomes ready");
    eprintln!("runtime ready in {} ms", started.elapsed().as_millis());

    // Start from the launcher's home screen rather than whatever an earlier test
    // or the last person left open.
    runtime.device_key(HOME).await.expect("Home is dispatched");
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    let captured = std::time::Instant::now();
    let (before, width, height) = runtime
        .device_screen()
        .await
        .expect("the phone's screen is captured");
    eprintln!(
        "capture: {width}x{height}, {} bytes, {} ms",
        before.len(),
        captured.elapsed().as_millis()
    );
    assert!(width > 0 && height > 0, "a capture reports its own size");

    // The header, the decoded frame and the reasoned size must agree, or the
    // screen would be scaled by one number and clicked through another.
    let facts = frame_facts(&before).expect("the capture decodes as 8-bit colour");
    assert_eq!(
        (facts.0, facts.1),
        (width, height),
        "the reported size is the captured size"
    );
    eprintln!("flat colour: {:?}", facts.2);
    assert!(
        facts.2.is_none(),
        "the phone's screen has content, not one flat colour {:?}",
        facts.2
    );

    let artifacts = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".droidwin");
    std::fs::write(artifacts.join("phone-before.png"), &before).expect("write the capture");

    // A touch that has to change the picture: from Home, swiping up from the
    // bottom opens the launcher's app drawer, so the drawing changes. This is the
    // evidence that DroidWin's input reaches the device's own display.
    let swipe = std::time::Instant::now();
    runtime
        .device_swipe(
            (width / 2, height * 9 / 10),
            (width / 2, height / 3),
            Duration::from_millis(250),
        )
        .await
        .expect("the swipe is dispatched");
    eprintln!("swipe dispatched in {} ms", swipe.elapsed().as_millis());
    tokio::time::sleep(Duration::from_millis(800)).await;

    let (after, after_width, _) = runtime
        .device_screen()
        .await
        .expect("the phone's screen is captured again");
    std::fs::write(artifacts.join("phone-after.png"), &after).expect("write the second capture");
    assert_eq!(after_width, width, "the display did not change size");
    assert_ne!(
        before, after,
        "the phone's screen changed after a swipe through DroidWin's input path"
    );

    // A tap goes through the same path. What it lands on decides whether anything
    // visible happens, so it is exercised rather than asserted.
    runtime
        .device_tap(width / 2, height / 2)
        .await
        .expect("the tap is dispatched");
    eprintln!("tap dispatched at {}, {}", width / 2, height / 2);

    harness.shutdown().await;
}
