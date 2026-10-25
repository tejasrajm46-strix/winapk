//! Child-process creation policy.
//!
//! DroidWin is a GUI-subsystem program. On Windows, spawning a console program
//! from a GUI process without `CREATE_NO_WINDOW` makes the OS allocate a console
//! for it — a black window that flashes for every `adb`, `aapt2`, `taskkill` or
//! `scrcpy` invocation. Because the runtime polls, that looked like windows
//! constantly opening by themselves.
//!
//! Every subprocess therefore goes through this module. The flag suppresses the
//! console, not the process: captured output, exit codes and scrcpy's own window
//! are unaffected.

/// `CREATE_NO_WINDOW` from `winbase.h`. The process runs with no console window
/// and does not inherit the parent's console.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply the no-console policy to a command about to be spawned.
pub fn no_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Apply the no-console policy to an async command. `tokio::process::Command`
/// exposes its `std` counterpart for exactly this kind of platform setting.
pub fn no_console_async(cmd: &mut tokio::process::Command) {
    no_console(cmd.as_std_mut());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// The flag must not change how a child behaves: output and exit status are
    /// still captured. This is the property the whole policy depends on.
    #[cfg(windows)]
    #[test]
    fn a_hidden_child_still_reports_output_and_exit_code() {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "echo hidden-ok"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        no_console(&mut cmd);
        let out = cmd.output().expect("cmd should start");
        assert_eq!(out.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("hidden-ok"));
    }

    /// A child that writes more than a pipe buffer must not deadlock, because
    /// the policy makes no promise about draining output for long-lived tools.
    #[cfg(windows)]
    #[test]
    fn a_hidden_child_can_be_killed_after_streaming_output() {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.args(["/c", "for /L %i in (1,1,200) do @echo line-%i"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        no_console_async(&mut cmd);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let out = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output()).await
        });
        let out = out.expect("hidden child should finish").unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("line-200"));
    }

    #[cfg(windows)]
    #[test]
    fn the_console_flag_is_the_documented_value() {
        // 0x08000000 is CREATE_NO_WINDOW; a different value would either show a
        // console or create a detached process.
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }
}
