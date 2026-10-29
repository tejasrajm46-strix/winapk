//! One-time Android runtime installation.
//!
//! A store-bought Windows PC has no Android SDK, and asking a player to install
//! Android Studio first is asking them to install a development environment to
//! play a game. This module installs only the four pieces DroidWin actually
//! runs on — command-line tools, platform-tools, the emulator and one system
//! image — into DroidWin's own directory, then creates the virtual device.
//!
//! Every download comes from `dl.google.com` (the Android SDK repository) or
//! Eclipse Adoptium (the JRE `sdkmanager` runs on), and the command-line tools
//! archive is checked against the SHA-1 the repository publishes for it. Nothing
//! is accepted on the strength of a URL alone.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::{emulator, sdk};
use crate::domain::{
    now_ms, DwError, DwResult, ErrorCode, SetupComponent, SetupProgress, SetupReport, SetupState,
};
use crate::proc;
use crate::store::Store;

/// The Android version DroidWin runs apps on.
pub const API_LEVEL: u32 = 36;
pub const PLATFORM_PACKAGE: &str = "platforms;android-36";
pub const SYSTEM_IMAGE: &str = "system-images;android-36;google_apis;x86_64";
pub const AVD_NAME: &str = "DroidWin_API36";

const REPOSITORY: &str = "https://dl.google.com/android/repository/repository2-3.xml";
const REPOSITORY_ROOT: &str = "https://dl.google.com/android/repository/";
/// Temurin 21 is the newest LTS the Android tools are tested against; 25 is the
/// newest, but a runtime the tooling does not expect is not worth the risk.
const JRE_URL: &str =
    "https://api.adoptium.net/v3/binary/latest/21/ga/windows/{arch}/jre/hotspot/normal/eclipse";

/// Sizes the setup screen shows before anything is downloaded. They are the
/// published archive sizes rounded up, and are labelled approximate there.
const TOOLS_MB: u32 = 155;
const PLATFORM_TOOLS_MB: u32 = 15;
const EMULATOR_MB: u32 = 450;
const IMAGE_MB: u32 = 1_700;
const JRE_MB: u32 = 60;
/// Room for unpacked copies plus the extracted packages, over the raw download.
const HEADROOM_MB: u64 = 2_500;

const KEY_LICENSES: &str = "runtime.licenses_accepted";
const KEY_INSTALLED_AT: &str = "runtime.installed_at";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

pub type Sink = Arc<dyn Fn(SetupProgress) + Send + Sync>;

fn emit(sink: &Sink, step: &str, message: impl Into<String>, percent: Option<f64>) {
    sink(SetupProgress {
        step: step.to_string(),
        message: message.into(),
        percent,
        done: false,
        failed: None,
    });
}

fn fail(sink: &Sink, step: &str, message: impl Into<String>) {
    let message = message.into();
    sink(SetupProgress {
        step: step.to_string(),
        message: message.clone(),
        percent: None,
        done: true,
        failed: Some(message),
    });
}

/// One archive Google publishes for a package.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Archive {
    pub url: String,
    pub size: Option<u64>,
    pub sha1: Option<String>,
}

/// The newest Windows command-line tools in `repository2-3.xml`.
///
/// The archive is chosen by `host-os`, and the highest `cmdline-tools;X.Y`
/// revision wins. Hardcoding a zip name would rot the day Google publishes a new
/// one, which is why the repository — not a constant — is the source.
pub fn cmdline_tools_archive(xml: &str) -> Option<(String, Archive)> {
    let mut best: Option<(u32, u32, Archive)> = None;
    let mut rest = xml;
    while let Some(start) = rest.find("<remotePackage path=\"cmdline-tools;") {
        rest = &rest[start + "<remotePackage".len()..];
        let Some(end) = rest.find("</remotePackage>") else {
            break;
        };
        let block = &rest[..end];
        let Some(path_start) = block.find("path=\"") else {
            continue;
        };
        let path_start = path_start + "path=\"".len();
        let Some(path_len) = block[path_start..].find('"') else {
            continue;
        };
        let Some(version) = block[path_start..path_start + path_len].strip_prefix("cmdline-tools;")
        else {
            continue;
        };
        let (major, minor) = match version.split_once('.') {
            Some((major, minor)) => (
                major.parse::<u32>().unwrap_or(0),
                minor.parse::<u32>().unwrap_or(0),
            ),
            None => continue,
        };
        let windows = block
            .split("<archive>")
            .skip(1)
            .find(|archive| archive.contains("<host-os>windows</host-os>"));
        let Some(archive) = windows else {
            continue;
        };
        let url = tag(archive, "url").unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        let candidate = Archive {
            url: format!("{REPOSITORY_ROOT}{url}"),
            size: tag(archive, "size").and_then(|value| value.parse().ok()),
            sha1: tag(archive, "checksum")
                .filter(|value| value.len() == 40)
                .map(str::to_string),
        };
        if best
            .as_ref()
            .map(|(best_major, best_minor, _)| (major, minor) > (*best_major, *best_minor))
            .unwrap_or(true)
        {
            best = Some((major, minor, candidate));
        }
    }
    best.map(|(major, minor, archive)| (format!("{major}.{minor}"), archive))
}

