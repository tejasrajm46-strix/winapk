//! One scrcpy window per running Android app. scrcpy creates a fresh virtual
//! Android display for that window and starts the package on it, which is what
//! makes the window belong to that app rather than to a mirrored phone screen.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};

use crate::domain::{DwError, DwResult, ErrorCode};
use crate::proc;

/// How scrcpy delivers keystrokes.
///
/// `sdk` injects Android key events through the system API: modifiers travel as
/// a meta state on the character key, so apps that look for a Shift key-down (a
/// game, a remote desktop, anything reading raw key events) never see one.
/// `uhid` presents a real HID keyboard to Android, so Shift is its own key-down
/// and the system routes it to whichever display has focus. That is what this
/// product needs, so it is the default, with `sdk` kept as a fallback and as a
/// user switch for the apps that dislike HID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardMode {
    /// uhid when the runtime is Android 9+, sdk otherwise.
    Auto,
    Uhid,
    Sdk,
}

impl KeyboardMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(KeyboardMode::Auto),
            "uhid" => Some(KeyboardMode::Uhid),
            "sdk" => Some(KeyboardMode::Sdk),
            _ => None,
        }
    }

    /// Resolve `Auto` against the runtime's actual API level.
    pub fn resolve(self, api: Option<u32>) -> Self {
        match self {
            KeyboardMode::Auto => match api {
                Some(api) if api >= 28 => KeyboardMode::Uhid,
                _ => KeyboardMode::Sdk,
            },
            other => other,
        }
    }

    /// The value handed to scrcpy. `Auto` has already been resolved by then.
    pub fn as_arg(self) -> &'static str {
        match self {
            KeyboardMode::Uhid | KeyboardMode::Auto => "uhid",
            KeyboardMode::Sdk => "sdk",
        }
    }

    /// The value stored in settings and shown in the UI.
    pub fn as_str(self) -> &'static str {
        match self {
            KeyboardMode::Auto => "auto",
            KeyboardMode::Uhid => "uhid",
            KeyboardMode::Sdk => "sdk",
        }
    }

    pub fn is_hid(self) -> bool {
        self == KeyboardMode::Uhid
    }
}

#[derive(Debug, Clone)]
pub struct WindowRequest {
    pub serial: String,
    pub package_id: String,
    pub title: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Virtual display geometry on the Android side.
    pub display_width: u32,
    pub display_height: u32,
    pub dpi: u32,
    pub keyboard: KeyboardMode,
}

/// scrcpy's own audio forwarding, kept here for reference rather than used.
/// Measurements in docs/RELEASE_VERIFICATION.md recorded 6–19 dropped audio
/// chunks per run even with no media playing, and a larger buffer did not fix
/// it. Audio now comes from the emulator itself (one output for the runtime),
/// which also avoids N windows playing N copies of the same sound.
pub const AUDIO_ARGUMENTS: [&str; 2] = ["--audio-source=playback", "--audio-buffer=100"];

impl WindowRequest {
    fn arguments(&self) -> Vec<String> {
        vec![
            format!("--serial={}", self.serial),
            format!(
                "--new-display={}x{}/{}",
                self.display_width, self.display_height, self.dpi
            ),
            format!("--start-app={}", self.package_id),
            "--no-vd-system-decorations".to_string(),
            format!("--window-title={}", self.title),
            format!("--window-x={}", self.x),
            format!("--window-y={}", self.y),
            format!("--window-width={}", self.width),
            format!("--window-height={}", self.height),
            // The window is a picture of one app, not the sound of the device:
            // Android's audio is played once by the emulator. Forwarding it in
            // every window would multiply it by the number of open apps. See
            // AUDIO_ARGUMENTS for why scrcpy's forwarding is not used either.
            "--no-audio".to_string(),
            // scrcpy's default autosync copies the *device* clipboard to the
            // computer's whenever the guest changes it — a one-way path from an
            // untrusted app into the user's Windows clipboard. DroidWin reports
            // clipboard support as unverified, so it does not open that path.
            "--no-clipboard-autosync".to_string(),
            format!("--keyboard={}", self.keyboard.as_arg()),
        ]
    }
}

