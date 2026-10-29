//! AI-agent control: the native action layer, its MCP surface, the local
//! transport that serves it, and the skills an agent can read.
//!
//! The desktop UI is the only owner of the runtime. An agent talks to this
//! module, this module talks to `EmulatorRuntime`, and nothing here starts an
//! emulator, an adb server or a window of its own.

pub mod control;
pub mod mcp;
pub mod server;
pub mod skills;
pub mod stats;
pub mod stdio;
pub mod triggers;
pub mod ui;
pub mod workflow;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::domain::{now_ms, DwError, DwResult, ErrorCode};
use crate::runtime::{EmulatorRuntime, EventBus};
use crate::store::Store;
use control::{Cancel, ControlService};
use triggers::TriggerEngine;
use workflow::Workflow;

/// How much an agent may do. Read-only inspection is the safe half: it never
/// changes what is on the Android screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadOnly,
    Control,
}

impl Permission {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "read_only" | "readonly" => Some(Permission::ReadOnly),
            "control" => Some(Permission::Control),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Permission::ReadOnly => "read_only",
            Permission::Control => "control",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub id: String,
    pub at_ms: i64,
    pub tool: String,
    pub ok: bool,
    pub summary: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_id: Option<String>,
}

/// Bounded memory of what agents did. This is the audit trail the UI shows and
/// the source of `get_recent_runtime_errors`; it deliberately holds no
/// screenshots and no typed text.
pub struct ActionLog {
    entries: Mutex<VecDeque<ActionRecord>>,
    counter: AtomicU64,
}

impl ActionLog {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            counter: AtomicU64::new(0),
        }
    }

    pub fn next_id(&self) -> String {
        format!(
            "act-{}-{}",
            now_ms(),
            self.counter.fetch_add(1, Ordering::Relaxed)
        )
    }

    pub fn record(&self, record: ActionRecord) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.push_front(record);
            while entries.len() > 200 {
                entries.pop_back();
            }
        }
    }

    pub fn recent(&self, limit: usize) -> Vec<ActionRecord> {
        self.entries
            .lock()
            .map(|entries| entries.iter().take(limit).cloned().collect())
            .unwrap_or_default()
    }

    pub fn failures(&self, limit: usize) -> Vec<ActionRecord> {
        self.entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| !entry.ok)
                    .take(limit)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for ActionLog {
    fn default() -> Self {
        Self::new()
    }
}

pub struct AgentState {
    pub control: Arc<ControlService>,
    /// Trigger detection and firing. One engine per process, one workflow runner.
    pub triggers: Arc<TriggerEngine>,
    store: Arc<Store>,
    data_dir: PathBuf,
    log: Arc<ActionLog>,
    permission: Mutex<Permission>,
    paused: Mutex<bool>,
    cancellations: Mutex<HashMap<String, Vec<Cancel>>>,
    server: tokio::sync::Mutex<Option<server::RunningServer>>,
}

impl AgentState {
    /// Wiring with no lifecycle feed: triggers still fire by hand and still poll
    /// state, but app events never reach them. The desktop uses `with_events`.
    pub fn new(runtime: Arc<EmulatorRuntime>, store: Arc<Store>, data_dir: PathBuf) -> Arc<Self> {
        Self::with_events(runtime, store, data_dir, EventBus::new())
    }

    /// `bus` is the same feed the runtime publishes app lifecycle on, so a
    /// trigger watching `app_opened` sees the window the runtime opened.
    pub fn with_events(
        runtime: Arc<EmulatorRuntime>,
        store: Arc<Store>,
        data_dir: PathBuf,
        bus: EventBus,
    ) -> Arc<Self> {
        Self::build(runtime, store, data_dir, bus, None, None)
    }

    pub fn with_events_and_input(
        runtime: Arc<EmulatorRuntime>,
        store: Arc<Store>,
        data_dir: PathBuf,
        bus: EventBus,
        input: Arc<crate::input::InputController>,
    ) -> Arc<Self> {
        Self::build(runtime, store, data_dir, bus, Some(input), None)
    }

    /// The desktop app's wiring: the agent shares the one input controller and
    /// the one capture session, so a command line recording and the phone screen
    /// are the same recording rather than two competing ones.
    pub fn with_events_input_and_capture(
        runtime: Arc<EmulatorRuntime>,
        store: Arc<Store>,
        data_dir: PathBuf,
        bus: EventBus,
        input: Arc<crate::input::InputController>,
        capture: Arc<crate::capture::Capture>,
    ) -> Arc<Self> {
        Self::build(runtime, store, data_dir, bus, Some(input), Some(capture))
    }

