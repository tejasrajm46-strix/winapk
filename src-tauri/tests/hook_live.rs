//! Live check for the Windows keyboard hook itself: a real desktop key press has
//! to reach Android through DroidWin's `WH_KEYBOARD_LL` hook.
//!
//! `mapping_live` drives the decision layer (`on_key_for`) because a harness that
//! does not own the keyboard cannot test focus. This test owns it: it installs the
//! real hook through the app's own `arm()` path, gives a real app window the
//! keyboard focus with Win32, and injects genuine key events into the desktop
//! input queue — the same queue a physical key uses. Nothing between the desktop
//! and Android is called directly, so a hook that is never installed, or one that
//! resolves the wrong window, fails here instead of passing.
//!
//! Everything asserted is read back out of Android: `screen_brightness` is an
//! integer Android writes down, so a press that fires twice shows up as the wrong
//! number, and `dumpsys input` lists the pointers a display has down, so a finger
//! that is never lifted stays visible.
//!
//! Failures are collected and reported after the window is closed and Android's
//! settings are put back: a panic in the middle of this test leaves a runtime that
//! cannot shut down, and a regression guard that hangs is worse than useless.
//!
//! It needs an interactive desktop session, so it is ignored by default:
//!
//! ```text
//! cargo test --test hook_live -- --ignored --nocapture
//! ```
//!
//! It moves the keyboard focus (including one real Alt+Tab), then gives it back.

mod support;

use std::sync::Arc;
use std::time::Duration;

use droidwin_lib::input::{InputAction, InputController};
use droidwin_lib::runtime::adb::Adb;
use support::{touching_windows, Harness};

/// Settings ships on every AVD, so the test never depends on a package the
/// machine happens to have installed.
const PACKAGE: &str = "com.android.settings";

const VK_K: u32 = 0x4B;
const VK_L: u32 = 0x4C;
const VK_SHIFT: u32 = 0x10;
const VK_CTRL: u32 = 0x11;
const VK_MENU: u32 = 0x12;
const VK_TAB: u32 = 0x09;

const BRIGHTNESS_START: i32 = 10;
const AFTER_ONE_PUSH: i32 = 16;
const AFTER_TWO_PUSHES: i32 = 25;

/// Record a failed expectation and keep unwinding the matrix by returning.
macro_rules! need {
    ($condition:expr, $($message:tt)*) => {
        if !($condition) {
            return Err(format!($($message)*));
        }
    };
}

async fn shell(adb: &Adb, serial: &str, line: &str) -> Result<String, String> {
    let out = adb
        .run(&["-s", serial, "shell", line], Duration::from_secs(40))
        .await
        .map_err(|err| format!("`{line}` could not be run: {}", err.summary))?;
    need!(out.ok(), "`{line}` failed: {}", out.combined());
    Ok(out.stdout.trim().to_string())
}

async fn brightness(adb: &Adb, serial: &str) -> Result<i32, String> {
    shell(adb, serial, "settings get system screen_brightness")
        .await?
        .parse()
        .map_err(|_| "Android reported a brightness that is not a number".to_string())
}

async fn reset_brightness(adb: &Adb, serial: &str) -> Result<(), String> {
    shell(
        adb,
        serial,
        &format!("settings put system screen_brightness {BRIGHTNESS_START}"),
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(600)).await;
    Ok(())
}

async fn pointers(adb: &Adb, serial: &str, display: u32) -> Result<Vec<String>, String> {
    let dump = shell(adb, serial, "dumpsys input").await?;
    Ok(touching_windows(&dump, display))
}

/// The Windows side, declared by hand: a dozen user32 calls and one struct, the
/// same choice `input::hook` makes rather than pulling in a binding crate.
#[cfg(windows)]
mod desk {
    use std::ffi::c_void;

    pub type Hwnd = *mut c_void;

    pub const KEYUP: u32 = 0x0002;
    pub const MOUSEEVENTF_LEFTDOWN: u32 = 0x0002;
    pub const MOUSEEVENTF_LEFTUP: u32 = 0x0004;

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn EnumWindows(
            proc: Option<unsafe extern "system" fn(Hwnd, isize) -> i32>,
            param: isize,
        ) -> i32;
        pub fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        pub fn IsWindowVisible(hwnd: Hwnd) -> i32;
        pub fn GetWindowTextLengthW(hwnd: Hwnd) -> i32;
        pub fn GetWindowTextW(hwnd: Hwnd, buffer: *mut u16, capacity: i32) -> i32;
        pub fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        pub fn GetForegroundWindow() -> Hwnd;
        pub fn SetForegroundWindow(hwnd: Hwnd) -> i32;
        pub fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        pub fn SetCursorPos(x: i32, y: i32) -> i32;
        pub fn mouse_event(flags: u32, dx: i32, dy: i32, data: u32, extra: usize);
        pub fn keybd_event(vk: u8, scan: u8, flags: u32, extra: usize);
    }
}

