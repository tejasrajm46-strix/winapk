//! Locating the Android SDK and the tools DroidWin drives.
//!
//! Two kinds of SDK exist as far as DroidWin is concerned: one the user already
//! has (Android Studio, a platform-tools install, `ANDROID_SDK_ROOT`), and one
//! DroidWin installs for itself into its own data directory. Both are ordinary
//! SDK roots; this module chooses between them by how complete they are, so a
//! developer's machine keeps using its own SDK and a fresh PC gets the managed
//! one. Nothing here downloads anything: `runtime::install` owns that.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Where DroidWin keeps its own state. Set once at startup.
static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();
/// A root the user chose on the setup screen. Overrides everything else.
static CONFIGURED: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Told once at startup, before anything probes the SDK.
pub fn configure(data_dir: &Path) {
    let _ = DATA_DIR.set(data_dir.to_path_buf());
}

/// An SDK directory named by `DROIDWIN_ANDROID_SDK`. This is an explicit choice
/// by whoever started the process (a portable run, a test), so it outranks every
/// discovered root — including an incomplete one, which is how a first run on a
/// fresh PC is reproduced.
pub fn explicit_override() -> Option<PathBuf> {
    match std::env::var("DROIDWIN_ANDROID_SDK") {
        Ok(value) if !value.trim().is_empty() => Some(PathBuf::from(value)),
        _ => None,
    }
}

/// `<data>/android-sdk`: the directory DroidWin installs into. `None` before
/// `configure`, which only happens in a unit test.
pub fn managed_root() -> Option<PathBuf> {
    DATA_DIR.get().map(|dir| dir.join("android-sdk"))
}

pub fn configured_root() -> Option<PathBuf> {
    CONFIGURED.lock().ok().and_then(|slot| slot.clone())
}

pub fn set_configured_root(path: Option<PathBuf>) {
    if let Ok(mut slot) = CONFIGURED.lock() {
        *slot = path;
    }
}

/// Where an installation writes when no SDK is found: the user's choice, else
/// the managed directory.
pub fn install_root() -> Option<PathBuf> {
    configured_root()
        .or_else(explicit_override)
        .or_else(managed_root)
}

/// The one SDK root DroidWin is using.
///
/// The setup screen, the installer and the runtime all ask this question, so a
/// PC with a complete Android Studio SDK cannot be told it has nothing: that
/// SDK is discovered here and adopted, and the setup screen never offers a
/// second multi-gigabyte download into the managed directory.
///
/// A root named by the user (or `DROIDWIN_ANDROID_SDK`) outranks discovery even
/// when it is empty, because it is a decision rather than a hint. Only when no
/// SDK is discoverable does this fall back to the directory DroidWin installs
/// into.
pub fn active_root() -> Option<PathBuf> {
    Sdk::locate(None)
        .map(|sdk| sdk.root().to_path_buf())
        .or_else(install_root)
}

pub fn is_managed(root: &Path) -> bool {
    managed_root()
        .map(|managed| same_path(&managed, root))
        .unwrap_or(false)
}

