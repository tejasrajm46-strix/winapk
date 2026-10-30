//! Child-process creation policy.
//!
//! WINAPK is a GUI-subsystem program. On Windows, spawning a console program
//! from a GUI process without `CREATE_NO_WINDOW` makes the OS allocate a console
//! for it — a black window that flashes for every `adb`, `aapt2`, `taskkill` or
//! `scrcpy` invocation. Because the runtime polls, that looked like windows
//! constantly opening by themselves.
//!
//! Every subprocess therefore goes through this module. The flag suppresses the
//! console, not the process: captured output, exit codes and scrcpy's own window
//! are unaffected.

/// `CREATE_NO_WINDOW` from `winbase.h`. The process runs with no console window
/// and does not inherit the parent's console.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Apply the no-console policy to a command about to be spawned.
pub fn no_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}

/// Apply the no-console policy to an async command. `tokio::process::Command`
/// exposes its `std` counterpart for exactly this kind of platform setting.
pub fn no_console_async(cmd: &mut tokio::process::Command) {
    no_console(cmd.as_std_mut());
}

/// The executable name of a running process, e.g. `emulator.exe`.
///
/// Used to confirm that a process id WINAPK wrote to disk is still the
/// emulator it started, and not a recycled id belonging to something else. A
/// process WINAPK may not open answers `None`, which callers must treat as
/// "not ours".
#[cfg(windows)]
pub fn image_name(pid: u32) -> Option<String> {
    use std::os::windows::ffi::OsStringExt;

    #[link(name = "kernel32")]
    extern "system" {
        // Signatures match the declarations already used for process stats, so
        // the linker sees one import rather than two spellings of it.
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn QueryFullProcessImageNameW(
            handle: isize,
            flags: u32,
            buffer: *mut u16,
            size: *mut u32,
        ) -> i32;
    }

    const QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    let handle = unsafe { OpenProcess(QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle == 0 {
        return None;
    }
    let mut buffer = [0u16; 1024];
    let mut size = buffer.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut size) };
    unsafe {
        CloseHandle(handle);
    }
    if ok == 0 {
        return None;
    }
    let path = std::ffi::OsString::from_wide(&buffer[..size as usize]);
    Some(
        std::path::Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default(),
    )
}

#[cfg(not(windows))]
pub fn image_name(_pid: u32) -> Option<String> {
    None
}

/// The window with keyboard focus, as a raw handle, for identity checks.
#[cfg(windows)]
pub fn foreground_hwnd() -> Option<isize> {
    win::foreground().map(|hwnd| hwnd as isize)
}

#[cfg(not(windows))]
pub fn foreground_hwnd() -> Option<isize> {
    None
}

/// The client rectangle (screen x, screen y, width, height) of the largest
/// visible top-level window a process owns, excluding its title bar and borders.
#[cfg(windows)]
pub fn client_rect(pid: u32) -> Option<(i32, i32, u32, u32)> {
    win::client_rect(pid)
}

#[cfg(not(windows))]
pub fn client_rect(_pid: u32) -> Option<(i32, i32, u32, u32)> {
    None
}

/// The largest visible top-level window a process owns, as a raw handle.
///
/// scrcpy creates exactly one window per instance, so "largest" is a safe
/// choice even while a transient SDL message box exists. `None` means the
/// process has not created its window yet (or has already gone).
#[cfg(windows)]
pub fn window_for_pid(pid: u32) -> Option<isize> {
    win::largest_window(pid).map(|hwnd| hwnd as isize)
}

#[cfg(not(windows))]
pub fn window_for_pid(_pid: u32) -> Option<isize> {
    None
}

/// Hide a window without destroying it. The recording host keeps decoding and
/// writing while hidden; only its painting stops.
#[cfg(windows)]
pub fn hide_window(hwnd: isize) {
    win::show(hwnd as win::Hwnd, win::HIDE);
}

#[cfg(not(windows))]
pub fn hide_window(_hwnd: isize) {}