#[cfg(windows)]
fn title_of(hwnd: desk::Hwnd) -> String {
    unsafe {
        let length = desk::GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let written = desk::GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32);
        buffer.truncate(written.max(0) as usize);
        String::from_utf16_lossy(&buffer)
    }
}

/// Every visible top-level window of one process, with its rectangle.
#[cfg(windows)]
fn windows_of(pid: u32) -> Vec<(desk::Hwnd, String, desk::Rect)> {
    unsafe extern "system" fn visit(hwnd: desk::Hwnd, param: isize) -> i32 {
        let found = &mut *(param as *mut Vec<(desk::Hwnd, String, desk::Rect)>);
        // Ownership is decided by the caller after the walk; the callback only
        // reports what is visible.
        if desk::IsWindowVisible(hwnd) != 0 {
            let mut rect = desk::Rect::default();
            desk::GetWindowRect(hwnd, &mut rect);
            found.push((hwnd, title_of(hwnd), rect));
        }
        1
    }
    let mut found: Vec<(desk::Hwnd, String, desk::Rect)> = Vec::new();
    unsafe {
        desk::EnumWindows(Some(visit), &mut found as *mut _ as isize);
    }
    found
        .into_iter()
        .filter(|(hwnd, _, _)| {
            let mut owner = 0u32;
            unsafe { desk::GetWindowThreadProcessId(*hwnd, &mut owner) };
            owner == pid
        })
        .collect()
}

#[cfg(windows)]
fn foreground_is(hwnd: desk::Hwnd) -> bool {
    unsafe { desk::GetForegroundWindow() == hwnd }
}

