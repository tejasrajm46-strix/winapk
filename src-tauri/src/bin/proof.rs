//! Runtime feasibility harness (Gate A + Gate B).
//!
//! Uses the same modules the desktop app uses — not a parallel implementation —
//! so a passing run is evidence about the product, not about a demo.
//!
//!   cargo run --bin proof -- [testdata dir]
//!
//! Writes `.droidwin/home/proof-report.json` and prints the same facts.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use droidwin_lib::apk;
use droidwin_lib::domain::{now_ms, StageEvent};
use droidwin_lib::runtime::sdk::Sdk;
use droidwin_lib::runtime::{EmulatorRuntime, RuntimeEvents};
use droidwin_lib::store::Store;

struct PrintEvents;

impl RuntimeEvents for PrintEvents {
    fn stage(&self, event: &StageEvent) {
        println!("    [{:?}] {}", event.stage, event.message);
    }
    fn status(&self, status: &droidwin_lib::domain::RuntimeStatus) {
        println!(
            "    runtime: {} — {}",
            status.state.as_label(),
            status.detail
        );
    }
    fn apps_changed(&self) {}
}

#[derive(Debug, Default, Serialize)]
struct Report {
    started_at: i64,
    finished_at: Option<i64>,
    sdk_root: Option<String>,
    emulator: Option<String>,
    accel: Option<String>,
    scrcpy: Option<String>,
    scrcpy_version: Option<String>,
    system_image: Option<String>,
    avd: Option<String>,
    runtime_ready_ms: Option<u128>,
    android_api: Option<u32>,
    packages: Vec<PackageResult>,
    windows: Vec<WindowRow>,
    checks: Vec<Check>,
    gate_a: bool,
    gate_b: bool,
    notes: Vec<String>,
}

