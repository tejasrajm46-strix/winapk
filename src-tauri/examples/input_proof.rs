//! Keyboard-mapping proof on a real runtime.
//!
//!   cargo run --example input_proof -- <apk> [--key J] [--x 50] [--y 80]
//!
//! Opens the package through DroidWin's own runtime, binds one physical key to a
//! touch point on that app's display, then presses that key at the OS level while
//! the app window has focus. Screenshots of the app's display are taken before,
//! after the mapped press, and after the same press with mapping switched off.
//!
//! Nothing here fakes the pipeline: the key goes through the Windows hook, the
//! store, and the runtime's adb input path.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::apk;
use droidwin_lib::domain::{RuntimeStatus, StageEvent};
use droidwin_lib::input::{InputAction, InputController};
use droidwin_lib::runtime::sdk::Sdk;
use droidwin_lib::runtime::{EmulatorRuntime, RuntimeEvents};
use droidwin_lib::store::Store;

struct PrintEvents;

impl RuntimeEvents for PrintEvents {
    fn stage(&self, event: &StageEvent) {
        println!("    [{:?}] {}", event.stage, event.message);
    }
    fn status(&self, status: &RuntimeStatus) {
        println!(
            "    runtime: {} — {}",
            status.state.as_label(),
            status.detail
        );
    }
    fn apps_changed(&self) {}
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("input proof failed: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut apk_path: Option<PathBuf> = None;
    let mut key = "J".to_string();
    let mut x = 50.0f32;
    let mut y = 80.0f32;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--key" => key = args.next().unwrap_or(key),
            "--x" => x = args.next().and_then(|v| v.parse().ok()).unwrap_or(x),
            "--y" => y = args.next().and_then(|v| v.parse().ok()).unwrap_or(y),
            other => apk_path = Some(PathBuf::from(other)),
        }
    }
    let apk_path = apk_path.ok_or("pass one .apk path")?;

    let home = std::env::var("DROIDWIN_PROOF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".droidwin").join("home"));
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let _guard = droidwin_lib::diag::init(&home.join("logs"));
    let store = Arc::new(Store::open(&home.join("droidwin.db")).map_err(|e| e.summary)?);
    let runtime = EmulatorRuntime::new(home.clone(), Arc::new(PrintEvents));
    let runtime = Arc::new(runtime);
    let input = InputController::new(Arc::clone(&runtime), Arc::clone(&store));

    runtime
        .ensure_ready(&store, "input-proof")
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    let adb = runtime
        .adb()
        .ok_or_else(|| "adb is unavailable".to_string())?;
    let serial = runtime.serial();
    let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
    let evidence = PathBuf::from(".droidwin").join("evidence");
    std::fs::create_dir_all(&evidence).map_err(|e| e.to_string())?;

    let inspection = apk::inspect(&apk_path, aapt2.as_deref()).map_err(|e| e.summary)?;
    let package = inspection.package_id.clone();
    let label = inspection.label.clone().unwrap_or_else(|| package.clone());
    let allow_old = inspection.target_sdk.is_some_and(|sdk| sdk < 23);
    runtime
        .install(&inspection, &store, "input-proof", allow_old)
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;

    let open = |label: &str| {
        let runtime = Arc::clone(&runtime);
        let store = Arc::clone(&store);
        let package = package.clone();
        let label = label.to_string();
        async move {
            runtime
                .launch(&package, Some(&label), &store, "input-proof")
                .await
                .map_err(|e| format!("{} {:?}", e.summary, e.detail))
        }
    };
    let window = open(&label).await?;
    let surface = wait_surface(&runtime, &package).await?;
    let display_id = surface
        .display_id
        .ok_or_else(|| "Android never reported a display for this app".to_string())?;
    println!(
        "surface: display {display_id}, {:?}, screen {:?}",
        surface.geometry,
        surface.screen_size()
    );

    sleep(2.0).await;
    let before = shoot(&adb, &serial, display_id, &evidence, "input-01-before").await?;
    println!("start screen: {}", before.display());

    // Bind a key to a point on this app's display and switch mapping on.
    let vk = key
        .to_ascii_uppercase()
        .chars()
        .next()
        .map(|c| c as u32)
        .unwrap();
    input
        .save(&package, vk, InputAction::TouchTap { x, y })
        .map_err(|e| e.summary)?;
    input.set_enabled(&package, true).map_err(|e| e.summary)?;
    println!("bound VK {vk:#04X} to a tap at {x}% / {y}%, mapping enabled");
    tokio::time::sleep(Duration::from_millis(400)).await;

    press_key(&label, window.pid, &key)?;
    sleep(3.0).await;
    let mapped = shoot(&adb, &serial, display_id, &evidence, "input-02-mapped").await?;
    println!("after mapped press: {}", mapped.display());

    // Switch mapping off, reopen the app so its start screen is back, and press
    // the same key again. Nothing should be tapped this time.
    input.set_enabled(&package, false).map_err(|e| e.summary)?;
    runtime
        .close_window(&package)
        .await
        .map_err(|e| e.summary)?;
    sleep(1.0).await;
    let window = open(&label).await?;
    let surface = wait_surface(&runtime, &package).await?;
    let display_id = surface
        .display_id
        .ok_or_else(|| "Android never reported a display for this app".to_string())?;
    sleep(2.0).await;
    let reset = shoot(&adb, &serial, display_id, &evidence, "input-03-mapping-off").await?;
    press_key(&label, window.pid, &key)?;
    sleep(2.5).await;
    let inert = shoot(&adb, &serial, display_id, &evidence, "input-04-key-inert").await?;
    println!("mapping off: {} then {}", reset.display(), inert.display());

    input.release_all();
    runtime.shutdown().await;
    Ok(())
}