    fn build(
        runtime: Arc<EmulatorRuntime>,
        store: Arc<Store>,
        data_dir: PathBuf,
        bus: EventBus,
        input: Option<Arc<crate::input::InputController>>,
        capture: Option<Arc<crate::capture::Capture>>,
    ) -> Arc<Self> {
        let log = Arc::new(ActionLog::new());
        let mut control = ControlService::new(
            Arc::clone(&runtime),
            Arc::clone(&store),
            data_dir.clone(),
            Arc::clone(&log),
            input,
        );
        if let Some(capture) = capture {
            control = control.with_capture(capture);
        }
        let control = Arc::new(control);
        let triggers = TriggerEngine::new(
            runtime,
            Arc::clone(&control),
            Arc::clone(&store),
            Arc::clone(&log),
            bus,
        );
        Arc::new(Self {
            control,
            triggers,
            store,
            data_dir,
            log,
            // Starting the endpoint is the user's explicit grant; the UI can
            // narrow it to read-only at any time.
            permission: Mutex::new(Permission::Control),
            paused: Mutex::new(false),
            cancellations: Mutex::new(HashMap::new()),
            server: tokio::sync::Mutex::new(None),
        })
    }

    pub fn log(&self) -> &Arc<ActionLog> {
        &self.log
    }

    pub fn permission(&self) -> Permission {
        self.permission
            .lock()
            .map(|value| *value)
            .unwrap_or(Permission::ReadOnly)
    }

    pub fn set_permission(&self, permission: Permission) -> DwResult<()> {
        let mut guard = self.permission.lock().map_err(|_| locked())?;
        *guard = permission;
        Ok(())
    }

    pub fn paused(&self) -> bool {
        self.paused.lock().map(|value| *value).unwrap_or(true)
    }

    pub fn set_paused(&self, paused: bool) -> DwResult<()> {
        let mut guard = self.paused.lock().map_err(|_| locked())?;
        *guard = paused;
        // Pausing must also stop waits that are already in flight, not only
        // refuse the next call.
        if paused {
            if let Ok(tokens) = self.cancellations.lock() {
                for requests in tokens.values() {
                    for request in requests {
                        request.trip();
                    }
                }
            }
        }
        Ok(())
    }

    pub fn register(&self, request_id: &str) -> Cancel {
        let cancel = Cancel::new();
        if let Ok(mut tokens) = self.cancellations.lock() {
            tokens
                .entry(request_id.to_string())
                .or_default()
                .push(cancel.clone());
        }
        cancel
    }

    pub fn finish(&self, request_id: &str, cancel: &Cancel) {
        if let Ok(mut tokens) = self.cancellations.lock() {
            if let Some(requests) = tokens.get_mut(request_id) {
                requests.retain(|request| !request.same(cancel));
                if requests.is_empty() {
                    tokens.remove(request_id);
                }
            }
        }
    }

    pub fn cancel_request(&self, request_id: &str) -> bool {
        self.cancellations
            .lock()
            .ok()
            .and_then(|tokens| tokens.get(request_id).cloned())
            .map(|requests| {
                for request in &requests {
                    request.trip();
                }
                true
            })
            .unwrap_or(false)
    }

    /// Start the endpoint. Takes the `Arc` by value so the accept loop can hold
    /// the state it serves.
    pub async fn start(self: Arc<Self>) -> DwResult<()> {
        let mut server = self.server.lock().await;
        if server.is_some() {
            return Ok(());
        }
        let token = new_token()?;
        let running = server::serve(Arc::clone(&self), token).await?;
        if let Err(error) = write_endpoint_file(&self.data_dir, &running.url, &running.token) {
            running.task.abort();
            return Err(error);
        }
        // Triggers are automation: they detect while the endpoint is up, and
        // stop with it so nothing keeps driving Android after control is off.
        self.triggers.start().await;
        // Remember the choice so the endpoint comes back next launch; stopping
        // it is equally explicit and turns that off.
        let _ = self.store.set_setting("agent.enabled", "1");
        tracing::info!("agent MCP endpoint listening on {}", running.url);
        *server = Some(running);
        Ok(())
    }

    pub async fn stop(&self) {
        if let Ok(tokens) = self.cancellations.lock() {
            for requests in tokens.values() {
                for request in requests {
                    request.trip();
                }
            }
        }
        self.triggers.stop().await;
        let mut slot = self.server.lock().await;
        if let Some(server) = slot.take() {
            server.task.abort();
            let _ = server.task.await;
            let _ = std::fs::remove_file(endpoint_file(&self.data_dir));
            let _ = self.store.set_setting("agent.enabled", "0");
            tracing::info!("agent MCP endpoint stopped");
        }
    }