#[derive(Debug, Serialize)]
struct PackageResult {
    file: String,
    package_id: String,
    label: Option<String>,
    version_name: Option<String>,
    min_sdk: Option<u32>,
    abis: Vec<String>,
    install_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowRow {
    pid: u32,
    hwnd: i64,
    title: String,
}

#[derive(Debug, Serialize)]
struct Check {
    name: String,
    passed: bool,
    detail: String,
}

impl Check {
    fn new(name: &str, passed: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            passed,
            detail: detail.into(),
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("proof failed: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let home = PathBuf::from(".droidwin").join("home");
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let _guard = droidwin_lib::diag::init(&home.join("logs"));

    let store = Arc::new(Store::open(&home.join("droidwin.db")).map_err(|e| e.summary)?);
    let runtime = Arc::new(EmulatorRuntime::new(home.clone(), Arc::new(PrintEvents)));

    let mut report = Report {
        started_at: now_ms(),
        ..Default::default()
    };

    println!("== DroidWin runtime proof ==");
    let env = runtime.probe();
    report.sdk_root = env.sdk_root.clone();
    report.emulator = env.emulator.clone();
    report.accel = env.accel.clone();
    report.scrcpy = env.scrcpy.clone();
    report.system_image = env.system_image.clone();
    report.avd = Some(droidwin_lib::runtime::emulator_runtime::AVD_NAME.to_string());
    report.scrcpy_version = env
        .scrcpy
        .as_deref()
        .and_then(|p| scrcpy_version(Path::new(p)));

    println!("  sdk:      {:?}", env.sdk_root);
    println!("  emulator: {:?}", env.emulator);
    println!("  image:    {:?}", env.system_image);
    println!("  scrcpy:   {:?} {:?}", env.scrcpy, report.scrcpy_version);
    println!("  accel:    {:?}", env.accel.as_deref().map(str::trim));
    if !env.blocking.is_empty() {
        for item in &env.blocking {
            println!("  MISSING: {item}");
        }
        report
            .checks
            .push(Check::new("environment", false, env.blocking.join("; ")));
        finish(&home, report);
        return Ok(());
    }
    report.checks.push(Check::new(
        "environment",
        true,
        "SDK, emulator, image and scrcpy found",
    ));

    // ---- Gate A: start the runtime and install real packages -----------------
    println!("\n[1] Starting the runtime (cold boot may take minutes)");
    let t0 = Instant::now();
    let status = match runtime.ensure_ready(&store, "proof").await {
        Ok(status) => status,
        Err(err) => {
            report.checks.push(Check::new(
                "runtime ready",
                false,
                format!("{:?}: {}", err.code, err.detail.unwrap_or_default()),
            ));
            finish(&home, report);
            return Ok(());
        }
    };
    report.runtime_ready_ms = Some(t0.elapsed().as_millis());
    report.android_api = status.android_api;
    println!(
        "    ready in {} ms, API {:?}",
        t0.elapsed().as_millis(),
        status.android_api
    );
    report.checks.push(Check::new(
        "runtime ready",
        true,
        format!(
            "{} ms, API {:?}",
            t0.elapsed().as_millis(),
            status.android_api
        ),
    ));

    let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
    let testdata = std::env::args()
        .nth(1)
        .unwrap_or_else(|| ".droidwin/testdata".to_string());
    let mut apks = list_apks(Path::new(&testdata));
    apks.truncate(2);
    if apks.is_empty() {
        report
            .notes
            .push(format!("No APKs in {testdata}; Gate A cannot be proven."));
        report
            .checks
            .push(Check::new("test packages", false, "no .apk files found"));
        finish(&home, report);
        return Ok(());
    }

    let mut installed: Vec<(String, String)> = Vec::new();
    for (index, file) in apks.iter().enumerate() {
        println!("\n[2.{}] Inspecting {}", index + 1, file.display());
        let inspection = match apk::inspect(file, aapt2.as_deref()) {
            Ok(i) => i,
            Err(err) => {
                println!("    inspect failed: {} {:?}", err.summary, err.detail);
                report.checks.push(Check::new(
                    "inspect",
                    false,
                    format!("{}: {}", file.display(), err.summary),
                ));
                continue;
            }
        };
        println!(
            "    {} {} · {} · min SDK {:?} · abis {:?}",
            inspection.package_id,
            inspection.version_name.clone().unwrap_or_default(),
            inspection
                .label
                .clone()
                .unwrap_or_else(|| "label unresolved".into()),
            inspection.min_sdk,
            inspection.abis
        );
        report.checks.push(Check::new(
            "inspect",
            true,
            format!(
                "{} → {} (label {:?}, min SDK {:?}, abis {:?})",
                file.file_name().unwrap_or_default().to_string_lossy(),
                inspection.package_id,
                inspection.label,
                inspection.min_sdk,
                inspection.abis
            ),
        ));

        let install_started = Instant::now();
        match runtime.install(&inspection, &store, "proof").await {
            Ok(outcome) => {
                let ms = install_started.elapsed().as_millis();
                println!("    installed in {ms} ms ({})", outcome.message);
                installed.push((
                    outcome.package_id.clone(),
                    inspection
                        .label
                        .clone()
                        .unwrap_or_else(|| outcome.package_id.clone()),
                ));
                report.packages.push(PackageResult {
                    file: file.display().to_string(),
                    package_id: outcome.package_id,
                    label: inspection.label.clone(),
                    version_name: inspection.version_name.clone(),
                    min_sdk: inspection.min_sdk,
                    abis: inspection.abis.clone(),
                    install_ms: ms,
                });
                report
                    .checks
                    .push(Check::new("install", true, format!("{ms} ms")));
            }
            Err(err) => {
                println!("    install failed: {} {:?}", err.summary, err.detail);
                report.checks.push(Check::new(
                    "install",
                    false,
                    format!("{}: {}", inspection.package_id, err.summary),
                ));
            }
        }
    }

    let apps = runtime.list_apps(&store).await.unwrap_or_default();
    println!("\n[3] Runtime lists {} third-party package(s)", apps.len());
    for app in &apps {
        println!(
            "    {} · {} · running={}",
            app.package_id,
            app.label.as_deref().unwrap_or("label unresolved"),
            app.running
        );
    }
    report.checks.push(Check::new(
        "enumerate",
        apps.len() >= installed.len() && !installed.is_empty(),
        format!(
            "{} listed, {} installed by this run",
            apps.len(),
            installed.len()
        ),
    ));

    // ---- Gate B: independent windows -----------------------------------------
    println!("\n[4] Launching each app in its own window");
    let mut launched = 0;
    for (package, label) in &installed {
        match runtime.launch(package, Some(label), &store, "proof").await {
            Ok(window) => {
                launched += 1;
                println!(
                    "    {} → window {:?} pid {}",
                    package, window.title, window.pid
                );
            }
            Err(err) => {
                println!(
                    "    {package} launch failed: {} {:?}",
                    err.summary, err.detail
                );
                report.checks.push(Check::new(
                    "launch",
                    false,
                    format!("{package}: {}", err.summary),
                ));
            }
        }
    }
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;

    let windows = scrcpy_windows().unwrap_or_default();
    report.windows = windows.clone();
    println!("    host windows ({}):", windows.len());
    for w in &windows {
        println!("      hwnd {} · {} · {}", w.hwnd, w.title, w.pid);
    }
    report.checks.push(Check::new(
        "independent windows",
        windows.len() >= 2 && distinct(&windows),
        format!(
            "{} window(s), {} distinct title(s)",
            windows.len(),
            windows
                .iter()
                .map(|w| w.title.clone())
                .collect::<std::collections::HashSet<_>>()
                .len()
        ),
    ));

    // Move + resize the first window and confirm the host actually applied it.
    if let Some(first) = windows.first() {
        match move_and_resize(first.hwnd) {
            Ok((before, after)) => {
                let ok = before != after;
                println!(
                    "    move/resize hwnd {}: {} → {}",
                    first.hwnd, before, after
                );
                report
                    .checks
                    .push(Check::new("move/resize", ok, format!("{before} → {after}")));
            }
            Err(err) => {
                println!("    move/resize failed: {err}");
                report.checks.push(Check::new("move/resize", false, err));
            }
        }
    }

    // Closing one window must not take the other with it.
    let mut close_ok = false;
    if let Some((package, _)) = installed.first() {
        println!("\n[5] Closing the window for {package}");
        if let Err(err) = runtime.close_window(package).await {
            println!("    close failed: {}", err.summary);
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let after = scrcpy_windows().unwrap_or_default();
        println!("    windows after close: {}", after.len());
        close_ok = after.len() + 1 == windows.len().max(1);
        report.checks.push(Check::new(
            "close isolation",
            close_ok,
            format!("{} window(s) remain", after.len()),
        ));
    }

    // A fresh runtime instance must still see the packages: installation lives in
    // the runtime, not in the manager's memory.
    println!("\n[6] Restarting the manager and re-reading runtime state");
    let fresh_store = Arc::new(Store::open(&home.join("droidwin.db")).map_err(|e| e.summary)?);
    let fresh_runtime = EmulatorRuntime::new(home.clone(), Arc::new(PrintEvents));
    let after_restart = fresh_runtime
        .list_apps(&fresh_store)
        .await
        .unwrap_or_default();
    let still_installed = installed
        .iter()
        .all(|(pkg, _)| after_restart.iter().any(|a| &a.package_id == pkg));
    println!(
        "    {} package(s) still installed after restart",
        after_restart.len()
    );
    report.checks.push(Check::new(
        "persists across manager restart",
        still_installed,
        format!("{} listed", after_restart.len()),
    ));

    report.gate_a = !installed.is_empty() && launched >= 1;
    report.gate_b = windows.len() >= 2
        && distinct(&windows)
        && close_ok
        && report
            .checks
            .iter()
            .find(|c| c.name == "move/resize")
            .map(|c| c.passed)
            .unwrap_or(false);

    if report.gate_b {
        let _ = store.set_setting("runtime.independent_windows_verified", "1");
        println!("\n  recorded verified independent windows for this backend");
    } else {
        let _ = store.set_setting("runtime.independent_windows_verified", "0");
        report.notes.push(
            "Gate B not satisfied: per-app windows could not be proven here. Do not claim independent windows in the product."
                .to_string(),
        );
    }

    runtime.shutdown().await;
    finish(&home, report);
    Ok(())
}

fn finish(home: &Path, mut report: Report) {
    report.finished_at = Some(now_ms());
    let path = home.join("proof-report.json");
    match serde_json::to_string_pretty(&report) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                eprintln!("could not write {}: {e}", path.display());
            } else {
                println!("\nreport written to {}", path.display());
            }
        }
        Err(e) => eprintln!("could not serialise report: {e}"),
    }

