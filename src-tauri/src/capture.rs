//! Saving what Android is showing: one still, and one recording.
//!
//! Both go through the running emulator's own adb — the same device, the same
//! display 0 the virtual phone mirrors. Nothing here captures the Windows
//! desktop: `screencap` and `screenrecord` run inside Android, so what a file
//! contains is what the virtual phone shows.
//!
//! The save location is always explicit: the caller passes the directory (the
//! user's Pictures or Videos folder, or WINAPK's data directory as a
//! fallback). Nothing is written anywhere else, and nothing is written without
//! being asked for.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::sync::Mutex;

use crate::domain::{now_ms, DwError, DwResult, ErrorCode};
use crate::runtime::adb::Adb;
use crate::runtime::EmulatorRuntime;

/// Folder created inside the user's Pictures/Videos directory.
pub const FOLDER: &str = "WINAPK";
/// `screenrecord` stops itself here even if WINAPK is killed, so a crashed
/// host cannot leave a recorder running forever.
const MAX_RECORD_SECONDS: u32 = 300;
/// Video bit rate. 8 Mbit/s holds detail in a 1080x2400 game without making
/// files that are painful to move around.
const BIT_RATE: &str = "8M";

#[derive(Debug, Clone, serde::Serialize)]
pub struct SavedShot {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub captured_at: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RecordingState {
    pub active: bool,
    pub started_at: Option<i64>,
    pub elapsed_ms: u64,
    /// Where the file will be written, so the UI can say it before stopping.
    pub directory: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct SavedRecording {
    pub path: String,
    pub bytes: u64,
    pub started_at: i64,
    pub finished_at: i64,
    pub duration_ms: i64,
}

struct Session {
    remote: String,
    pid: u32,
    started_at: i64,
    started: Instant,
    directory: PathBuf,
}

/// One app window being recorded by a second, windowless window host attached
/// to that window's own Android display.
///
/// `screenrecord` cannot be used here: its `--display-id` takes a *physical*
/// display id and a virtual display has no physical token (measured; the tool
/// prints "Invalid physical display ID"). scrcpy already receives exactly the
/// display's video, so it records it instead — the same pixels the window shows,
/// not the phone's screen.
struct WindowSession {
    package_id: String,
    child: tokio::process::Child,
    /// The recorder's own hidden window. Its close button is what finalises the
    /// MP4; killing the process would leave a file without a `moov` atom.
    hwnd: isize,
    path: PathBuf,
    started_at: i64,
    started: Instant,
}

#[derive(Default)]
pub struct Capture {
    session: Mutex<Option<Session>>,
    window: Mutex<Option<WindowSession>>,
}

impl Capture {
    /// One still of Android's own display, written where the caller asked.
    ///
    /// The PNG comes from `screencap`, so it is the device's pixels at the
    /// device's own resolution and orientation, uncompressed.
    pub async fn save_screenshot(
        runtime: &EmulatorRuntime,
        directory: &Path,
    ) -> DwResult<SavedShot> {
        let (png, width, height) = runtime.device_screen().await?;
        create_directory(directory)?;
        let path = directory.join(format!("droidwin-{}.png", timestamp()));
        tokio::fs::write(&path, &png)
            .await
            .map_err(|e| storage_error("Could not write the screenshot.", &path, &e))?;
        Ok(SavedShot {
            path: path.display().to_string(),
            width,
            height,
            bytes: png.len() as u64,
            captured_at: now_ms(),
        })
    }

    /// One still of a single app's display — what that window is showing, not
    /// the phone's screen. Every window owns its own Android display, so the
    /// rail's Screenshot button captures the game under it and nothing else.
    pub async fn save_display_screenshot(
        runtime: &EmulatorRuntime,
        directory: &Path,
        display: u32,
    ) -> DwResult<SavedShot> {
        let adb = adb_for(runtime)?;
        let png = adb.screenshot_png(&runtime.serial(), display).await?;
        let (width, height) = crate::runtime::display::png_size(&png).ok_or_else(|| {
            DwError::new(
                ErrorCode::BackendFailure,
                "Android returned a capture with no readable size.",
            )
        })?;
        create_directory(directory)?;
        let path = directory.join(format!("droidwin-{}.png", timestamp()));
        tokio::fs::write(&path, &png)
            .await
            .map_err(|e| storage_error("Could not write the screenshot.", &path, &e))?;
        Ok(SavedShot {
            path: path.display().to_string(),
            width,
            height,
            bytes: png.len() as u64,
            captured_at: now_ms(),
        })
    }