fn tag<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let start = block.find(&open)?;
    // Skip the rest of the opening tag: `<checksum type="sha1">` carries an
    // attribute, so the tag's text starts at the first `>`.
    let start = start + block[start..].find('>')? + 1;
    let end = block[start..].find(&close)? + start;
    let value = block[start..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

/// The state of the runtime installation and everything the setup screen shows.
pub struct Setup {
    data_dir: PathBuf,
    sink: Sink,
    /// The last progress event, so a screen that opens mid-install is not blank.
    progress: Mutex<Option<SetupProgress>>,
    running: AtomicBool,
    cancel: Arc<AtomicBool>,
    last_error: Mutex<Option<String>>,
}

impl Setup {
    pub fn new(data_dir: PathBuf, sink: Sink) -> Self {
        Self {
            data_dir,
            sink,
            progress: Mutex::new(None),
            running: AtomicBool::new(false),
            cancel: Arc::new(AtomicBool::new(false)),
            last_error: Mutex::new(None),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn progress(&self) -> Option<SetupProgress> {
        self.progress.lock().ok().and_then(|slot| slot.clone())
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    fn publish(&self, event: SetupProgress) {
        if let Ok(mut slot) = self.progress.lock() {
            *slot = Some(event.clone());
        }
        (self.sink)(event);
    }

    /// What is installed, what is missing, and whether it can be installed here.
    ///
    /// `booted` is the runtime's own answer (adb saw Android finish booting).
    /// Files on disk are not readiness: with every component present but no
    /// verified boot the state is `Starting`, never `Ready`.
    pub fn report(&self, store: &Store, booted: bool) -> SetupReport {
        // The same resolution the runtime uses, so both cannot disagree about
        // which SDK this PC has.
        let root = sdk::active_root().unwrap_or_else(|| self.data_dir.join("android-sdk"));
        let managed = sdk::is_managed(&root);
        let configured = sdk::configured_root().is_some();
        let sdk = sdk::Sdk::at(root.clone());

        let tools = sdk.has_cmdline_tools();
        let platform_tools = sdk.has_platform_tools();
        let emu = sdk.has_emulator();
        let images = sdk.system_images();
        let image = images.iter().any(|found| found == SYSTEM_IMAGE);
        let avd = sdk::avd_exists(AVD_NAME);

        let mut components = vec![
            SetupComponent {
                id: "tools".into(),
                label: "Android command-line tools".into(),
                present: tools,
                detail: if tools {
                    sdk.sdkmanager().display().to_string()
                } else {
                    "Needed to install the rest; downloaded from Google's SDK repository.".into()
                },
                download_mb: (!tools).then_some(TOOLS_MB),
                // Installed on demand; an SDK that already has everything does
                // not need it, so its absence is not a blocking item.
                required: false,
            },
            SetupComponent {
                id: "platform-tools".into(),
                label: "Android platform-tools (adb)".into(),
                present: platform_tools,
                detail: if platform_tools {
                    sdk.adb().display().to_string()
                } else {
                    "adb talks to the emulator and installs packages.".into()
                },
                download_mb: (!platform_tools).then_some(PLATFORM_TOOLS_MB),
                required: true,
            },
            SetupComponent {
                id: "emulator".into(),
                label: "Android Emulator".into(),
                present: emu,
                detail: if emu {
                    sdk.emulator().display().to_string()
                } else {
                    "The emulator runs the Android system image.".into()
                },
                download_mb: (!emu).then_some(EMULATOR_MB),
                required: true,
            },
            SetupComponent {
                id: "system-image".into(),
                label: format!("Android {API_LEVEL} system image"),
                present: image,
                detail: if image {
                    SYSTEM_IMAGE.to_string()
                } else if images.is_empty() {
                    "No system image is installed.".into()
                } else {
                    format!("Installed: {}", images.join(", "))
                },
                download_mb: (!image).then_some(IMAGE_MB),
                required: true,
            },
            SetupComponent {
                id: "avd".into(),
                label: "Virtual device".into(),
                present: avd,
                detail: if avd {
                    format!("{AVD_NAME} in {}", sdk::avd_home().display())
                } else {
                    "Created from the system image once it is installed.".into()
                },
                download_mb: None,
                required: true,
            },
        ];
        if let Some(scrcpy) = sdk::scrcpy(&self.data_dir) {
            components.push(SetupComponent {
                id: "window-host".into(),
                label: "Window host (scrcpy)".into(),
                present: true,
                detail: scrcpy.display().to_string(),
                download_mb: None,
                required: true,
            });
        } else {
            components.push(SetupComponent {
                id: "window-host".into(),
                label: "Window host (scrcpy)".into(),
                present: false,
                detail: "Ships with DroidWin; reinstall the app to restore it.".into(),
                download_mb: None,
                required: true,
            });
        }

        let download_mb: u64 = components
            .iter()
            .filter_map(|component| component.download_mb)
            .map(u64::from)
            .sum::<u64>()
            // sdkmanager and avdmanager are Java programs. A PC with no JRE
            // downloads one, and the size belongs in the estimate.
            + if sdk::java_home(&root).is_none() {
                u64::from(JRE_MB)
            } else {
                0
            };
        let disk_free_mb = sdk::disk_free_mb_for_new(&root).unwrap_or(0);
        let licenses = store.setting(KEY_LICENSES).as_deref() == Some("1");
        let installed_at = store
            .setting(KEY_INSTALLED_AT)
            .and_then(|value| value.parse::<i64>().ok());
        let missing: Vec<String> = components
            .iter()
            .filter(|component| component.required && !component.present)
            .map(|component| component.label.clone())
            .collect();
        let error = self.last_error.lock().ok().and_then(|slot| slot.clone());

        let state = if self.is_running() {
            SetupState::Installing
        } else if missing.is_empty() && booted {
            SetupState::Ready
        } else if missing.is_empty() {
            SetupState::Starting
        } else if error.is_some() {
            SetupState::Failed
        } else {
            SetupState::NeedsInstall
        };

        // The image this build installs is x86_64. Windows on ARM needs the
        // arm64 emulator and image, which DroidWin does not fetch; saying so is
        // better than downloading 2.3 GB that cannot boot.
        let unsupported = if cfg!(target_arch = "x86_64") {
            None
        } else {
            Some(
                "This build installs the x86_64 Android image, which cannot run on this CPU. Windows on \
                 ARM is not supported by this version."
                    .to_string(),
            )
        };

        let message = match state {
            _ if unsupported.is_some() => unsupported.clone().unwrap_or_default(),
            SetupState::Ready => {
                if managed {
                    "DroidWin installed this Android runtime and is using it.".to_string()
                } else {
                    format!(
                        "Using the Android SDK already on this PC: {}",
                        root.display()
                    )
                }
            }
            SetupState::Installing => "Installing the Android runtime.".to_string(),
            SetupState::Starting => {
                "Android is installed. Starting the emulator; DroidWin calls the runtime ready \
                 only after adb sees Android finish booting."
                    .to_string()
            }
            SetupState::Failed => error
                .clone()
                .unwrap_or_else(|| "The last installation did not finish.".to_string()),
            SetupState::NeedsInstall => {
                let total = download_mb.max(1);
                if disk_free_mb > 0 && disk_free_mb < total + HEADROOM_MB {
                    format!(
                        "Missing: {}. Not enough free space: {:.1} GB free, about {:.1} GB needed.",
                        missing.join(", "),
                        disk_free_mb as f64 / 1024.0,
                        (total + HEADROOM_MB) as f64 / 1024.0
                    )
                } else {
                    format!(
                        "Missing: {}. About {:.1} GB to download.",
                        missing.join(", "),
                        total as f64 / 1024.0
                    )
                }
            }
        };

        SetupReport {
            state,
            sdk_root: root.display().to_string(),
            managed,
            configured,
            components,
            installed_at,
            error,
            disk_free_mb,
            download_mb,
            licenses_accepted: licenses,
            system_image: SYSTEM_IMAGE.to_string(),
            avd: AVD_NAME.to_string(),
            message,
        }
    }

    /// Install whatever is missing, then leave the SDK ready to boot.
    ///
    /// `consent` is the user's acceptance of the Android SDK licences. It is not
    /// a formality: the installer refuses to run without it, and the licence
    /// prompt is the SDK manager's own, not a click DroidWin invents.
    pub async fn run(&self, store: &Store, consent: bool) -> DwResult<()> {
        if self.running.swap(true, Ordering::SeqCst) {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "The Android runtime is already being installed.",
            ));
        }
        self.cancel.store(false, Ordering::SeqCst);
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = None;
        }
        let result = self.install(store, consent).await;
        self.running.store(false, Ordering::SeqCst);
        match &result {
            Ok(()) => {
                self.publish(SetupProgress {
                    step: "done".into(),
                    message: "The Android runtime is installed.".into(),
                    percent: Some(100.0),
                    done: true,
                    failed: None,
                });
            }
            Err(err) => {
                let message = err.summary.clone();
                // A cancellation is not a failure: the components that were
                // installed stay installed, and the next attempt continues from
                // what is on disk rather than reporting a broken setup.
                if err.code != ErrorCode::Cancelled {
                    if let Ok(mut slot) = self.last_error.lock() {
                        *slot = Some(match &err.detail {
                            Some(detail) if !detail.is_empty() => {
                                format!("{message} {}", detail.lines().next().unwrap_or(""))
                            }
                            _ => message.clone(),
                        });
                    }
                    fail(&self.sink, "failed", message);
                } else {
                    self.publish(SetupProgress {
                        step: "cancelled".into(),
                        message,
                        percent: None,
                        done: true,
                        failed: None,
                    });
                }
            }
        }
        result
    }

    async fn install(&self, store: &Store, consent: bool) -> DwResult<()> {
        if !cfg!(target_arch = "x86_64") {
            return Err(DwError::new(
                ErrorCode::NotSupported,
                "This build installs the x86_64 Android image, which cannot run on this CPU.",
            )
            .detail("Windows on ARM is not supported by this version."));
        }
        let sink = Arc::clone(&self.sink);
        let root = sdk::active_root().ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "DroidWin has no directory to install the Android runtime into.",
            )
        })?;
        ensure_writable_root(&root)?;

        emit(&sink, "prepare", "Checking disk space.", None);
        let free = sdk::disk_free_mb_for_new(&root).unwrap_or(0);
        if free > 0 && free < HEADROOM_MB {
            return Err(DwError::new(
                ErrorCode::InvalidConfig,
                format!(
                    "Not enough free space: {:.1} GB free, about {:.1} GB needed.",
                    free as f64 / 1024.0,
                    HEADROOM_MB as f64 / 1024.0
                ),
            )
            .detail(format!(
                "Free space on the volume holding {}",
                root.display()
            )));
        }
        std::fs::create_dir_all(&root).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "The Android SDK directory could not be created.",
                &e,
            )
        })?;
        let download_dir = root.join(".downloads");

        self.ensure_java(&root, &download_dir).await?;
        self.ensure_tools(&root, &download_dir).await?;

        if !consent {
            return Err(DwError::new(
                ErrorCode::InvalidRequest,
                "Android's SDK licences must be accepted before the runtime can be installed.",
            ));
        }
        self.accept_licenses(&root, store).await?;
        self.install_packages(&root).await?;
        self.ensure_virtual_device(&root).await?;

        emit(&sink, "validate", "Checking the installed runtime.", None);
        validate(&root)?;
        let _ = store.set_setting(KEY_INSTALLED_AT, &now_ms().to_string());
        // A finished bootstrap leaves 150 MB of archives behind that will never
        // be read again.
        let _ = std::fs::remove_dir_all(&download_dir);
        Ok(())
    }

    /// A JRE for `sdkmanager` and `avdmanager`, which are Java programs. Android
    /// Studio would normally provide one; a clean PC has none, so a Temurin JRE
    /// is installed beside the SDK and pointed at by `JAVA_HOME`.
    async fn ensure_java(&self, root: &Path, download_dir: &Path) -> DwResult<()> {
        if sdk::java_home(root).is_some() {
            return Ok(());
        }
        let sink = Arc::clone(&self.sink);
        emit(
            &sink,
            "tools",
            "Downloading a Java runtime for the Android SDK tools.",
            None,
        );
        let arch = match std::env::consts::ARCH {
            "aarch64" => "aarch64",
            _ => "x64",
        };
        let url = JRE_URL.replace("{arch}", arch);
        let zip = download_dir.join("jre.zip");
        download(
            &url,
            &zip,
            None,
            "tools",
            "Java runtime",
            &sink,
            &self.cancel,
        )
        .await?;
        let unpacked = download_dir.join("jre");
        let _ = std::fs::remove_dir_all(&unpacked);
        extract_zip(&zip, &unpacked, None)?;
        let jre = root.join("jre");
        let _ = std::fs::remove_dir_all(&jre);
        // The archive holds one top-level directory; flatten it so the JRE is
        // exactly `<sdk>/jre/bin/java.exe`.
        let inner = single_dir(&unpacked).unwrap_or(unpacked.clone());
        std::fs::rename(&inner, &jre).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "The Java runtime could not be moved into the SDK.",
                &e,
            )
        })?;
        let _ = std::fs::remove_dir_all(&unpacked);
        let _ = std::fs::remove_file(&zip);
        if !jre.join("bin").join(sdk::exe("java")).is_file() {
            return Err(DwError::new(
                ErrorCode::CorruptArchive,
                "The downloaded Java runtime does not contain bin/java.",
            )
            .detail(url));
        }
        Ok(())
    }

    /// The command-line tools. Reused when present; otherwise the newest Windows
    /// archive listed in Google's repository is downloaded and checked.
    async fn ensure_tools(&self, root: &Path, download_dir: &Path) -> DwResult<()> {
        let sdk = sdk::Sdk::at(root.to_path_buf());
        if sdk.has_cmdline_tools() {
            return Ok(());
        }
        let sink = Arc::clone(&self.sink);
        emit(
            &sink,
            "tools",
            "Reading Google's Android SDK repository.",
            None,
        );
        let xml = curl_text(REPOSITORY)?;
        let (version, archive) = cmdline_tools_archive(&xml).ok_or_else(|| {
            DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "Google's SDK repository did not list Windows command-line tools.",
            )
            .detail(REPOSITORY)
        })?;
        emit(
            &sink,
            "tools",
            format!("Downloading Android command-line tools {version}."),
            None,
        );
        let zip = download_dir.join("cmdline-tools.zip");
        download(
            &archive.url,
            &zip,
            archive.size,
            "tools",
            "command-line tools",
            &sink,
            &self.cancel,
        )
        .await?;
        if let Some(expected) = &archive.sha1 {
            emit(&sink, "tools", "Verifying the download.", None);
            verify_sha1(&zip, expected)?;
        }
        emit(&sink, "tools", "Unpacking the command-line tools.", None);
        let unpacked = download_dir.join("cmdline-tools");
        let _ = std::fs::remove_dir_all(&unpacked);
        extract_zip(&zip, &unpacked, Some("cmdline-tools"))?;
        let target = root.join("cmdline-tools").join("latest");
        let _ = std::fs::remove_dir_all(&target);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DwError::io(
                    ErrorCode::StorageFailure,
                    "The SDK's cmdline-tools directory could not be created.",
                    &e,
                )
            })?;
        }
        std::fs::rename(&unpacked, &target).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "The command-line tools could not be moved into the SDK.",
                &e,
            )
        })?;
        let _ = std::fs::remove_file(&zip);
        if !sdk.has_cmdline_tools() {
            return Err(DwError::new(
                ErrorCode::CorruptArchive,
                "The command-line tools archive did not contain bin/sdkmanager.bat.",
            )
            .detail(archive.url));
        }
        Ok(())
    }

    /// Accept the SDK licences through the SDK manager itself. The user already
    /// agreed on the setup screen; this is the tool recording it.
    async fn accept_licenses(&self, root: &Path, store: &Store) -> DwResult<()> {
        let sink = Arc::clone(&self.sink);
        emit(
            &sink,
            "licenses",
            "Recording Android SDK licence acceptance.",
            None,
        );
        let output =
            run_sdkmanager(root, &["--licenses"], 300, &sink, "licenses", &self.cancel).await?;
        if !output.ok {
            return Err(DwError::new(
                ErrorCode::RuntimeNotConfigured,
                "The Android SDK licence prompt did not complete.",
            )
            .detail(output.tail));
        }
        store.set_setting(KEY_LICENSES, "1")?;
        Ok(())
    }

    async fn install_packages(&self, root: &Path) -> DwResult<()> {
        // Package names contain semicolons, and the command-line tools 23.x
        // replaced the Java `sdkmanager` with a native `android` binary whose
        // `sdk install` takes the name as one argument. The deprecated shim in
        // that release splits `system-images;android-36;google_apis;x86_64` into
        // four packages and reports each as "not found" with exit code 0 — which
        // is why the native tool is preferred and the result is always verified
        // on disk afterwards.
        let sdk = sdk::Sdk::at(root.to_path_buf());
        let sink = Arc::clone(&self.sink);
        let jobs: Vec<(&str, bool)> = vec![
            ("platform-tools", sdk.has_platform_tools()),
            ("emulator", sdk.has_emulator()),
            (
                SYSTEM_IMAGE,
                sdk.system_images()
                    .iter()
                    .any(|image| image == SYSTEM_IMAGE),
            ),
        ];
        for (package, present) in jobs {
            if present {
                continue;
            }
            emit(&sink, "packages", format!("Installing {package}."), None);
            let output =
                run_install_package(root, package, 1800, &sink, "packages", &self.cancel).await?;
            if !output.ok {
                return Err(DwError::new(
                    ErrorCode::RuntimeNotConfigured,
                    format!("The Android SDK manager could not install {package}."),
                )
                .detail(output.tail));
            }
            let now = sdk::Sdk::at(root.to_path_buf());
            let installed = match package {
                "platform-tools" => now.has_platform_tools(),
                "emulator" => now.has_emulator(),
                _ => now
                    .system_images()
                    .iter()
                    .any(|image| image == SYSTEM_IMAGE),
            };
            if !installed {
                return Err(DwError::new(
                    ErrorCode::RuntimeNotConfigured,
                    format!("The SDK manager reported success, but {package} is not on disk."),
                ));
            }
        }
        Ok(())
    }

    async fn ensure_virtual_device(&self, root: &Path) -> DwResult<()> {
        if sdk::avd_exists(AVD_NAME) {
            return Ok(());
        }
        let sink = Arc::clone(&self.sink);
        emit(&sink, "avd", "Creating the Android virtual device.", None);
        let sdk = sdk::Sdk::at(root.to_path_buf());
        let config = emulator::AvdConfig {
            name: AVD_NAME.to_string(),
            system_image: SYSTEM_IMAGE.to_string(),
            port: 5554,
        };
        emulator::ensure_avd(&sdk, &config).await?;
        Ok(())
    }
}

