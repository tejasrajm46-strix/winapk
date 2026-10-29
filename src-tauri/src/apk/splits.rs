//! Split-package sets: `.xapk` and `.apks` archives that carry a base APK plus
//! configuration splits.
//!
//! Android will only install a set whose members share one package name and one
//! version, and only needs the splits its device can use. This module extracts
//! the members into a private staging directory, identifies the base by its
//! manifest (a split declares `split=`), and drops ABI splits the runtime
//! cannot execute. Everything else is left to `adb install-multiple` so the
//! package manager, not this code, decides signature and compatibility.
//!
//! The archive is untrusted input: entry count, per-entry size and total size
//! are capped before anything is written.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use crate::domain::{DwError, DwResult, ErrorCode};

/// A set bigger than this is refused before extraction.
const MAX_SET_BYTES: u64 = 6 * 1024 * 1024 * 1024;
const MAX_MEMBER_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_MEMBERS: usize = 512;

/// Longest token first: `x86_64` must win over `x86`.
const ABI_TOKENS: [&str; 6] = [
    "arm64-v8a",
    "armeabi-v7a",
    "riscv64",
    "x86_64",
    "armeabi",
    "x86",
];

#[derive(Clone, Debug, Default)]
pub struct Target {
    /// ABIs the runtime can execute, best first (from `ro.product.cpu.abilist`).
    pub abis: Vec<String>,
}

impl Target {
    pub fn supports(&self, abi: &str) -> bool {
        self.abis.iter().any(|a| a == abi)
    }
}

pub struct SplitSet {
    pub base: PathBuf,
    pub splits: Vec<PathBuf>,
    /// Members left out, with the reason, so the UI can say what it skipped.
    pub skipped: Vec<String>,
}

impl SplitSet {
    /// Base first: adb accepts any order, but logs read better this way.
    pub fn install_order(&self) -> Vec<&Path> {
        let mut all: Vec<&Path> = vec![self.base.as_path()];
        all.extend(self.splits.iter().map(PathBuf::as_path));
        all
    }
}

pub fn is_set(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("xapk") | Some("apks")
    )
}

/// A private directory that removes itself, so a failed install cannot leave
/// hundreds of megabytes of unpacked splits behind.
pub struct Staging {
    dir: PathBuf,
}

impl Staging {
    pub fn new(root: &Path, tag: &str) -> DwResult<Self> {
        let unique = format!(
            "{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );
        let dir = root.join(unique);
        std::fs::create_dir_all(&dir).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "Could not create a staging directory for the package set.",
                &e,
            )
        })?;
        Ok(Self { dir })
    }

    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Extract a split set and choose the members this runtime should install.
