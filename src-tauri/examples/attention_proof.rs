//! "Attention Needed" proof on a real runtime.
//!
//!   cargo run --example attention_proof -- <apk-a> <apk-b> [--blank <apk>]
//!
//! Everything below is produced through DroidWin's own runtime and diagnostic
//! pass — no fake store, no invented events:
//!
//! 1. opens two apps, each on its own display;
//! 2. kills exactly one app's window host and asks for a diagnosis, which must
//!    be confirmed, must name the lost window, and must leave the other app
//!    running;
//! 3. runs that diagnosis's own recovery (reopen the window) and shows the app
//!    back on a display while the other window is untouched;
//! 4. installs and opens a third package to look for a genuinely blank render,
//!    capturing screenshots and the app's log lines as evidence;
//! 5. stops the runtime and shows the confirmed "runtime is stopped" problem
//!    with its Start action, then runs it.
//!
//! Screenshots and a JSON digest land in `.droidwin/evidence/`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::apk;
use droidwin_lib::diagnose::{Diagnoser, Problem, Recovery, Report};
use droidwin_lib::domain::{RuntimeStatus, StageEvent};
use droidwin_lib::runtime::adb::Adb;
use droidwin_lib::runtime::sdk::Sdk;
use droidwin_lib::runtime::{AppEvent, EmulatorRuntime, EventBus, RuntimeEvents};
use droidwin_lib::store::Store;

struct ProofEvents {
    bus: EventBus,
}