pub fn spawn(exe: &Path, req: &WindowRequest) -> DwResult<Child> {
    if !exe.is_file() {
        return Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The scrcpy window host is missing from this installation.",
        )
        .detail(format!("Expected {}", exe.display())));
    }

    let mut cmd = Command::new(exe);
    cmd.args(req.arguments())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(false);
    // No console for the window host. Its SDL window is unaffected: the flag
    // suppresses the console, not the app window.
    proc::no_console_async(&mut cmd);

    cmd.spawn().map_err(|e| {
        DwError::io(
            ErrorCode::LaunchFailed,
            format!("DroidWin could not open a window for {}.", req.package_id),
            &e,
        )
    })
}

/// Watch the first moments of a window host. A backend it cannot use makes it
/// exit immediately with the reason on stderr; that is worth catching before the
/// window is registered, so a fallback can be tried instead of reporting a dead
/// window. Returns the exit code and captured stderr when it failed.
pub async fn early_failure(child: &mut Child, window: Duration) -> Option<String> {
    let deadline = Instant::now() + window;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut text = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = tokio::time::timeout(
                        Duration::from_secs(1),
                        stderr.read_to_string(&mut text),
                    )
                    .await;
                }
                let text = text.trim();
                return Some(if text.is_empty() {
                    format!("the window host exited ({status})")
                } else {
                    format!("the window host exited ({status}): {text}")
                });
            }
            Ok(None) => {}
            Err(err) => return Some(format!("the window host could not be observed: {err}")),
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Keep draining stderr for the lifetime of a healthy window. Without a reader
/// the pipe eventually fills and blocks the window host.
pub fn drain_stderr(child: &mut Child) {
    let Some(mut stderr) = child.stderr.take() else {
        return;
    };
    tokio::spawn(async move {
        let mut buffer = [0u8; 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => break,
                Ok(read) => tracing::debug!(
                    "window host: {}",
                    String::from_utf8_lossy(&buffer[..read]).trim()
                ),
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_arguments_target_one_package_on_its_own_display() {
        let request = WindowRequest {
            serial: "emulator-5554".into(),
            package_id: "org.example.app".into(),
            title: "DroidWin — Example".into(),
            x: 40,
            y: 40,
            width: 420,
            height: 900,
            display_width: 540,
            display_height: 1170,
            dpi: 420,
            keyboard: KeyboardMode::Uhid,
        };
        let args = request.arguments();
        assert!(args.contains(&"--serial=emulator-5554".to_string()));
        assert!(args.contains(&"--new-display=540x1170/420".to_string()));
        assert!(args.contains(&"--start-app=org.example.app".to_string()));
        assert!(args.contains(&"--no-vd-system-decorations".to_string()));
        assert!(args.contains(&"--no-audio".to_string()));
        // The guest must not be able to write into the Windows clipboard.
        assert!(args.contains(&"--no-clipboard-autosync".to_string()));
        assert!(args.contains(&"--keyboard=uhid".to_string()));
        // Each argument stays a single argv entry even when the title has spaces.
        assert!(args
            .iter()
            .any(|a| a == "--window-title=DroidWin — Example"));
    }

    #[test]
    fn auto_keyboard_becomes_uhid_on_modern_android() {
        assert_eq!(KeyboardMode::Auto.resolve(Some(36)), KeyboardMode::Uhid);
        assert_eq!(KeyboardMode::Auto.resolve(Some(28)), KeyboardMode::Uhid);
        assert_eq!(KeyboardMode::Auto.resolve(Some(27)), KeyboardMode::Sdk);
        // No API observed: the conservative choice is the one that always works.
        assert_eq!(KeyboardMode::Auto.resolve(None), KeyboardMode::Sdk);
    }

    #[test]
    fn explicit_keyboard_modes_are_not_overridden() {
        assert_eq!(KeyboardMode::Sdk.resolve(Some(36)), KeyboardMode::Sdk);
        assert_eq!(KeyboardMode::Uhid.resolve(Some(19)), KeyboardMode::Uhid);
    }

    #[test]
    fn keyboard_mode_parsing_is_case_insensitive() {
        assert_eq!(KeyboardMode::parse("UHID"), Some(KeyboardMode::Uhid));
        assert_eq!(KeyboardMode::parse("auto"), Some(KeyboardMode::Auto));
        assert_eq!(KeyboardMode::parse("usb"), None);
        assert_eq!(KeyboardMode::Sdk.as_arg(), "sdk");
    }
}