/// The installation directory must be somewhere DroidWin may write: absolute,
/// not a drive root, not inside Windows. A path that fails here is refused
/// rather than attempted.
pub fn ensure_writable_root(root: &Path) -> DwResult<()> {
    if !root.is_absolute() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "The Android SDK directory must be an absolute path.",
        )
        .detail(root.display().to_string()));
    }
    if root.parent().is_none() {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "A drive root cannot be used as the Android SDK directory.",
        )
        .detail(root.display().to_string()));
    }
    let lowered = root.to_string_lossy().to_ascii_lowercase();
    let windows = std::env::var("SystemRoot").unwrap_or_else(|_| "c:\\windows".into());
    if lowered.starts_with(&windows.to_ascii_lowercase()) {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "The Android SDK cannot be installed inside the Windows directory.",
        )
        .detail(root.display().to_string()));
    }
    Ok(())
}

/// Every required component, checked on disk. Called after an install so a
/// partial package cannot be reported as ready.
pub fn validate(root: &Path) -> DwResult<()> {
    let sdk = sdk::Sdk::at(root.to_path_buf());
    let mut missing = Vec::new();
    if !sdk.has_platform_tools() {
        missing.push("platform-tools");
    }
    if !sdk.has_emulator() {
        missing.push("emulator");
    }
    if !sdk
        .system_images()
        .iter()
        .any(|image| image == SYSTEM_IMAGE)
    {
        missing.push(SYSTEM_IMAGE);
    }
    if !sdk::avd_exists(AVD_NAME) {
        missing.push("the DroidWin virtual device");
    }
    if missing.is_empty() {
        Ok(())
    } else {
        Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            format!("The runtime is still missing: {}.", missing.join(", ")),
        ))
    }
}