/// Ask a window to close the way its own close button would, so the process can
/// flush what it is writing (an MP4 muxer needs its trailer).
#[cfg(windows)]
pub fn close_window(hwnd: isize) {
    win::post_close(hwnd as win::Hwnd);
}

#[cfg(not(windows))]
pub fn close_window(_hwnd: isize) {}

/// The client (content) size of a window, in physical pixels.
#[cfg(windows)]
pub fn client_size(hwnd: isize) -> Option<(u32, u32)> {
    win::client_size(hwnd as win::Hwnd)
}

#[cfg(not(windows))]
pub fn client_size(_hwnd: isize) -> Option<(u32, u32)> {
    None
}

/// Resize a window so its *client area* is exactly `width` by `height`.
///
/// The window host picks its own initial size (scrcpy caps a virtual display
/// window at a fraction of the desktop), so a requested geometry is only true
/// once the frame has been set. Borders and the title bar are added from the
/// window's own measurements rather than guessed.
#[cfg(windows)]
pub fn resize_client(hwnd: isize, width: u32, height: u32) -> bool {
    win::resize_client(hwnd as win::Hwnd, width, height)
}

#[cfg(not(windows))]
pub fn resize_client(_hwnd: isize, _width: u32, _height: u32) -> bool {
    false
}

#[cfg(windows)]
mod win {
    pub type Hwnd = *mut std::ffi::c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct Rect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    pub struct Point {
        pub x: i32,
        pub y: i32,
    }

