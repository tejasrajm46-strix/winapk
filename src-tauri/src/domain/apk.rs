//! Package inspection result. Optional fields are genuinely optional: when we
//! cannot determine a value we leave it `None` and say why in `unavailable`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SignatureInfo {
    /// A v1 (JAR) signature block was present.
    pub v1: bool,
    /// An APK Signing Block (v2/v3/v4) was present.
    pub v2_plus: bool,
}

impl SignatureInfo {
    pub fn any(&self) -> bool {
        self.v1 || self.v2_plus
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApkInspection {
    pub path: String,
    pub file_name: String,
    pub package_id: String,
    pub label: Option<String>,
    pub version_name: Option<String>,
    pub version_code: Option<u64>,
    pub min_sdk: Option<u32>,
    pub target_sdk: Option<u32>,
    /// CPU ABIs that ship native libraries in this archive.
    pub abis: Vec<String>,
    pub size_bytes: u64,
    /// PNG data URL extracted from the archive, when the SDK's aapt2 could name it.
    pub icon: Option<String>,
    pub signature: SignatureInfo,
    /// Conditions the user should know about. Never blocking on their own.
    pub warnings: Vec<String>,
    /// Fields we could not determine, each with the reason. Rendered verbatim.
    pub unavailable: Vec<String>,
}