fn single_dir(dir: &Path) -> Option<PathBuf> {
    let mut entries = std::fs::read_dir(dir).ok()?.filter_map(|e| e.ok());
    let first = entries.next()?.path();
    if entries.next().is_some() {
        return None;
    }
    first.is_dir().then_some(first)
}

fn curl() -> String {
    if cfg!(windows) {
        "curl.exe".to_string()
    } else {
        "curl".to_string()
    }
}

fn run_curl(args: &[String]) -> DwResult<Vec<u8>> {
    let mut cmd = std::process::Command::new(curl());
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    proc::no_console(&mut cmd);
    let output = cmd.output().map_err(|e| {
        DwError::io(
            ErrorCode::RuntimeNotConfigured,
            "Windows' curl is needed to download the Android runtime.",
            &e,
        )
    })?;
    if !output.status.success() {
        return Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android download did not complete.",
        )
        .detail(String::from_utf8_lossy(&output.stderr).trim().to_string()));
    }
    Ok(output.stdout)
}

fn curl_text(url: &str) -> DwResult<String> {
    let bytes = run_curl(&[
        "-sS".into(),
        "-L".into(),
        "--fail".into(),
        "--max-time".into(),
        CONNECT_TIMEOUT.as_secs().to_string(),
        url.into(),
    ])?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

/// The server's own length for a URL, when it states one.
fn remote_size(url: &str) -> Option<u64> {
    let head = run_curl(&[
        "-sSI".into(),
        "-L".into(),
        "--max-time".into(),
        CONNECT_TIMEOUT.as_secs().to_string(),
        url.into(),
    ])
    .ok()?;
    let text = String::from_utf8_lossy(&head).to_ascii_lowercase();
    text.lines()
        .filter_map(|line| line.strip_prefix("content-length:"))
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .next_back()
}

/// Download `url` to `dest`, reporting real progress from the file's growing
/// length. Resumes a partial file once, then starts over: a server that ignores
/// a range request must not strand a half-written archive forever.
async fn download(
    url: &str,
    dest: &Path,
    expected: Option<u64>,
    step: &str,
    label: &str,
    sink: &Sink,
    cancel: &AtomicBool,
) -> DwResult<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "The download directory could not be created.",
                &e,
            )
        })?;
    }
    let part = dest.with_extension("part");
    let total = expected.or_else(|| remote_size(url));
    let mut resumed = part.is_file();
    let mut attempt = 0;
    loop {
        let mut args: Vec<String> = vec![
            "-L".into(),
            "--fail".into(),
            "--silent".into(),
            "--show-error".into(),
            "--max-time".into(),
            "3600".into(),
            "--output".into(),
            part.display().to_string(),
        ];
        if resumed {
            args.push("-C".into());
            args.push("-".into());
        }
        args.push(url.to_string());
        let mut cmd = Command::new(curl());
        cmd.args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        proc::no_console_async(&mut cmd);
        let mut child = cmd.spawn().map_err(|e| {
            DwError::io(
                ErrorCode::RuntimeNotConfigured,
                "Windows' curl could not start the download.",
                &e,
            )
        })?;
        let mut last_percent = u32::MAX;
        emit(sink, step, format!("Downloading {label}."), None);
        let status = loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = child.start_kill();
                let _ = child.wait().await;
                return Err(DwError::new(
                    ErrorCode::Cancelled,
                    "The installation was cancelled.",
                ));
            }
            if let Ok(Some(status)) = child.try_wait() {
                break Some(status);
            }
            let written = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
            if let Some(total) = total.filter(|total| *total > 0) {
                let percent = ((written.min(total) * 100) / total) as u32;
                if percent != last_percent {
                    last_percent = percent;
                    emit(
                        sink,
                        step,
                        format!(
                            "Downloading {label}: {:.0} of {:.0} MB.",
                            written as f64 / (1024.0 * 1024.0),
                            total as f64 / (1024.0 * 1024.0)
                        ),
                        Some(percent as f64),
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        };
        match status {
            Some(status) if status.success() => break,
            _ => {
                let mut detail = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = tokio::io::AsyncReadExt::read_to_string(&mut pipe, &mut detail).await;
                }
                if resumed && attempt == 0 {
                    attempt = 1;
                    resumed = false;
                    let _ = std::fs::remove_file(&part);
                    continue;
                }
                let _ = std::fs::remove_file(&part);
                return Err(DwError::new(
                    ErrorCode::RuntimeNotConfigured,
                    format!("The download of {label} failed."),
                )
                .detail(detail.trim().to_string()));
            }
        }
    }
    let written = std::fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
    if let Some(total) = total {
        if written != total {
            let _ = std::fs::remove_file(&part);
            return Err(DwError::new(
                ErrorCode::CorruptArchive,
                format!("The download of {label} is incomplete."),
            )
            .detail(format!("{written} of {total} bytes")));
        }
    }
    std::fs::rename(&part, dest).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "The downloaded archive could not be saved.",
            &e,
        )
    })?;
    Ok(())
}

