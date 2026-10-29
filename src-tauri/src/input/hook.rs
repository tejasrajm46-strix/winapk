//! Windows low-level keyboard hook.
//!
//! DroidWin does not own the app windows: scrcpy does, and scrcpy forwards the
//! keyboard itself. So mapping cannot live inside the window — the only place
//! that sees a key *before* scrcpy does is a `WH_KEYBOARD_LL` hook, and that is
//! what this module installs.
//!
//! The hook is global but inert by default: it swallows a key only when the
//! foreground process is one of DroidWin's own app windows **and** that package
//! has an enabled mapping for the key. Everything else is handed straight back to
//! the next hook in the chain, which is how scrcpy keeps receiving normal typing.
//!
//! Declared by hand rather than through a binding crate: five user32 functions
//! and one struct are all this needs, and a dependency would be a larger surface
//! than the code it replaces.

use std::sync::{Arc, OnceLock};

use super::InputController;

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    pub type Hhook = *mut c_void;
    pub type Hwnd = *mut c_void;

    pub const WH_KEYBOARD_LL: i32 = 13;
    pub const WM_KEYDOWN: usize = 0x0100;
    pub const WM_KEYUP: usize = 0x0101;
    pub const WM_SYSKEYDOWN: usize = 0x0104;
    pub const WM_SYSKEYUP: usize = 0x0105;

    pub const VK_SHIFT: i32 = 0x10;
    pub const VK_CONTROL: i32 = 0x11;
    pub const VK_MENU: i32 = 0x12;
    pub const VK_LWIN: i32 = 0x5B;
    pub const VK_RWIN: i32 = 0x5C;

    #[repr(C)]
    #[derive(Default)]
    pub struct KbdLlHookStruct {
        pub vk_code: u32,
        pub scan_code: u32,
        pub flags: u32,
        pub time: u32,
        pub extra_info: usize,
    }

    #[repr(C)]
    pub struct Msg {
        pub hwnd: Hwnd,
        pub message: u32,
        pub w_param: usize,
        pub l_param: isize,
        pub time: u32,
        pub point: [i32; 2],
        pub private: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        pub fn SetWindowsHookExW(
            id_hook: i32,
            proc: unsafe extern "system" fn(i32, usize, isize) -> isize,
            module: *mut c_void,
            thread_id: u32,
        ) -> Hhook;
        pub fn CallNextHookEx(hook: Hhook, code: i32, w_param: usize, l_param: isize) -> isize;
        pub fn UnhookWindowsHookEx(hook: Hhook) -> i32;
        pub fn GetForegroundWindow() -> Hwnd;
        pub fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        pub fn GetAsyncKeyState(vk: i32) -> i16;
        pub fn GetMessageW(msg: *mut Msg, hwnd: Hwnd, min: u32, max: u32) -> i32;
        pub fn TranslateMessage(msg: *const Msg) -> i32;
        pub fn DispatchMessageW(msg: *const Msg) -> isize;
    }
}

static CONTROLLER: OnceLock<Arc<InputController>> = OnceLock::new();

/// Install the hook on its own thread. The thread runs the message loop the hook
/// requires and lives for the process; the hook itself decides per key.
pub fn start(controller: Arc<InputController>) {
    if CONTROLLER.set(controller).is_err() {
        return;
    }
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("droidwin-keyhook".into())
            .spawn(|| unsafe {
                let hook = win::SetWindowsHookExW(
                    win::WH_KEYBOARD_LL,
                    keyboard_proc,
                    std::ptr::null_mut(),
                    0,
                );
                if hook.is_null() {
                    tracing::warn!(
                        "the keyboard hook could not be installed; mapped keys are inactive"
                    );
                    return;
                }
                tracing::info!("keyboard mapping hook installed");
                let mut msg: win::Msg = std::mem::zeroed();
                // A null window handle with no range pumps this thread's queue.
                while win::GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    win::TranslateMessage(&msg);
                    win::DispatchMessageW(&msg);
                }
                win::UnhookWindowsHookEx(hook);
            })
            .ok();
    }
    #[cfg(not(windows))]
    {
        tracing::warn!("keyboard mapping is only implemented on Windows");
    }
}

/// The process owning the window with keyboard focus, when there is one.
pub fn foreground_process() -> Option<u32> {
    #[cfg(windows)]
    {
        unsafe {
            let hwnd = win::GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut pid = 0u32;
            if win::GetWindowThreadProcessId(hwnd, &mut pid) == 0 {
                return None;
            }
            (pid != 0).then_some(pid)
        }
    }
    #[cfg(not(windows))]
    {
        None
    }
}

#[cfg(windows)]
unsafe extern "system" fn keyboard_proc(code: i32, w_param: usize, l_param: isize) -> isize {
    if code >= 0 && l_param != 0 {
        let info = &*(l_param as *const win::KbdLlHookStruct);
        let is_down = matches!(w_param, win::WM_KEYDOWN | win::WM_SYSKEYDOWN);
        let is_up = matches!(w_param, win::WM_KEYUP | win::WM_SYSKEYUP);
        // Synthetic events are handled like real ones: a mapper that acted on
        // them would loop only if it also injected keys, and DroidWin never
        // does — it dispatches through adb, not through the keyboard.
        if is_down || is_up {
            if let Some(controller) = CONTROLLER.get() {
                if controller.on_key(info.vk_code, is_down, modifiers_down()) {
                    // Consumed: scrcpy never sees this key, so no duplicate input.
                    return 1;
                }
            }
        }
    }
    win::CallNextHookEx(std::ptr::null_mut(), code, w_param, l_param)
}

/// Modifier bitmask: Ctrl=1, Alt=2, Shift=4, Win=8.
#[cfg(windows)]
fn modifiers_down() -> u32 {
    unsafe {
        let down = |vk| win::GetAsyncKeyState(vk) as u16 & 0x8000 != 0;
        (down(win::VK_CONTROL) as u32)
            | ((down(win::VK_MENU) as u32) << 1)
            | ((down(win::VK_SHIFT) as u32) << 2)
            | (((down(win::VK_LWIN) || down(win::VK_RWIN)) as u32) << 3)
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
fn modifiers_down() -> u32 {
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_can_be_asked_for_without_a_window() {
        // No assertion about a value: a headless test session may have no
        // foreground window at all. The call must simply not panic.
        let _ = foreground_process();
    }

    #[test]
    fn modifiers_are_reported_as_a_bool() {
        let _ = modifiers_down();
    }
}