fn same_path(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[derive(Debug, Clone)]
pub struct Sdk {
    root: PathBuf,
}

impl Sdk {
    /// An SDK rooted exactly here, with no discovery. The installer uses this
    /// while a root is still half-built.
    pub fn at(root: PathBuf) -> Self {
        Sdk { root }
    }

    /// The best root on this machine: the user's choice first, then the
    /// environment, then DroidWin's own directory, then the conventional
    /// locations. Ties keep the earlier candidate, so a developer's SDK wins
    /// over a managed one of the same completeness.
    pub fn locate(explicit: Option<PathBuf>) -> Option<Self> {
        if let Some(path) = explicit {
            return Some(Sdk { root: path });
        }
        // A named root is a decision, not a hint: use it even when it is empty so
        // the setup screen describes the SDK the user asked for.
        if let Some(root) = configured_root().or_else(explicit_override) {
            return Some(Sdk { root });
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        let mut push = |path: PathBuf| {
            if !candidates.iter().any(|existing| same_path(existing, &path)) {
                candidates.push(path);
            }
        };
        for var in ["ANDROID_SDK_ROOT", "ANDROID_HOME"] {
            if let Ok(value) = std::env::var(var) {
                if !value.trim().is_empty() {
                    push(PathBuf::from(value));
                }
            }
        }
        if let Some(managed) = managed_root() {
            push(managed);
        }
        if let Some(managed) = explicit_override() {
            push(managed);
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            push(PathBuf::from(&local).join("Android").join("Sdk"));
        }
        if let Ok(profile) = std::env::var("USERPROFILE") {
            push(
                PathBuf::from(&profile)
                    .join("AppData")
                    .join("Local")
                    .join("Android")
                    .join("Sdk"),
            );
        }
        push(PathBuf::from("C:/Android/Sdk"));

        candidates
            .into_iter()
            .filter(|root| is_sdk_root(root))
            .max_by_key(|root| completeness(root))
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

    pub fn has_platform_tools(&self) -> bool {
        self.adb().is_file()
    }

    pub fn has_cmdline_tools(&self) -> bool {
        self.sdkmanager().is_file()
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
        system_images_at(&self.root)
    }
}

/// A root is only a root if platform tools are actually there.
pub fn is_sdk_root(root: &Path) -> bool {
    root.join("platform-tools").is_dir()
}

/// How much of a working runtime a root already holds. Only used to choose
/// between roots on the same machine; the setup report does the fine-grained
/// answer component by component.
fn completeness(root: &Path) -> u8 {
    let mut score = 1;
    if root
        .join("cmdline-tools")
        .join("latest")
        .join("bin")
        .is_dir()
    {
        score += 1;
    }
    if root.join("platform-tools").join(exe("adb")).is_file() {
        score += 2;
    }
    if root.join("emulator").join(exe("emulator")).is_file() {
        score += 2;
    }
    if !system_images_at(root).is_empty() {
        score += 2;
    }
    score
}

pub fn system_images_at(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(apis) = std::fs::read_dir(root.join("system-images")) else {
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

/// Every AVD configuration on this machine, by name.
pub fn avds() -> Vec<String> {
    std::fs::read_dir(avd_home())
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    name.strip_suffix(".avd").map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Free space in megabytes on the volume `path` lives on.
#[cfg(windows)]
pub fn disk_free_mb(path: &Path) -> Option<u64> {
    #[repr(C)]
    struct FreeSpace {
        free_for_caller: u64,
        total: u64,
        total_free: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_for_caller: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }

    let wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let mut space = FreeSpace {
        free_for_caller: 0,
        total: 0,
        total_free: 0,
    };
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut space.free_for_caller,
            &mut space.total,
            &mut space.total_free,
        )
    };
    if ok == 0 {
        None
    } else {
        Some(space.free_for_caller / (1024 * 1024))
    }
}

/// Walking up to an existing parent keeps the answer useful for a directory the
/// installer has not created yet.
#[cfg(windows)]
pub fn disk_free_mb_for_new(path: &Path) -> Option<u64> {
    let mut probe = path;
    loop {
        if probe.exists() {
            return disk_free_mb(probe);
        }
        probe = probe.parent()?;
    }
}

#[cfg(not(windows))]
pub fn disk_free_mb(path: &Path) -> Option<u64> {
    let _ = path;
    None
}

#[cfg(not(windows))]
pub fn disk_free_mb_for_new(path: &Path) -> Option<u64> {
    let _ = path;
    None
}

/// The JRE this SDK uses: one DroidWin installed beside it, else the machine's
/// `JAVA_HOME`. `None` means `java` is expected on `PATH`.
pub fn java_home(root: &Path) -> Option<PathBuf> {
    let bundled = root.join("jre");
    if bundled.join("bin").join(exe("java")).is_file() {
        return Some(bundled);
    }
    if let Ok(home) = std::env::var("JAVA_HOME") {
        if Path::new(&home).join("bin").join(exe("java")).is_file() {
            return Some(PathBuf::from(home));
        }
    }
    None
}

/// Give a tool the Java the SDK manager and AVD manager must run under. Both
/// are `.bat` wrappers that read `JAVA_HOME`, so a bundled JRE is invisible to
/// them unless it is set here.
pub fn apply_java(root: &Path, cmd: &mut std::process::Command) {
    let Some(home) = java_home(root) else {
        return;
    };
    let path = std::env::var("PATH").unwrap_or_default();
    cmd.env("JAVA_HOME", &home);
    cmd.env("PATH", format!("{};{path}", home.join("bin").display()));
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
    let mut cmd = std::process::Command::new(emulator);
    cmd.arg("-accel-check");
    crate::proc::no_console(&mut cmd);
    let output = cmd.output().ok()?;
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

    #[test]
    fn a_root_with_more_component_files_scores_higher() {
        let base = std::env::temp_dir().join(format!("droidwin-score-{}", std::process::id()));
        let bare = base.join("bare");
        let full = base.join("full");
        std::fs::create_dir_all(bare.join("platform-tools")).unwrap();
        std::fs::create_dir_all(full.join("platform-tools")).unwrap();
        std::fs::create_dir_all(full.join("emulator")).unwrap();
        std::fs::create_dir_all(
            full.join("system-images")
                .join("android-36")
                .join("google_apis")
                .join("x86_64"),
        )
        .unwrap();
        std::fs::write(full.join("platform-tools").join(exe("adb")), b"").unwrap();
        std::fs::write(full.join("emulator").join(exe("emulator")), b"").unwrap();
        assert!(completeness(&full) > completeness(&bare));
        assert!(is_sdk_root(&bare) && is_sdk_root(&full));
        assert_eq!(
            system_images_at(&full),
            vec!["system-images;android-36;google_apis;x86_64"]
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_install_root_follows_the_users_choice() {
        let configured = std::env::temp_dir().join("droidwin-chosen");
        set_configured_root(Some(configured.clone()));
        assert_eq!(install_root(), Some(configured.clone()));
        set_configured_root(None);
        assert_ne!(install_root(), Some(configured));
    }
}
