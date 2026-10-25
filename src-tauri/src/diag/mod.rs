//! Diagnostics: one log file under the app data directory, plus redaction for
//! anything the UI shows or the user exports.

use std::path::{Path, PathBuf};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Install the global subscriber. The returned guard must stay alive for the
/// process lifetime or buffered log lines are lost.
pub fn init(log_dir: &Path) -> Option<WorkerGuard> {
    std::fs::create_dir_all(log_dir).ok()?;

    let file_appender = tracing_appender::rolling::daily(log_dir, "droidwin.log");
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,droidwin=debug"));

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_ansi(false).with_writer(file_writer))
        .with(fmt::layer().with_ansi(true).with_writer(std::io::stderr));

    let _ = registry.try_init();
    Some(guard)
}

pub fn log_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("logs")
}

/// Replace the user's home directory with `~` so diagnostics stay useful without
/// publishing an account name.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    if let Some(home) = dirs::home_dir() {
        let home = home.to_string_lossy().to_string();
        if !home.is_empty() {
            out = out.replace(&home, "~");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn redaction_replaces_home_prefix() {
        let Some(home) = dirs::home_dir() else { return };
        let home = home.to_string_lossy().to_string();
        let text = format!("{home}\\AppData\\file.apk");
        let redacted = super::redact(&text);
        assert!(!redacted.starts_with(&home));
        assert!(redacted.starts_with('~'));
    }
}