async fn wait_surface(
    runtime: &Arc<EmulatorRuntime>,
    package: &str,
) -> Result<droidwin_lib::runtime::emulator_runtime::AppSurface, String> {
    for _ in 0..20 {
        if let Some(surface) = runtime.geometry_of(package) {
            if surface.display_id.is_some() {
                return Ok(surface);
            }
        }
        sleep(0.25).await;
    }
    Err("the app window never resolved to an Android display".into())
}

async fn shoot(
    adb: &droidwin_lib::runtime::adb::Adb,
    serial: &str,
    display_id: u32,
    evidence: &std::path::Path,
    name: &str,
) -> Result<PathBuf, String> {
    let png = adb
        .screenshot_png(serial, display_id)
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    let file = evidence.join(format!("{name}.png"));
    std::fs::write(&file, png).map_err(|e| e.to_string())?;
    Ok(file)
}

async fn sleep(seconds: f64) {
    tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
}

/// Focus the app's own window and press the key through the OS, exactly as a
/// keyboard would. PowerShell's SendKeys goes through the same input path, so the
/// hook sees the press it would see from hardware — and because a press into a
/// window that never got focus would prove nothing, the script checks which
/// process owns the foreground window and fails loudly if it is not the app.
fn press_key(label: &str, pid: u32, key: &str) -> Result<(), String> {
    let lower = key.to_ascii_lowercase();
    let script = format!(
        r#"$p = Get-Process -Id {pid} -ErrorAction SilentlyContinue
if (-not $p) {{ Write-Error "scrcpy process {pid} is gone"; exit 1 }}
Add-Type -Namespace W -Name U -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetForegroundWindow(System.IntPtr h);
[DllImport("user32.dll")] public static extern bool ShowWindow(System.IntPtr h, int c);
[DllImport("user32.dll")] public static extern System.IntPtr GetForegroundWindow();
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(System.IntPtr h, out uint pid);
'@
$h = $p.MainWindowHandle
[void][W.U]::ShowWindow($h, 9)
[void][W.U]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 700
$fg = [W.U]::GetForegroundWindow()
$fgpid = 0
[void][W.U]::GetWindowThreadProcessId($fg, [ref]$fgpid)
if ($fgpid -ne {pid}) {{ Write-Error ("focus is on process " + $fgpid + ", not " + {pid} + " (" + $p.MainWindowTitle + ")"); exit 1 }}
Add-Type -AssemblyName System.Windows.Forms
[System.Windows.Forms.SendKeys]::SendWait("{lower}")
Start-Sleep -Milliseconds 250
Write-Output ("pressed '" + "{lower}" + "' into pid " + $fgpid + " (" + $p.MainWindowTitle + ")")"#
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "could not press the key: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    println!("  {label}: {}", String::from_utf8_lossy(&out.stdout).trim());
    Ok(())
}