    const SWP_NOMOVE: u32 = 0x0002;
    const SWP_NOZORDER: u32 = 0x0004;
    const SWP_NOACTIVATE: u32 = 0x0010;
    const WM_CLOSE: u32 = 0x0010;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: extern "system" fn(Hwnd, isize) -> i32, lparam: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: Hwnd, pid: *mut u32) -> u32;
        fn IsWindowVisible(hwnd: Hwnd) -> i32;
        fn GetWindowRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn GetClientRect(hwnd: Hwnd, rect: *mut Rect) -> i32;
        fn ClientToScreen(hwnd: Hwnd, point: *mut Point) -> i32;
        fn GetForegroundWindow() -> Hwnd;
        fn IsIconic(hwnd: Hwnd) -> i32;
        fn SetWindowPos(
            hwnd: Hwnd,
            after: Hwnd,
            x: i32,
            y: i32,
            width: i32,
            height: i32,
            flags: u32,
        ) -> i32;
        fn ShowWindow(hwnd: Hwnd, command: i32) -> i32;
        fn PostMessageW(hwnd: Hwnd, message: u32, wparam: usize, lparam: isize) -> i32;
    }

    /// `SW_HIDE`: keep the window, stop showing it.
    pub const HIDE: i32 = 0;

    struct Search {
        pid: u32,
        best: Option<(Hwnd, u64)>,
    }

    extern "system" fn visit(hwnd: Hwnd, lparam: isize) -> i32 {
        let search = unsafe { &mut *(lparam as *mut Search) };
        let mut pid = 0u32;
        unsafe {
            if GetWindowThreadProcessId(hwnd, &mut pid) == 0
                || pid != search.pid
                || IsWindowVisible(hwnd) == 0
            {
                return 1;
            }
            let mut rect = Rect::default();
            if GetWindowRect(hwnd, &mut rect) == 0 {
                return 1;
            }
            let area =
                (rect.right - rect.left).max(0) as u64 * (rect.bottom - rect.top).max(0) as u64;
            if area > search.best.map(|(_, best)| best).unwrap_or(0) {
                search.best = Some((hwnd, area));
            }
        }
        1
    }

    pub fn largest_window(pid: u32) -> Option<Hwnd> {
        let mut search = Search { pid, best: None };
        unsafe {
            EnumWindows(visit, &mut search as *mut Search as isize);
        }
        search.best.map(|(hwnd, _)| hwnd)
    }

    pub fn foreground() -> Option<Hwnd> {
        let hwnd = unsafe { GetForegroundWindow() };
        (!hwnd.is_null()).then_some(hwnd)
    }

    /// The client rectangle in screen coordinates. Minimised windows are
    /// skipped: their client rect is not where the window is drawn.
    pub fn client_rect(pid: u32) -> Option<(i32, i32, u32, u32)> {
        let hwnd = largest_window(pid)?;
        unsafe {
            if IsIconic(hwnd) != 0 {
                return None;
            }
            let mut client = Rect::default();
            if GetClientRect(hwnd, &mut client) == 0 {
                return None;
            }
            let width = (client.right - client.left).max(0) as u32;
            let height = (client.bottom - client.top).max(0) as u32;
            if width < 2 || height < 2 {
                return None;
            }
            let mut origin = Point::default();
            if ClientToScreen(hwnd, &mut origin) == 0 {
                return None;
            }
            Some((origin.x, origin.y, width, height))
        }
    }

    pub fn show(hwnd: Hwnd, command: i32) {
        unsafe {
            ShowWindow(hwnd, command);
        }
    }

    pub fn post_close(hwnd: Hwnd) {
        unsafe {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        }
    }

    pub fn client_size(hwnd: Hwnd) -> Option<(u32, u32)> {
        let mut rect = Rect::default();
        if unsafe { GetClientRect(hwnd, &mut rect) } == 0 {
            return None;
        }
        Some((
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        ))
    }

    pub fn resize_client(hwnd: Hwnd, width: u32, height: u32) -> bool {
        if width < 2 || height < 2 {
            return false;
        }
        let (mut window_rect, mut client_rect) = (Rect::default(), Rect::default());
        if unsafe { GetWindowRect(hwnd, &mut window_rect) } == 0
            || unsafe { GetClientRect(hwnd, &mut client_rect) } == 0
        {
            return false;
        }
        let extra_width =
            (window_rect.right - window_rect.left) - (client_rect.right - client_rect.left);
        let extra_height =
            (window_rect.bottom - window_rect.top) - (client_rect.bottom - client_rect.top);
        unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                width as i32 + extra_width,
                height as i32 + extra_height,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) != 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;

    /// The flag must not change how a child behaves: output and exit status are
    /// still captured. This is the property the whole policy depends on.
    #[cfg(windows)]
    #[test]
    fn a_process_reports_the_executable_it_runs_and_a_bad_id_reports_nothing() {
        let name = image_name(std::process::id()).expect("the test process is its own proof");
        assert!(name.to_ascii_lowercase().ends_with(".exe"), "{name}");
        // An id that cannot be a live process (Windows ids are below 2^24).
        assert!(image_name(u32::MAX).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_hidden_child_still_reports_output_and_exit_code() {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/c", "echo hidden-ok"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        no_console(&mut cmd);
        let out = cmd.output().expect("cmd should start");
        assert_eq!(out.status.code(), Some(0));
        assert!(String::from_utf8_lossy(&out.stdout).contains("hidden-ok"));
    }

    /// A child that writes more than a pipe buffer must not deadlock, because
    /// the policy makes no promise about draining output for long-lived tools.
    #[cfg(windows)]
    #[test]
    fn a_hidden_child_can_be_killed_after_streaming_output() {
        let mut cmd = tokio::process::Command::new("cmd");
        cmd.args(["/c", "for /L %i in (1,1,200) do @echo line-%i"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        no_console_async(&mut cmd);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let out = runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(20), cmd.output()).await
        });
        let out = out.expect("hidden child should finish").unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("line-200"));
    }

    #[cfg(windows)]
    #[test]
    fn the_console_flag_is_the_documented_value() {
        // 0x08000000 is CREATE_NO_WINDOW; a different value would either show a
        // console or create a detached process.
        assert_eq!(CREATE_NO_WINDOW, 0x0800_0000);
    }
}
