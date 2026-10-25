//! The Android Emulator + scrcpy adapter.
//!
//! One headless emulator hosts every app; each launched app gets its own scrcpy
//! window on its own virtual display. The emulator is user-provided — DroidWin
//! locates it, it does not ship it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::process::Child;

use super::adb::Adb;
use super::emulator::{self, AvdConfig};
use super::scrcpy::{self, WindowRequest};
use super::sdk::{self, Sdk};
use super::{InstallOutcome, RuntimeEvents};
use crate::domain::{
    now_ms, ApkInspection, AppWindow, Capabilities, DwError, DwResult, EnvReport, ErrorCode,
    InstalledApp, RuntimeState, RuntimeStatus, Stage, StageEvent,
};
use crate::store::{CachedApp, Store};

pub const AVD_NAME: &str = "DroidWin_API36";
pub const SYSTEM_IMAGE: &str = "system-images;android-36;google_apis;x86_64";
const EMULATOR_PORT: u16 = 5554;
const BOOT_BUDGET: Duration = Duration::from_secs(420);
/// Virtual display geometry: a phone-shaped surface, so portrait apps look right.
const DISPLAY_WIDTH: u32 = 540;
const DISPLAY_HEIGHT: u32 = 1170;
const DISPLAY_DPI: u32 = 420;
const WINDOW_WIDTH: u32 = 420;
const WINDOW_HEIGHT: u32 = 900;

struct WindowEntry {
    child: Arc<tokio::sync::Mutex<Child>>,
    title: String,
}

pub struct EmulatorRuntime {
    data_dir: PathBuf,
    events: Arc<dyn RuntimeEvents>,
    emulator_child: tokio::sync::Mutex<Option<Child>>,
    windows: Arc<Mutex<HashMap<String, WindowEntry>>>,
}

