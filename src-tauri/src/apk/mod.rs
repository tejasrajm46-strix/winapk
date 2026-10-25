//! APK inspection. Reads the archive structure and compiled manifest only —
//! package code is never executed, and unavailable values are reported as
//! unavailable instead of guessed.
//!
//! The archive is untrusted input, so the guards here matter: size caps,
//! entry-count cap, declared-size ratio cap, and bounds-checked parsing.

pub mod axml;

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use base64::Engine as _;

use crate::domain::{ApkInspection, DwError, DwResult, ErrorCode, SignatureInfo};

const MAX_APK_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_MANIFEST_BYTES: u64 = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 200_000;
const MAX_ICON_BYTES: u64 = 512 * 1024;
const SIGNING_BLOCK_MAGIC: &[u8; 16] = b"APK Sig Block 42";

/// Cheap checks before any archive work: path, extension, sane size.
/// Returns the file size in bytes.
pub fn validate(path: &Path) -> DwResult<u64> {
    let meta = std::fs::metadata(path).map_err(|e| {
        DwError::io(
            ErrorCode::InvalidPath,
            format!("Cannot read {}", display(path)),
            &e,
        )
    })?;
    if !meta.is_file() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "That path is not a file.",
        ));
    }
    if !path
        .extension()
        .map(|e| e.eq_ignore_ascii_case("apk"))
        .unwrap_or(false)
    {
        return Err(DwError::new(
            ErrorCode::UnsupportedPackage,
            "DroidWin installs .apk packages.",
        )
        .detail("Expected a file whose extension is .apk"));
    }
    if meta.len() == 0 {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That file is empty.",
        ));
    }
    if meta.len() > MAX_APK_BYTES {
        return Err(DwError::new(
            ErrorCode::PackageTooLarge,
            "That package is larger than the 4 GiB inspection limit.",
        ));
    }
    Ok(meta.len())
}

/// Everything the installer needs, plus the reason for anything missing.
pub fn inspect(path: &Path, aapt2: Option<&Path>) -> DwResult<ApkInspection> {
    let size = validate(path)?;

    let file = std::fs::File::open(path).map_err(|e| {
        DwError::io(
            ErrorCode::InvalidPath,
            format!("Cannot open {}", display(path)),
            &e,
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        DwError::new(
            ErrorCode::CorruptArchive,
            "That file is not a valid APK archive (zip structure is unreadable).",
        )
        .detail(e.to_string())
    })?;

    if archive.len() > MAX_ENTRIES {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That archive declares an unreasonable number of entries.",
        )
        .detail(format!("{} entries", archive.len())));
    }

    let mut abis: Vec<String> = Vec::new();
    let mut manifest_entry = None;
    let mut v1_signature = false;
    for i in 0..archive.len() {
        let entry = archive.by_index(i).map_err(|e| {
            DwError::new(ErrorCode::CorruptArchive, "Archive entry is unreadable.")
                .detail(e.to_string())
        })?;
        let name = entry.name().to_string();
        if name == "AndroidManifest.xml" {
            manifest_entry = Some(i);
        }
        if let Some(rest) = name.strip_prefix("lib/") {
            if let Some(abi) = rest.split('/').next() {
                if !abi.is_empty() && !abis.iter().any(|a| a == abi) {
                    abis.push(abi.to_string());
                }
            }
        }
        let upper = name.to_ascii_uppercase();
        if upper.starts_with("META-INF/")
            && (upper.ends_with(".RSA") || upper.ends_with(".DSA") || upper.ends_with(".EC"))
        {
            v1_signature = true;
        }
    }
    let abis = normalize_abis(abis);

    let Some(index) = manifest_entry else {
        return Err(DwError::new(
            ErrorCode::MissingManifest,
            "That archive has no AndroidManifest.xml, so it is not an installable Android package.",
        ));
    };

    let entry = archive.by_index(index).map_err(|e| {
        DwError::new(ErrorCode::CorruptArchive, "Manifest entry is unreadable.")
            .detail(e.to_string())
    })?;
    if entry.size() > MAX_MANIFEST_BYTES {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That package's manifest is implausibly large.",
        )
        .detail(format!("{} bytes declared", entry.size())));
    }
    let mut manifest_bytes = Vec::with_capacity(entry.size().min(MAX_MANIFEST_BYTES) as usize);
    entry
        .take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut manifest_bytes)
        .map_err(|e| {
            DwError::new(ErrorCode::CorruptArchive, "Manifest entry is truncated.")
                .detail(e.to_string())
        })?;
    if manifest_bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That package's manifest exceeds the inspection limit.",
        ));
    }

    let attrs = axml::parse_manifest(&manifest_bytes).map_err(|e| {
        DwError::new(
            ErrorCode::InvalidPackage,
            "The manifest could not be read, so this package cannot be inspected safely.",
        )
        .detail(e)
    })?;

    let Some(package_id) = attrs.package.clone() else {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "The manifest declares no package name.",
        ));
    };

    let mut unavailable = Vec::new();
    let mut warnings = Vec::new();

    let signature = SignatureInfo {
        v1: v1_signature,
        v2_plus: detect_signing_block(path),
    };

    let mut label = attrs.label.clone();
    let mut icon = None;

    if let Some(tool) = aapt2 {
        match enrich_with_aapt2(tool, path) {
            Ok(badging) => {
                if label.is_none() {
                    label = badging.label;
                }
                if let Some(icon_entry) = badging.icon_entry {
                    icon = read_icon(path, &icon_entry);
                }
            }
            Err(err) => {
                tracing::debug!("aapt2 enrichment unavailable: {err}");
            }
        }
    }

    if label.is_none() {
        unavailable.push(if attrs.label_is_reference {
            "App name — declared as a resource reference; needs the SDK's aapt2 to resolve."
                .to_string()
        } else {
            "App name — the manifest declares none.".to_string()
        });
    }
    if icon.is_none() {
        unavailable.push("App icon — not resolvable from the manifest alone.".to_string());
    }
    unavailable.push(
        "Signature validity — DroidWin reports that a signature block exists, not that it is trusted."
            .to_string(),
    );

    if !signature.any() {
        warnings.push(
            "No signature block was found. The runtime will normally reject an unsigned package."
                .to_string(),
        );
    }
    if abis.is_empty() {
        warnings.push(
            "No native libraries are bundled (pure Java/Kotlin, or libraries loaded at runtime)."
                .to_string(),
        );
    }
    if attrs.min_sdk.is_none() {
        unavailable.push("Minimum SDK — not declared in the manifest.".to_string());
    }

    Ok(ApkInspection {
        path: display(path),
        file_name: path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| display(path)),
        package_id,
        label,
        version_name: attrs.version_name,
        version_code: attrs.version_code,
        min_sdk: attrs.min_sdk,
        target_sdk: attrs.target_sdk,
        abis,
        size_bytes: size,
        icon,
        signature,
        warnings,
        unavailable,
    })
}

