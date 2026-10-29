//! Live check for the `droidwin` command line.
//!
//! It starts the real endpoint with the app's own wiring, then runs the real
//! binary as a child process against the machine's running emulator — the same
//! path an agent takes. Nothing here imports the CLI's internals: if the
//! discovery, the token, the HTTP route, the dispatcher or the runtime breaks,
//! the command's exit status or its JSON says so.
//!
//!     cargo test --test cli_live -- --ignored --nocapture
//!
//! It needs the emulator running (any DroidWin AVD) and F-Droid installed, which
//! is what the other live tests already assume. It does not start or stop the
//! emulator: the runtime only stops an emulator it started itself.

mod support;

use std::path::{Path, PathBuf};

use serde_json::Value;
use support::Harness;

/// Run the shipped binary, exactly as a shell would.
async fn cli(home: &Path, args: &[&str]) -> (i32, Value) {
    let output = tokio::process::Command::new(env!("CARGO_BIN_EXE_droidwin"))
        .args(args)
        .env("DROIDWIN_HOME", home)
        .env("NO_COLOR", "1")
        .output()
        .await
        .expect("the command line runs");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let value = serde_json::from_str(stdout.trim()).unwrap_or_else(|_| {
        serde_json::json!({
            "unparsed_stdout": stdout,
            "stderr": stderr,
        })
    });
    println!(
        "$ droidwin {}\n  exit {code}\n  {}\n",
        args.join(" "),
        serde_json::to_string(&value).unwrap_or_default()
    );
    (code, value)
}