    /// Start recording one app window's Android display.
    ///
    /// A second scrcpy instance mirrors that display with no visible window and
    /// writes it straight to an MP4. `--no-control` keeps it from accepting
    /// input: it is a recorder, not a second way to touch the app.
    pub async fn start_window_recording(
        &self,
        runtime: &EmulatorRuntime,
        package_id: &str,
        display_id: u32,
        directory: &Path,
    ) -> DwResult<RecordingState> {
        if display_id == 0 {
            return Err(DwError::new(
                ErrorCode::NotSupported,
                "Android has not reported a display for that window yet.",
            )
            .detail("Wait for the window to show its picture, then record it."));
        }
        let mut window = self.window.lock().await;
        if let Some(active) = window.as_ref() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                format!("WINAPK is already recording {}.", active.package_id),
            )
            .detail(format!(
                "Recording started {} s ago into {}.",
                active.started.elapsed().as_secs(),
                active.path.display()
            )));
        }
        if self.session.lock().await.is_some() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "WINAPK is already recording the phone's screen.",
            )
            .detail("Stop that recording before recording an app window."));
        }
        let exe = runtime.scrcpy_exe().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The window host (scrcpy) is missing from this installation.",
            )
        })?;
        create_directory(directory)?;
        let path = directory.join(format!("droidwin-{}-recording.mp4", timestamp()));
        let mut command = tokio::process::Command::new(&exe);
        command
            .args(window_recording_args(
                &runtime.serial(),
                display_id,
                package_id,
                &path,
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(false);
        crate::proc::no_console_async(&mut command);
        let mut child = command.spawn().map_err(|e| {
            DwError::io(
                ErrorCode::CaptureFailed,
                "WINAPK could not start the recording host.",
                &e,
            )
        })?;
        let pid = child.id().unwrap_or(0);

        // The window appears after the connection is up. Hiding it keeps the
        // desktop unchanged while the recorder runs; the handle is kept because
        // closing that window is how scrcpy finalises the file.
        let mut hwnd = None;
        for _ in 0..60 {
            if let Some(found) = crate::proc::window_for_pid(pid) {
                hwnd = Some(found);
                break;
            }
            if let Ok(Some(status)) = child.try_wait() {
                let mut detail = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(1),
                        stderr.read_to_string(&mut detail),
                    )
                    .await;
                }
                return Err(DwError::new(
                    ErrorCode::CaptureFailed,
                    "The recording host closed before it started recording.",
                )
                .detail(if detail.trim().is_empty() {
                    format!("scrcpy exited ({status})")
                } else {
                    detail.trim().to_string()
                }));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let Some(hwnd) = hwnd else {
            let _ = child.kill().await;
            return Err(DwError::new(
                ErrorCode::CaptureFailed,
                "The recording host did not open in time.",
            )
            .detail("No recording was written; the app window is unchanged."));
        };
        crate::proc::hide_window(hwnd);
        // A 48-byte header with no frames is what a dead recorder leaves. Give
        // the first keyframe a moment so "recording" means frames exist.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let started_at = now_ms();
        *window = Some(WindowSession {
            package_id: package_id.to_string(),
            child,
            hwnd,
            path: path.clone(),
            started_at,
            started: Instant::now(),
        });
        tracing::info!(
            "recording {} from display {} into {}",
            package_id,
            display_id,
            path.display()
        );
        Ok(RecordingState {
            active: true,
            started_at: Some(started_at),
            elapsed_ms: 0,
            directory: Some(directory.display().to_string()),
        })
    }

    pub async fn window_recording_state(&self) -> RecordingState {
        match self.window.lock().await.as_ref() {
            Some(session) => RecordingState {
                active: true,
                started_at: Some(session.started_at),
                elapsed_ms: session.started.elapsed().as_millis() as u64,
                directory: session.path.parent().map(|path| path.display().to_string()),
            },
            None => RecordingState {
                active: false,
                started_at: None,
                elapsed_ms: 0,
                directory: None,
            },
        }
    }

    /// Stop the window recording and return the finished file.
    ///
    /// The close message is the graceful path: scrcpy writes the MP4 trailer on
    /// exit. If the host does not leave within the budget it is killed, and the
    /// file's own header decides whether the result is reported or discarded.
    pub async fn stop_window_recording(&self) -> DwResult<SavedRecording> {
        let Some(mut session) = self.window.lock().await.take() else {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "WINAPK is not recording an app window.",
            ));
        };
        crate::proc::close_window(session.hwnd);
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut exited = false;
        while Instant::now() < deadline {
            match session.child.try_wait() {
                Ok(Some(_)) => {
                    exited = true;
                    break;
                }
                Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
                Err(_) => break,
            }
        }
        if !exited {
            tracing::warn!(
                "recording host for {} did not stop in time; killing it",
                session.package_id
            );
            let _ = session.child.kill().await;
            let _ = session.child.wait().await;
        }

        let bytes = match std::fs::read(&session.path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return Err(storage_error(
                    "The recording produced no file.",
                    &session.path,
                    &e,
                ))
            }
        };
        if !is_mp4(&bytes) || !has_moov(&bytes) || bytes.len() < 1_024 {
            let _ = std::fs::remove_file(&session.path);
            return Err(DwError::new(
                ErrorCode::CaptureFailed,
                "The app window recording is not a playable video, so it was discarded.",
            )
            .detail(format!(
                "{} bytes were written{}.",
                bytes.len(),
                if exited {
                    ""
                } else {
                    "; the host had to be killed, so its MP4 trailer may be missing"
                }
            )));
        }
        let finished_at = now_ms();
        Ok(SavedRecording {
            path: session.path.display().to_string(),
            bytes: bytes.len() as u64,
            started_at: session.started_at,
            finished_at,
            duration_ms: finished_at - session.started_at,
        })
    }

    /// Finalise a window recording without reporting a result: used when the
    /// runtime is going away, so no recorder is left running.
    pub async fn abort_window_recording(&self) {
        let Some(mut session) = self.window.lock().await.take() else {
            return;
        };
        crate::proc::close_window(session.hwnd);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            match session.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
                Err(_) => break,
            }
        }
        let _ = session.child.kill().await;
        let _ = session.child.wait().await;
    }

    pub async fn recording_state(&self) -> RecordingState {
        let session = self.session.lock().await;
        match session.as_ref() {
            Some(session) => RecordingState {
                active: true,
                started_at: Some(session.started_at),
                elapsed_ms: session.started.elapsed().as_millis() as u64,
                directory: Some(session.directory.display().to_string()),
            },
            None => RecordingState {
                active: false,
                started_at: None,
                elapsed_ms: 0,
                directory: None,
            },
        }
    }

    /// Start recording Android's display 0 into a file on the device.
    ///
    /// Recording in the guest and pulling the file afterwards is deliberate:
    /// `screenrecord` writes a real MP4 container as it goes, so the result
    /// plays in any player. Piping the raw H.264 the live view uses would leave
    /// a file no player can open without a second muxing step.
    pub async fn start_recording(
        &self,
        runtime: &EmulatorRuntime,
        directory: &Path,
    ) -> DwResult<RecordingState> {
        let mut session = self.session.lock().await;
        if let Some(active) = session.as_ref() {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "WINAPK is already recording the phone's screen.",
            )
            .detail(format!(
                "Recording started {} s ago into {}.",
                active.started.elapsed().as_secs(),
                active.remote
            )));
        }
        let adb = adb_for(runtime)?;
        let serial = runtime.serial();
        let remote = format!("/data/local/tmp/droidwin-rec-{}.mp4", timestamp());
        let log = format!("{remote}.log");
        // No `--display-id`: that flag takes a *physical* display id from
        // `dumpsys SurfaceFlinger --display-id`, not Android's display 0, and
        // passing 0 makes screenrecord exit with "Invalid physical display ID".
        // Its default is the primary display, which is the phone screen.
        let line = format!(
            "screenrecord --bit-rate {BIT_RATE} --time-limit {MAX_RECORD_SECONDS} --output-format=mp4 {remote} >{log} 2>&1 & echo DROIDWIN_REC:$!"
        );
        let out = adb
            .run(&["-s", &serial, "shell", &line], Duration::from_secs(20))
            .await?;
        let pid = out
            .stdout
            .lines()
            .find_map(|line| line.trim().strip_prefix("DROIDWIN_REC:"))
            .and_then(|value| value.trim().parse::<u32>().ok())
            .filter(|pid| *pid > 1);
        let Some(pid) = pid else {
            // Nothing was started, so nothing is left behind. The remote path is
            // removed in case screenrecord made an empty file before failing.
            let _ = adb
                .run(
                    &["-s", &serial, "shell", &format!("rm -f {remote} {log}")],
                    Duration::from_secs(10),
                )
                .await;
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Android did not start screen recording.",
            )
            .detail(out.combined()));
        };
        // A background job is reported by the shell before screenrecord can
        // reject its arguments, so a recorder that is already gone means Android
        // refused the request. Its own stderr says why.
        tokio::time::sleep(Duration::from_millis(350)).await;
        let alive = adb
            .run(
                &[
                    "-s",
                    &serial,
                    "shell",
                    &format!("if [ -d /proc/{pid} ]; then echo alive; else echo gone; fi"),
                ],
                Duration::from_secs(10),
            )
            .await
            .map(|out| out.stdout.contains("alive"))
            .unwrap_or(true);
        if !alive {
            let detail = adb
                .run(
                    &["-s", &serial, "shell", &format!("cat {log} 2>/dev/null")],
                    Duration::from_secs(10),
                )
                .await
                .map(|out| out.combined())
                .unwrap_or_default();
            let _ = adb
                .run(
                    &["-s", &serial, "shell", &format!("rm -f {remote} {log}")],
                    Duration::from_secs(10),
                )
                .await;
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "Android refused to start screen recording.",
            )
            .detail(if detail.trim().is_empty() {
                "screenrecord exited without a reason.".to_string()
            } else {
                detail.trim().to_string()
            }));
        }
        *session = Some(Session {
            remote,
            pid,
            started_at: now_ms(),
            started: Instant::now(),
            directory: directory.to_path_buf(),
        });
        Ok(RecordingState {
            active: true,
            started_at: Some(now_ms()),
            elapsed_ms: 0,
            directory: Some(directory.display().to_string()),
        })
    }

    /// Stop, finalise and pull the recording. The file is what makes this
    /// complete: a session that cannot produce a playable MP4 reports a failure
    /// instead of a path.
    pub async fn stop_recording(&self, runtime: &EmulatorRuntime) -> DwResult<SavedRecording> {
        let Some(session) = self.session.lock().await.take() else {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "WINAPK is not recording.",
            ));
        };
        let adb = adb_for(runtime)?;
        let serial = runtime.serial();
        create_directory(&session.directory)?;

        // SIGINT is how screenrecord closes its muxer; without it the file has
        // no moov atom and no player will open it.
        let _ = adb
            .run(
                &[
                    "-s",
                    &serial,
                    "shell",
                    &format!("kill -INT {} 2>/dev/null", session.pid),
                ],
                Duration::from_secs(10),
            )
            .await;
        let finalized = wait_for_finalize(&adb, &serial, &session).await;

        let path = session
            .directory
            .join(format!("droidwin-{}-recording.mp4", timestamp()));
        let pull = adb
            .run(
                &[
                    "-s",
                    &serial,
                    "pull",
                    &session.remote,
                    &path.display().to_string(),
                ],
                Duration::from_secs(180),
            )
            .await;
        // The remote copy and screenrecord's log are always cleaned up, pulled
        // or not.
        let _ = adb
            .run(
                &[
                    "-s",
                    &serial,
                    "shell",
                    &format!("rm -f {} {}.log", session.remote, session.remote),
                ],
                Duration::from_secs(10),
            )
            .await;

        // Pulling nothing is the visible symptom; adb's own message is the
        // explanation, so it is kept before the remote files are removed.
        let pull_detail = match &pull {
            Ok(out) => out.combined(),
            Err(err) => err.summary.clone(),
        };
        let bytes = std::fs::read(&path).map_err(|e| {
            let detail = if pull_detail.trim().is_empty() {
                "adb pull returned no file.".to_string()
            } else {
                pull_detail.trim().to_string()
            };
            storage_error("Android produced no recording to save.", &path, &e).detail(detail)
        })?;
        if !is_mp4(&bytes) || bytes.len() < 1_024 {
            let _ = std::fs::remove_file(&path);
            return Err(DwError::new(
                ErrorCode::BackendFailure,
                "The recording Android produced is not a playable video, so it was discarded.",
            )
            .detail(format!(
                "{} bytes were pulled{}.",
                bytes.len(),
                if pull_detail.trim().is_empty() {
                    String::new()
                } else {
                    format!("; adb said: {}", pull_detail.trim())
                }
            )));
        }

        if !finalized {
            // The header decided the file is usable; this only records that the
            // wait budget ran out, so the log names it if playback is odd.
            tracing::warn!(
                "screen recording finished without confirming the encoder stopped; {} bytes pulled",
                bytes.len()
            );
        }

        let finished_at = now_ms();
        Ok(SavedRecording {
            path: path.display().to_string(),
            bytes: bytes.len() as u64,
            started_at: session.started_at,
            finished_at,
            duration_ms: finished_at - session.started_at,
        })
    }

    /// Drop a recording in progress without saving it: the runtime is going
    /// away, so leaving a recorder running in the guest would be a leak.
    pub async fn abort_recording(&self, runtime: &EmulatorRuntime) {
        let Some(session) = self.session.lock().await.take() else {
            return;
        };
        let Ok(adb) = adb_for(runtime) else {
            return;
        };
        let serial = runtime.serial();
        let _ = adb
            .run(
                &[
                    "-s",
                    &serial,
                    "shell",
                    &format!(
                        "kill -INT {} 2>/dev/null; sleep 1; rm -f {} {}.log",
                        session.pid, session.remote, session.remote
                    ),
                ],
                Duration::from_secs(15),
            )
            .await;
    }
}