fn verify_sha1(path: &Path, expected: &str) -> DwResult<()> {
    let mut file = std::fs::File::open(path).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "The downloaded archive could not be read.",
            &e,
        )
    })?;
    let mut hasher = Sha1::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "The downloaded archive could not be read.",
                &e,
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        let _ = std::fs::remove_file(path);
        Err(DwError::new(
            ErrorCode::CorruptArchive,
            "The downloaded command-line tools do not match Google's published checksum.",
        )
        .detail(format!("expected {expected}, got {actual}")))
    }
}

/// A zip entry name reduced to a path inside the destination, or `None` for
/// anything that could escape it. Absolute paths, drive letters and `..` are
/// dropped rather than sanitised into something surprising.
fn safe_relative(name: &str) -> Option<PathBuf> {
    let name = name.replace('\\', "/");
    if name.starts_with('/') || name.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in name.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return None;
        }
        out.push(part);
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// Extract a zip, dropping the first path component when it matches `strip`.
fn extract_zip(zip: &Path, dest: &Path, strip: Option<&str>) -> DwResult<()> {
    let file = std::fs::File::open(zip).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "The downloaded archive could not be opened.",
            &e,
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        DwError::new(
            ErrorCode::CorruptArchive,
            "The downloaded file is not a valid archive.",
        )
        .detail(e.to_string())
    })?;
    std::fs::create_dir_all(dest).map_err(|e| {
        DwError::io(
            ErrorCode::StorageFailure,
            "The archive destination could not be created.",
            &e,
        )
    })?;
    for index in 0..archive.len() {
        let mut entry = match archive.by_index(index) {
            Ok(entry) => entry,
            Err(err) => {
                return Err(DwError::new(
                    ErrorCode::CorruptArchive,
                    "An entry in the downloaded archive could not be read.",
                )
                .detail(err.to_string()))
            }
        };
        let Some(mut relative) = safe_relative(entry.name()) else {
            continue;
        };
        if let Some(strip) = strip {
            let mut parts = relative.components();
            if parts
                .next()
                .map(|part| part.as_os_str().to_string_lossy() == strip)
                == Some(true)
            {
                relative = parts.as_path().to_path_buf();
            }
        }
        if relative.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(&relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| {
                DwError::io(
                    ErrorCode::StorageFailure,
                    "An archive directory could not be created.",
                    &e,
                )
            })?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DwError::io(
                    ErrorCode::StorageFailure,
                    "An archive directory could not be created.",
                    &e,
                )
            })?;
        }
        let mut out = std::fs::File::create(&target).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "An archive file could not be written.",
                &e,
            )
        })?;
        std::io::copy(&mut entry, &mut out).map_err(|e| {
            DwError::io(
                ErrorCode::StorageFailure,
                "An archive file could not be extracted.",
                &e,
            )
        })?;
    }
    Ok(())
}

