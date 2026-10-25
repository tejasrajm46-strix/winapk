// Hide the console window in release builds; keep it in debug for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    droidwin_lib::run()
}
