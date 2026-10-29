//! End-to-end proof: an MCP client installs a package, opens it on its own
//! display, looks at the screen, types, presses a key, waits, verifies and
//! closes the window — all through the same HTTP endpoint and tool dispatch the
//! desktop app uses.
//!
//! This boots the Android emulator and installs a real APK, so it is ignored by
//! default. Run it explicitly:
//!
//! ```text
//! cargo test --test agent_live -- --ignored --nocapture
//! ```

mod support;

use serde_json::json;
use support::Harness;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "boots the Android emulator and installs a real package"]
async fn an_agent_can_install_open_look_and_act() {
    let apk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".droidwin")
        .join("testdata")
        .join("fdroid.apk");
    let apk = apk
        .canonicalize()
        .expect("the F-Droid test package is in .droidwin/testdata");
    assert!(apk.is_file());

    let harness = Harness::start("live").await;

    let started_status = harness.call_ok("start_runtime", json!({}));
    assert_eq!(started_status["state"], "ready");
    let started = std::time::Instant::now();
    eprintln!("install + open (cold boot if the runtime was stopped)...");
    let installed = harness.call_ok(
        "install_package",
        json!({ "path": apk.display().to_string(), "launch": true }),
    );
    eprintln!("  took {} ms", started.elapsed().as_millis());
    let package = installed["install"]["package_id"]
        .as_str()
        .expect("a package id")
        .to_string();
    let display = installed["display_id"]
        .as_u64()
        .expect("a resolved Android display") as u32;
    assert!(display > 0, "an app window must have its own display");

    let info = harness.call_ok("get_app_info", json!({"package_id": package}));
    assert_eq!(info["window"]["display_id"], display);
    let status = harness.call_ok("get_runtime_status", json!({}));
    assert_eq!(status["state"], "ready");
    assert!(
        status["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|window| window["package_id"] == package.as_str()
                && window["display_id"] == display),
        "the window registry must map the package to its display: {status}"
    );

    let started = std::time::Instant::now();
    let screenshot = harness.call("get_screenshot", json!({ "package_id": package }));
    eprintln!("screenshot took {} ms", started.elapsed().as_millis());
    assert_eq!(screenshot["isError"], false);
    let content = screenshot["content"].as_array().unwrap();
    let image = content
        .iter()
        .find(|block| block["type"] == "image")
        .expect("the screenshot travels as an MCP image block");
    assert_eq!(image["mimeType"], "image/png");
    assert!(image["data"].as_str().unwrap().len() > 1_000);
    // The pixel data must not be duplicated into the JSON result.
    assert!(screenshot["structuredContent"]["base64"].is_null());
    // The per-app geometry policy (runtime/display.rs) gives a portrait app a
    // 1080x2400 surface at 420 dpi. It was 540x1170 before that policy existed,
    // which is why this number is pinned here next to its test.
    assert_eq!(
        screenshot["structuredContent"]["width"].as_u64(),
        Some(1080)
    );
    assert_eq!(
        screenshot["structuredContent"]["height"].as_u64(),
        Some(2400)
    );
    assert!(screenshot["structuredContent"]["bytes"].as_u64().unwrap() > 1_000);

    let started = std::time::Instant::now();
    let screen = harness.call_ok("get_screen_state", json!({ "package_id": package }));
    eprintln!("screen state took {} ms", started.elapsed().as_millis());
    assert_eq!(screen["display_id"].as_u64(), Some(display as u64));
    println!(
        "screen state: foreground={:?} elements={} note={:?}",
        screen["foreground_package"], screen["element_count"], screen["note"]
    );

    let started = std::time::Instant::now();
    let hierarchy = harness.call_ok("get_ui_hierarchy", json!({ "package_id": package }));
    eprintln!("hierarchy took {} ms", started.elapsed().as_millis());
    println!(
        "hierarchy: {} nodes, {} visible",
        hierarchy["node_count"],
        hierarchy["elements"].as_array().unwrap().len()
    );
    // Verified on this image: uiautomator returns the app's windows even when
    // they live on a scrcpy virtual display.
    assert!(hierarchy["node_count"].as_u64().unwrap() > 0);
    assert!(!hierarchy["elements"].as_array().unwrap().is_empty());

    // Tap the first clickable element by meaning, not by guessing a coordinate.
    let found = harness.call_ok(
        "find_ui_element",
        json!({ "package_id": package, "selector": { "clickable": true } }),
    );
    assert_eq!(found["found"], true, "{found}");
    let node = &found["element"];
    let selector = if node["text"]
        .as_str()
        .is_some_and(|text| !text.trim().is_empty())
    {
        json!({ "text": node["text"] })
    } else if node["content_desc"]
        .as_str()
        .is_some_and(|desc| !desc.trim().is_empty())
    {
        json!({ "content_desc": node["content_desc"] })
    } else if node["resource_id"]
        .as_str()
        .is_some_and(|id| !id.trim().is_empty())
    {
        json!({ "resource_id": node["resource_id"] })
    } else {
        json!({ "class_name": node["class"] })
    };
    let tapped = harness.call_ok(
        "tap",
        json!({ "package_id": package, "selector": selector }),
    );
    eprintln!(
        "tapped {:?} at {},{} on display {}",
        tapped["element"]["text"], tapped["x"], tapped["y"], tapped["display_id"]
    );

    let started = std::time::Instant::now();
    let typed = harness.call_ok(
        "type_text",
        json!({ "package_id": package, "text": "droidwin" }),
    );
    eprintln!("type_text took {} ms", started.elapsed().as_millis());
    assert_eq!(typed["characters"], 8);

    let refused = harness.call(
        "type_text",
        json!({ "package_id": package, "text": "日本語" }),
    );
    assert_eq!(refused["isError"], true);
    assert_eq!(
        refused["structuredContent"]["error"]["code"],
        "not_supported"
    );

    let started = std::time::Instant::now();
    harness.call_ok("press_key", json!({ "package_id": package, "key": "BACK" }));
    eprintln!("press_key took {} ms", started.elapsed().as_millis());

    let waited = harness.call_ok(
        "wait_for_app_state",
        json!({ "package_id": package, "state": "running", "timeout_ms": 5_000 }),
    );
    assert_eq!(waited["reached"], true);

    let verified = harness.call_ok(
        "verify_action",
        json!({ "expect": { "window_open": package, "app_running": package } }),
    );
    assert_eq!(verified["verified"], true, "{verified}");

    let stats = harness.call_ok("get_runtime_stats", json!({}));
    assert!(stats["android"]["cores"].as_u64().unwrap_or(0) >= 2);
    println!(
        "runtime stats: cores={:?} free_mb={:?} renderer={:?}",
        stats["android"]["cores"],
        stats["android"]["memory_available_mb"],
        stats["android"]["gpu_renderer"]
    );

    let devices = harness.call_ok("list_devices", json!({}));
    assert!(devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .any(|device| device["managed"] == true));

    harness.call_ok("close_app_window", json!({ "package_id": package }));
    let after = harness.call_ok("get_active_window", json!({ "package_id": package }));
    assert_eq!(after["count"], 0);

    let started = std::time::Instant::now();
    let reopened = harness.call_ok("open_app", json!({ "package_id": package }));
    eprintln!("warm re-open took {} ms", started.elapsed().as_millis());
    // A re-opened window gets a fresh virtual display; only the old one was
    // destroyed, so reusing the old id would have been wrong.
    let reopened_display = reopened["display_id"].as_u64().expect("a display") as u32;
    assert!(reopened_display > 0);

    // A second app gets its own display, and driving one must not touch the
    // other. The emulator stays the same process throughout.
    let second_apk = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".droidwin")
        .join("testdata")
        .join("newpipe.apk")
        .canonicalize()
        .expect("the NewPipe test package is in .droidwin/testdata");
    let second = harness.call_ok(
        "install_package",
        json!({ "path": second_apk.display().to_string(), "launch": true }),
    );
    let second_package = second["install"]["package_id"]
        .as_str()
        .unwrap()
        .to_string();
    let second_display = second["display_id"].as_u64().expect("a display") as u32;
    assert_ne!(second_display, reopened_display);

    let windows = harness.call_ok("get_active_window", json!({}));
    assert_eq!(windows["count"], 2, "{windows}");

    // uiautomator may expose only the newly focused window. An empty dump for
    // the other display must never be accepted as proof that an element is absent.
    let hierarchy = harness.call_ok(
        "get_ui_hierarchy",
        json!({ "display_id": reopened_display }),
    );
    if hierarchy["node_count"] == 0 {
        let result = harness.call(
            "verify_action",
            json!({ "display_id": reopened_display, "expect": { "element_absent": { "text": "not-present" } } }),
        );
        assert_eq!(result["isError"], true, "{result}");
        assert_eq!(
            result["structuredContent"]["error"]["code"],
            "runtime_unavailable"
        );
    }
    let stats_after = harness.call_ok("get_runtime_stats", json!({}));
    assert_eq!(
        stats_after["emulator"]["pid"], stats["emulator"]["pid"],
        "tool calls must reuse the one emulator"
    );

    // Closing one window leaves the other running.
    harness.call_ok("close_app_window", json!({ "package_id": package }));
    let survivors = harness.call_ok("get_active_window", json!({}));
    assert!(survivors["windows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|window| window["package_id"] == second_package.as_str()));
    harness.call_ok("close_app_window", json!({ "package_id": second_package }));

    // Cancellation: a long wait must stop when the client cancels it.
    let port = harness.port;
    let token = harness.token.clone();
    let request = json!({
        "jsonrpc":"2.0", "id":424242, "method":"tools/call",
        "params":{ "name":"wait_for_app_state", "arguments":{
            "package_id":"com.example.droidwin.absent", "state":"running", "timeout_ms":30000
        }}
    });
    let body = serde_json::to_string(&request).unwrap();
    let waiting = std::thread::spawn(move || support::http(port, Some(&token), None, &body));
    std::thread::sleep(std::time::Duration::from_millis(1_200));
    harness.post(&json!({
        "jsonrpc":"2.0", "method":"notifications/cancelled", "params":{ "requestId":424242 }
    }));
    let cancelled_at = std::time::Instant::now();
    let (_, reply) = waiting.join().unwrap();
    eprintln!("cancelled after {} ms", cancelled_at.elapsed().as_millis());
    assert!(reply.contains("cancelled"), "{reply}");
    assert!(cancelled_at.elapsed() < std::time::Duration::from_secs(5));

    let errors = harness.call_ok("get_recent_runtime_errors", json!({}));
    assert!(
        errors["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["tool"] == "type_text" && error["ok"] == false),
        "a refused action must appear in the error trail"
    );

    harness.shutdown().await;
}
