//! Headless Android Emulator lifecycle. The emulator draws nothing itself here —
//! app windows are produced by scrcpy against virtual displays — so it runs with
//! `-no-window` and is driven entirely through adb.

use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

use super::adb::Adb;
use super::sdk::{self, Sdk};
use crate::domain::{DwError, DwResult, ErrorCode};

#[derive(Debug, Clone)]
pub struct AvdConfig {
    pub name: String,
    pub system_image: String,
    pub port: u16,
}

impl AvdConfig {
    pub fn serial(&self) -> String {
        format!("emulator-{}", self.port)
    }
}

/// Create the AVD when it is absent. Returns true when it had to be created.
pub async fn ensure_avd(sdk: &Sdk, cfg: &AvdConfig) -> DwResult<bool> {
    if sdk::avd_exists(&cfg.name) {
        return Ok(false);
    }
    let manager = sdk.avdmanager();
    if !manager.is_file() {
        return Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The SDK's avdmanager was not found, so DroidWin cannot create its virtual device.",
        )
        .detail(format!("Expected {}", manager.display())));
    }

    // A .bat needs cmd.exe; arguments are separate so no shell interpolation happens.
    let mut child = Command::new("cmd")
        .arg("/c")
        .arg(&manager)
        .args([
            "create",
            "avd",
            "-n",
            &cfg.name,
            "-k",
            &cfg.system_image,
            "-d",
            "pixel_6",
            "--force",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            DwError::io(
                ErrorCode::RuntimeStartFailed,
                "avdmanager could not start.",
                &e,
            )
        })?;

    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        // avdmanager asks whether to customise the hardware profile.
        let _ = stdin.write_all(b"no\n").await;
        let _ = stdin.flush().await;
    }

    let output =
        match tokio::time::timeout(Duration::from_secs(180), child.wait_with_output()).await {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                return Err(DwError::io(
                    ErrorCode::RuntimeStartFailed,
                    "avdmanager did not finish.",
                    &e,
                ))
            }
            Err(_) => {
                return Err(DwError::new(
                    ErrorCode::Timeout,
                    "avdmanager did not answer within 180s.",
                ))
            }
        };

    let code = output.status.code();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if code == Some(0) && sdk::avd_exists(&cfg.name) {
        tracing::info!("created AVD {} from {}", cfg.name, cfg.system_image);
        Ok(true)
    } else {
        Err(DwError::new(
            ErrorCode::RuntimeStartFailed,
            "DroidWin could not create its virtual device.",
        )
        .detail(format!(
            "avdmanager exit {code:?}\n{}\n{}",
            stdout.trim(),
            stderr.trim()
        )))
    }
}

/// Start the emulator headless and hand back the child so it can be stopped.
pub fn spawn(sdk: &Sdk, cfg: &AvdConfig) -> DwResult<Child> {
    let exe = sdk.emulator();
    if !exe.is_file() {
        return Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android Emulator is not installed in your SDK.",
        )
        .detail("Install it with sdkmanager \"emulator\""));
    }
    let mut cmd = Command::new(&exe);
    cmd.args([
        "-avd",
        &cfg.name,
        "-no-window",
        "-no-audio",
        "-no-boot-anim",
        "-gpu",
        "auto",
        "-no-snapshot-save",
        "-netdelay",
        "none",
        "-netspeed",
        "full",
        "-port",
        &cfg.port.to_string(),
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::piped())
    .kill_on_drop(false);

    let child = cmd.spawn().map_err(|e| {
        DwError::io(
            ErrorCode::RuntimeStartFailed,
            "The emulator could not start.",
            &e,
        )
    })?;
    tracing::info!("emulator spawned for {} (pid {:?})", cfg.name, child.id());
    Ok(child)
}

/// Terminate an emulator and every process it started. `emulator.exe` launches
/// qemu as a child, so killing only the launcher leaves a VM holding gigabytes.
pub fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    tracing::info!("terminated emulator process tree at pid {pid}");
}

/// Wait for a real boot: adb sees the device, and Android reports boot completed.
pub async fn wait_boot(adb: &Adb, serial: &str, budget: Duration) -> DwResult<u32> {
    let started = Instant::now();
    let mut attempts = 0u32;
    loop {
        attempts += 1;
        if adb.connect_ready(serial).await && adb.boot_completed(serial).await {
            let api = adb.api_level(serial).await;
            tracing::info!(
                "{serial} booted after {:.1}s (api {api:?})",
                started.elapsed().as_secs_f32()
            );
            return api.ok_or_else(|| {
                DwError::new(
                    ErrorCode::RuntimeStartFailed,
                    "The runtime booted but reported no API level.",
                )
            });
        }
        if started.elapsed() > budget {
            return Err(DwError::new(
                ErrorCode::Timeout,
                format!(
                    "The runtime did not finish booting within {}s.",
                    budget.as_secs()
                ),
            )
            .detail(format!("{attempts} probes; last state could not be read")));
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}