/// Wait until screenrecord is gone and the remote file has stopped growing.
/// Returns whether both were observed within the budget; the MP4 header, not
/// this, decides whether the recording is usable.
async fn wait_for_finalize(adb: &Adb, serial: &str, session: &Session) -> bool {
    let probe = format!(
        "if [ -d /proc/{0} ]; then echo alive; else echo gone; fi; wc -c < {1} 2>/dev/null || echo 0",
        session.pid, session.remote
    );
    let mut last_size = 0u64;
    for _ in 0..60 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(out) = adb
            .run(&["-s", serial, "shell", &probe], Duration::from_secs(10))
            .await
        else {
            return false;
        };
        let text = out.stdout;
        let gone = text.contains("gone");
        let size = text
            .lines()
            .filter_map(|line| line.trim().parse::<u64>().ok())
            .max()
            .unwrap_or(0);
        if gone && size > 0 && size == last_size {
            return true;
        }
        last_size = size;
    }
    false
}

/// A real MP4 starts with a `ftyp` box: the size of that box, then `ftyp`.
pub fn is_mp4(bytes: &[u8]) -> bool {
    bytes.len() > 12 && &bytes[4..8] == b"ftyp"
}

/// Whether an MP4 carries its index. A recording host that was killed instead of
/// closed leaves `ftyp` and frames but no `moov`, which no player will open.
pub fn has_moov(bytes: &[u8]) -> bool {
    bytes.windows(4).any(|window| window == b"moov")
}