impl EmulatorRuntime {
    pub fn new(data_dir: PathBuf, events: Arc<dyn RuntimeEvents>) -> Self {
        Self {
            data_dir,
            events,
            emulator_child: tokio::sync::Mutex::new(None),
            windows: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn config() -> AvdConfig {
        AvdConfig {
            name: AVD_NAME.to_string(),
            system_image: SYSTEM_IMAGE.to_string(),
            port: EMULATOR_PORT,
        }
    }

    fn adb(&self) -> Option<Adb> {
        Sdk::locate(None).map(|sdk| Adb::new(sdk.adb()))
    }

    /// Filesystem + emulator self-report. Cheap enough to call on every refresh.
    pub fn probe(&self) -> EnvReport {
        let mut report = EnvReport::default();
        let Some(sdk) = Sdk::locate(None) else {
            report.blocking.push(
                "No Android SDK found. Install the Android command-line tools and set ANDROID_SDK_ROOT."
                    .to_string(),
            );
            return report;
        };
        report.sdk_root = Some(sdk.root().display().to_string());
        if sdk.adb().is_file() {
            report.adb = Some(sdk.adb().display().to_string());
        } else {
            report
                .blocking
                .push("Platform tools are missing: sdkmanager \"platform-tools\"".to_string());
        }
        if sdk.has_emulator() {
            report.emulator = Some(sdk.emulator().display().to_string());
            report.accel = sdk::accel_report(&sdk.emulator());
        } else {
            report
                .blocking
                .push("The Android Emulator is not installed: sdkmanager \"emulator\"".to_string());
        }
        if sdk.avdmanager().is_file() {
            report.avdmanager = Some(sdk.avdmanager().display().to_string());
        }
        if sdk.sdkmanager().is_file() {
            report.sdkmanager = Some(sdk.sdkmanager().display().to_string());
        }
        report.aapt2 = sdk.aapt2().map(|p| p.display().to_string());

        let images = sdk.system_images();
        report.system_image = images
            .iter()
            .find(|i| i.as_str() == SYSTEM_IMAGE)
            .or_else(|| {
                images
                    .iter()
                    .find(|i| i.contains("android-36") && i.ends_with("x86_64"))
            })
            .cloned();
        if report.system_image.is_none() {
            report.blocking.push(format!(
                "A system image is missing: sdkmanager \"{SYSTEM_IMAGE}\""
            ));
        }

        report.avds = std::fs::read_dir(sdk::avd_home())
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        name.strip_suffix(".avd").map(str::to_string)
                    })
                    .collect()
            })
            .unwrap_or_default();

        match sdk::scrcpy(&self.data_dir) {
            Some(path) => report.scrcpy = Some(path.display().to_string()),
            None => report.blocking.push(
                "The window host (scrcpy) is missing from this installation; reinstall DroidWin."
                    .to_string(),
            ),
        }

        report
    }

    fn capabilities(
        &self,
        _api: Option<u32>,
        accel_ok: bool,
        verified_windows: bool,
    ) -> Capabilities {
        Capabilities {
            install: true,
            uninstall: true,
            enumerate: true,
            launch: true,
            independent_windows: verified_windows,
            concurrent_apps: verified_windows,
            // Provided by the window host, but not verified on this machine yet,
            // so the product does not claim them.
            clipboard: false,
            file_drop: false,
            // Windows are launched with --no-audio until forwarding is verified.
            audio: false,
            hardware_accel: accel_ok,
        }
    }

    fn limitations(&self, verified_windows: bool) -> Vec<String> {
        let mut items = vec![
            "All apps share one Android user, so they are not isolated from each other.".to_string(),
            "Each app window is a streamed virtual display: Android's back key works, but there is no host-side back button.".to_string(),
            "Audio is not forwarded in this build.".to_string(),
            "Clipboard sharing and dropping files onto an app window come from the window host and have not been verified in this build."
                .to_string(),
            "ARM-only packages cannot run on the x86_64 system image.".to_string(),
            "The Android SDK and emulator are user-provided; DroidWin does not redistribute them.".to_string(),
        ];
        if !verified_windows {
            items.insert(
                0,
                "Independent app windows have not been verified on this machine yet.".to_string(),
            );
        }
        items
    }

    pub async fn status(&self, store: &Store) -> RuntimeStatus {
        let env = self.probe();
        let verified = store
            .setting("runtime.independent_windows_verified")
            .as_deref()
            == Some("1");
        let accel_ok = env
            .accel
            .as_deref()
            .map(|a| {
                a.contains("\n0\n") || a.starts_with('0') || a.contains("is installed and usable")
            })
            .unwrap_or(false);
        let api = self.adb_api().await;

        let (state, detail) = if !env.ready() {
            (RuntimeState::NotConfigured, env.blocking.join(" "))
        } else if let Some(api) = api {
            (
                RuntimeState::Ready,
                format!(
                    "Android API {api} on {} ({}).",
                    Self::config().serial(),
                    AVD_NAME
                ),
            )
        } else {
            let child_alive = self.emulator_child.lock().await.is_some();
            if child_alive {
                (
                    RuntimeState::Starting,
                    "The Android runtime is booting.".to_string(),
                )
            } else {
                (
                    RuntimeState::NotRunning,
                    "The Android runtime is stopped. Starting it takes a moment; the first boot is slower."
                        .to_string(),
                )
            }
        };

        let blocking = if matches!(state, RuntimeState::NotConfigured) {
            env.blocking.clone()
        } else {
            Vec::new()
        };

        RuntimeStatus {
            backend: "Android Emulator (user-provided SDK) + scrcpy".to_string(),
            state,
            detail,
            avd: Some(AVD_NAME.to_string()),
            serial: if api.is_some() {
                Some(Self::config().serial())
            } else {
                None
            },
            android_api: api,
            capabilities: self.capabilities(api, accel_ok, verified),
            limitations: {
                let mut l = self.limitations(verified);
                l.extend(blocking);
                l
            },
        }
    }

    async fn adb_api(&self) -> Option<u32> {
        let adb = self.adb()?;
        let serial = Self::config().serial();
        if !adb.connect_ready(&serial).await {
            return None;
        }
        if !adb.boot_completed(&serial).await {
            return None;
        }
        adb.api_level(&serial).await
    }

    fn emit(&self, op: &str, stage: Stage, message: impl Into<String>, package_id: Option<String>) {
        self.events.stage(&StageEvent {
            op: op.to_string(),
            package_id,
            stage,
            message: message.into(),
            at_ms: now_ms(),
        });
    }

    /// Bring the runtime to a point where adb install can run.
    pub async fn ensure_ready(&self, store: &Store, op: &str) -> DwResult<RuntimeStatus> {
        let env = self.probe();
        if !env.ready() {
            return Err(DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android runtime is not ready on this machine.",
            )
            .detail(env.blocking.join("\n")));
        }

        let sdk = Sdk::locate(None).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK could not be located.",
            )
        })?;
        let adb = Adb::new(sdk.adb());
        let config = Self::config();
        let serial = config.serial();

        if adb.connect_ready(&serial).await && adb.boot_completed(&serial).await {
            return Ok(self.status(store).await);
        }

        self.emit(
            op,
            Stage::PreparingRuntime,
            "Preparing the virtual device.",
            None,
        );
        let created = emulator::ensure_avd(&sdk, &config).await?;
        if created {
            tracing::info!("created AVD {AVD_NAME}");
        }

        let already_running = adb.connect_ready(&serial).await;
        if !already_running {
            let child = emulator::spawn(&sdk, &config)?;
            *self.emulator_child.lock().await = Some(child);
            adb.start().await.ok();
        }

        emulator::wait_boot(&adb, &serial, BOOT_BUDGET).await?;
        Ok(self.status(store).await)
    }

    pub async fn install(
        &self,
        inspection: &ApkInspection,
        store: &Store,
        op: &str,
    ) -> DwResult<InstallOutcome> {
        self.ensure_ready(store, op).await?;
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = Self::config().serial();
        let package_id = inspection.package_id.clone();

        let was_installed = adb.package_path(&serial, &package_id).await.is_some();

        self.emit(
            op,
            Stage::Installing,
            format!("Installing {package_id}."),
            Some(package_id.clone()),
        );
        let output = adb.install(&serial, Path::new(&inspection.path)).await?;
        if !output.ok() || !output.stdout.contains("Success") {
            let text = output.combined();
            let summary = if text.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
                "The installed app has a different signature; Android will not let this package replace it."
            } else if text.contains("INSTALL_FAILED_INSUFFICIENT_STORAGE") {
                "The runtime has run out of storage for this package."
            } else if text.contains("INSTALL_FAILED_OLDER_SDK") {
                "This package requires a newer Android version than the runtime provides."
            } else if text.contains("INSTALL_FAILED_NO_MATCHING_ABIS") {
                "This package has no native code for this runtime's CPU architecture."
            } else if text.contains("INSTALL_PARSE_FAILED") {
                "Android could not parse this package."
            } else {
                "Android rejected the installation."
            };
            return Err(DwError::new(ErrorCode::InstallFailed, summary).detail(text));
        }

        self.emit(
            op,
            Stage::Verifying,
            "Confirming the runtime sees the package.",
            Some(package_id.clone()),
        );
        if adb.package_path(&serial, &package_id).await.is_none() {
            return Err(DwError::new(
                ErrorCode::InstallFailed,
                "The installer reported success, but the runtime does not list the package.",
            ));
        }

        let version_code = adb
            .third_party_packages(&serial)
            .await
            .ok()
            .and_then(|list| list.into_iter().find(|p| p.package_id == package_id))
            .and_then(|p| p.version_code);

        let _ = store.cache_app(
            &CachedApp {
                package_id: package_id.clone(),
                label: inspection.label.clone(),
                version_name: inspection.version_name.clone(),
                version_code: version_code.or(inspection.version_code),
                icon: inspection.icon.clone(),
            },
            now_ms(),
        );
        self.events.apps_changed();

        Ok(InstallOutcome {
            package_id: package_id.clone(),
            version_name: inspection.version_name.clone(),
            version_code: version_code.or(inspection.version_code),
            updated: was_installed,
            message: if was_installed {
                format!("{package_id} updated")
            } else {
                format!("{package_id} installed")
            },
        })
    }

    pub async fn list_apps(&self, store: &Store) -> DwResult<Vec<InstalledApp>> {
        let adb = self.adb().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK could not be located.",
            )
        })?;
        let serial = Self::config().serial();
        if !adb.connect_ready(&serial).await || !adb.boot_completed(&serial).await {
            return Err(DwError::new(
                ErrorCode::RuntimeUnavailable,
                "The Android runtime is not running.",
            ));
        }

        let packages = adb.third_party_packages(&serial).await?;
        let cache: HashMap<String, CachedApp> = store
            .cached_apps()
            .into_iter()
            .map(|c| (c.package_id.clone(), c))
            .collect();
        let open_windows: HashMap<String, String> = self
            .windows
            .lock()
            .map(|w| {
                w.iter()
                    .map(|(package, entry)| (package.clone(), entry.title.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let mut apps = Vec::new();
        for pkg in packages {
            let cached = cache.get(&pkg.package_id);
            apps.push(InstalledApp {
                label: cached.and_then(|c| c.label.clone()),
                version_name: cached.and_then(|c| c.version_name.clone()),
                version_code: pkg.version_code.or(cached.and_then(|c| c.version_code)),
                running: open_windows.contains_key(&pkg.package_id),
                window_title: open_windows.get(&pkg.package_id).cloned(),
                icon: cached.and_then(|c| c.icon.clone()),
                user_installed: true,
                package_id: pkg.package_id,
            });
        }
        apps.sort_by(|a, b| {
            a.label
                .as_deref()
                .unwrap_or(&a.package_id)
                .to_lowercase()
                .cmp(&b.label.as_deref().unwrap_or(&b.package_id).to_lowercase())
        });
        Ok(apps)
    }

    pub async fn launch(
        &self,
        package_id: &str,
        label: Option<&str>,
        store: &Store,
        op: &str,
    ) -> DwResult<AppWindow> {
        if let Some(existing) = self.windows.lock().ok().and_then(|w| {
            w.get(package_id).map(|e| AppWindow {
                package_id: package_id.to_string(),
                title: e.title.clone(),
                pid: 0,
            })
        }) {
            return Ok(existing);
        }

        self.ensure_ready(store, op).await?;
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = Self::config().serial();
        if adb.package_path(&serial, package_id).await.is_none() {
            return Err(DwError::new(
                ErrorCode::NotInstalled,
                format!("{package_id} is not installed in the runtime."),
            ));
        }

        let exe = sdk::scrcpy(&self.data_dir).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The window host (scrcpy) is missing.",
            )
        })?;

        let index = self.windows.lock().map(|w| w.len()).unwrap_or(0) as i32;
        let title = format!("DroidWin — {}", label.unwrap_or(package_id));
        let request = WindowRequest {
            serial: serial.clone(),
            package_id: package_id.to_string(),
            title: title.clone(),
            x: 80 + index * 36,
            y: 80 + index * 36,
            width: WINDOW_WIDTH,
            height: WINDOW_HEIGHT,
            display_width: DISPLAY_WIDTH,
            display_height: DISPLAY_HEIGHT,
            dpi: DISPLAY_DPI,
        };

        self.emit(
            op,
            Stage::Launching,
            format!("Opening a window for {package_id}."),
            Some(package_id.to_string()),
        );
        let child = scrcpy::spawn(&exe, &request)?;
        let pid = child.id().unwrap_or(0);
        let child = Arc::new(tokio::sync::Mutex::new(child));

        if let Ok(mut windows) = self.windows.lock() {
            windows.insert(
                package_id.to_string(),
                WindowEntry {
                    child: Arc::clone(&child),
                    title: title.clone(),
                },
            );
        }
        self.watch_window(package_id.to_string(), child);
        self.events.apps_changed();

        // If scrcpy cannot start the app, it exits almost immediately; report that
        // instead of leaving the UI claiming a window opened.
        tokio::time::sleep(Duration::from_millis(1200)).await;
        let still_open = self
            .windows
            .lock()
            .map(|w| w.contains_key(package_id))
            .unwrap_or(false);
        if !still_open {
            return Err(DwError::new(
                ErrorCode::LaunchFailed,
                format!("The window for {package_id} closed immediately."),
            )
            .detail("scrcpy could not start this package on a virtual display. This app may not support secondary displays."));
        }

        Ok(AppWindow {
            package_id: package_id.to_string(),
            title,
            pid,
        })
    }

    fn watch_window(&self, package_id: String, child: Arc<tokio::sync::Mutex<Child>>) {
        let windows = Arc::clone(&self.windows);
        let events = Arc::clone(&self.events);
        tokio::spawn(async move {
            loop {
                let exited = {
                    let mut guard = child.lock().await;
                    match guard.try_wait() {
                        Ok(Some(status)) => Some(status.code()),
                        Ok(None) => None,
                        Err(_) => Some(None),
                    }
                };
                if let Some(code) = exited {
                    tracing::info!("window for {package_id} closed (exit {code:?})");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
            if let Ok(mut map) = windows.lock() {
                map.remove(&package_id);
            }
            events.apps_changed();
        });
    }

    pub async fn close_window(&self, package_id: &str) -> DwResult<()> {
        let entry = self
            .windows
            .lock()
            .ok()
            .and_then(|mut w| w.remove(package_id));
        let Some(entry) = entry else {
            return Ok(());
        };
        let mut child = entry.child.lock().await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        self.events.apps_changed();
        Ok(())
    }

    pub async fn stop_app(&self, package_id: &str) -> DwResult<()> {
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let out = adb.force_stop(&Self::config().serial(), package_id).await?;
        if out.ok() {
            self.events.apps_changed();
            Ok(())
        } else {
            Err(
                DwError::new(ErrorCode::BackendFailure, "Android did not stop the app.")
                    .detail(out.combined()),
            )
        }
    }

    pub async fn uninstall(
        &self,
        package_id: &str,
        delete_data: bool,
        store: &Store,
    ) -> DwResult<()> {
        let adb = self
            .adb()
            .ok_or_else(|| DwError::new(ErrorCode::RuntimeUnavailable, "adb is unavailable."))?;
        let serial = Self::config().serial();
        if adb.package_path(&serial, package_id).await.is_none() {
            return Err(DwError::new(
                ErrorCode::NotInstalled,
                format!("{package_id} is not installed."),
            ));
        }
        self.close_window(package_id).await?;
        let out = adb.uninstall(&serial, package_id, !delete_data).await?;
        if !out.ok() || !out.stdout.contains("Success") {
            return Err(DwError::new(
                ErrorCode::UninstallFailed,
                format!("Android refused to uninstall {package_id}."),
            )
            .detail(out.combined()));
        }
        if delete_data {
            let _ = store.forget_app(package_id);
        }
        self.events.apps_changed();
        Ok(())
    }

    pub async fn shutdown(&self) {
        let packages: Vec<String> = self
            .windows
            .lock()
            .map(|w| w.keys().cloned().collect())
            .unwrap_or_default();
        for package in packages {
            let _ = self.close_window(&package).await;
        }
        // Stop Android first (clean console shutdown), then make sure no qemu
        // process survives its launcher.
        if let Some(adb) = self.adb() {
            let _ = adb.emu_kill(&Self::config().serial()).await;
        }
        if let Some(mut child) = self.emulator_child.lock().await.take() {
            if let Some(pid) = child.id() {
                emulator::kill_tree(pid);
            }
            let _ = child.start_kill();
            let _ = child.wait().await;
        }
    }
}