fn normalize_abis(mut abis: Vec<String>) -> Vec<String> {
    const ORDER: [&str; 4] = ["arm64-v8a", "armeabi-v7a", "x86_64", "x86"];
    abis.sort_by_key(|a| ORDER.iter().position(|o| o == a).unwrap_or(ORDER.len()));
    abis
}

fn display(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// True when an APK Signing Block sits between the entries and the central directory.
fn detect_signing_block(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(len) = file.metadata().map(|m| m.len()) else {
        return false;
    };
    if len < 32 {
        return false;
    }
    let tail_len = len.min(66_000) as usize;
    let mut tail = vec![0u8; tail_len];
    if file.seek(SeekFrom::End(-(tail_len as i64))).is_err() || file.read_exact(&mut tail).is_err()
    {
        return false;
    }
    let Some(eocd_in_tail) = tail.windows(4).rposition(|w| w == b"PK\x05\x06") else {
        return false;
    };
    let eocd = eocd_in_tail + (len as usize - tail_len);
    if eocd + 20 > len as usize {
        return false;
    }
    let rd = |offset: usize| -> u32 {
        let i = eocd + offset - (len as usize - tail_len);
        u32::from_le_bytes([tail[i], tail[i + 1], tail[i + 2], tail[i + 3]])
    };
    let cd_offset = rd(16) as u64;
    if cd_offset < 32 || cd_offset > len {
        return false;
    }
    let magic_at = cd_offset - 16;
    let mut magic = [0u8; 16];
    if file.seek(SeekFrom::Start(magic_at)).is_err() || file.read_exact(&mut magic).is_err() {
        return false;
    }
    &magic == SIGNING_BLOCK_MAGIC
}

struct Badging {
    label: Option<String>,
    icon_entry: Option<String>,
}

/// The SDK's aapt2 resolves resource references (label, icon) that the manifest
/// alone cannot. Entirely optional: without it those fields stay unavailable.
fn enrich_with_aapt2(aapt2: &Path, apk: &Path) -> DwResult<Badging> {
    let output = std::process::Command::new(aapt2)
        .arg("dump")
        .arg("badging")
        .arg(apk)
        .output()
        .map_err(|e| DwError::io(ErrorCode::BackendFailure, "aapt2 could not be started.", &e))?;
    if !output.status.success() {
        return Err(
            DwError::new(ErrorCode::BackendFailure, "aapt2 rejected the package.")
                .detail(String::from_utf8_lossy(&output.stderr).to_string()),
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let mut label = None;
    let mut best_icon: Option<(u32, String)> = None;

    for line in text.lines() {
        if label.is_none() {
            if let Some(rest) = line.strip_prefix("application-label:") {
                label = unquote(rest);
            }
        }
        if let Some(rest) = line.strip_prefix("application-icon-") {
            if let Some((dpi, path)) = rest.split_once(':') {
                if let (Ok(dpi), Some(path)) = (dpi.trim().parse::<u32>(), unquote(path)) {
                    if best_icon
                        .as_ref()
                        .map(|(best, _)| dpi > *best)
                        .unwrap_or(true)
                    {
                        best_icon = Some((dpi, path));
                    }
                }
            }
        }
    }

    Ok(Badging {
        label,
        icon_entry: best_icon.map(|(_, p)| p),
    })
}

fn unquote(s: &str) -> Option<String> {
    let s = s.trim();
    let s = s
        .strip_prefix('\'')
        .and_then(|v| v.strip_suffix('\''))
        .unwrap_or(s);
    if s.is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

fn read_icon(apk: &Path, entry_name: &str) -> Option<String> {
    let file = std::fs::File::open(apk).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(entry_name).ok()?;
    if entry.size() == 0 || entry.size() > MAX_ICON_BYTES {
        return None;
    }
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).ok()?;
    if !bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        // Only PNG is embedded directly: WebView renders it without extra decoding work.
        return None;
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Some(format!("data:image/png;base64,{encoded}"))
}

/// Locate the newest `aapt2` shipped in an SDK build-tools directory.
pub fn find_aapt2(build_tools_root: &Path) -> Option<PathBuf> {
    let mut versions: Vec<PathBuf> = std::fs::read_dir(build_tools_root)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.join("aapt2.exe").is_file())
        .collect();
    versions.sort();
    versions.pop().map(|p| p.join("aapt2.exe"))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::{inspect, validate};
    use crate::domain::ErrorCode;

    /// Unique directory per test, removed even when an assertion fails.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let dir = std::env::temp_dir().join(format!(
                "droidwin-apk-{}-{}-{label}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    /// Minimal stored-entry ZIP. Hand-built so the tests do not depend on the
    /// zip crate's writer surface, and so a deliberately broken archive can be
    /// produced by writing the same bytes minus the manifest.
    fn write_zip(path: &std::path::Path, entries: &[(&str, &[u8])]) {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in entries {
            let offset = out.len() as u32;
            let crc = crc32(data);

            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes()); // stored
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes()); // stored
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }

        let cd_offset = out.len() as u32;
        let cd_size = central.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&cd_size.to_le_bytes());
        out.extend_from_slice(&cd_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        std::fs::write(path, out).expect("write fixture zip");
    }

    #[test]
    fn a_missing_path_is_an_invalid_path() {
        let temp = TempDir::new("missing");
        let err = validate(&temp.file("absent.apk")).expect_err("absent file must fail");
        assert_eq!(err.code, ErrorCode::InvalidPath);
    }

    #[test]
    fn a_directory_named_like_a_package_is_not_a_package() {
        let temp = TempDir::new("dir");
        let dir = temp.file("folder.apk");
        std::fs::create_dir_all(&dir).expect("create directory fixture");
        let err = validate(&dir).expect_err("directory must fail");
        assert_eq!(err.code, ErrorCode::InvalidPath);
    }

    #[test]
    fn a_file_with_another_extension_is_refused() {
        let temp = TempDir::new("ext");
        let path = temp.file("notes.txt");
        std::fs::write(&path, b"not a package").expect("write fixture");
        let err = validate(&path).expect_err("text file must fail");
        assert_eq!(err.code, ErrorCode::UnsupportedPackage);
    }

    #[test]
    fn an_empty_package_is_refused_before_parsing() {
        let temp = TempDir::new("empty");
        let path = temp.file("empty.apk");
        std::fs::write(&path, b"").expect("write fixture");
        let err = validate(&path).expect_err("empty file must fail");
        assert_eq!(err.code, ErrorCode::InvalidPackage);
    }

    #[test]
    fn bytes_that_are_not_a_zip_are_a_corrupt_archive() {
        let temp = TempDir::new("garbage");
        let path = temp.file("garbage.apk");
        std::fs::write(&path, b"this is plainly not a zip container").expect("write fixture");
        let err = inspect(&path, None).expect_err("garbage must fail");
        assert_eq!(err.code, ErrorCode::CorruptArchive);
    }

    #[test]
    fn a_zip_without_a_manifest_is_not_an_android_package() {
        let temp = TempDir::new("nomanifest");
        let path = temp.file("plain.apk");
        write_zip(&path, &[("classes.dex", b"dex".as_slice())]);
        let err = inspect(&path, None).expect_err("zip without manifest must fail");
        assert_eq!(err.code, ErrorCode::MissingManifest);
    }

    #[test]
    fn an_unreadable_manifest_is_rejected_without_guessing() {
        let temp = TempDir::new("badmanifest");
        let path = temp.file("bad-manifest.apk");
        write_zip(
            &path,
            &[("AndroidManifest.xml", [0x03u8, 0x00, 0x08, 0x00].as_slice())],
        );
        let err = inspect(&path, None).expect_err("garbage manifest must fail");
        assert_eq!(err.code, ErrorCode::InvalidPackage);
        assert!(
            err.detail.is_some(),
            "the parser reason is kept for diagnosis"
        );
    }

    #[test]
    fn a_unicode_path_with_spaces_is_accepted() {
        let temp = TempDir::new("unicode");
        let path = temp.file("音楽 アプリ.apk");
        write_zip(&path, &[("classes.dex", b"dex".as_slice())]);
        let size = validate(&path).expect("unicode path must validate");
        let on_disk = std::fs::metadata(&path).expect("fixture metadata").len();
        assert_eq!(size, on_disk);
    }
}