/// The window host arguments for recording one app display.
///
/// Separated from the spawn so the policy is readable (and testable) without a
/// device: the recorder mirrors the named display, takes no input, plays no
/// sound, and cannot write into the Windows clipboard.
fn window_recording_args(
    serial: &str,
    display_id: u32,
    package_id: &str,
    path: &Path,
) -> Vec<String> {
    vec![
        format!("--serial={serial}"),
        format!("--display-id={display_id}"),
        format!("--record={}", path.display()),
        "--record-format=mp4".to_string(),
        "--no-audio".to_string(),
        "--no-control".to_string(),
        "--no-clipboard-autosync".to_string(),
        "--no-vd-system-decorations".to_string(),
        format!("--window-title=WINAPK recorder {package_id}"),
    ]
}

fn adb_for(runtime: &EmulatorRuntime) -> DwResult<Adb> {
    runtime.adb().ok_or_else(|| {
        DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK is not configured.",
        )
        .detail("Point WINAPK at an SDK with platform-tools before capturing.")
    })
}

fn create_directory(directory: &Path) -> DwResult<()> {
    std::fs::create_dir_all(directory).map_err(|e| {
        DwError::new(
            ErrorCode::StorageFailure,
            "WINAPK could not create the folder it saves captures in.",
        )
        .detail(format!("{}: {e}", directory.display()))
    })
}

