//! End-to-end proof of triggers, workflows and event detection over the real MCP
//! transport.
//!
//!   cargo run --manifest-path src-tauri/Cargo.toml --example mcp_proof -- <apk>
//!
//! Every call below is JSON-RPC over the localhost HTTP endpoint, authorised with
//! the token the app writes to `agent/mcp.json` — the same transport a client
//! uses. Nothing calls the Rust API directly except starting the runtime and
//! reading the endpoint file.
//!
//! What it shows:
//!   1. the tool surface a client sees
//!   2. a launch → wait → screenshot → interact → verify workflow whose per-step
//!      result separates "dispatched" from "verified"
//!   3. a lifecycle trigger that fires when the app's window opens
//!   4. a trigger fired explicitly
//!   5. cancellation mid-run over `notifications/cancelled`
//!   6. a client that disconnects mid-call, leaving nothing behind
//!   7. process and adb state after all of it

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use droidwin_lib::agent::AgentState;
use droidwin_lib::domain::{RuntimeStatus, StageEvent};
use droidwin_lib::runtime::{AppEvent, BusEvents, EmulatorRuntime, EventBus, RuntimeEvents};
use droidwin_lib::store::Store;

struct PrintEvents;

impl RuntimeEvents for PrintEvents {
    fn stage(&self, event: &StageEvent) {
        println!("    [{:?}] {}", event.stage, event.message);
    }
    fn status(&self, status: &RuntimeStatus) {
        println!(
            "    runtime: {} — {}",
            status.state.as_label(),
            status.detail
        );
    }
    fn apps_changed(&self) {}
    fn app_event(&self, event: &AppEvent) {
        println!(
            "    event: {:?} {} ({})",
            event.kind, event.package_id, event.reason
        );
    }
}