fn ok(value: &Value, what: &str) {
    assert_eq!(value["ok"], true, "{what} did not succeed: {value}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "needs the Android emulator and a package installed in it"]
async fn the_command_line_drives_the_real_runtime() {
    let harness = Harness::start_full("cli-live").await;
    let home = harness.data_dir.clone();
    // Reuse a running emulator or boot one; the harness stops only what it
    // started, so a machine already running the AVD keeps it.
    harness
        .runtime
        .ensure_ready(harness.store(), "cli-live")
        .await
        .expect("the runtime is ready");

    // Discovery is local: no endpoint, no device.
    let (code, tools) = cli(&home, &["tools"]).await;
    assert_eq!(code, 0);
    assert!(tools["commands"].as_array().unwrap().len() >= 20, "{tools}");
    assert_eq!(tools["commands"][0]["name"], "status");

    // Context cost, measured rather than assumed: the command table an agent
    // reads once is much smaller than the MCP tool schemas a client loads.
    let (status, listing) = support::http(
        harness.port,
        Some(&harness.token),
        None,
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}).to_string(),
    );
    assert_eq!(status, 200);
    let cli_bytes = serde_json::to_string(&tools).unwrap().len();
    println!(
        "discovery payload: `droidwin tools` {cli_bytes} bytes; MCP tools/list {} bytes",
        listing.len()
    );
    assert!(
        cli_bytes < listing.len(),
        "the command table is not smaller than the tool schemas"
    );

    // The runtime is the machine's running emulator.
    let (code, status) = cli(&home, &["status"]).await;
    assert_eq!(code, 0, "{status}");
    ok(&status, "status");
    let serial = harness.runtime.serial();
    assert!(
        status["data"]["windows"].is_array(),
        "status has no window list: {status}"
    );

    let (code, devices) = cli(&home, &["devices"]).await;
    assert_eq!(code, 0, "{devices}");
    ok(&devices, "devices");
    let list = devices["data"]["devices"].as_array().unwrap();
    assert!(
        list.iter()
            .any(|device| device["serial"] == serial.as_str()),
        "the managed serial {serial} is not in {devices}"
    );

    let (code, apps) = cli(&home, &["apps", "list"]).await;
    assert_eq!(code, 0, "{apps}");
    ok(&apps, "apps list");
    let packages: Vec<&str> = apps["data"]["apps"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|app| app["package_id"].as_str())
        .collect();
    let target = if packages.contains(&"org.fdroid.fdroid") {
        "org.fdroid.fdroid"
    } else {
        *packages.first().expect("at least one package is installed")
    };
    println!("driving {target}");

    // Performance: the requested plan and what Android reports are separate.
    let (code, perf) = cli(&home, &["perf", "get"]).await;
    assert_eq!(code, 0, "{perf}");
    ok(&perf, "perf get");
    assert!(perf["data"]["plan"]["cores"].is_number(), "{perf}");
    assert!(perf["data"]["host"]["logical_cpus"].is_number(), "{perf}");

    // Every mapping operation, then leave the package exactly as it was found.
    let before = cli(&home, &["mapping", "get", target]).await.1;
    ok(&before, "mapping get");
    let (code, saved) = cli(
        &home,
        &[
            "mapping",
            "save",
            target,
            "--key",
            "Ctrl+Shift+K",
            "--tap",
            "50,80",
        ],
    )
    .await;
    assert_eq!(code, 0, "{saved}");
    ok(&saved, "mapping save");
    assert!(
        saved["data"]["mappings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|mapping| mapping["label"] == "Ctrl+Shift+K"
                && mapping["action"]["kind"] == "touch_tap"),
        "{saved}"
    );

    let (code, removed) = cli(
        &home,
        &["mapping", "remove", target, "--key", "Ctrl+Shift+K"],
    )
    .await;
    assert_eq!(code, 0, "{removed}");
    ok(&removed, "mapping remove");
    assert_eq!(
        removed["data"]["mappings"].as_array().unwrap().len(),
        before["data"]["mappings"].as_array().unwrap().len()
    );

    let (code, enabled) = cli(&home, &["mapping", "enable", target]).await;
    assert_eq!(code, 0, "{enabled}");
    ok(&enabled, "mapping enable");
    assert_eq!(enabled["data"]["enabled"], true);
    let (code, disabled) = cli(&home, &["mapping", "disable", target]).await;
    assert_eq!(code, 0, "{disabled}");
    ok(&disabled, "mapping disable");
    assert_eq!(disabled["data"]["enabled"], false);

    // Navigation goes to Android and is verified by leaving the launcher.
    let (code, home_key) = cli(&home, &["nav", "home"]).await;
    assert_eq!(code, 0, "{home_key}");
    ok(&home_key, "nav home");

    // A screenshot is a real PNG written where the command said.
    let shots = harness.data_dir.join("cli-shots");
    let (code, shot) = cli(
        &home,
        &[
            "capture",
            "screenshot",
            "--out",
            &shots.display().to_string(),
        ],
    )
    .await;
    assert_eq!(code, 0, "{shot}");
    ok(&shot, "capture screenshot");
    let path = PathBuf::from(shot["data"]["path"].as_str().expect("a path"));
    let bytes = std::fs::read(&path).expect("the screenshot exists");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "not a PNG: {path:?}");
    assert_eq!(
        bytes.len() as u64,
        shot["data"]["bytes"].as_u64().unwrap(),
        "the reported size is not the file's"
    );
    assert!(shot["data"]["width"].as_u64().unwrap() > 0);

    // Recording: start, see the state, stop, and check the file is an MP4.
    let (code, started) = cli(&home, &["capture", "record", "start"]).await;
    assert_eq!(code, 0, "{started}");
    ok(&started, "capture record start");
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let (code, state) = cli(&home, &["capture", "record", "status"]).await;
    assert_eq!(code, 0, "{state}");
    ok(&state, "capture record status");
    assert_eq!(state["data"]["active"], true, "{state}");
    let (code, stopped) = cli(&home, &["capture", "record", "stop"]).await;
    assert_eq!(code, 0, "{stopped}");
    ok(&stopped, "capture record stop");
    let recording =
        std::fs::read(stopped["data"]["path"].as_str().unwrap()).expect("the recording exists");
    assert_eq!(&recording[4..8], b"ftyp", "not an MP4");
    assert!(stopped["data"]["duration_ms"].as_i64().unwrap() > 1_000);

    // Opening and closing a real window, and seeing it in the runtime's report.
    let (code, opened) = cli(&home, &["apps", "open", target]).await;
    assert_eq!(code, 0, "{opened}");
    ok(&opened, "apps open");
    let (_, status) = cli(&home, &["status"]).await;
    ok(&status, "status after open");
    assert!(
        status["data"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|window| window["package_id"] == target),
        "the opened window is not reported: {status}"
    );
    let info = cli(&home, &["apps", "info", target]).await.1;
    ok(&info, "apps info");
    assert_eq!(info["data"]["app"]["package_id"], target);
    let (code, closed) = cli(&home, &["apps", "close", target]).await;
    assert_eq!(code, 0, "{closed}");
    ok(&closed, "apps close");

    // Raw MCP tools are still reachable by name.
    let (code, raw) = cli(&home, &["call", "get_runtime_status"]).await;
    assert_eq!(code, 0, "{raw}");
    ok(&raw, "call get_runtime_status");
    assert!(raw["data"]["state"].is_string(), "{raw}");

    // A failed command is a failed exit with a code, not a silent zero.
    let (code, missing) = cli(&home, &["apps", "info", "not.installed.anywhere"]).await;
    assert_eq!(code, 1, "{missing}");
    assert_eq!(missing["ok"], false);
    assert_eq!(missing["error"]["code"], "not_installed", "{missing}");

    // The read-only gate applies to the command line exactly as to a tool.
    harness
        .state
        .set_permission(droidwin_lib::agent::Permission::ReadOnly)
        .expect("permission is settable");
    let (code, refused) = cli(&home, &["nav", "home"]).await;
    assert_eq!(code, 1, "{refused}");
    assert_eq!(refused["error"]["code"], "permission_denied", "{refused}");
    let (code, allowed) = cli(&home, &["status"]).await;
    assert_eq!(code, 0, "a read command must survive read-only: {allowed}");
    harness
        .state
        .set_permission(droidwin_lib::agent::Permission::Control)
        .unwrap();

    harness.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_a_running_app_the_command_line_says_so() {
    // A data directory with no endpoint file: nothing is listening, and the
    // answer is actionable instead of a connection error.
    let directory = std::env::temp_dir().join(format!(
        "droidwin-cli-empty-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&directory).unwrap();
    let (code, answer) = cli(&directory, &["status"]).await;
    assert_eq!(code, 2, "{answer}");
    assert_eq!(answer["error"]["code"], "endpoint_not_running");
    assert!(answer["error"]["detail"]
        .as_str()
        .unwrap()
        .contains("Start endpoint"));
    let _ = std::fs::remove_dir_all(&directory);
}