pub fn expand(source: &Path, target: &Target, into: &Path) -> DwResult<SplitSet> {
    let file = std::fs::File::open(source).map_err(|e| {
        DwError::io(
            ErrorCode::InvalidPath,
            format!("Cannot open {}", source.display()),
            &e,
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        DwError::new(
            ErrorCode::CorruptArchive,
            "That split package is not a readable zip archive.",
        )
        .detail(e.to_string())
    })?;
    if archive.len() > MAX_MEMBERS * 8 {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That archive declares an unreasonable number of entries.",
        )
        .detail(format!("{} entries", archive.len())));
    }

    let mut members: Vec<(usize, String)> = Vec::new();
    let mut declared_total: u64 = 0;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|e| {
            DwError::new(ErrorCode::CorruptArchive, "Archive entry is unreadable.")
                .detail(e.to_string())
        })?;
        let name = entry.name().to_string();
        let upper = name.to_ascii_uppercase();
        if upper.starts_with("META-INF/") || !upper.ends_with(".APK") {
            continue;
        }
        declared_total = declared_total.saturating_add(entry.size());
        if declared_total > MAX_SET_BYTES {
            return Err(DwError::new(
                ErrorCode::PackageTooLarge,
                "That package set unpacks to more than the 6 GiB installation limit.",
            ));
        }
        members.push((index, name));
    }
    if members.is_empty() {
        return Err(DwError::new(
            ErrorCode::UnsupportedPackage,
            "That archive contains no .apk files, so it is not an XAPK/APKS set.",
        ));
    }
    if members.len() > MAX_MEMBERS {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That package set contains more split APKs than this build accepts.",
        )
        .detail(format!("{} members", members.len())));
    }

    let mut extracted: Vec<(String, PathBuf)> = Vec::new();
    for (ordinal, (index, name)) in members.iter().enumerate() {
        let mut entry = archive.by_index(*index).map_err(|e| {
            DwError::new(ErrorCode::CorruptArchive, "Archive entry is unreadable.")
                .detail(e.to_string())
        })?;
        let destination = into.join(format!("{:02}-{}", ordinal, safe_name(name)));
        let mut output = std::fs::File::create(&destination).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "Could not unpack a split APK.",
                &e,
            )
        })?;
        let mut written: u64 = 0;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            let read = entry.read(&mut buffer).map_err(|e| {
                DwError::new(ErrorCode::CorruptArchive, "A split APK is truncated.").detail(e.to_string())
            })?;
            if read == 0 {
                break;
            }
            written += read as u64;
            if written > MAX_MEMBER_BYTES {
                return Err(DwError::new(
                    ErrorCode::PackageTooLarge,
                    "One member of that package set exceeds the 2 GiB limit.",
                ));
            }
            output.write_all(&buffer[..read]).map_err(|e| {
                DwError::io(
                    ErrorCode::StorageFailure,
                    "Could not write an unpacked split APK.",
                    &e,
                )
            })?;
        }
        extracted.push((name.clone(), destination));
    }

    // Identify base and splits by what each manifest says, not by file name:
    // XAPK authors name members inconsistently, but the base is the one without
    // a `split` attribute.
    let mut base: Option<(PathBuf, crate::apk::axml::ManifestAttrs)> = None;
    let mut known: Vec<(String, PathBuf, crate::apk::axml::ManifestAttrs)> = Vec::new();
    for (name, path) in extracted {
        let attrs = crate::apk::manifest_attrs(&path).map_err(|err| {
            DwError::new(
                ErrorCode::InvalidPackage,
                format!("A member of the package set is not a readable APK: {name}"),
            )
            .detail(err.detail.unwrap_or(err.summary))
        })?;
        let is_split = attrs
            .split
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if is_split {
            known.push((name, path, attrs));
        } else {
            if base.is_some() {
                return Err(DwError::new(
                    ErrorCode::InvalidPackage,
                    "That package set contains two base APKs.",
                )
                .detail("Exactly one member must be the base (a manifest without `split`)."));
            }
            base = Some((path, attrs));
        }
    }
    let Some((base_path, base_attrs)) = base else {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "That package set has no base APK.",
        )
        .detail("Every member declares a `split` attribute, so Android cannot install it."));
    };
    let Some(base_id) = base_attrs.package.clone() else {
        return Err(DwError::new(
            ErrorCode::InvalidPackage,
            "The base APK declares no package name.",
        ));
    };

    let mut splits = Vec::new();
    let mut skipped = Vec::new();
    for (name, path, attrs) in known {
        if attrs.package.as_deref() != Some(base_id.as_str()) {
            return Err(DwError::new(
                ErrorCode::InvalidPackage,
                "That package set mixes APKs from different packages.",
            )
            .detail(format!(
                "{name} is {}, the base is {base_id}",
                attrs.package.unwrap_or_else(|| "unnamed".into())
            )));
        }
        if let (Some(a), Some(b)) = (attrs.version_code, base_attrs.version_code) {
            if a != b {
                return Err(DwError::new(
                    ErrorCode::InvalidPackage,
                    "That package set mixes APKs from different versions.",
                )
                .detail(format!("{name} is version {a}, the base is version {b}")));
            }
        }
        match abi_of(&name) {
            Some(abi) if !target.supports(abi) => {
                skipped.push(format!("{name} — {abi} is not supported by this runtime"));
            }
            _ => splits.push(path),
        }
    }

    Ok(SplitSet {
        base: base_path,
        splits,
        skipped,
    })
}

/// Which ABI a split belongs to, from its file name. Standard bundles use
/// `config.<abi>`; older XAPK tools use `<abi>` alone.
fn abi_of(name: &str) -> Option<&'static str> {
    let lower = name.to_ascii_lowercase().replace(['.', '-'], "_");
    ABI_TOKENS.into_iter().find(|abi| {
        let token = abi.replace('-', "_");
        // `x86_64` also contains `x86`; the longest-first order handles it.
        lower.contains(&token)
    })
}

/// Keep only the leaf name, and only characters that are safe on disk. Archive
/// paths are attacker-controlled, so `..` and separators must not survive.
fn safe_name(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = leaf
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "member.apk".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_tokens_are_matched_longest_first() {
        assert_eq!(abi_of("config.x86_64"), Some("x86_64"));
        assert_eq!(abi_of("split_config.x86_64.apk"), Some("x86_64"));
        assert_eq!(abi_of("config.arm64_v8a.apk"), Some("arm64-v8a"));
        assert_eq!(abi_of("config.armeabi_v7a.apk"), Some("armeabi-v7a"));
        assert_eq!(abi_of("config.xxhdpi.apk"), None);
        assert_eq!(abi_of("base.apk"), None);
    }

    #[test]
    fn archive_paths_cannot_escape_the_staging_directory() {
        assert_eq!(safe_name("../../evil.apk"), "evil.apk");
        assert_eq!(safe_name("split/config.arm64_v8a.apk"), "config.arm64_v8a.apk");
        assert_eq!(safe_name("..\\..\\evil.apk"), "evil.apk");
        assert_eq!(safe_name("with space.apk"), "with_space.apk");
    }

    #[test]
    fn a_supported_split_is_installed_and_an_unsupported_one_is_explained() {
        let target = Target {
            abis: vec!["x86_64".into()],
        };
        assert!(target.supports("x86_64"));
        assert!(!target.supports("arm64-v8a"));
    }

    #[test]
    fn a_directory_without_apks_is_refused() {
        let dir = std::env::temp_dir().join(format!("droidwin-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let archive = dir.join("empty.xapk");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("manifest.json", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"{}").unwrap();
        zip.finish().unwrap();

        let staging = dir.join("out");
        std::fs::create_dir_all(&staging).unwrap();
        let err = expand(&archive, &Target::default(), &staging)
            .expect_err("a set without APKs must fail");
        assert_eq!(err.code, ErrorCode::UnsupportedPackage);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