    pub async fn endpoint(&self) -> Option<(String, u16)> {
        let slot = self.server.lock().await;
        slot.as_ref()
            .map(|server| (server.url.clone(), server.port))
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Whether the endpoint was left running last session.
    pub fn should_restore(&self) -> bool {
        self.store.setting("agent.enabled").as_deref() == Some("1")
    }

    pub fn store(&self) -> &Arc<Store> {
        &self.store
    }

    /// Run steps through the one action layer. The same call serves the
    /// `run_workflow` tool and every trigger fire.
    pub async fn run_workflow(
        &self,
        workflow: &Workflow,
        cancel: &Cancel,
    ) -> DwResult<serde_json::Value> {
        workflow::run(&self.control, workflow, cancel).await
    }

    pub async fn status(&self) -> AgentStatus {
        let endpoint = {
            let slot = self.server.lock().await;
            slot.as_ref().map(|server| server.url.clone())
        };
        let permission = self.permission();
        let paused = self.paused();
        let skills = skills::discover(&self.data_dir, &self.store).unwrap_or_default();
        let actions = self.log.recent(20);
        let error_count = self.log.failures(200).len();
        let running = endpoint.is_some();
        AgentStatus {
            running,
            endpoint,
            token_file: running.then(|| endpoint_file(&self.data_dir).display().to_string()),
            stdio_command: format!(
                "\"{}\" --mcp-stdio",
                std::env::current_exe()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| "droidwin.exe".into())
            ),
            permission,
            paused,
            tool_count: mcp::tool_count(),
            actions,
            error_count,
            skills,
        }
    }
}

fn locked() -> DwError {
    DwError::new(
        ErrorCode::BackendFailure,
        "The agent control state is unavailable.",
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentStatus {
    pub running: bool,
    pub endpoint: Option<String>,
    /// Where an MCP client (or the stdio bridge) reads the URL and token.
    pub token_file: Option<String>,
    pub stdio_command: String,
    pub permission: Permission,
    pub paused: bool,
    pub tool_count: usize,
    pub actions: Vec<ActionRecord>,
    pub error_count: usize,
    pub skills: Vec<skills::SkillInfo>,
}

pub fn endpoint_file(data_dir: &Path) -> PathBuf {
    data_dir.join("agent").join("mcp.json")
}

fn write_endpoint_file(data_dir: &Path, url: &str, token: &str) -> DwResult<()> {
    let path = endpoint_file(data_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "Cannot create the agent directory.",
                &e,
            )
        })?;
    }
    let body = serde_json::json!({
        "url": url,
        "token": token,
        "pid": std::process::id(),
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&body).unwrap_or_default()).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "Cannot write the agent endpoint file.",
            &e,
        )
    })
}

/// 128 bits of randomness as hex. Windows' system-preferred RNG is used when it
/// is available; the fallback still produces a unique value per start, which is
/// what the local-only endpoint needs if BCrypt is refused.
pub fn new_token() -> DwResult<String> {
    let mut bytes = [0u8; 16];
    if !system_random(&mut bytes) {
        let nano = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mixed = (nano as u64) ^ ((std::process::id() as u64) << 32) ^ now_ms() as u64;
        bytes[..8].copy_from_slice(&mixed.to_le_bytes());
        bytes[8..].copy_from_slice(&(!mixed).to_le_bytes());
    }
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

#[cfg(windows)]
fn system_random(buffer: &mut [u8]) -> bool {
    const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
    #[link(name = "bcrypt")]
    extern "system" {
        fn BCryptGenRandom(algorithm: isize, buffer: *mut u8, len: u32, flags: u32) -> i32;
    }
    let status = unsafe {
        BCryptGenRandom(
            0,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        )
    };
    status == 0
}

#[cfg(not(windows))]
fn system_random(buffer: &mut [u8]) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open("/dev/urandom") else {
        return false;
    };
    file.read_exact(buffer).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_32_hex_characters_and_unique() {
        let first = new_token().unwrap();
        let second = new_token().unwrap();
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn the_action_log_is_bounded_and_newest_first() {
        let log = ActionLog::new();
        for index in 0..250 {
            log.record(ActionRecord {
                id: log.next_id(),
                at_ms: index,
                tool: "tap".into(),
                ok: index % 2 == 0,
                summary: "tapped".into(),
                duration_ms: 1,
                display_id: Some(2),
                package_id: None,
            });
        }
        let recent = log.recent(5);
        assert_eq!(recent.len(), 5);
        assert_eq!(recent[0].at_ms, 249);
        assert!(log.recent(500).len() <= 200);
        assert!(log.failures(500).iter().all(|entry| !entry.ok));
    }

    #[test]
    fn permission_strings_round_trip() {
        assert_eq!(Permission::parse("read_only"), Some(Permission::ReadOnly));
        assert_eq!(Permission::parse("control"), Some(Permission::Control));
        assert_eq!(Permission::parse("root"), None);
        assert_eq!(Permission::Control.as_str(), "control");
    }

    #[test]
    fn the_endpoint_file_lives_under_the_agent_directory() {
        let path = endpoint_file(Path::new("C:/data"));
        assert!(path.ends_with("agent/mcp.json") || path.ends_with("agent\\mcp.json"));
    }
}
