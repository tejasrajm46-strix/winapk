//! What window does this process own, and how big is its client area?
//!
//! Used when a window host ignores or reverts a requested frame: the answer says
//! whether the lookup found the window at all, before any resizing is blamed.
//!
//! ```text
//! cargo run --example window_probe -- 1234
//! cargo run --example window_probe -- 1234 933 420
//! ```

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(pid) = args.next().and_then(|value| value.parse::<u32>().ok()) else {
        eprintln!("usage: window_probe <pid> [width height]");
        std::process::exit(2);
    };
    let hwnd = droidwin_lib::proc::window_for_pid(pid);
    println!("pid {pid}: hwnd {hwnd:?}");
    let Some(hwnd) = hwnd else {
        return;
    };
    println!("client before {:?}", droidwin_lib::proc::client_size(hwnd));
    let size = args
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .zip(args.next().and_then(|value| value.parse::<u32>().ok()));
    if let Some((width, height)) = size {
        println!(
            "resize {width}x{height}: {}",
            droidwin_lib::proc::resize_client(hwnd, width, height)
        );
        std::thread::sleep(std::time::Duration::from_millis(400));
        println!("client after {:?}", droidwin_lib::proc::client_size(hwnd));
    }
}
