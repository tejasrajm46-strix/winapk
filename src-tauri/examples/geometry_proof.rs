//! Display-geometry proof.
//!
//!   cargo run --example geometry_proof -- <apk> [<apk> ...]
//!
//! Opens each package through the product's own runtime — the same install and
//! launch path the desktop app uses — then reports the Android display it was
//! given, what Android reports for that display, and saves a PNG of the display
//! through DroidWin's own screenshot path. Windows stay open for a minute so a
//! person can look at them; nothing here makes display decisions of its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::apk;
use droidwin_lib::domain::{RuntimeStatus, StageEvent};
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
        eprintln!("geometry proof failed: {err}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut apks: Vec<PathBuf> = Vec::new();
    let mut keep_open = 60u64;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--keep-open" {
            keep_open = args
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(keep_open);
        } else {
            apks.push(PathBuf::from(arg));
        }
    }
    if apks.is_empty() {
        return Err("pass at least one .apk path".into());
    }

    // Same home as the other harness unless told otherwise: it is where this
    // checkout keeps the window host the runtime needs.
    let home = std::env::var("DROIDWIN_PROOF_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".droidwin").join("home"));
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let _guard = droidwin_lib::diag::init(&home.join("logs"));
    let store = Arc::new(Store::open(&home.join("droidwin.db")).map_err(|e| e.summary)?);
    let runtime = EmulatorRuntime::new(home.clone(), Arc::new(PrintEvents));

    let status = runtime
        .ensure_ready(&store, "geometry")
        .await
        .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
    println!(
        "runtime ready: API {:?} on {}",
        status.android_api,
        runtime.serial()
    );

    let adb = runtime
        .adb()
        .ok_or_else(|| "adb is unavailable".to_string())?;
    let serial = runtime.serial();
    let aapt2 = Sdk::locate(None).and_then(|sdk| sdk.aapt2());
    let evidence = PathBuf::from(".droidwin").join("evidence");
    std::fs::create_dir_all(&evidence).map_err(|e| e.to_string())?;

    let mut opened: Vec<(String, String)> = Vec::new();
    for path in &apks {
        let inspection = apk::inspect(path, aapt2.as_deref()).map_err(|e| e.summary)?;
        println!(
            "\n{} · {} · target SDK {:?} · orientation {:?}",
            inspection.package_id,
            inspection.label.clone().unwrap_or_default(),
            inspection.target_sdk,
            inspection.orientation
        );
        // A pre-23 target is refused by modern Android unless the override is
        // named; this harness is an explicit testing tool, so it names it.
        let allow_old = inspection.target_sdk.is_some_and(|sdk| sdk < 23);
        let outcome = runtime
            .install(&inspection, &store, "geometry", allow_old)
            .await
            .map_err(|e| format!("{} {:?}", e.summary, e.detail))?;
        println!("  installed: {}", outcome.message);
        let label = inspection
            .label
            .clone()
            .unwrap_or_else(|| outcome.package_id.clone());
        match runtime
            .launch(&outcome.package_id, Some(&label), &store, "geometry")
            .await
        {
            Ok(window) => {
                println!("  window: {} (pid {})", window.title, window.pid);
                opened.push((outcome.package_id, label));
            }
            Err(err) => println!("  launch failed: {} {:?}", err.summary, err.detail),
        }
    }

    tokio::time::sleep(Duration::from_secs(6)).await;
    let displays = adb.display_focus(&serial).await.unwrap_or_default();

    println!("\n== surfaces ==");
    for (package, _) in &opened {
        let surface = runtime.geometry_of(package);
        let display_id = surface.and_then(|s| s.display_id);
        let focus = display_id.and_then(|id| displays.iter().find(|d| d.display_id == id).cloned());
        println!(
            "{package}: display {:?} · android composes {}x{} · droidwin created {:?}",
            display_id,
            focus.as_ref().and_then(|f| f.width).unwrap_or(0),
            focus.as_ref().and_then(|f| f.height).unwrap_or(0),
            surface.map(|s| (s.geometry.width, s.geometry.height)),
        );
        println!("  surface: {surface:?}");
        if let Some(id) = display_id {
            match adb.screenshot_png(&serial, id).await {
                Ok(png) => {
                    let file = evidence.join(format!("{}-display.png", file_stem(package)));
                    std::fs::write(&file, png).map_err(|e| e.to_string())?;
                    println!("  display screenshot: {}", file.display());
                }
                Err(err) => println!("  screenshot failed: {} {:?}", err.summary, err.detail),
            }
        }
    }

    println!("\nwindows stay open for {keep_open}s so they can be inspected");
    tokio::time::sleep(Duration::from_secs(keep_open)).await;
    runtime.shutdown().await;
    Ok(())
}

fn file_stem(package_id: &str) -> String {
    package_id.replace(['.', '/', '\\'], "_")
}

#[allow(dead_code)]
fn _assert_path(_: &Path) {}