pub struct SdkManagerOutput {
    pub ok: bool,
    pub tail: String,
}

/// The native `android` CLI that ships with command-line tools 23 and later.
fn android_cli(root: &Path) -> Option<PathBuf> {
    let exe = root
        .join("cmdline-tools")
        .join("latest")
        .join("bin")
        .join(sdk::exe("android"));
    exe.is_file().then_some(exe)
}

/// Install one package, with the tool that can actually name it in one piece.
async fn run_install_package(
    root: &Path,
    package: &str,
    timeout_secs: u64,
    sink: &Sink,
    step: &str,
    cancel: &AtomicBool,
) -> DwResult<SdkManagerOutput> {
    match android_cli(root) {
        Some(cli) => {
            let mut cmd = Command::new(&cli);
            cmd.arg(format!("--sdk={}", root.display()))
                .args(["sdk", "install", package]);
            proc::no_console_async(&mut cmd);
            run_tool(cmd, sink, step, timeout_secs, cancel).await
        }
        None => {
            run_sdkmanager(
                root,
                &["--install", package],
                timeout_secs,
                sink,
                step,
                cancel,
            )
            .await
        }
    }
}

/// Run the SDK manager with Java pointed at the SDK's own JRE.
///
/// `y` lines answer its licence prompt; they are written only after the user's
/// consent reached this point. Progress lines are forwarded as they arrive —
/// the SDK manager is the only source of a real package download figure.
async fn run_sdkmanager(
    root: &Path,
    args: &[&str],
    timeout_secs: u64,
    sink: &Sink,
    step: &str,
    cancel: &AtomicBool,
) -> DwResult<SdkManagerOutput> {
    let manager = sdk::Sdk::at(root.to_path_buf()).sdkmanager();
    if !manager.is_file() {
        return Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android SDK manager is not installed.",
        )
        .detail(manager.display().to_string()));
    }
    // `cmd /c` mangles a quoted command path (it strips the outer quotes), so the
    // batch file is named relative to its own directory instead. This keeps
    // SDK paths with spaces working, which %USERPROFILE% often contains.
    let Some(bin) = manager.parent() else {
        return Err(DwError::new(
            ErrorCode::InvalidPath,
            "The Android SDK manager has no directory.",
        ));
    };
    // `cmd /c` looks the batch file up in the current directory only when
    // `NoDefaultCurrentDirectoryInExePath` is unset, and some Windows policies
    // set it. The explicit `.\` prefix works either way and keeps the path free
    // of quotes for `cmd /c` to mangle.
    let script = format!(
        ".\\{}",
        manager.file_name().unwrap_or_default().to_string_lossy()
    );
    let mut cmd = Command::new("cmd");
    cmd.current_dir(bin)
        .arg("/c")
        .arg(script)
        .args(args.iter().copied());
    sdk::apply_java(root, cmd.as_std_mut());
    proc::no_console_async(&mut cmd);
    run_tool(cmd, sink, step, timeout_secs, cancel).await
}

