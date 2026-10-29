//! Shared helpers for the agent integration tests.
//!
//! These speak to the endpoint the same way an MCP client does: a raw HTTP/1.1
//! POST on loopback with the token read from the endpoint file.

#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use droidwin_lib::agent::AgentState;
use droidwin_lib::runtime::{EmulatorRuntime, NoopEvents};
use droidwin_lib::store::Store;

pub struct Harness {
    pub state: Arc<AgentState>,
    pub runtime: Arc<EmulatorRuntime>,
    pub data_dir: PathBuf,
    pub token: String,
    pub port: u16,
}

impl Harness {
    pub async fn start(tag: &str) -> Self {
        Self::start_with(tag, false).await
    }

    /// The desktop app's own wiring: an input controller and a capture session,
    /// so the mapping and recording commands have somewhere to go.
    pub async fn start_full(tag: &str) -> Self {
        Self::start_with(tag, true).await
    }

    async fn start_with(tag: &str, full: bool) -> Self {
        // The runtime's own log explains where a live test is; without a
        // subscriber every line is discarded.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_writer(std::io::stderr)
            .try_init();
        let data_dir = std::env::temp_dir().join(format!(
            "droidwin-agent-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&data_dir).unwrap();
        // The app provisions its bundled window host into the data directory on
        // first run. A test data directory has none, so point PATH at the
        // checkout's copy instead of duplicating 40 MB per test.
        let scrcpy = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".droidwin")
            .join("bin")
            .join("scrcpy-win64-v5.0.1");
        if scrcpy.is_dir() {
            let path = std::env::var("PATH").unwrap_or_default();
            let joined = format!("{};{path}", scrcpy.display());
            std::env::set_var("PATH", joined);
        }
        let store = Arc::new(Store::open(&data_dir.join("droidwin.db")).unwrap());
        let runtime = Arc::new(EmulatorRuntime::new(data_dir.clone(), Arc::new(NoopEvents)));
        let state = if full {
            let input =
                droidwin_lib::input::InputController::new(Arc::clone(&runtime), Arc::clone(&store));
            let capture = Arc::new(droidwin_lib::capture::Capture::default());
            AgentState::with_events_input_and_capture(
                Arc::clone(&runtime),
                Arc::clone(&store),
                data_dir.clone(),
                droidwin_lib::runtime::EventBus::new(),
                input,
                capture,
            )
        } else {
            AgentState::new(Arc::clone(&runtime), store, data_dir.clone())
        };
        Arc::clone(&state).start().await.expect("endpoint starts");

        let (_, port) = state.endpoint().await.expect("endpoint is running");
        let text = std::fs::read_to_string(data_dir.join("agent").join("mcp.json"))
            .expect("endpoint file is written");
        let value: Value = serde_json::from_str(&text).unwrap();
        let token = value["token"].as_str().unwrap().to_string();
        Self {
            state,
            runtime,
            data_dir,
            token,
            port,
        }
    }

    /// Stop the endpoint and the emulator this harness started. Leaving qemu
    /// running after a test would poison every later run on the machine.
    pub async fn shutdown(self) {
        self.state.stop().await;
        self.runtime.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }

    pub fn post(&self, body: &Value) -> (u16, String) {
        self.post_raw(&serde_json::to_string(body).unwrap())
    }

    pub fn post_raw(&self, body: &str) -> (u16, String) {
        http(self.port, Some(&self.token), None, body)
    }

    /// One command for the command-line route, with this session's token.
    pub fn post_cli(&self, body: &Value) -> (u16, String) {
        http_path(
            self.port,
            "/cli",
            Some(&self.token),
            None,
            &serde_json::to_string(body).unwrap(),
        )
    }

    pub fn post_with(
        &self,
        token: Option<&str>,
        origin: Option<&str>,
        body: &Value,
    ) -> (u16, String) {
        http(
            self.port,
            token,
            origin,
            &serde_json::to_string(body).unwrap(),
        )
    }

    /// A `tools/call`, returning the tool result object (not the JSON-RPC frame).
    pub fn call(&self, name: &str, arguments: Value) -> Value {
        let (status, body) = self.post(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }));
        assert_eq!(status, 200, "{name} answered HTTP {status}: {body}");
        let reply: Value = serde_json::from_str(&body).unwrap();
        reply["result"].clone()
    }

    pub fn call_ok(&self, name: &str, arguments: Value) -> Value {
        let result = self.call(name, arguments);
        assert_eq!(
            result["isError"], false,
            "{name} failed: {}",
            result["content"][0]["text"]
        );
        result["structuredContent"].clone()
    }

    pub fn store(&self) -> &Arc<Store> {
        self.state.store()
    }
}

/// Windows a display currently reports a finger on.
///
/// `dumpsys input` prints, per display, one block of window lines, and a window
/// holding a pointer carries `touchingPointers=[Pointer(id=0, FINGER)]`. Anything
/// after that display's block belongs to another display.
pub fn touching_windows(dump: &str, display: u32) -> Vec<String> {
    let mut inside = false;
    let mut found = Vec::new();
    let header = format!("{display} :");
    for line in dump
        .lines()
        .skip_while(|l| !l.contains("TouchStatesByDisplay"))
    {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent <= 4 {
            if !trimmed.starts_with(&header) {
                if inside {
                    // The next display, or the section after this one.
                    break;
                }
                continue;
            }
            inside = true;
            continue;
        }
        if inside && trimmed.contains("touchingPointers=[Pointer") {
            let name = trimmed
                .split("name='")
                .nth(1)
                .and_then(|rest| rest.split('\'').next())
                .unwrap_or("unnamed window")
                .to_string();
            found.push(name);
        }
    }
    found
}

pub fn http(port: u16, token: Option<&str>, origin: Option<&str>, body: &str) -> (u16, String) {
    http_path(port, "/mcp", token, origin, body)
}

pub fn http_path(
    port: u16,
    path: &str,
    token: Option<&str>,
    origin: Option<&str>,
    body: &str,
) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(900)))
        .unwrap();
    let mut head = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    if let Some(origin) = origin {
        head.push_str(&format!("Origin: {origin}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(body.as_bytes()).unwrap();
    stream.flush().unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).unwrap();
    let text = String::from_utf8_lossy(&response).to_string();
    let (headers, payload) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    let status: u16 = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    (status, payload.to_string())
}
