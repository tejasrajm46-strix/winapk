//! Proof that the setup screen and the runtime resolve the *same* Android SDK.
//!
//! Run on a PC that already has an SDK, with no DroidWin override in the
//! environment:
//!
//! ```text
//! cargo test --test sdk_live -- --ignored --nocapture
//! ```
//!
//! The failure this guards against: a complete Android Studio SDK sits at
//! `%LOCALAPPDATA%\Android\Sdk`, but the setup report looked only at DroidWin's
//! own (empty) directory. The app then showed "Set up Android" with everything
//! missing, refused to start the runtime that was already installed, and would
//! have downloaded a second multi-gigabyte SDK.
//!
//! It is ignored by default because a CI runner has no SDK; the assertion is
//! about this machine's real installation, not a fixture.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use droidwin_lib::domain::SetupState;
use droidwin_lib::runtime::install::Setup;
use droidwin_lib::runtime::sdk;
use droidwin_lib::store::Store;

/// The window host ships inside the installer and the app copies it into its
/// data directory at startup; the report counts it as a required component, so
/// the proof has to do the same copy or it would measure the test, not the SDK.
fn provision_window_host(data: &Path) {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("scrcpy");
    let target = data.join("runtime").join("bin").join("scrcpy");
    std::fs::create_dir_all(&target).expect("scrcpy target");
    for entry in std::fs::read_dir(&source)
        .expect("bundled window host")
        .flatten()
    {
        if entry.path().is_file() {
            std::fs::copy(entry.path(), target.join(entry.file_name())).expect("copy scrcpy");
        }
    }
}

#[test]
#[ignore = "needs a real Android SDK installed on this PC"]
fn the_setup_report_adopts_the_sdk_already_installed_here() {
    for name in [
        "DROIDWIN_ANDROID_SDK",
        "DROIDWIN_HOME",
        "ANDROID_SDK_ROOT",
        "ANDROID_HOME",
    ] {
        if let Ok(value) = std::env::var(name) {
            assert!(
                value.trim().is_empty(),
                "{name} is set to {value}; this proof must run without overrides"
            );
        }
    }
    assert!(
        sdk::configured_root().is_none(),
        "a saved SDK choice is active; the adoption path would not be exercised"
    );

    // A fresh data directory: DroidWin's own SDK directory inside it does not
    // exist, which is exactly the state of the first run on a PC that already
    // has Android Studio.
    let data = std::env::temp_dir().join(format!("droidwin-sdk-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&data);
    std::fs::create_dir_all(&data).expect("temp data dir");
    sdk::configure(&data);

    let located = sdk::Sdk::locate(None).expect("an Android SDK is installed on this machine");
    assert!(
        located
            .root()
            .join("platform-tools")
            .join(sdk::exe("adb"))
            .is_file(),
        "{} is not a usable SDK",
        located.root().display()
    );
    assert_eq!(sdk::active_root().as_deref(), Some(located.root()));

    provision_window_host(&data);
    let store = Store::open(&data.join("droidwin.db")).expect("store");
    let setup = Setup::new(data.clone(), Arc::new(|_| {}));

    let report = setup.report(&store, false);
    assert_eq!(
        PathBuf::from(&report.sdk_root),
        located.root().to_path_buf(),
        "the setup screen and the runtime disagree about which SDK this PC has"
    );
    assert!(!report.managed, "DroidWin's empty directory was chosen");
    assert!(
        report.missing().is_empty(),
        "still called missing: {:?}",
        report
            .missing()
            .iter()
            .map(|component| component.label.clone())
            .collect::<Vec<_>>()
    );
    // Files on disk are not readiness: without a verified boot the honest state
    // is `starting`, never `ready`.
    assert_eq!(report.state, SetupState::Starting);
    assert_eq!(setup.report(&store, true).state, SetupState::Ready);

    println!("adopted {} (managed: {})", report.sdk_root, report.managed);
    let _ = std::fs::remove_dir_all(&data);
}
