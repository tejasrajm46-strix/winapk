// Hide the console window in release builds; keep it in debug for logs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // MCP clients launch a process and speak newline-delimited JSON-RPC on its
    // stdin/stdout. That mode must not build a window, a webview or a runtime;
    // it only forwards messages to the running app.
    if args.iter().any(|arg| arg == "--mcp-stdio") {
        std::process::exit(droidwin_lib::agent::stdio::run());
    }
    // A subcommand is a client for the running window, not a second app: it
    // prints one JSON answer and exits. An .apk path or no argument at all still
    // starts the window.
    if let Some(code) = droidwin_lib::cli::maybe_run(&args) {
        std::process::exit(code);
    }
    droidwin_lib::run()
}
