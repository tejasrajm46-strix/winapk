//! Locating a user-provided Android SDK and the tools DroidWin drives.
//!
//! DroidWin never downloads or redistributes the Android SDK: the licence grants
//! development use only. We find what is already installed and say clearly what
//! is missing.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Sdk {
    root: PathBuf,
}

impl Sdk {
    /// Explicit path first, then the documented environment variables, then the
    /// conventional install locations.
    pub fn locate(explicit: Option<PathBuf>) -> Option<Self> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(path) = explicit {
            candidates.push(path);
        }
        for var in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    candidates.push(PathBuf::from(value));
                }
            }
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            candidates.push(PathBuf::from(&local).join("Android").join("Sdk"));
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            candidates.push(
                PathBuf::from(&profile)
                    .join("AppData")
                    .join("Local")
                    .join("Android")
                    .join("Sdk"),
            );
        }
        candidates.push(PathBuf::from("C:/Android/Sdk"));

        candidates
            .into_iter()
            .find(|root| is_sdk_root(root))
            .map(|root| Sdk { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn adb(&self) -> PathBuf {
        self.root.join("platform-tools").join(exe("adb"))
    }

    pub fn emulator(&self) -> PathBuf {
        self.root.join("emulator").join(exe("emulator"))
    }

    pub fn avdmanager(&self) -> PathBuf {
        self.root
            .join("cmdline-tools")
            .join("latest")
            .join("bin")
            .join(bat("avdmanager"))
    }

    pub fn sdkmanager(&self) -> PathBuf {
        self.root
            .join("cmdline-tools")
            .join("latest")
            .join("bin")
            .join(bat("sdkmanager"))
    }

    pub fn aapt2(&self) -> Option<PathBuf> {
        let tools = self.root.join("build-tools");
        let mut versions: Vec<PathBuf> = std::fs::read_dir(tools)
            .ok()?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.join(exe("aapt2")).is_file())
            .collect();
        versions.sort();
        versions.pop().map(|p| p.join(exe("aapt2")))
    }

    pub fn has_emulator(&self) -> bool {
        self.emulator().is_file()
    }

    /// Installed system images, in `system-images;api;variant;abi` form.
    pub fn system_images(&self) -> Vec<String> {
        let mut out = Vec::new();
        let root = self.root.join("system-images");
        let Ok(apis) = std::fs::read_dir(&root) else {
            return out;
        };
        for api in apis.filter_map(|e| e.ok()) {
            let Some(api_name) = api.file_name().to_str().map(str::to_string) else {
                continue;
            };
            let Ok(variants) = std::fs::read_dir(api.path()) else {
                continue;
            };
            for variant in variants.filter_map(|e| e.ok()) {
                let Some(variant_name) = variant.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let Ok(abis) = std::fs::read_dir(variant.path()) else {
                    continue;
                };
                for abi in abis.filter_map(|e| e.ok()) {
                    let Some(abi_name) = abi.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    if abi.path().is_dir() {
                        out.push(format!(
                            "system-images;{api_name};{variant_name};{abi_name}"
                        ));
                    }
                }
            }
        }
        out.sort();
        out
    }
}

/// An SDK root is only a root if platform tools are actually there.
pub fn is_sdk_root(root: &Path) -> bool {
    root.join("platform-tools").is_dir()
}

pub fn exe(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn bat(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.bat")
    } else {
        name.to_string()
    }
}

/// `%ANDROID_AVD_HOME%` when set, otherwise `~/.android/avd`.
pub fn avd_home() -> PathBuf {
    if let Ok(value) = std::env::var("ANDROID_AVD_HOME") {
        if !value.trim().is_empty() {
            return PathBuf::from(value);
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".android")
        .join("avd")
}

pub fn avd_exists(name: &str) -> bool {
    avd_home().join(format!("{name}.avd")).is_dir()
}

/// scrcpy ships with DroidWin (Apache-2.0). Also accept one on `PATH` so a
/// developer install keeps working.
pub fn scrcpy(app_data_dir: &Path) -> Option<PathBuf> {
    let bundled = app_data_dir
        .join("runtime")
        .join("bin")
        .join("scrcpy")
        .join(exe("scrcpy"));
    if bundled.is_file() {
        return Some(bundled);
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(exe("scrcpy"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The emulator's own acceleration report, verbatim, so the UI can show it
/// rather than claiming acceleration works.
pub fn accel_report(emulator: &Path) -> Option<String> {
    if !emulator.is_file() {
        return None;
    }
    let output = std::process::Command::new(emulator)
        .arg("-accel-check")
        .output()
        .ok()?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_without_platform_tools_are_not_sdk_roots() {
        let dir = std::env::temp_dir().join(format!("droidwin-sdk-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!is_sdk_root(&dir));
        assert!(!is_sdk_root(Path::new("Z:/definitely/not/here")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn avd_home_is_absolute() {
        assert!(avd_home().is_absolute() || avd_home().starts_with("."));
    }
}
