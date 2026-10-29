//! `droidwin.exe --mcp-stdio`: the stdio transport an MCP client can launch.
//!
//! MCP clients that start a process expect newline-delimited JSON-RPC on stdin
//! and nothing else on stdout. This bridge is only that: it forwards each
//! message to the endpoint the running DroidWin wrote to its data directory and
//! writes the reply back. Diagnostics go to stderr, never to stdout, so the
//! protocol stream stays clean.

use std::io::{BufRead, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

struct Endpoint {
    host: String,
    port: u16,
    path: String,
    token: String,
}

/// The app's data directory, resolved the same way the app resolves it, so the
/// bridge finds the endpoint file without being told where it is.
pub fn data_dir() -> Option<PathBuf> {
    if let Ok(value) = std::env::var("DROIDWIN_HOME") {
        if !value.trim().is_empty() {
            return Some(PathBuf::from(value));
        }
    }
    let identifier = "app.droidwin.desktop";
    if cfg!(windows) {
        return std::env::var("APPDATA")
            .ok()
            .map(|appdata| PathBuf::from(appdata).join(identifier));
    }
    dirs::home_dir().map(|home| home.join(".local").join("share").join(identifier))
}

fn parse_endpoint(url: &str, token: &str) -> Option<Endpoint> {
    let rest = url.strip_prefix("http://")?;
    let (authority, path) = match rest.split_once('/') {
        Some((authority, path)) => (authority, format!("/{path}")),
        None => (rest, "/mcp".to_string()),
    };
    let (host, port) = authority.rsplit_once(':')?;
    Some(Endpoint {
        host: host.to_string(),
        port: port.parse().ok()?,
        path,
        token: token.to_string(),
    })
}

fn load_endpoint() -> Option<Endpoint> {
    let text = std::fs::read_to_string(data_dir()?.join("agent").join("mcp.json")).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    let url = value.get("url")?.as_str()?;
    let token = value.get("token")?.as_str()?;
    parse_endpoint(url, token)
}

/// One request/response round trip. The endpoint answers with one JSON body and
/// closes the connection, so reading to end is exact.
fn post(endpoint: &Endpoint, body: &str) -> Result<Option<String>, String> {
    let mut stream = TcpStream::connect((endpoint.host.as_str(), endpoint.port))
        .map_err(|_| "DroidWin is not running, or its agent endpoint is stopped.".to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(30)));
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        endpoint.path,
        endpoint.host,
        endpoint.port,
        endpoint.token,
        body.len(),
        body
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("DroidWin could not receive that request: {e}"))?;
    let mut response = Vec::new();
    std::io::Read::read_to_end(&mut stream, &mut response)
        .map_err(|e| format!("DroidWin did not answer: {e}"))?;
    parse_response(&response)
}

fn parse_response(response: &[u8]) -> Result<Option<String>, String> {
    let text = String::from_utf8_lossy(response);
    let (head, payload) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("DroidWin answered HTTP {status}."));
    }
    let payload = payload.trim();
    if payload.is_empty() {
        Ok(None)
    } else {
        Ok(Some(payload.to_string()))
    }
}

fn error_reply(id: Value, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": -32000, "message": message }
    })
}

/// Run the bridge until stdin closes. Returns the process exit code.
pub fn run() -> i32 {
    let endpoint = load_endpoint();
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    if endpoint.is_none() {
        eprintln!(
            "droidwin-mcp: no agent endpoint found under {}. Start DroidWin and enable the agent endpoint.",
            data_dir()
                .map(|dir| dir.display().to_string())
                .unwrap_or_else(|| "the app data directory".into())
        );
    }

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let reply = error_reply(Value::Null, &format!("Parse error: {err}"));
                let _ = writeln!(stdout, "{reply}");
                let _ = stdout.flush();
                continue;
            }
        };
        let id = request.get("id").cloned();
        let response = match (&endpoint, id) {
            (Some(endpoint), Some(id)) => match post(endpoint, trimmed) {
                Ok(Some(reply)) => reply,
                Ok(None) => {
                    error_reply(id, "DroidWin returned no reply to this request.").to_string()
                }
                Err(message) => error_reply(id, &message).to_string(),
            },
            // Notifications must be forwarded, but they never get a stdout reply.
            (Some(endpoint), None) => {
                if let Err(message) = post(endpoint, trimmed) {
                    eprintln!("droidwin-mcp: {message}");
                }
                continue;
            }
            // A request still deserves a reply; a notification does not.
            (None, Some(id)) => error_reply(
                id,
                "DroidWin is not running, or its agent endpoint is stopped.",
            )
            .to_string(),
            (None, None) => continue,
        };
        if writeln!(stdout, "{response}").is_err() {
            break;
        }
        let _ = stdout.flush();
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_url_is_split_into_host_port_and_path() {
        let endpoint = parse_endpoint("http://127.0.0.1:51234/mcp", "abc").unwrap();
        assert_eq!(endpoint.host, "127.0.0.1");
        assert_eq!(endpoint.port, 51234);
        assert_eq!(endpoint.path, "/mcp");
        assert_eq!(endpoint.token, "abc");
    }

    #[test]
    fn a_url_without_a_path_still_targets_mcp() {
        let endpoint = parse_endpoint("http://127.0.0.1:9000", "abc").unwrap();
        assert_eq!(endpoint.path, "/mcp");
    }

    #[test]
    fn a_non_http_url_is_not_a_local_endpoint() {
        assert!(parse_endpoint("https://example.com/mcp", "abc").is_none());
        assert!(parse_endpoint("nonsense", "abc").is_none());
    }

    #[test]
    fn an_error_reply_carries_the_request_id() {
        let reply = error_reply(json!(7), "down");
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["message"], "down");
    }

    #[test]
    fn empty_success_response_is_a_notification_not_an_error() {
        assert_eq!(
            parse_response(b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\n\r\n"),
            Ok(None)
        );
        assert_eq!(
            parse_response(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"),
            Ok(Some("{}".into()))
        );
        assert!(parse_response(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n").is_err());
    }
}
