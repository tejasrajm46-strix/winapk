//! The local MCP transport.
//!
//! Streamable HTTP on 127.0.0.1 only, one JSON-RPC message per POST, protected
//! by a token that is generated per session and written to the app data
//! directory. Localhost is not an authentication boundary — any process on the
//! machine can reach a loopback port — so the token is required on every
//! request, the Host header must be loopback, and a browser Origin is refused.
//! That combination blocks the cross-site request a hostile page could make.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{AgentState, Permission};
use crate::domain::{DwError, DwResult, ErrorCode};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

pub struct RunningServer {
    pub url: String,
    pub token: String,
    pub port: u16,
    pub task: tokio::task::JoinHandle<()>,
}

pub async fn serve(state: Arc<AgentState>, token: String) -> DwResult<RunningServer> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| {
        DwError::io(
            ErrorCode::RuntimeUnavailable,
            "The local agent endpoint could not start.",
            &e,
        )
    })?;
    let port = listener
        .local_addr()
        .map_err(|e| {
            DwError::io(
                ErrorCode::RuntimeUnavailable,
                "The local agent endpoint has no address.",
                &e,
            )
        })?
        .port();
    let url = format!("http://127.0.0.1:{port}/mcp");
    let session = token.clone();
    let task = tokio::spawn(async move {
        let mut connections = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => match accepted {
                    Ok((socket, _)) => {
                        let state = Arc::clone(&state);
                        let token = session.clone();
                        connections.spawn(async move {
                            if let Err(err) = handle_connection(state, &token, socket).await {
                                tracing::debug!("agent connection closed: {err}");
                            }
                        });
                    }
                    Err(err) => {
                        tracing::warn!("agent listener could not accept: {err}");
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                },
                Some(result) = connections.join_next(), if !connections.is_empty() => {
                    if let Err(err) = result {
                        tracing::debug!("agent connection task failed: {err}");
                    }
                }
            }
        }
    });
    Ok(RunningServer {
        url,
        token,
        port,
        task,
    })
}

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

enum Failure {
    Status(u16, &'static str),
    Io,
}

impl From<std::io::Error> for Failure {
    fn from(_: std::io::Error) -> Self {
        Failure::Io
    }
}

async fn handle_connection(
    state: Arc<AgentState>,
    token: &str,
    mut socket: TcpStream,
) -> std::io::Result<()> {
    let request = match tokio::time::timeout(Duration::from_secs(15), read_request(&mut socket))
        .await
    {
        Ok(Ok(request)) => request,
        Ok(Err(Failure::Status(code, message))) => {
            return respond(&mut socket, code, json!({ "error": message })).await
        }
        Ok(Err(Failure::Io)) => return Ok(()),
        Err(_) => return respond(&mut socket, 408, json!({ "error": "Request timed out" })).await,
    };

    let cli = request.path == "/cli" || request.path == "/cli/";
    let mcp = request.path == "/mcp" || request.path == "/mcp/";
    if !cli && !mcp {
        return respond(&mut socket, 404, json!({ "error": "Not found" })).await;
    }
    if request.method != "POST" {
        return respond_with_allow(&mut socket, 405, json!({ "error": "Use POST" })).await;
    }
    if !authorized(&request.headers, token) {
        return respond(&mut socket, 401, json!({ "error": "Unauthorized" })).await;
    }
    if !loopback_host(&request.headers) || !acceptable_origin(&request.headers) {
        return respond(&mut socket, 403, json!({ "error": "Refused" })).await;
    }

    let message: Value = match serde_json::from_slice(&request.body) {
        Ok(value) => value,
        Err(err) => {
            let body = json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("Parse error: {err}") }
            });
            return respond(&mut socket, 400, body).await;
        }
    };

    if cli {
        return serve_cli(&state, &mut socket, message).await;
    }

    match super::mcp::handle(&state, &message).await {
        Some(reply) => respond(&mut socket, 200, reply).await,
        // A notification is acknowledged without a body, as the protocol says.
        None => {
            let head = "HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n";
            socket.write_all(head.as_bytes()).await?;
            socket.flush().await
        }
    }
}