impl RuntimeEvents for ProofEvents {
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
    fn app_event(&self, event: &AppEvent) {
        println!(
            "    event: {} {:?} — {} (exit {:?})",
            event.package_id, event.kind, event.reason, event.exit_code
        );
        self.bus.publish(event.clone());
    }
}

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("attention proof failed: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let mut blank: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--blank" {
            blank = args.next().map(PathBuf::from);
        } else {
            paths.push(PathBuf::from(arg));
        }
    }
    if paths.len() < 2 {
        return Err("pass two .apk paths (and optionally --blank <apk>)".into());
    }

    let home = std::env::var("DROIDWIN_PROOF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".droidwin").join("attention-home"));
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let _guard = droidwin_lib::diag::init(&home.join("logs"));
    let store = Arc::new(Store::open(&home.join("droidwin.db")).map_err(|e| e.summary)?);
    let bus = EventBus::new();
    let runtime = Arc::new(EmulatorRuntime::new(
        home.clone(),
        Arc::new(ProofEvents { bus: bus.clone() }),
    ));
    let diagnose = Diagnoser::new(Arc::clone(&runtime), Arc::clone(&store), bus.clone());
    let evidence = PathBuf::from(".droidwin").join("evidence");
    std::fs::create_dir_all(&evidence).map_err(|e| e.to_string())?;

    let status = runtime
        .ensure_ready(&store, "attention-proof")
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    println!("\nbooted: {}", status.detail);
    let adb = runtime
        .adb()
        .ok_or_else(|| "adb is unavailable".to_string())?;
    let serial = runtime.serial();

    let mut digest = serde_json::Value::Null;
    let result: Result<(), String> = async {
        // 1. Two real apps, two displays.
        let mut installed: Vec<(String, String)> = Vec::new();
    for path in &paths {
        let (package, label) = install_and_launch(&runtime, &store, path).await?;
        wait_surface(&runtime, &package).await?;
        installed.push((package, label));
    }
    sleep(2.0).await;
    let a = installed[0].0.clone();
    let b = installed[1].0.clone();
    let windows = runtime.open_windows();
    println!("\ntwo windows open:");
    for window in &windows {
        println!(
            "  {} on display {:?} (host pid {})",
            window.package_id, window.display_id, window.pid
        );
    }
    if windows.len() != 2 {
        return Err(format!("expected two windows, saw {}", windows.len()));
    }
    let pid_b_before = windows
        .iter()
        .find(|window| window.package_id == b)
        .map(|window| window.pid)
        .ok_or("the second app's window disappeared")?;
    let display_b_before = runtime.display_of(&b);
    for (package, name) in [(&a, "03-a-before"), (&b, "04-b-before")] {
        shoot(&adb, &serial, &runtime, package, &evidence, name).await?;
    }

    let baseline = diagnose.report(false).await;
    print_report("after both apps opened (quick check)", &baseline);

    // 2. Kill exactly one window host. The app itself keeps running.
    let victim = runtime
        .open_windows()
        .into_iter()
        .find(|window| window.package_id == a)
        .ok_or("the first app's window is missing")?;
    println!(
        "\ninducing a lost window: killing window host {} (pid {}) for {a}",
        a, victim.pid
    );
    kill_pid(victim.pid)?;
    let mut gone = false;
    for _ in 0..20 {
        sleep(0.5).await;
        if !runtime.open_windows().iter().any(|w| w.package_id == a) {
            gone = true;
            break;
        }
    }
    if !gone {
        return Err("the runtime still lists the killed window".into());
    }
    let app_alive = adb.process_running(&serial, &a).await;
    println!("  {a} process alive after the window died: {app_alive}");

    let after = diagnose.report(false).await;
    print_report("after the window host was killed", &after);
    let problem = after
        .problems
        .iter()
        .find(|problem| problem.id.starts_with("app.window-lost:") && problem.id.ends_with(&a))
        .ok_or_else(|| format!("no lost-window diagnosis for {a}"))?;
    if problem.confidence != droidwin_lib::diagnose::Confidence::Confirmed {
        return Err("the lost window was not reported as confirmed".into());
    }
    if !matches!(problem.recovery, Recovery::ReopenWindow { .. }) {
        return Err(format!(
            "the lost window should be repaired by reopening it, got {:?}",
            problem.recovery
        ));
    }
    if after
        .problems
        .iter()
        .any(|problem| problem.package_id.as_deref() == Some(b.as_str()))
    {
        return Err(format!("{b} was diagnosed even though it is fine"));
    }

    // 3. Targeted recovery, in place.
    let outcome = diagnose.recover(&problem.recovery).await;
    println!("\nrecovery: {} (ok={})", outcome.summary, outcome.ok);
    if let Some(detail) = &outcome.detail {
        println!("  detail: {detail}");
    }
    if !outcome.ok {
        return Err("reopening the lost window failed".into());
    }
    wait_surface(&runtime, &a).await?;
    sleep(2.0).await;
    let windows = runtime.open_windows();
    let pid_b_after = windows
        .iter()
        .find(|window| window.package_id == b)
        .map(|window| window.pid);
    let display_b_after = runtime.display_of(&b);
    println!("\nwindows after recovery:");
    for window in &windows {
        println!(
            "  {} on display {:?} (host pid {})",
            window.package_id, window.display_id, window.pid
        );
    }
    if pid_b_after != Some(pid_b_before) || display_b_after != display_b_before {
        return Err(format!(
            "{b} was disturbed by the recovery: pid {:?} → {:?}, display {:?} → {:?}",
            pid_b_before, pid_b_after, display_b_before, display_b_after
        ));
    }
    println!("  {b} kept host pid {pid_b_before} and display {display_b_before:?} throughout");
    for (package, name) in [(&a, "05-a-recovered"), (&b, "06-b-still-running")] {
        shoot(&adb, &serial, &runtime, package, &evidence, name).await?;
    }
    let recovered = diagnose.report(false).await;
    print_report("after the targeted recovery", &recovered);
    if recovered
        .problems
        .iter()
        .any(|problem| problem.package_id.as_deref() == Some(a.as_str()))
    {
        return Err(format!("{a} is still reported as a problem after recovery"));
    }

    // 4. A package that may render nothing. Its own log lines are evidence.
    let mut blank_evidence = serde_json::Value::Null;
    if let Some(path) = blank.as_ref() {        println!(
            "\nopening a candidate for a blank render: {}",
            path.display()
        );
        // The candidate is a fixture that gets rebuilt while it is installed.
        // Clearing just that package lets a new copy replace it; the two apps
        // under test are never touched.
        let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
        if let Ok(inspection) = apk::inspect(path, aapt2.as_deref()) {
            if adb
                .uninstall(&serial, &inspection.package_id, false)
                .await
                .map(|out| out.ok())
                .unwrap_or(false)
            {
                println!("  reset the fixture package {}", inspection.package_id);
                sleep(0.5).await;
            }
        }
        match install_and_launch(&runtime, &store, path).await {
            Ok((package, _)) => {
                wait_surface(&runtime, &package).await?;
                sleep(4.0).await;
                let deep = diagnose.report(true).await;
                print_report("full check after opening the candidate", &deep);
                let blank_problem = deep
                    .problems
                    .iter()
                    .find(|problem| problem.id.starts_with("app.blank-render:"));
                match blank_problem {
                    Some(problem) => {
                        blank_evidence = serde_json::json!({
                            "package_id": package,
                            "confirmed": problem.confidence == droidwin_lib::diagnose::Confidence::Confirmed,
                            "observed": problem.observed,
                            "evidence": problem.evidence,
                        });
                        shoot(&adb, &serial, &runtime, &package, &evidence, "07-blank-window")
                            .await?;
                    }
                    None => {
                        let crash = deep
                            .problems
                            .iter()
                            .find(|problem| problem.package_id.as_deref() == Some(package.as_str()));
                        println!(
                            "  no blank render confirmed for {package}; the diagnosis says: {}",
                            crash
                                .map(|problem| problem.title.as_str())
                                .unwrap_or("nothing")
                        );
                        blank_evidence = serde_json::json!({
                            "package_id": package,
                            "blank": false,
                            "reported": crash.map(|problem| problem.title.clone()),
                        });
                        if runtime.display_of(&package).is_some() {
                            shoot(&adb, &serial, &runtime, &package, &evidence, "07-candidate-render")
                                .await?;
                        }
                    }
                }
            }
            Err(err) => {
                println!("  the candidate did not launch: {err}");
                blank_evidence = serde_json::json!({
                    "package_id": path.display().to_string(),
                    "blank": false,
                    "launch_error": err,
                });
            }
        }
    }

    // 5. A confirmed environment problem: the runtime is stopped.
    for (package, _) in &installed {
        let _ = runtime.close_window(package).await;
    }
    runtime.shutdown().await;
    sleep(1.0).await;
    let stopped = diagnose.report(false).await;
    print_report("after stopping the runtime", &stopped);
    let stopped_problem = stopped
        .problems
        .iter()
        .find(|problem| problem.id == "runtime.stopped");
    let stopped_evidence = match stopped_problem {
        Some(problem) => {
            let result = diagnose.recover(&problem.recovery).await;
            println!(
                "\nrunning the diagnosis's own Start action: {} (ok={})",
                result.summary, result.ok
            );
            if !result.ok {
                return Err("the diagnosis's Start action failed".into());
            }
            serde_json::json!({
                "problem": problem.title,
                "confidence": problem.confidence,
                "recovery": problem.recovery,
                "recovered": result.ok,
                "after": result.report.problems.iter().map(|p| p.title.clone()).collect::<Vec<_>>(),
            })
        }
        None => serde_json::json!({ "missing": true }),
    };

        digest = serde_json::json!({
            "apps": installed.iter().map(|(package, label)| serde_json::json!({
                "package_id": package, "label": label,
            })).collect::<Vec<_>>(),
            "lost_window_problem": {
                "id": problem.id,
                "title": problem.title,
                "confidence": problem.confidence,
                "observed": problem.observed,
                "recovery": problem.recovery,
                "app_process_alive": app_alive,
            },
            "other_app_untouched": {
                "package_id": b,
                "host_pid_before": pid_b_before,
                "host_pid_after": pid_b_after,
                "display_before": display_b_before,
                "display_after": display_b_after,
            },
            "after_recovery": recovered.problems.iter().map(|p| p.title.clone()).collect::<Vec<_>>(),
            "blank": blank_evidence,
            "stopped_runtime": stopped_evidence,
        });
        Ok(())
    }
    .await;

    // Teardown always runs, including after a failed step: a proof harness must
    // not leave an emulator or a window behind, and that is itself part of the
    // evidence the mission asks for.
    for window in runtime.open_windows() {
        let _ = runtime.close_window(&window.package_id).await;
    }
    runtime.shutdown().await;

    if result.is_ok() {
        std::fs::write(
            evidence.join("attention-proof.json"),
            serde_json::to_vec_pretty(&digest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;

        println!("\nfinal process state:");
        list_processes()?;
        let devices = adb
            .devices()
            .await
            .map(|devices| {
                devices
                    .into_iter()
                    .map(|device| format!("{} {}", device.serial, device.state))
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|_| "adb did not answer".into());
        println!("  adb devices: {devices}");
        println!(
            "\nevidence: {}",
            evidence.join("attention-proof.json").display()
        );
    }
    result
}

async fn install_and_launch(
    runtime: &Arc<EmulatorRuntime>,
    store: &Arc<Store>,
    path: &Path,
) -> Result<(String, String), String> {
    let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
    let inspection = apk::inspect(path, aapt2.as_deref())
        .map_err(|e| format!("{} ({})", e.summary, path.display()))?;
    let package = inspection.package_id.clone();
    let label = inspection.label.clone().unwrap_or_else(|| package.clone());
    let allow_old = inspection.target_sdk.is_some_and(|sdk| sdk < 23);
    runtime
        .install(&inspection, store, "attention-proof", allow_old)
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    runtime
        .launch(&package, Some(&label), store, "attention-proof")
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    Ok((package, label))
}

async fn wait_surface(
    runtime: &Arc<EmulatorRuntime>,
    package: &str,
) -> Result<droidwin_lib::runtime::emulator_runtime::AppSurface, String> {
    for _ in 0..24 {
        if let Some(surface) = runtime.geometry_of(package) {
            if surface.display_id.is_some() {
                return Ok(surface);
            }
        }
        sleep(0.25).await;
    }
    Err(format!("{package} never resolved to an Android display"))
}

async fn shoot(
    adb: &Adb,
    serial: &str,
    runtime: &Arc<EmulatorRuntime>,
    package: &str,
    evidence: &Path,
    name: &str,
) -> Result<PathBuf, String> {
    let display = runtime
        .display_of(package)
        .ok_or_else(|| format!("{package} has no display to capture"))?;
    let png = adb
        .screenshot_png(serial, display)
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    let file = evidence.join(format!("{name}.png"));
    std::fs::write(&file, &png).map_err(|e| e.to_string())?;
    println!(
        "  screenshot {name}: {} ({} bytes, display {display})",
        file.display(),
        png.len()
    );
    Ok(file)
}

fn print_report(title: &str, report: &Report) {
    println!("\n== {title} ==");
    println!(
        "  checked at {} ({}): {} confirmed, {} suspected, {} problem(s)",
        report.checked_at_ms,
        if report.deep { "full" } else { "quick" },
        report.confirmed,
        report.suspected,
        report.problems.len()
    );
    if report.problems.is_empty() {
        println!("  nothing needs attention");
    }
    for problem in &report.problems {
        print_problem(problem);
    }
    if !report.checked.is_empty() {
        println!("  what was checked:");
        for line in &report.checked {
            println!("    - {line}");
        }
    }
}

fn print_problem(problem: &Problem) {
    println!(
        "  [{} · {}] {}",
        match problem.confidence {
            droidwin_lib::diagnose::Confidence::Confirmed => "confirmed",
            droidwin_lib::diagnose::Confidence::Suspected => "suspected",
        },
        match problem.severity {
            droidwin_lib::diagnose::Severity::Blocking => "blocking",
            droidwin_lib::diagnose::Severity::Degraded => "degraded",
            droidwin_lib::diagnose::Severity::Notice => "notice",
        },
        problem.title
    );
    println!("    observed: {}", problem.observed);
    for item in &problem.evidence {
        println!(
            "    evidence[{}]: {}",
            item.source,
            item.detail.replace('\n', "\n                ")
        );
    }
    println!("    recommended: {}", problem.recommendation);
    println!("    recovery: {:?}", problem.recovery);
}

/// Kill one process and wait for it to be gone. Used to simulate a window host
/// dying under its app, exactly as a crash of the host would look to the watcher.
fn kill_pid(pid: u32) -> Result<(), String> {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/PID", &pid.to_string(), "/F"]);
    droidwin_lib::proc::no_console(&mut cmd);
    let out = cmd.output().map_err(|e| e.to_string())?;
    println!(
        "  taskkill: {}",
        String::from_utf8_lossy(&out.stdout).trim()
    );
    if !out.status.success() {
        return Err(format!(
            "taskkill failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

fn list_processes() -> Result<(), String> {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq scrcpy.exe"])
        .output()
        .map_err(|e| e.to_string())?;
    print!("{}", String::from_utf8_lossy(&out.stdout));
    Ok(())
}

async fn sleep(seconds: f64) {
    tokio::time::sleep(Duration::from_secs_f64(seconds)).await;
}