    println!("\n== checks ==");
    for check in &report.checks {
        println!(
            "  [{}] {} — {}",
            if check.passed { "pass" } else { "FAIL" },
            check.name,
            check.detail
        );
    }
    println!(
        "\nGate A (install + launch): {}",
        if report.gate_a { "PASS" } else { "FAIL" }
    );
    println!(
        "Gate B (independent windows): {}",
        if report.gate_b { "PASS" } else { "FAIL" }
    );
    for note in &report.notes {
        println!("note: {note}");
    }
}

fn distinct(windows: &[WindowRow]) -> bool {
    let mut titles: Vec<&str> = windows.iter().map(|w| w.title.as_str()).collect();
    let before = titles.len();
    titles.sort_unstable();
    titles.dedup();
    titles.len() == before
}

fn list_apks(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|e| e.eq_ignore_ascii_case("apk"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn scrcpy_version(exe: &Path) -> Option<String> {
    let out = std::process::Command::new(exe)
        .arg("--version")
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .map(|l| l.trim().to_string())
}

fn powershell(script: &str) -> Result<String, String> {
    // Read PowerShell output as UTF-8 so window titles survive the round trip.
    let script =
        format!("$OutputEncoding=[Console]::OutputEncoding=[Text.Encoding]::UTF8; {script}");
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "powershell exit {:?}: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Visible scrcpy windows on this desktop, read from the OS rather than from our
/// own bookkeeping.
fn scrcpy_windows() -> Result<Vec<WindowRow>, String> {
    let script = r#"$p = @(Get-Process scrcpy -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowHandle -ne 0 } | ForEach-Object { [pscustomobject]@{ pid = $_.Id; hwnd = $_.MainWindowHandle.ToInt64(); title = $_.MainWindowTitle } }); if ($p.Count -eq 0) { '[]' } else { ConvertTo-Json -InputObject $p -Compress -Depth 3 }"#;
    let text = powershell(script)?;
    let text = text.trim();
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let value: Value = serde_json::from_str(text).map_err(|e| format!("{e}: {text}"))?;
    let rows = match value {
        Value::Array(items) => items,
        Value::Object(_) => vec![value],
        _ => Vec::new(),
    };
    Ok(rows
        .into_iter()
        .filter_map(|v| {
            Some(WindowRow {
                pid: v.get("pid")?.as_u64()? as u32,
                hwnd: v.get("hwnd")?.as_i64()?,
                title: v.get("title")?.as_str()?.to_string(),
            })
        })
        .collect())
}

/// Ask the OS to move and resize a window, then read the rectangle back.
fn move_and_resize(hwnd: i64) -> Result<(String, String), String> {
    let script = format!(
        r#"Add-Type -Namespace W -Name U -MemberDefinition '[DllImport("user32.dll")] public static extern bool MoveWindow(System.IntPtr hWnd, int X, int Y, int nWidth, int nHeight, bool bRepaint); [DllImport("user32.dll")] public static extern bool GetWindowRect(System.IntPtr hWnd, out RECT lpRect); public struct RECT {{ public int Left; public int Top; public int Right; public int Bottom; }}'
$h = [IntPtr]::new({hwnd})
$before = New-Object W.U+RECT
[void][W.U]::GetWindowRect($h, [ref]$before)
[void][W.U]::MoveWindow($h, 120, 140, 520, 760, $true)
Start-Sleep -Milliseconds 600
$after = New-Object W.U+RECT
[void][W.U]::GetWindowRect($h, [ref]$after)
"$($before.Left),$($before.Top) $($before.Right - $before.Left)x$($before.Bottom - $before.Top) -> $($after.Left),$($after.Top) $($after.Right - $after.Left)x$($after.Bottom - $after.Top)""#
    );
    let text = powershell(&script)?;
    let line = text.trim().to_string();
    let mut parts = line.splitn(2, " -> ");
    let before = parts.next().unwrap_or_default().to_string();
    let after = parts.next().unwrap_or_default().to_string();
    if before.is_empty() || after.is_empty() {
        return Err(format!("unexpected window geometry output: {line}"));
    }
    Ok((before, after))
}
