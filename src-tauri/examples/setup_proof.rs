//! Runtime installation proof: installs the Android runtime into the SDK
//! directory named by the environment, then boots it once.
//!
//! It is the same code the setup screen calls — `Setup::run` and
//! `EmulatorRuntime::ensure_ready` — with the progress sink printed instead of
//! forwarded to a webview.
//!
//! ```text
//! DROIDWIN_ANDROID_SDK=… ANDROID_AVD_HOME=… DROIDWIN_HOME=… \
//!   cargo run --example setup_proof
//! ```

use std::sync::Arc;

use droidwin_lib::domain::SetupProgress;
use droidwin_lib::runtime::install::Setup;
use droidwin_lib::runtime::{sdk, EmulatorRuntime, NoopEvents};
use droidwin_lib::store::Store;

#[tokio::main]
async fn main() {
    let data_dir = match std::env::var("DROIDWIN_HOME") {
        Ok(value) if !value.trim().is_empty() => std::path::PathBuf::from(value),
        _ => panic!("DROIDWIN_HOME is required: the proof must not touch the app's real data"),
    };
    std::fs::create_dir_all(&data_dir).expect("data dir");
    sdk::configure(&data_dir);

    let root = sdk::install_root().expect("an SDK root from the environment");
    println!("sdk root: {}", root.display());
    println!("avd home: {}", sdk::avd_home().display());
    println!("java: {:?}", sdk::java_home(&root));

    // The real app provisions the bundled window host into its data directory on
    // first run; the proof data directory has none, so copy the checkout's.
    let bundled = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(".droidwin")
        .join("bin")
        .join("scrcpy-win64-v5.0.1");
    let target = data_dir.join("runtime").join("bin").join("scrcpy");
    if bundled.is_dir() && !target.join("scrcpy.exe").is_file() {
        let _ = std::fs::create_dir_all(&target);
        if let Ok(entries) = std::fs::read_dir(&bundled) {
            for entry in entries.flatten() {
                let _ = std::fs::copy(entry.path(), target.join(entry.file_name()));
            }
        }
    }

    let store = Arc::new(Store::open(&data_dir.join("droidwin.db")).expect("store"));
    let sink = Arc::new(|progress: SetupProgress| {
        println!(
            "[{}] {} {:?}",
            progress.step, progress.message, progress.percent
        );
    });
    let setup = Setup::new(data_dir.clone(), sink);

    let before = setup.report(&store, false);
    println!(
        "state before: {:?} · missing: {:?}",
        before.state,
        before
            .missing()
            .iter()
            .map(|component| component.label.clone())
            .collect::<Vec<_>>()
    );
    println!("message before: {}", before.message);

    match setup.run(&store, true).await {
        Ok(()) => println!("install: ok"),
        Err(err) => {
            println!("install: FAILED {} · {:?}", err.summary, err.detail);
            std::process::exit(1);
        }
    }

    let after = setup.report(&store, false);
    println!("state after: {:?} · {}", after.state, after.message);
    if !after.missing().is_empty() {
        println!("still missing: {:?}", after.missing());
        std::process::exit(1);
    }

    let runtime = Arc::new(EmulatorRuntime::new(data_dir, Arc::new(NoopEvents)));
    match runtime.ensure_ready(&store, "setup-proof").await {
        Ok(status) => {
            println!(
                "runtime: {} · {} · api {:?}",
                status.state.as_label(),
                status.detail,
                status.android_api
            );
        }
        Err(err) => {
            println!("runtime: FAILED {} · {:?}", err.summary, err.detail);
            runtime.shutdown().await;
            std::process::exit(1);
        }
    }
    runtime.shutdown().await;
    println!("proof: done");
}