/// One command from the `droidwin` command line.
///
/// Same listener, same token and same permission gate as `tools/call`; only the
/// envelope differs, so the command line gets one compact result instead of an
/// MCP content block list. Command failures are answers (HTTP 200, `ok:false`)
/// because the request itself was well formed.
async fn serve_cli(
    state: &Arc<AgentState>,
    socket: &mut TcpStream,
    message: Value,
) -> std::io::Result<()> {
    let Some(command) = message
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return respond(
            socket,
            400,
            json!({ "ok": false, "error": { "code": "invalid_request", "summary": "A CLI request needs a command." } }),
        )
        .await;
    };
    let args = message.get("args").cloned().unwrap_or_else(|| json!({}));
    if !args.is_object() {
        return respond(
            socket,
            400,
            json!({ "ok": false, "command": command, "error": { "code": "invalid_request", "summary": "A CLI command's arguments must be an object." } }),
        )
        .await;
    }

    if super::control::cli_command_is_control(command) {
        if state.paused() {
            return respond(
                socket,
                200,
                json!({
                    "ok": false,
                    "command": command,
                    "error": {
                        "code": "automation_paused",
                        "summary": "Automation is paused in WINAPK.",
                        "detail": "Read-only commands still work. Resume automation in the Agent screen to use this one."
                    }
                }),
            )
            .await;
        }
        if state.permission() == Permission::ReadOnly {
            return respond(
                socket,
                200,
                json!({
                    "ok": false,
                    "command": command,
                    "error": {
                        "code": "permission_denied",
                        "summary": "WINAPK is in read-only mode for agents.",
                        "detail": "Allow control in WINAPK's Agent screen to use this command."
                    }
                }),
            )
            .await;
        }
    }

    let started = std::time::Instant::now();
    let outcome = state.control.dispatch_cli(command, &args).await;
    let duration_ms = started.elapsed().as_millis() as u64;
    match outcome {
        Ok(outcome) => {
            state.log().record(super::ActionRecord {
                id: state.log().next_id(),
                at_ms: crate::domain::now_ms(),
                tool: format!("cli:{command}"),
                ok: true,
                summary: outcome.summary.clone(),
                duration_ms,
                display_id: outcome.display_id,
                package_id: outcome.package_id.clone(),
            });
            respond(
                socket,
                200,
                json!({
                    "ok": true,
                    "command": command,
                    "summary": outcome.summary,
                    "data": outcome.payload
                }),
            )
            .await
        }
        Err(err) => {
            state.log().record(super::ActionRecord {
                id: state.log().next_id(),
                at_ms: crate::domain::now_ms(),
                tool: format!("cli:{command}"),
                ok: false,
                summary: err.summary.clone(),
                duration_ms,
                display_id: None,
                package_id: None,
            });
            respond(
                socket,
                200,
                json!({
                    "ok": false,
                    "command": command,
                    "error": {
                        "code": err.code.as_str(),
                        "summary": err.summary,
                        "detail": err.detail
                    }
                }),
            )
            .await
        }
    }
}

async fn respond(socket: &mut TcpStream, status: u16, body: Value) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Payload Too Large",
        431 => "Request Header Fields Too Large",
        _ => "Error",
    };
    let payload = serde_json::to_vec(&body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        payload.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&payload).await?;
    socket.flush().await
}

async fn respond_with_allow(
    socket: &mut TcpStream,
    status: u16,
    body: Value,
) -> std::io::Result<()> {
    let payload = serde_json::to_vec(&body).unwrap_or_default();
    let head = format!(
        "HTTP/1.1 {status} Method Not Allowed\r\nAllow: POST\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&payload).await?;
    socket.flush().await
}

async fn read_request(socket: &mut TcpStream) -> Result<Request, Failure> {
    let mut buffer: Vec<u8> = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let header_end = loop {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            return Err(Failure::Status(400, "Empty request"));
        }
        buffer.extend_from_slice(&chunk[..read]);
        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(Failure::Status(431, "Headers too large"));
        }
    };

    let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let mut headers = Vec::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }

    let content_length: usize = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    if content_length > MAX_BODY_BYTES {
        return Err(Failure::Status(413, "Request body too large"));
    }

    let body_start = header_end + 4;
    let mut body = buffer[body_start.min(buffer.len())..].to_vec();
    while body.len() < content_length {
        let read = socket.read(&mut chunk).await?;
        if read == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..read]);
        if body.len() > MAX_BODY_BYTES {
            return Err(Failure::Status(413, "Request body too large"));
        }
    }
    body.truncate(content_length);

    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn value_for<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// Accept `Authorization: Bearer <token>` or the explicit header, so a bridge