#[tokio::main]
async fn main() {
    let home = std::env::var("DROIDWIN_PROOF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".droidwin").join("home"));
    let bus = EventBus::new();
    let runtime = Arc::new(EmulatorRuntime::new(
        home.clone(),
        Arc::new(BusEvents::new(bus.clone())),
    ));
    // The runtime is created here so a failed proof still tears it down. When it
    // lived inside `run`, an early `?` skipped `shutdown()` and left scrcpy
    // windows (and an emulator it had started) behind, which then polluted the
    // next run's "before" sample.
    let outcome = run(Arc::clone(&runtime), bus, home).await;
    runtime.shutdown().await;
    if let Err(err) = outcome {
        eprintln!("mcp proof failed: {err}");
        std::process::exit(1);
    }
}

async fn run(runtime: Arc<EmulatorRuntime>, bus: EventBus, home: PathBuf) -> Result<(), String> {
    let apk = std::env::args()
        .skip(1)
        .find(|arg| !arg.starts_with('-'))
        .ok_or("pass one .apk path")?;
    let apk = PathBuf::from(apk);

    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let _guard = droidwin_lib::diag::init(&home.join("logs"));
    let store = Arc::new(Store::open(&home.join("droidwin.db")).map_err(|e| e.summary)?);
    let agent = AgentState::with_events(
        Arc::clone(&runtime),
        Arc::clone(&store),
        home.clone(),
        bus.clone(),
    );

    // A window host event must reach the same bus the agent listens on; this
    // second subscriber is only the harness printer.
    let mut printer = bus.subscribe();
    tokio::spawn(async move {
        while let Ok(event) = printer.recv().await {
            PrintEvents.app_event(&event);
        }
    });

    let adb = runtime.adb().ok_or("adb is unavailable")?;
    let serial = runtime.serial();
    let before = environment(&adb, &serial, &runtime).await;
    println!("before: {before}");

    runtime
        .ensure_ready(&store, "mcp-proof")
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;

    Arc::clone(&agent).start().await.map_err(|e| e.summary)?;
    let (url, token) = endpoint(&home)?;
    println!("endpoint: {url}");

    // 1. The surface a client gets.
    let init = rpc(
        &url,
        &token,
        1,
        "initialize",
        json!({"protocolVersion":"2025-06-18"}),
    )
    .await?;
    println!(
        "initialize: {} {}",
        init["serverInfo"]["name"], init["serverInfo"]["version"]
    );
    let tools = rpc(&url, &token, 2, "tools/list", json!({})).await?;
    let names: Vec<String> = tools["tools"]
        .as_array()
        .ok_or("tools/list had no tools")?
        .iter()
        .filter_map(|tool| tool["name"].as_str().map(str::to_string))
        .collect();
    println!("tools/list: {} tools", names.len());
    for required in [
        "run_workflow",
        "set_trigger",
        "fire_trigger",
        "get_app_events",
        "list_triggers",
        "delete_trigger",
    ] {
        if !names.iter().any(|name| name == required) {
            return Err(format!("the endpoint does not offer {required}"));
        }
    }

    // 2. Install, then one workflow that launches, waits, looks and taps.
    let installed = call(
        &url,
        &token,
        3,
        "install_package",
        json!({ "path": apk.display().to_string() }),
    )
    .await?;
    let package = installed["install"]["package_id"]
        .as_str()
        .ok_or_else(|| format!("install_package did not name the package: {installed}"))?
        .to_string();
    println!("install: {package} {}", installed["install"]["message"]);

    // A fixed coordinate is not a tap target. This used to be 1200,870, a point
    // written for a 2400x1080 landscape game surface; per-app geometry
    // (runtime/display.rs) gives a portrait package 1080x2400, so that point
    // falls outside the display and is refused rather than clamped to an edge.
    // Ask the app what is clickable and aim at that.
    call(
        &url,
        &token,
        31,
        "open_app",
        json!({ "package_id": package }),
    )
    .await?;
    let found = call(
        &url,
        &token,
        32,
        "find_ui_element",
        json!({ "package_id": package, "selector": { "clickable": true } }),
    )
    .await?;
    if found["found"] != json!(true) {
        return Err(format!("no clickable element on {package}: {found}"));
    }
    let node = &found["element"];
    let tap_selector = if node["text"]
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
    println!("tap target: {tap_selector}");

    let workflow = json!({
        "name": "launch, wait, look, act",
        "package_id": package,
        "steps": [
            { "tool": "open_app", "arguments": { "package_id": package },
              "verify": { "kind": "window_open", "package_id": package } },
            { "tool": "wait_for_app_state", "arguments": { "package_id": package, "state": "foreground", "timeout_ms": 20000 },
              "verify": { "kind": "display_present", "package_id": package } },
            { "tool": "get_screen_state", "arguments": { "package_id": package, "screenshot": false },
              "verify": { "kind": "app_running", "package_id": package } },
            { "tool": "tap", "arguments": { "package_id": package, "selector": tap_selector },
              "verify": { "kind": "screen_changed" } },
            { "tool": "get_screenshot", "arguments": { "package_id": package },
              "verify": { "kind": "app_running", "package_id": package } }
        ]
    });
    let started = Instant::now();
    let structured = call(&url, &token, 4, "run_workflow", workflow).await?;
    report_workflow("launch, wait, look, act", &structured);
    let steps = structured["steps"].as_array().ok_or("no steps")?;
    if structured["dispatched_count"] != json!(steps.len()) {
        return Err(format!(
            "not every step dispatched: {}",
            structured["dispatched_count"]
        ));
    }
    for (index, step) in steps.iter().enumerate() {
        let dispatched = step["dispatched"].as_bool().unwrap_or(false);
        let verified = step["verified"].as_bool();
        if index < 4 {
            if !dispatched || verified != Some(true) {
                return Err(format!("step {index} did not verify: {step}"));
            }
        } else if !dispatched {
            return Err(format!("step {index} was not dispatched: {step}"));
        }
    }
    println!("workflow took {:?}", started.elapsed());

    // Save what the tap produced, as evidence rather than a claim. The image
    // travels as an MCP image block, not in the structured payload.
    save_screenshot(
        &screenshot(&url, &token, 5, &package).await?,
        "mcp-01-workflow-tap",
    )?;

    // 3. A lifecycle trigger: start the game whenever its window opens.
    let trigger = call(
        &url,
        &token,
        6,
        "set_trigger",
        json!({
            "name": "start the game on open",
            "package_id": package,
            "when": "app_opened",
            "steps": [
                { "tool": "wait_for_app_state", "arguments": { "package_id": package, "state": "foreground", "timeout_ms": 15000 },
                  "verify": { "kind": "display_present", "package_id": package } },
                { "tool": "tap", "arguments": { "package_id": package, "selector": tap_selector },
                  "verify": { "kind": "screen_changed" } }
            ]
        }),
    )
    .await?;
    let trigger_id = trigger["trigger"]["id"]
        .as_str()
        .ok_or("set_trigger returned no id")?
        .to_string();
    println!("trigger created: {trigger_id}");

    // Close the window and reopen it; the trigger must fire on its own.
    call(
        &url,
        &token,
        7,
        "close_app_window",
        json!({ "package_id": package }),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    call(
        &url,
        &token,
        8,
        "open_app",
        json!({ "package_id": package }),
    )
    .await?;

    let mut fired = Vec::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let events = call(&url, &token, 9, "get_app_events", json!({ "limit": 10 })).await?;
        fired = events["events"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter(|event| {
                        event["kind"] == json!("app_opened")
                            && event["fired"]
                                .as_array()
                                .map(|ids| ids.iter().any(|id| id == &json!(trigger_id)))
                                .unwrap_or(false)
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if !fired.is_empty() {
            break;
        }
    }
    if fired.is_empty() {
        return Err("the app_opened trigger never fired".into());
    }
    println!(
        "detector: {}",
        serde_json::to_string(&fired[0]).unwrap_or_default()
    );

    // Give the trigger's own tap time to land, then look.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    save_screenshot(
        &screenshot(&url, &token, 10, &package).await?,
        "mcp-02-trigger-started-game",
    )?;

    let listed = call(&url, &token, 11, "list_triggers", json!({})).await?;
    let stored = listed["triggers"]
        .as_array()
        .and_then(|list| {
            list.iter()
                .find(|item| item["id"] == json!(trigger_id))
                .cloned()
        })
        .ok_or("the trigger is not listed")?;
    println!(
        "trigger history: fire_count={} last_fired_at={}",
        stored["fire_count"], stored["last_fired_at"]
    );
    if stored["fire_count"].as_u64().unwrap_or(0) < 1 {
        return Err("the trigger fired but its history says otherwise".into());
    }

    // 4. An explicit trigger, fired by hand.
    let manual = call(
        &url,
        &token,
        12,
        "set_trigger",
        json!({
            "name": "tap once on request",
            "package_id": package,
            "when": "manual",
            "steps": [ { "tool": "tap", "arguments": { "package_id": package, "selector": tap_selector },
                         "verify": { "kind": "app_running", "package_id": package } } ]
        }),
    )
    .await?;
    let manual_id = manual["trigger"]["id"].as_str().ok_or("no id")?.to_string();
    let explicit = call(&url, &token, 13, "fire_trigger", json!({ "id": manual_id })).await?;
    report_workflow("explicit fire", &explicit);
    if explicit["cancelled"] != json!(false) {
        return Err("an explicit fire reported a cancellation".into());
    }

    // 5. Cancellation mid-run: a wait that cannot succeed, cancelled from another
    //    connection while it is in flight.
    let slow = json!({
        "name": "wait for something that will not happen",
        "package_id": package,
        "steps": [
            { "tool": "wait_for_app_state",
              "arguments": { "package_id": package, "state": "stopped", "timeout_ms": 60000 } }
        ]
    });
    let cancel_url = url.clone();
    let cancel_token = token.clone();
    let in_flight = tokio::spawn(async move {
        let started = Instant::now();
        let value = rpc(
            &cancel_url,
            &cancel_token,
            900,
            "tools/call",
            json!({ "name": "run_workflow", "arguments": slow }),
        )
        .await;
        (started.elapsed(), value)
    });
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let cancelled = rpc(
        &url,
        &token,
        901,
        "notifications/cancelled",
        json!({ "requestId": "900" }),
    )
    .await;
    // A notification gets 202 with no body, so this is the expected `Err`.
    let cancelled = cancelled.err().unwrap_or_default();
    let (took, value) = in_flight.await.map_err(|e| e.to_string())?;
    let value = value?;
    println!(
        "cancelled run: took {:?}, candidate result {}",
        took, cancelled
    );
    if took >= Duration::from_secs(20) {
        return Err(format!("cancellation did not shorten the wait: {took:?}"));
    }
    println!(
        "cancelled workflow result: {}",
        serde_json::to_string(&value).map_err(|e| e.to_string())?
    );

    // 6. A client that disconnects mid-call. The request is accepted, then the
    //    socket is closed before reading a reply.
    drop_connection(
        &url,
        &token,
        &json!({
            "jsonrpc": "2.0",
            "id": 902,
            "method": "tools/call",
            "params": { "name": "run_workflow", "arguments": {
                "package_id": package,
                "steps": [ { "tool": "wait_for_app_state",
                             "arguments": { "package_id": package, "state": "stopped", "timeout_ms": 8000 } } ]
            }}
        }),
    )?;
    println!("dropped a connection mid-call; checking the endpoint still answers");
    // The server must still be healthy for the next client, and must not have
    // left a workflow blocking anything.
    let after_drop = rpc(&url, &token, 903, "initialize", json!({})).await?;
    println!(
        "endpoint after disconnect: {}",
        after_drop["serverInfo"]["name"]
    );
    let _ = call(&url, &token, 904, "get_runtime_status", json!({})).await?;

    // 7. Put everything back and show what is left behind.
    call(
        &url,
        &token,
        20,
        "delete_trigger",
        json!({ "id": manual_id }),
    )
    .await?;
    call(
        &url,
        &token,
        21,
        "delete_trigger",
        json!({ "id": trigger_id }),
    )
    .await?;
    call(
        &url,
        &token,
        22,
        "close_app_window",
        json!({ "package_id": package }),
    )
    .await?;
    // The window host exits a moment after it is killed; wait for it rather than
    // reporting whatever the first sample happened to catch.
    let mut after = environment(&adb, &serial, &runtime).await;
    for _ in 0..20 {
        if after["windows"] == json!(0) && after["scrcpy"] == before["scrcpy"] {
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        after = environment(&adb, &serial, &runtime).await;
    }
    println!("after: {after}");
    if after["windows"].as_u64().unwrap_or(1) != 0 {
        return Err(format!("windows are still open after teardown: {after}"));
    }
    if after["scrcpy"] != before["scrcpy"] {
        return Err(format!(
            "window hosts did not all exit: before {}, after {}",
            before["scrcpy"], after["scrcpy"]
        ));
    }

    agent.stop().await;
    let closed = rpc(&url, &token, 30, "initialize", json!({})).await;
    println!(
        "endpoint after stop: {}",
        if closed.is_err() {
            "refuses connections"
        } else {
            "STILL ANSWERING"
        }
    );
    println!("done");
    Ok(())
}

/// What is running right now: DroidWin's own windows, adb's device list and the
/// window hosts on the machine. Comparing the before and after lines is the
/// orphan check.
async fn environment(
    adb: &droidwin_lib::runtime::adb::Adb,
    serial: &str,
    runtime: &Arc<EmulatorRuntime>,
) -> Value {
    let devices = adb
        .devices()
        .await
        .map(|list| {
            list.iter()
                .map(|device| format!("{} {}", device.serial, device.state))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    json!({
        "windows": runtime.open_windows().len(),
        "scrcpy": scrcpy_processes(),
        "adb": devices,
        "managed_serial": serial,
    })
}

#[cfg(windows)]
fn scrcpy_processes() -> usize {
    use std::os::windows::process::CommandExt;
    let mut cmd = std::process::Command::new("powershell");
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        "(Get-Process scrcpy -ErrorAction SilentlyContinue | Measure-Object).Count",
    ]);
    cmd.creation_flags(0x0800_0000);
    cmd.output()
        .ok()
        .and_then(|out| {
            String::from_utf8_lossy(&out.stdout)
                .trim()
                .parse::<usize>()
                .ok()
        })
        .unwrap_or(0)
}

#[cfg(not(windows))]
fn scrcpy_processes() -> usize {
    0
}

fn report_workflow(name: &str, structured: &Value) {
    println!("\n{name}:");
    if let Some(steps) = structured["steps"].as_array() {
        for step in steps {
            let observation = &step["observation"];
            println!(
                "  {} {} dispatched={} verified={} observed(running={}, window={}, display={})",
                step["index"],
                step["tool"].as_str().unwrap_or("?"),
                step["dispatched"],
                match &step["verified"] {
                    Value::Null => "not checked".to_string(),
                    other => other.to_string(),
                },
                observation["app_running"],
                observation["window_open"],
                observation["display_id"],
            );
        }
    }
    println!(
        "  dispatched {}/{} step(s), verified={}, stopped={}, {}ms",
        structured["dispatched_count"],
        structured["steps"].as_array().map(|s| s.len()).unwrap_or(0),
        structured["verified"],
        structured["stopped"],
        structured["duration_ms"]
    );
}

/// One HTTP exchange, blocking, on its own connection. Returns the parsed
/// JSON-RPC reply; a 202 (a notification) has no body and is an `Err` here.
fn post(url: &str, token: &str, body: &Value) -> Result<Value, String> {
    use std::io::{Read, Write};
    let address = url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .trim_end_matches("/mcp");
    let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let head = format!(
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let mut stream = std::net::TcpStream::connect(address).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(200)))
        .map_err(|e| e.to_string())?;
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&payload).map_err(|e| e.to_string())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).map_err(|e| e.to_string())?;
    let split = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("the server closed without a response")?;
    let head_text = String::from_utf8_lossy(&raw[..split]).to_string();
    if !head_text.starts_with("HTTP/1.1 200") {
        return Err(format!(
            "unexpected status: {}",
            head_text.lines().next().unwrap_or_default()
        ));
    }
    serde_json::from_slice(&raw[split + 4..]).map_err(|e| e.to_string())
}

/// Send one request and close the socket without reading the reply, which is what
/// a client that dies mid-call looks like to the server.
fn drop_connection(url: &str, token: &str, body: &Value) -> Result<(), String> {
    use std::io::Write;
    let address = url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .trim_end_matches("/mcp");
    let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let head = format!(
        "POST /mcp HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    let mut stream = std::net::TcpStream::connect(address).map_err(|e| e.to_string())?;
    stream
        .write_all(head.as_bytes())
        .map_err(|e| e.to_string())?;
    stream.write_all(&payload).map_err(|e| e.to_string())?;
    stream.flush().map_err(|e| e.to_string())?;
    // Dropping the socket here is the disconnect.
    drop(stream);
    Ok(())
}

/// The endpoint file the app writes for a client: the URL and the token.
fn endpoint(home: &std::path::Path) -> Result<(String, String), String> {
    let path = droidwin_lib::agent::endpoint_file(home);
    for _ in 0..20 {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Ok(value) = serde_json::from_slice::<Value>(&bytes) {
                let url = value["url"].as_str().unwrap_or_default().to_string();
                let token = value["token"].as_str().unwrap_or_default().to_string();
                if !url.is_empty() && !token.is_empty() {
                    return Ok((url, token));
                }
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err(format!("no endpoint file at {}", path.display()))
}

/// One JSON-RPC exchange on a fresh connection. `Err` means a transport or
/// protocol failure, or a notification (which has no body by design).
async fn rpc(
    url: &str,
    token: &str,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let body = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    let (url, token) = (url.to_string(), token.to_string());
    let reply = tokio::task::spawn_blocking(move || post(&url, &token, &body))
        .await
        .map_err(|e| e.to_string())??;
    if let Some(error) = reply.get("error") {
        return Err(format!(
            "{method}: {}",
            error["message"].as_str().unwrap_or("JSON-RPC error")
        ));
    }
    reply
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{method} answered without a result"))
}

/// One tool call. A tool-level failure becomes an `Err` carrying what the server
/// said, so the proof never mistakes a refusal for success.
async fn call(
    url: &str,
    token: &str,
    id: u64,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let result = rpc(
        url,
        token,
        id,
        "tools/call",
        json!({ "name": name, "arguments": arguments }),
    )
    .await?;
    if result["isError"] == json!(true) {
        return Err(format!(
            "{name} failed: {}",
            serde_json::to_string(&result["structuredContent"]).unwrap_or_default()
        ));
    }
    Ok(result["structuredContent"].clone())
}

/// The PNG an MCP image block carries, decoded.
async fn screenshot(url: &str, token: &str, id: u64, package: &str) -> Result<Vec<u8>, String> {
    let result = rpc(
        url,
        token,
        id,
        "tools/call",
        json!({ "name": "get_screenshot", "arguments": { "package_id": package } }),
    )
    .await?;
    let encoded = result["content"]
        .as_array()
        .and_then(|blocks| {
            blocks.iter().find_map(|block| {
                (block["type"] == json!("image"))
                    .then(|| block["data"].as_str())
                    .flatten()
            })
        })
        .ok_or("the screenshot result carried no image block")?;
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())
}

fn save_screenshot(bytes: &[u8], name: &str) -> Result<(), String> {
    let dir = PathBuf::from(".droidwin").join("evidence");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let file = dir.join(format!("{name}.png"));
    std::fs::write(&file, bytes).map_err(|e| e.to_string())?;
    println!("  screenshot: {} ({} bytes)", file.display(), bytes.len());
    Ok(())
}
