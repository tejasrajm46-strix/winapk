//! One scrcpy window per running Android app. scrcpy creates a fresh virtual
//! Android display for that window and starts the package on it, which is what
//! makes the window belong to that app rather than to a mirrored phone screen.

use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command};

use crate::domain::{DwError, DwResult, ErrorCode};

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
}

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
            // Audio forwarding needs an extra capture path we have not verified;
            // claiming it in the UI without testing it would be dishonest.
            "--no-audio".to_string(),
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

    cmd.spawn().map_err(|e| {
        DwError::io(
            ErrorCode::LaunchFailed,
            format!("DroidWin could not open a window for {}.", req.package_id),
            &e,
        )
    })
}

pub fn version(exe: &Path) -> Option<String> {
    let output = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .next()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
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
        };
        let args = request.arguments();
        assert!(args.contains(&"--serial=emulator-5554".to_string()));
        assert!(args.contains(&"--new-display=540x1170/420".to_string()));
        assert!(args.contains(&"--start-app=org.example.app".to_string()));
        assert!(args.contains(&"--no-vd-system-decorations".to_string()));
        assert!(args.contains(&"--no-audio".to_string()));
        // Each argument stays a single argv entry even when the title has spaces.
        assert!(args
            .iter()
            .any(|a| a == "--window-title=DroidWin — Example"));
    }
}
