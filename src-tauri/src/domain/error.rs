//! Typed errors. Every boundary returns these instead of strings, so the UI can
//! render a stable code plus a human summary and keep the detail for diagnostics.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidPath,
    InvalidPackage,
    PackageTooLarge,
    CorruptArchive,
    MissingManifest,
    UnsupportedPackage,
    RuntimeNotConfigured,
    RuntimeUnavailable,
    RuntimeStartFailed,
    InstallFailed,
    UninstallFailed,
    LaunchFailed,
    NotInstalled,
    Timeout,
    Cancelled,
    BackendFailure,
    StorageFailure,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorCode::InvalidPath => "invalid_path",
            ErrorCode::InvalidPackage => "invalid_package",
            ErrorCode::PackageTooLarge => "package_too_large",
            ErrorCode::CorruptArchive => "corrupt_archive",
            ErrorCode::MissingManifest => "missing_manifest",
            ErrorCode::UnsupportedPackage => "unsupported_package",
            ErrorCode::RuntimeNotConfigured => "runtime_not_configured",
            ErrorCode::RuntimeUnavailable => "runtime_unavailable",
            ErrorCode::RuntimeStartFailed => "runtime_start_failed",
            ErrorCode::InstallFailed => "install_failed",
            ErrorCode::UninstallFailed => "uninstall_failed",
            ErrorCode::LaunchFailed => "launch_failed",
            ErrorCode::NotInstalled => "not_installed",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Cancelled => "cancelled",
            ErrorCode::BackendFailure => "backend_failure",
            ErrorCode::StorageFailure => "storage_failure",
        }
    }
}

#[derive(Debug, Clone, Serialize, thiserror::Error)]
#[error("{summary}")]
pub struct DwError {
    pub code: ErrorCode,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl DwError {
    pub fn new(code: ErrorCode, summary: impl Into<String>) -> Self {
        Self {
            code,
            summary: summary.into(),
            detail: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn io(code: ErrorCode, summary: impl Into<String>, err: &std::io::Error) -> Self {
        Self::new(code, summary).detail(format!("{err}"))
    }
}

pub type DwResult<T> = Result<T, DwError>;