/// Run a tool to completion, forwarding its progress lines and keeping the end
/// of its output for an error message.
async fn run_tool(
    mut cmd: Command,
    sink: &Sink,
    step: &str,
    timeout_secs: u64,
    cancel: &AtomicBool,
) -> DwResult<SdkManagerOutput> {
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    proc::no_console_async(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
        DwError::io(
            ErrorCode::RuntimeStartFailed,
            "The Android SDK manager could not start.",
            &e,
        )
    })?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin
            .write_all(b"y\ny\ny\ny\ny\ny\ny\ny\ny\ny\ny\ny\n")
            .await;
        let _ = stdin.flush().await;
    }

    let tail = Arc::new(Mutex::new(Vec::<String>::new()));
    let percent = Arc::new(Mutex::new(f64::NAN));
    let mut readers: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    if let Some(out) = child.stdout.take() {
        readers.push(tokio::spawn(pump(
            out,
            Arc::clone(sink),
            step.to_string(),
            Arc::clone(&tail),
            Arc::clone(&percent),
        )));
    }
    if let Some(err) = child.stderr.take() {
        readers.push(tokio::spawn(pump(
            err,
            Arc::clone(sink),
            step.to_string(),
            Arc::clone(&tail),
            Arc::clone(&percent),
        )));
    }

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let status = loop {
        if cancel.load(Ordering::SeqCst) {
            let _ = child.start_kill();
            let _ = child.wait().await;
            for handle in &readers {
                handle.abort();
            }
            return Err(DwError::new(
                ErrorCode::Cancelled,
                "The installation was cancelled.",
            ));
        }
        if let Ok(Some(status)) = child.try_wait() {
            break Some(status);
        }
        if Instant::now() > deadline {
            let _ = child.start_kill();
            let _ = child.wait().await;
            for handle in &readers {
                handle.abort();
            }
            return Err(DwError::new(
                ErrorCode::Timeout,
                format!("The Android SDK manager did not finish within {timeout_secs}s."),
            ));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    // The pipes close when the process exits; wait for the last lines.
    for handle in readers {
        let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
    }
    let code = status.and_then(|status| status.code());
    let text = tail
        .lock()
        .map(|lines| lines.join("\n"))
        .unwrap_or_default();
    Ok(SdkManagerOutput {
        ok: code == Some(0),
        tail: if text.len() > 4000 {
            text[text.len() - 4000..].to_string()
        } else {
            text
        },
    })
}

async fn pump<R>(
    reader: R,
    sink: Sink,
    step: String,
    tail: Arc<Mutex<Vec<String>>>,
    percent: Arc<Mutex<f64>>,
) where
    R: tokio::io::AsyncRead + Unpin,
{
    // Both line endings are separators: the SDK tools redraw their progress
    // bar with a bare `\r`, and `lines()` waits for `\n`, so the one number the
    // download screen could show never reached it.
    let mut reader = BufReader::new(reader);
    let mut pending: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let read = match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        for &byte in &chunk[..read] {
            if byte == b'\n' || byte == b'\r' {
                report_line(&pending, &sink, &step, &tail, &percent);
                pending.clear();
            } else {
                pending.push(byte);
            }
        }
    }
    report_line(&pending, &sink, &step, &tail, &percent);
}

/// One line from a tool: keep it for an error message, and forward it when it
/// carries a percentage the screen has not shown yet.
fn report_line(
    bytes: &[u8],
    sink: &Sink,
    step: &str,
    tail: &Arc<Mutex<Vec<String>>>,
    percent: &Arc<Mutex<f64>>,
) {
    let line = String::from_utf8_lossy(bytes);
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }
    if let Ok(mut recent) = tail.lock() {
        recent.push(trimmed.to_string());
        if recent.len() > 40 {
            recent.remove(0);
        }
    }
    let Some(value) = percent_in(trimmed) else {
        return;
    };
    let changed = match percent.lock() {
        Ok(mut last) => {
            if (*last - value).abs() < f64::EPSILON {
                false
            } else {
                *last = value;
                true
            }
        }
        Err(_) => false,
    };
    if changed {
        emit(sink, step, trimmed.to_string(), Some(value));
    }
}

