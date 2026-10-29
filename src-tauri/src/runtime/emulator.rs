//! Headless Android Emulator lifecycle. The emulator draws nothing itself here —
//! app windows are produced by scrcpy against virtual displays — so it runs with
//! `-no-window` and is driven entirely through adb.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};

use super::adb::Adb;
use super::hardware::HardwarePlan;
use super::sdk::{self, Sdk};
use crate::domain::{DwError, DwResult, ErrorCode};
use crate::proc;

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
    } // A .bat needs cmd.exe; arguments are separate so no shell interpolation happens.
    let mut cmd = Command::new("cmd");
    // avdmanager is a Java program: the SDK's own JRE has to be visible to it.
    sdk::apply_java(sdk.root(), cmd.as_std_mut());
    // Named relative to its directory: `cmd /c` strips the outer quotes of a
    // quoted command path, which breaks any SDK directory containing a space.
    if let Some(bin) = manager.parent() {
        cmd.current_dir(bin);
    }
    // `.\` rather than a bare name: cmd only searches the current directory when
    // `NoDefaultCurrentDirectoryInExePath` is unset, and a quoted full path is
    // stripped by `cmd /c` when it contains a space.
    cmd.arg("/c")
        .arg(format!(
            ".\\{}",
            manager.file_name().unwrap_or_default().to_string_lossy()
        ))
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
        .kill_on_drop(true);
    // avdmanager is a .bat run through cmd.exe: hidden, never a console flash.
    proc::no_console_async(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| {
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

/// The emulator's argument list. Split out so the hardware plan is testable
/// without a machine of any particular size.
pub fn launch_args(cfg: &AvdConfig, plan: &HardwarePlan) -> Vec<String> {
    vec![
        "-avd".into(),
        cfg.name.clone(),
        "-no-window".into(),
        // Audio is left on: the emulator plays Android's own output on the
        // host's sound device, so an app's audio is audible while its window is
        // open. `-no-audio` was a leftover from the headless proof runs; it
        // silenced the whole runtime, not just a window.
        "-no-boot-anim".into(),
        // `auto` silently produced SwiftShader on this emulator build; the plan
        // chooses the renderer explicitly so the choice is visible and testable.
        "-gpu".into(),
        plan.gpu.as_arg().into(),
        "-cores".into(),
        plan.cores.to_string(),
        "-memory".into(),
        plan.ram_mb.to_string(),
        "-no-snapshot".into(),
        "-netdelay".into(),
        "none".into(),
        "-netspeed".into(),
        "full".into(),
        "-port".into(),
        cfg.port.to_string(),
    ]
}

/// Start the emulator headless and hand back the child so it can be stopped.
/// Its stderr goes to `log`: the emulator is chatty, and an unread pipe would
/// eventually fill and block the VM instead of merely losing diagnostics.
pub fn spawn(sdk: &Sdk, cfg: &AvdConfig, log: &Path, plan: &HardwarePlan) -> DwResult<Child> {
    let exe = sdk.emulator();
    if !exe.is_file() {
        return Err(DwError::new(
            ErrorCode::RuntimeNotConfigured,
            "The Android Emulator is not installed in your SDK.",
        )
        .detail("Install it with sdkmanager \"emulator\""));
    }
    let mut cmd = Command::new(&exe);
    cmd.args(launch_args(cfg, plan))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(emulator_log(log))
        .kill_on_drop(false);
    proc::no_console_async(&mut cmd);
    tracing::info!(
        "starting emulator: {} vCPUs, {} MB, GPU {}",
        plan.cores,
        plan.ram_mb,
        plan.gpu.as_arg()
    );

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

/// Append emulator output to a log file; fall back to discarding it if the file
/// cannot be opened, because that must not stop the runtime from starting.
fn emulator_log(path: &Path) -> Stdio {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map(Stdio::from)
        .unwrap_or_else(|_| Stdio::null())
}

/// Terminate an emulator and every process it started. `emulator.exe` launches
/// qemu as a child, so killing only the launcher leaves a VM holding gigabytes.
pub fn kill_tree(pid: u32) {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    proc::no_console(&mut cmd);
    let _ = cmd.status();
    tracing::info!("terminated emulator process tree at pid {pid}");
}

/// Wait for a real boot: adb sees the device, and Android reports boot completed.
/// One `getprop` dump per attempt, not three adb invocations.
pub async fn wait_boot(adb: &Adb, serial: &str, budget: Duration) -> DwResult<u32> {
    let started = Instant::now();
    let mut attempts = 0u32;
    let mut last: String;
    loop {
        attempts += 1;
        match adb.readiness(serial).await {
            Ok(ready) if ready.booted => {
                tracing::info!(
                    "{serial} booted after {:.1}s (api {:?})",
                    started.elapsed().as_secs_f32(),
                    ready.api
                );
                return ready.api.ok_or_else(|| {
                    DwError::new(
                        ErrorCode::RuntimeStartFailed,
                        "The runtime booted but reported no API level.",
                    )
                });
            }
            Ok(ready) => {
                last = format!(
                    "device present, boot_completed not set (api {:?})",
                    ready.api
                )
            }
            Err(err) => last = err.summary.clone(),
        }
        if started.elapsed() > budget {
            return Err(DwError::new(
                ErrorCode::Timeout,
                format!(
                    "The runtime did not finish booting within {}s.",
                    budget.as_secs()
                ),
            )
            .detail(format!("{attempts} probes; last state: {last}")));
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::hardware::{GpuMode, HardwarePlan, PlanMode};

    fn plan(cores: u32, ram_mb: u32, gpu: GpuMode) -> HardwarePlan {
        HardwarePlan {
            mode: PlanMode::Auto,
            cores,
            ram_mb,
            gpu,
            reason: String::new(),
            preset: None,
        }
    }

    fn config() -> AvdConfig {
        AvdConfig {
            name: "DroidWin_API36".into(),
            system_image: "system-images;android-36;google_apis;x86_64".into(),
            port: 5554,
        }
    }

    #[test]
    fn the_emulator_is_started_with_audio_enabled() {
        // Android's sound has to leave the emulator to be heard: the flag that
        // silenced the whole runtime must not come back.
        let args = launch_args(&config(), &plan(4, 4096, GpuMode::Host));
        assert!(!args.iter().any(|arg| arg == "-no-audio"));
    }

    #[test]
    fn the_hardware_plan_reaches_the_emulator_as_flags() {
        let args = launch_args(&config(), &plan(6, 4096, GpuMode::Host));
        let joined = args.join(" ");
        assert!(joined.contains("-cores 6"));
        assert!(joined.contains("-memory 4096"));
        assert!(joined.contains("-gpu host"));
        // Values stay separate argv entries: no shell parsing, no quoting bug.
        let gpu_at = args.iter().position(|a| a == "-gpu").unwrap();
        assert_eq!(args[gpu_at + 1], "host");
        let cores_at = args.iter().position(|a| a == "-cores").unwrap();
        assert_eq!(args[cores_at + 1], "6");
    }

    #[test]
    fn software_rendering_remains_available_as_a_fallback() {
        let args = launch_args(&config(), &plan(4, 2048, GpuMode::Software));
        let gpu_at = args.iter().position(|a| a == "-gpu").unwrap();
        assert_eq!(args[gpu_at + 1], "swiftshader_indirect");
    }

    #[test]
    fn the_emulator_still_runs_headless() {
        let args = launch_args(&config(), &plan(4, 2048, GpuMode::Host));
        assert!(args.contains(&"-no-window".to_string()));
        assert!(args.iter().any(|a| a == "-no-snapshot"));
    }
}