/// that cannot set Authorization still has a supported path.
pub fn authorized(headers: &[(String, String)], token: &str) -> bool {
    if let Some(value) = value_for(headers, "authorization") {
        let presented = value.strip_prefix("Bearer ").unwrap_or(value);
        if constant_time_eq(presented.trim(), token) {
            return true;
        }
    }
    value_for(headers, "x-droidwin-token")
        .map(|value| constant_time_eq(value.trim(), token))
        .unwrap_or(false)
}

/// A Host header naming anything but loopback is a DNS-rebinding attempt.
pub fn loopback_host(headers: &[(String, String)]) -> bool {
    let Some(host) = value_for(headers, "host") else {
        return false;
    };
    let name = host.rsplit_once(':').map(|(name, _)| name).unwrap_or(host);
    let name = name.trim_matches(|c| c == '[' || c == ']');
    name == "127.0.0.1" || name == "localhost" || name == "::1"
}

/// Browsers attach Origin to cross-site requests. Anything but a loopback
/// origin is refused; absence is normal for a native MCP client.
pub fn acceptable_origin(headers: &[(String, String)]) -> bool {
    let Some(origin) = value_for(headers, "origin") else {
        return true;
    };
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or(origin);
    let name = rest.rsplit_once(':').map(|(name, _)| name).unwrap_or(rest);
    name == "127.0.0.1" || name == "localhost" || name == "[::1]"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect()
    }

    #[test]
    fn a_bearer_token_is_required_and_must_match() {
        let token = "abc123";
        assert!(authorized(
            &headers(&[("authorization", "Bearer abc123")]),
            token
        ));
        assert!(authorized(
            &headers(&[("x-droidwin-token", "abc123")]),
            token
        ));
        assert!(!authorized(
            &headers(&[("authorization", "Bearer abc124")]),
            token
        ));
        assert!(!authorized(
            &headers(&[("authorization", "Bearer abc12")]),
            token
        ));
        assert!(!authorized(&headers(&[]), token));
    }

    #[test]
    fn the_host_header_must_be_loopback() {
        assert!(loopback_host(&headers(&[("host", "127.0.0.1:51234")])));
        assert!(loopback_host(&headers(&[("host", "localhost:51234")])));
        assert!(!loopback_host(&headers(&[("host", "example.com:51234")])));
        assert!(!loopback_host(&headers(&[("host", "127.0.0.1.evil.com")])));
        assert!(!loopback_host(&headers(&[])));
    }

    #[test]
    fn a_cross_site_origin_is_refused_but_a_native_client_is_allowed() {
        assert!(acceptable_origin(&headers(&[])));
        assert!(acceptable_origin(&headers(&[(
            "origin",
            "http://127.0.0.1:1420"
        )])));
        assert!(acceptable_origin(&headers(&[(
            "origin",
            "http://localhost:3000"
        )])));
        assert!(!acceptable_origin(&headers(&[(
            "origin",
            "https://evil.example"
        )])));
    }

    #[test]
    fn header_end_is_found_only_at_the_terminator() {
        assert_eq!(find_header_end(b"a: b\r\n\r\nbody"), Some(4));
        assert_eq!(find_header_end(b"a: b\r\n"), None);
    }

    #[test]
    fn tokens_compare_without_early_exit_on_length_or_content() {
        assert!(constant_time_eq("abcdef", "abcdef"));
        assert!(!constant_time_eq("abcdef", "abcdeg"));
        assert!(!constant_time_eq("short", "longer"));
    }
}