/// The percentage in an SDK manager progress line, if it states one.
pub fn percent_in(line: &str) -> Option<f64> {
    let index = line.find('%')?;
    let digits: String = line[..index]
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let value: f64 = digits.parse().ok()?;
    (0.0..=100.0).contains(&value).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPO: &str = r#"
  <remotePackage path="cmdline-tools;22.0">
    <revision><major>22</major><minor>0</minor></revision>
    <archives>
      <archive>
        <complete><size>170000000</size><checksum type="sha1">aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa</checksum><url>commandlinetools-linux-1_latest.zip</url></complete>
        <host-os>linux</host-os>
      </archive>
      <archive>
        <complete><size>150000000</size><checksum type="sha1">bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb</checksum><url>commandlinetools-win-1_latest.zip</url></complete>
        <host-os>windows</host-os>
      </archive>
    </archives>
  </remotePackage>
  <remotePackage path="cmdline-tools;23.0">
    <revision><major>23</major><minor>0</minor></revision>
    <archives>
      <archive>
        <complete><size>154957218</size><checksum type="sha1">57d04f2d75eb8e8fffc5000a987e5de4b5a63e9d</checksum><url>commandlinetools-win-16111833_latest.zip</url></complete>
        <host-os>windows</host-os>
      </archive>
    </archives>
  </remotePackage>
"#;

    #[test]
    fn the_newest_windows_command_line_tools_are_chosen() {
        let (version, archive) = cmdline_tools_archive(REPO).unwrap();
        assert_eq!(version, "23.0");
        assert_eq!(
            archive.url,
            "https://dl.google.com/android/repository/commandlinetools-win-16111833_latest.zip"
        );
        assert_eq!(archive.size, Some(154957218));
        assert_eq!(
            archive.sha1.as_deref(),
            Some("57d04f2d75eb8e8fffc5000a987e5de4b5a63e9d")
        );
    }

    #[test]
    fn a_repository_without_windows_tools_is_refused() {
        assert!(cmdline_tools_archive(
            "<remotePackage path=\"cmdline-tools;1.0\"></remotePackage>"
        )
        .is_none());
        assert!(cmdline_tools_archive("").is_none());
    }

    #[tokio::test]
    async fn carriage_return_progress_reaches_the_screen() {
        // What the SDK tools actually write while downloading: one redrawn bar
        // per update, separated by `\r`, never a newline until the step ends.
        let events: Arc<Mutex<Vec<SetupProgress>>> = Arc::new(Mutex::new(Vec::new()));
        let sink: Sink = {
            let events = Arc::clone(&events);
            Arc::new(move |progress: SetupProgress| {
                events.lock().unwrap().push(progress);
            })
        };
        let input = b"[==   ] 10% Downloading\r[==== ] 40% Downloading\r[=====] 100% Unzipping\r";
        pump(
            &input[..],
            sink,
            "image".into(),
            Arc::new(Mutex::new(Vec::new())),
            Arc::new(Mutex::new(f64::NAN)),
        )
        .await;
        let percents: Vec<Option<f64>> = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.percent)
            .collect();
        assert_eq!(percents, vec![Some(10.0), Some(40.0), Some(100.0)]);
    }

    #[test]
    fn sdk_manager_progress_is_read_only_from_a_real_percentage() {
        assert_eq!(percent_in("[=====   ] 45% Downloading"), Some(45.0));
        assert_eq!(percent_in("100% Unzipping..."), Some(100.0));
        assert_eq!(percent_in("Warning: 2 files skipped"), None);
        assert_eq!(percent_in("Downloading 1000000 bytes"), None);
    }

    #[test]
    fn archive_entries_cannot_leave_the_destination() {
        assert_eq!(
            safe_relative("cmdline-tools/bin/x.bat"),
            Some(PathBuf::from("cmdline-tools/bin/x.bat"))
        );
        assert_eq!(
            safe_relative("cmdline-tools\\bin\\x.bat"),
            Some(PathBuf::from("cmdline-tools/bin/x.bat"))
        );
        assert!(safe_relative("../escape.txt").is_none());
        assert!(safe_relative("C:/Windows/evil.txt").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("").is_none());
        assert!(safe_relative("./").is_none());
    }

    #[test]
    fn a_drive_root_is_not_an_install_directory() {
        assert!(ensure_writable_root(Path::new("C:\\")).is_err());
        assert!(ensure_writable_root(Path::new("relative/sdk")).is_err());
        assert!(ensure_writable_root(Path::new(
            "C:\\Users\\player\\AppData\\Local\\droidwin\\android-sdk"
        ))
        .is_ok());
    }

    #[test]
    fn a_checksum_mismatch_removes_the_download() {
        let path = std::env::temp_dir().join(format!("droidwin-sha-{}.zip", std::process::id()));
        std::fs::write(&path, b"not the archive").unwrap();
        let error = verify_sha1(&path, "0000000000000000000000000000000000000000").unwrap_err();
        assert_eq!(error.code, ErrorCode::CorruptArchive);
        assert!(!path.exists());
    }

    #[test]
    fn the_real_checksum_is_accepted() {
        let path = std::env::temp_dir().join(format!("droidwin-sha-ok-{}.txt", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        // sha1("abc")
        assert!(verify_sha1(&path, "a9993e364706816aba3e25717850c26c9cd0d89d").is_ok());
        let _ = std::fs::remove_file(&path);
    }
}
