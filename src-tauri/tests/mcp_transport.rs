//! The MCP transport without a device: authentication, framing, discovery and
//! the permission gate. Nothing here needs an emulator, so it runs on every
//! `cargo test`.

mod support;

use std::sync::Arc;

use serde_json::json;

use droidwin_lib::agent::Permission;
use support::Harness;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_without_the_session_token_is_refused() {
    let harness = Harness::start("auth").await;
    let body = json!({"jsonrpc":"2.0","id":1,"method":"tools/list"});
    let (status, _) = harness.post_with(None, None, &body);
    assert_eq!(status, 401);
    let (status, _) = harness.post_with(Some("wrong"), None, &body);
    assert_eq!(status, 401);
    let (status, reply) = harness.post_with(Some(&harness.token), None, &body);
    assert_eq!(status, 200);
    assert!(reply.contains("tools"));
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_browser_origin_is_refused_even_with_a_valid_token() {
    let harness = Harness::start("origin").await;
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    let (status, _) = harness.post_with(Some(&harness.token), Some("https://evil.example"), &body);
    assert_eq!(status, 403);
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_lists_tools_and_answers_a_call() {
    let harness = Harness::start("flow").await;

    let (status, body) = harness.post(&json!({
        "jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}
    }));
    assert_eq!(status, 200);
    let reply: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(reply["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(reply["result"]["serverInfo"]["name"], "droidwin");
    assert!(reply["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("display"));

    let (status, body) = harness.post(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    assert_eq!(status, 200);
    let reply: serde_json::Value = serde_json::from_str(&body).unwrap();
    let tools = reply["result"]["tools"].as_array().unwrap();
    assert!(tools.len() >= 25);
    assert!(tools.iter().any(|tool| tool["name"] == "get_app_info"));
    assert!(tools.iter().any(|tool| tool["name"] == "start_runtime"));
    for tool in tools {
        assert!(tool["name"].is_string());
        assert!(tool["description"].is_string());
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert!(tool["annotations"]["readOnlyHint"].is_boolean());
    }

    let result = harness.call_ok("get_recent_runtime_errors", json!({}));
    assert!(result["errors"].is_array());
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_notification_is_acknowledged_without_a_body() {
    let harness = Harness::start("notification").await;
    let (status, body) = harness.post(&json!({
        "jsonrpc":"2.0","method":"notifications/initialized"
    }));
    assert_eq!(status, 202);
    assert!(body.trim().is_empty());
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_mode_refuses_input_but_still_answers_reads() {
    let harness = Harness::start("readonly").await;
    harness
        .state
        .set_permission(Permission::ReadOnly)
        .expect("permission is settable");

    let result = harness.call("tap", json!({"display_id": 0, "x": 10, "y": 10}));
    assert_eq!(result["isError"], true);
    assert_eq!(
        result["structuredContent"]["error"]["code"],
        "permission_denied"
    );

    let result = harness.call_ok("get_recent_runtime_errors", json!({"limit": 5}));
    assert!(result["errors"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["ok"] == false));
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paused_agent_is_refused_and_the_reason_is_actionable() {
    let harness = Harness::start("paused").await;
    harness.state.set_paused(true).expect("pause is settable");
    let result = harness.call("press_key", json!({"display_id": 0, "key": "BACK"}));
    assert_eq!(
        result["structuredContent"]["error"]["code"],
        "automation_paused"
    );
    harness.state.set_paused(false).unwrap();
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_command_route_uses_the_same_token_gate_and_error_shape() {
    let harness = Harness::start("cli").await;
    let body = json!({"command":"mapping.get","args":{"package_id":"app.pkg"}});

    // No token is refused exactly like the MCP route.
    let (status, _) = support::http_path(harness.port, "/cli", None, None, &body.to_string());
    assert_eq!(status, 401);

    // A read-only command is dispatched. The harness has no input controller, so
    // the honest answer is a failure with a code — not a 500 and not `ok: true`.
    let (status, reply) = harness.post_cli(&body);
    assert_eq!(status, 200);
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "runtime_unavailable");

    let (_, reply) = harness.post_cli(&json!({"command":"teleport","args":{}}));
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["error"]["code"], "invalid_request");
    assert!(reply["error"]["detail"]
        .as_str()
        .unwrap()
        .contains("droidwin tools"));

    // A command that changes the runtime obeys the same permission gate.
    harness
        .state
        .set_permission(Permission::ReadOnly)
        .expect("permission is settable");
    let (_, reply) = harness.post_cli(&json!({"command":"runtime.stop","args":{}}));
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["error"]["code"], "permission_denied");

    harness.state.set_permission(Permission::Control).unwrap();
    harness.state.set_paused(true).unwrap();
    let (_, reply) = harness.post_cli(&json!({"command":"nav.home","args":{}}));
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["error"]["code"], "automation_paused");

    // A read command still answers while paused: pausing automation does not
    // blind an agent that is only looking. (`mapping.get` is answered from the
    // store and needs no device, so the failure here is the missing controller,
    // not the pause.)
    let (_, reply) =
        harness.post_cli(&json!({"command":"mapping.get","args":{"package_id":"app.pkg"}}));
    let reply: serde_json::Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["error"]["code"], "runtime_unavailable");

    harness.state.set_paused(false).unwrap();
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn malformed_json_is_a_parse_error_not_a_crash() {
    let harness = Harness::start("parse").await;
    let (status, body) = harness.post_raw("{not json");
    assert_eq!(status, 400);
    let reply: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(reply["error"]["code"], -32700);
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stopped_endpoint_closes_the_port() {
    let harness = Harness::start("stop").await;
    let port = harness.port;
    harness.state.stop().await;
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
    assert!(!harness.data_dir.join("agent").join("mcp.json").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stopping_the_endpoint_closes_already_accepted_connections() {
    use std::io::{Read, Write};
    use std::time::Duration;

    let harness = Harness::start("stop-active").await;
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", harness.port)).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_millis(250)))
        .unwrap();
    socket
        .write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n")
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    harness.state.stop().await;

    let mut byte = [0u8; 1];
    let result = socket.read(&mut byte);
    harness.shutdown().await;
    assert!(
        matches!(result, Ok(0)),
        "accepted socket remained open: {result:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn start_stop_restart_replaces_token_and_updates_endpoint() {
    let harness = Harness::start("restart").await;
    let old_token = harness.token.clone();
    harness.state.stop().await;

    Arc::clone(&harness.state).start().await.unwrap();
    let (url, new_port) = harness.state.endpoint().await.unwrap();
    assert!(url.ends_with(&format!("{new_port}/mcp")));
    let endpoint: serde_json::Value = serde_json::from_slice(
        &std::fs::read(harness.data_dir.join("agent").join("mcp.json")).unwrap(),
    )
    .unwrap();
    let new_token = endpoint["token"].as_str().unwrap();
    assert_ne!(new_token, old_token);
    let body = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    assert_eq!(
        support::http(new_port, Some(&old_token), None, &body.to_string()).0,
        401
    );
    assert_eq!(
        support::http(new_port, Some(new_token), None, &body.to_string()).0,
        200
    );
    harness.state.stop().await;
    assert!(!harness.data_dir.join("agent").join("mcp.json").exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn duplicate_json_rpc_ids_keep_concurrent_cancellations_separate() {
    use droidwin_lib::agent::control::Cancel;

    let harness = Harness::start("duplicate-id").await;
    let first = harness.state.register("same");
    let second = harness.state.register("same");
    harness.state.finish("same", &first);
    assert!(harness.state.cancel_request("same"));
    assert!(second.tripped());
    assert!(!first.tripped());
    harness.state.finish("same", &second);
    assert!(!harness.state.cancel_request("same"));
    let untouched = Cancel::new();
    assert!(!untouched.tripped());
    harness.state.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skills_are_listed_and_an_unknown_id_is_an_error() {
    let harness = Harness::start("skills").await;
    let listed = harness.call_ok("list_skills", json!({}));
    assert_eq!(listed["skills"].as_array().unwrap().len(), 0);

    let result = harness.call("read_skill", json!({"id": "../secrets"}));
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["error"]["code"], "invalid_path");
    harness.state.stop().await;
}