fn storage_error(summary: &str, path: &Path, error: &std::io::Error) -> DwError {
    DwError::new(ErrorCode::StorageFailure, summary).detail(format!("{}: {error}", path.display()))
}

/// `20261028-183012-345`. A local-time stamp is what makes a folder of captures
/// sortable and identifiable without opening them.
#[cfg(windows)]
fn timestamp() -> String {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct SystemTime {
        year: u16,
        month: u16,
        day_of_week: u16,
        day: u16,
        hour: u16,
        minute: u16,
        second: u16,
        milliseconds: u16,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetLocalTime(time: *mut SystemTime);
    }

    let mut time = SystemTime::default();
    unsafe { GetLocalTime(&mut time) };
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}",
        time.year, time.month, time.day, time.hour, time.minute, time.second, time.milliseconds
    )
}

#[cfg(not(windows))]
fn timestamp() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{seconds}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_recording_targets_the_windows_own_display_only() {
        let args = window_recording_args(
            "emulator-5554",
            7,
            "com.example.game",
            Path::new("C:/Videos/WINAPK/out.mp4"),
        );
        assert!(args.contains(&"--serial=emulator-5554".to_string()));
        // The app's display, never display 0 (the phone) and never a guessed id.
        assert!(args.contains(&"--display-id=7".to_string()));
        assert!(args.iter().all(|arg| arg != "--display-id=0"));
        assert!(args.iter().any(|arg| arg.starts_with("--record=")));
        assert!(args.contains(&"--no-control".to_string()));
        assert!(args.contains(&"--no-audio".to_string()));
        assert!(args.contains(&"--no-clipboard-autosync".to_string()));
    }

    #[test]
    fn a_file_without_its_moov_atom_is_not_reported_as_playable() {
        let mut killed = vec![0u8; 32];
        killed[4..8].copy_from_slice(b"ftyp");
        assert!(is_mp4(&killed));
        assert!(!has_moov(&killed));
        let mut finished = killed.clone();
        finished.extend_from_slice(b"....moov....");
        assert!(has_moov(&finished));
    }

    #[test]
    fn a_file_android_wrote_is_recognised_by_its_mp4_header() {
        let mut mp4 = vec![0u8, 0, 0, 24];
        mp4.extend_from_slice(b"ftypisom");
        mp4.extend_from_slice(&[0u8; 32]);
        assert!(is_mp4(&mp4));

        let mut png = vec![0x89, b'P', b'N', b'G'];
        png.extend_from_slice(&[0u8; 32]);
        assert!(!is_mp4(&png));
        assert!(!is_mp4(b""));
        assert!(!is_mp4(b"ftyp"));
    }

    #[test]
    fn a_timestamp_is_a_sortable_file_name_fragment() {
        let stamp = timestamp();
        assert!(
            stamp.chars().all(|c| c.is_ascii_digit() || c == '-'),
            "unexpected stamp {stamp}"
        );
        assert!(stamp.len() >= 15, "unexpected stamp {stamp}");
    }
}