/// Give one window the keyboard focus. Windows refuses a foreground change asked
/// for by a process the user is not using, so the fallback is the thing a person
/// does: click its title bar.
#[cfg(windows)]
fn focus(hwnd: desk::Hwnd, rect: &desk::Rect) -> bool {
    for _ in 0..3 {
        unsafe {
            desk::ShowWindow(hwnd, 9); // SW_RESTORE
            desk::SetForegroundWindow(hwnd);
        }
        std::thread::sleep(Duration::from_millis(300));
        if foreground_is(hwnd) {
            return true;
        }
    }
    unsafe {
        desk::SetCursorPos((rect.left + rect.right) / 2, rect.top + 6);
        desk::mouse_event(desk::MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        desk::mouse_event(desk::MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
    std::thread::sleep(Duration::from_millis(500));
    foreground_is(hwnd)
}

#[cfg(windows)]
fn key(vk: u32, down: bool) {
    unsafe { desk::keybd_event(vk as u8, 0, if down { 0 } else { desk::KEYUP }, 0) }
}

/// One press the way a person types it: down, a moment, up.
#[cfg(windows)]
async fn press(vk: u32) {
    key(vk, true);
    tokio::time::sleep(Duration::from_millis(60)).await;
    key(vk, false);
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "installs the real keyboard hook and needs an interactive desktop"]
async fn real_desktop_keys_reach_android_through_the_hook() {
    let harness = Harness::start("hook").await;
    let outcome = drive(&harness).await;
    harness.shutdown().await;
    if let Err(message) = outcome {
        panic!("{message}");
    }
}

/// The matrix. Returns the first failed expectation, after putting Android's
/// settings back, so a failure is a report instead of a hang.
#[cfg(windows)]
async fn drive(harness: &Harness) -> Result<(), String> {
    let runtime = Arc::clone(&harness.runtime);
    let store = Arc::clone(harness.store());
    runtime
        .ensure_ready(&store, "hook-live")
        .await
        .map_err(|err| format!("the runtime becomes ready: {}", err.summary))?;

    let adb = runtime.adb().ok_or("the SDK is configured")?;
    let serial = runtime.serial();

    let window = runtime
        .launch(PACKAGE, Some("Settings"), &store, "hook")
        .await
        .map_err(|err| format!("Settings opens in a window: {}", err.summary))?;
    let display = runtime
        .geometry_of(PACKAGE)
        .and_then(|surface| surface.display_id)
        .ok_or("the window reports the display Android composed it on")?;

    // Give the window a moment to appear and take its place in the z-order.
    tokio::time::sleep(Duration::from_millis(2_500)).await;
    let windows = windows_of(window.pid);
    need!(
        !windows.is_empty(),
        "the window host has no window for pid {}",
        window.pid
    );
    let (hwnd, title, rect) = windows[0].clone();
    eprintln!("window {title:?} (pid {}) on display {display}", window.pid);

    let original_mode = shell(&adb, &serial, "settings get system screen_brightness_mode").await?;
    let original_brightness = brightness(&adb, &serial).await?;
    shell(
        &adb,
        &serial,
        "settings put system screen_brightness_mode 0",
    )
    .await?;

    let controller = InputController::new(Arc::clone(&runtime), Arc::clone(&store));
    // K moves Android's brightness, so one press is one visible step; the holds
    // put a finger on the app, so a press that is never released stays visible in
    // `dumpsys input`.
    controller
        .save(
            PACKAGE,
            VK_K,
            InputAction::KeyEvent {
                keycode: "KEYCODE_BRIGHTNESS_UP".into(),
            },
        )
        .map_err(|err| format!("K is mapped: {}", err.summary))?;
    for (vk, name) in [
        (0x100 | VK_K, "Ctrl+K"),
        (VK_L, "L"),
        (0x500 | VK_K, "Ctrl+Shift+K"),
    ] {
        controller
            .save(PACKAGE, vk, InputAction::TouchHold { x: 50.0, y: 50.0 })
            .map_err(|err| format!("{name} is mapped: {}", err.summary))?;
    }
    // The app-level switch is what installs the hook (`arm`), so this test uses
    // the same path the app does rather than calling the hook directly.
    controller
        .set_enabled(PACKAGE, true)
        .map_err(|err| format!("the profile switches on: {}", err.summary))?;

    let outcome = async {
        need!(
            focus(hwnd, &rect),
            "the desktop never gave the app window the keyboard focus"
        );
        need!(
            droidwin_lib::input::hook::foreground_process() == Some(window.pid),
            "the hook's own focus lookup sees {:?}, not the app window at {}",
            droidwin_lib::input::hook::foreground_process(),
            window.pid
        );
        // A polling release runs every 250 ms; let it settle before the first
        // press so nothing from an earlier focus is lifted mid-test.
        tokio::time::sleep(Duration::from_millis(600)).await;

        // ----------------------------------------- one real key, one event ---
        reset_brightness(&adb, &serial).await?;
        press(VK_K).await;
        tokio::time::sleep(Duration::from_millis(900)).await;
        let after = brightness(&adb, &serial).await?;
        need!(
            after == AFTER_ONE_PUSH,
            "a key typed into the app window reaches Android exactly once: brightness {after}, expected {AFTER_ONE_PUSH}"
        );

        // --------------------------------------- held key: the OS auto-repeat ---
        reset_brightness(&adb, &serial).await?;
        for _ in 0..4 {
            // What the keyboard driver sends while a key is held.
            key(VK_K, true);
            tokio::time::sleep(Duration::from_millis(120)).await;
        }
        key(VK_K, false);
        tokio::time::sleep(Duration::from_millis(900)).await;
        let after = brightness(&adb, &serial).await?;
        need!(
            after == AFTER_ONE_PUSH,
            "holding a mapped key fires once instead of once per auto-repeat: brightness {after}, expected {AFTER_ONE_PUSH}"
        );

        // -------------------------------------------- two presses, no pause ---
        reset_brightness(&adb, &serial).await?;
        for _ in 0..2 {
            key(VK_K, true);
            tokio::time::sleep(Duration::from_millis(30)).await;
            key(VK_K, false);
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        tokio::time::sleep(Duration::from_millis(900)).await;
        let after = brightness(&adb, &serial).await?;
        need!(
            after == AFTER_TWO_PUSHES,
            "two quick presses both arrive, in order: brightness {after}, expected {AFTER_TWO_PUSHES}"
        );

        // --------------------------- a modifier released while its key is down ---
        need!(
            pointers(&adb, &serial, display).await?.is_empty(),
            "nothing is pressed before the combo"
        );
        key(VK_CTRL, true);
        tokio::time::sleep(Duration::from_millis(100)).await;
        key(VK_K, true);
        tokio::time::sleep(Duration::from_millis(900)).await;
        let combo = pointers(&adb, &serial, display).await?;
        eprintln!("pointers on display {display} while Ctrl+K is held: {combo:?}");
        need!(
            !combo.is_empty(),
            "Ctrl+K reaches Android as a held point on display {display}"
        );
        key(VK_CTRL, false); // released first, on purpose
        tokio::time::sleep(Duration::from_millis(200)).await;
        key(VK_K, false);
        tokio::time::sleep(Duration::from_millis(900)).await;
        need!(
            pointers(&adb, &serial, display).await?.is_empty(),
            "releasing Ctrl before K still lifts the point the combo pressed"
        );

        // ------------------ two modifiers, released one before the other ------
        // Ctrl+Shift+K is its own binding, and the key-up arrives with neither
        // modifier down, so it can only find that binding through the press it
        // belongs to. Shift rather than Alt on purpose: Alt alone opens a
        // window's menu, a different mechanism that would make this flaky.
        key(VK_CTRL, true);
        key(VK_SHIFT, true);
        tokio::time::sleep(Duration::from_millis(80)).await;
        key(VK_K, true);
        tokio::time::sleep(Duration::from_millis(900)).await;
        need!(
            !pointers(&adb, &serial, display).await?.is_empty(),
            "Ctrl+Shift+K presses its own point"
        );
        key(VK_CTRL, false);
        tokio::time::sleep(Duration::from_millis(150)).await;
        key(VK_SHIFT, false); // the second modifier, released before the key
        tokio::time::sleep(Duration::from_millis(150)).await;
        key(VK_K, false);
        tokio::time::sleep(Duration::from_millis(900)).await;
        need!(
            pointers(&adb, &serial, display).await?.is_empty(),
            "the key-up finds Ctrl+Shift+K after both modifiers are up, so nothing latches"
        );

        // --------------------------- a held key, and the focus leaving it ---
        key(VK_L, true);
        tokio::time::sleep(Duration::from_millis(900)).await;
        need!(
            !pointers(&adb, &serial, display).await?.is_empty(),
            "a held key presses a point"
        );
        // A real Alt+Tab: the OS moves the focus, and the poll has to notice.
        key(VK_MENU, true);
        key(VK_TAB, true);
        tokio::time::sleep(Duration::from_millis(150)).await;
        key(VK_TAB, false);
        key(VK_MENU, false);
        let started = std::time::Instant::now();
        let mut released = None;
        while started.elapsed() < Duration::from_secs(5) {
            if pointers(&adb, &serial, display).await?.is_empty() {
                released = Some(started.elapsed());
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        eprintln!("latch released after {released:?}");
        need!(
            released.is_some(),
            "Alt-Tab away with a key held must not leave a finger on the app"
        );
        need!(
            !foreground_is(hwnd),
            "the focus really did move away, so the release was the hook's doing"
        );
        key(VK_L, false); // the key-up that now goes to another window
        tokio::time::sleep(Duration::from_millis(400)).await;
        need!(
            pointers(&adb, &serial, display).await?.is_empty(),
            "the late key-up does not press anything"
        );

        // -------------------------------------- and the key still works after ---
        need!(focus(hwnd, &rect), "the window can be focused again");
        need!(
            droidwin_lib::input::hook::foreground_process() == Some(window.pid),
            "the hook sees the refocused window"
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        key(VK_L, true);
        tokio::time::sleep(Duration::from_millis(900)).await;
        need!(
            !pointers(&adb, &serial, display).await?.is_empty(),
            "the same key works after the focus came back"
        );
        key(VK_L, false);
        tokio::time::sleep(Duration::from_millis(900)).await;
        need!(
            pointers(&adb, &serial, display).await?.is_empty(),
            "and releases cleanly"
        );
        Ok(())
    }
    .await;

    // Whatever the outcome, the machine goes back the way it was found.
    let _ = shell(
        &adb,
        &serial,
        &format!("settings put system screen_brightness {original_brightness}"),
    )
    .await;
    let _ = shell(
        &adb,
        &serial,
        &format!("settings put system screen_brightness_mode {original_mode}"),
    )
    .await;
    let _ = controller.reset(PACKAGE);
    let _ = runtime.close_window(PACKAGE).await;
    outcome
}

#[cfg(not(windows))]
#[test]
#[ignore = "keyboard mapping is Windows-only"]
fn real_desktop_keys_reach_android_through_the_hook() {
    eprintln!("the keyboard hook only exists on Windows");
}
