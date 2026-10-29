//! A single, demand-driven capture of Android's default display.
//!
//! App windows remain scrcpy windows. The integrated WebView cannot consume a
//! scrcpy window, so this uses Android's H.264 screenrecord pipe and WebCodecs.

use std::process::Stdio;
use std::time::Duration;

use tauri::ipc::{Channel, Response};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};

use super::adb::Adb;
use crate::domain::{DwError, DwResult, ErrorCode};
use crate::proc;

struct Session {
    id: String,
    stop: oneshot::Sender<()>,
    done: oneshot::Receiver<()>,
}

#[derive(Default)]
pub struct PhoneStream {
    session: Mutex<Option<Session>>,
}

impl PhoneStream {
    pub async fn start(
        &self,
        adb: Adb,
        serial: String,
        id: String,
        output: Channel<Response>,
    ) -> DwResult<()> {
        let mut session = self.session.lock().await;
        if let Some(old) = session.take() {
            stop_session(old).await;
        }
        let mut command = Command::new(adb.exe());
        // `exec` makes the reported shell PID become the screenrecord PID. The
        // arguments are fixed, not constructed from user input.
        command.args([
            "-s", &serial, "exec-out",
            "sh -c 'echo DROIDWIN_PID:$$; exec screenrecord --output-format=h264 --bit-rate 4M --time-limit 180 -'",
        ]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        proc::no_console_async(&mut command);
        let mut child = command.spawn().map_err(|e| {
            DwError::io(
                ErrorCode::RuntimeUnavailable,
                "Could not start the phone video capture.",
                &e,
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            DwError::new(
                ErrorCode::BackendFailure,
                "Phone capture did not provide diagnostics.",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            DwError::new(
                ErrorCode::BackendFailure,
                "Phone capture did not provide video.",
            )
        })?;
        let mut stdout = BufReader::new(stdout);
        let mut line = Vec::new();
        let read =
            tokio::time::timeout(Duration::from_secs(5), stdout.read_until(b'\n', &mut line)).await;
        let text = String::from_utf8_lossy(&line);
        let pid = match read {
            Ok(Ok(_)) => text
                .trim()
                .strip_prefix("DROIDWIN_PID:")
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|pid| *pid > 1),
            _ => None,
        };
        let Some(pid) = pid else {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Android did not start phone video capture.",
            )
            .detail(text.trim().to_string()));
        };
        let (stop, stopped) = oneshot::channel();
        let (done_tx, done) = oneshot::channel();
        tokio::spawn(async move {
            let mut stdout = stdout;
            let diagnostics = tokio::spawn(async move {
                let mut stderr = stderr;
                let mut buffer = [0u8; 1024];
                let mut detail = String::new();
                while let Ok(n) = stderr.read(&mut buffer).await {
                    if n == 0 {
                        break;
                    }
                    if detail.len() < 16_384 {
                        detail.push_str(&String::from_utf8_lossy(&buffer[..n]));
                    }
                }
                detail
            });
            let mut child = child;
            let result = forward(&mut stdout, &output, stopped).await;
            cleanup(&adb, &serial, pid, &mut child).await;
            let detail = diagnostics.await.unwrap_or_default();
            if let Err(error) = result {
                tracing::warn!("phone video stopped: {error}; {}", detail.trim());
                // Empty payload marks an unexpected stop, rather than claiming a live stream.
                let _ = output.send(Response::new(Vec::<u8>::new()));
            }
            let _ = done_tx.send(());
        });
        *session = Some(Session { id, stop, done });
        Ok(())
    }

    pub async fn stop(&self, id: Option<&str>) {
        let mut session = self.session.lock().await;
        if session
            .as_ref()
            .is_some_and(|s| id.is_none() || id == Some(s.id.as_str()))
        {
            if let Some(old) = session.take() {
                stop_session(old).await;
            }
        }
    }
}

async fn stop_session(session: Session) {
    let _ = session.stop.send(());
    if tokio::time::timeout(Duration::from_secs(5), session.done)
        .await
        .is_err()
    {
        tracing::warn!("phone capture did not stop within five seconds");
    }
}

async fn cleanup(adb: &Adb, serial: &str, pid: u32, child: &mut Child) {
    // Only kill the exact process we started, and confirm it is still a
    // screenrecord process to avoid killing a recycled PID.
    let command = format!("if [ -r /proc/{pid}/cmdline ] && grep -q screenrecord /proc/{pid}/cmdline; then kill {pid}; fi");
    let _ = adb
        .run(&["-s", serial, "shell", &command], Duration::from_secs(2))
        .await;
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn forward(
    stdout: &mut BufReader<tokio::process::ChildStdout>,
    output: &Channel<Response>,
    mut stop: oneshot::Receiver<()>,
) -> Result<(), String> {
    let mut buf = [0u8; 32 * 1024];
    loop {
        tokio::select! {
            _ = &mut stop => return Ok(()),
            result = stdout.read(&mut buf) => {
                let n = result.map_err(|e| e.to_string())?;
                if n == 0 { return Err("capture ended".into()); }
                output.send(Response::new(buf[..n].to_vec())).map_err(|e| e.to_string())?;
            }
        }
    }
}
