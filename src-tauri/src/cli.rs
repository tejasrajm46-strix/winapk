//! The `droidwin` command line.
//!
//! It is a client, not a second runtime: every command is a request to the
//! running DroidWin window's own loopback endpoint, which dispatches it through
//! the same `ControlService` the MCP tools and the desktop UI use. That keeps one
//! emulator, one adb server, one window registry and one permission gate — and
//! it means an agent does not have to load a tool schema for every small
//! operation to change a performance plan or write a mapping.
//!
//! ```text
//! droidwin status                 one compact JSON answer on stdout
//! droidwin runtime restart
//! droidwin apps install ./game.apk --launch
//! droidwin perf preset performance
//! droidwin mapping save com.example.game --key W --tap 12,80
//! droidwin tools                  every command, without a description payload
//! ```
//!
//! Output is one JSON object. `ok` says whether the command succeeded; `data`
//! carries the result. Exit status is 0 for success, 1 for a refused command, 2
//! when the app's endpoint cannot be reached, and 64 for a usage error. The
//! command needs DroidWin running with the agent endpoint enabled — it never
//! starts an emulator of its own.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAILED: i32 = 1;
pub const EXIT_NO_ENDPOINT: i32 = 2;
pub const EXIT_USAGE: i32 = 64;

/// How long a command may take. Starting or restarting the runtime includes an
/// Android cold boot, which the runtime itself budgets at 450 s.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(900);

struct CommandSpec {
    name: &'static str,
    usage: &'static str,
    does: &'static str,
}

/// One line per command, in the order the README documents them. This is the
/// whole discovery surface: an agent reads it once instead of loading every MCP
/// tool's schema.
const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "status",
        usage: "droidwin status",
        does: "Runtime state, Android's report and every open window",
    },
    CommandSpec {
        name: "devices",
        usage: "droidwin devices",
        does: "Devices adb can see, marking the managed emulator",
    },
    CommandSpec {
        name: "runtime.start",
        usage: "droidwin runtime start",
        does: "Start the runtime, or reuse it when it is already healthy",
    },
    CommandSpec {
        name: "runtime.stop",
        usage: "droidwin runtime stop",
        does: "Stop the emulator DroidWin started and its windows",
    },
    CommandSpec {
        name: "runtime.restart",
        usage: "droidwin runtime restart",
        does: "Stop, then start, so a saved performance plan applies",
    },
    CommandSpec {
        name: "apps.list",
        usage: "droidwin apps list",
        does: "Installed third-party packages",
    },
    CommandSpec {
        name: "apps.info",
        usage: "droidwin apps info <package>",
        does: "One package, its window and its display",
    },
    CommandSpec {
        name: "apps.install",
        usage: "droidwin apps install <file.apk> [--launch] [--allow-old-target]",
        does: "Validate and install an APK",
    },
    CommandSpec {
        name: "apps.open",
        usage: "droidwin apps open <package>",
        does: "Open an app in its own window and return its display",
    },
    CommandSpec {
        name: "apps.close",
        usage: "droidwin apps close <package>",
        does: "Close one app window; its process keeps running",
    },
    CommandSpec {
        name: "apps.stop",
        usage: "droidwin apps stop <package>",
        does: "Close the window and force-stop the app",
    },
    CommandSpec {
        name: "perf.get",
        usage: "droidwin perf",
        does: "Requested plan next to the values Android confirms",
    },
    CommandSpec {
        name: "perf.preset",
        usage: "droidwin perf preset balanced|performance|compatibility",
        does: "Save a named performance plan",
    },
    CommandSpec {
        name: "perf.set",
        usage: "droidwin perf set [--mode auto|custom] [--cores 2|4|6|8] [--ram 2048..8192] [--gpu host|software|auto]",
        does: "Save a validated custom plan; the next start applies it",
    },
    CommandSpec {
        name: "mapping.get",
        usage: "droidwin mapping get <package>",
        does: "A package's control mapping and display geometry",
    },
    CommandSpec {
        name: "mapping.save",
        usage: "droidwin mapping save <package> --key W --tap 50,80 | --hold 50,80 | --keycode KEYCODE_SPACE",
        does: "Add or replace one mapping",
    },
    CommandSpec {
        name: "mapping.remove",
        usage: "droidwin mapping remove <package> --key W",
        does: "Delete one mapping",
    },
    CommandSpec {
        name: "mapping.reset",
        usage: "droidwin mapping reset <package>",
        does: "Delete every mapping and switch mapping off",
    },
    CommandSpec {
        name: "mapping.enable",
        usage: "droidwin mapping enable <package>",
        does: "Arm this package's mapping",
    },
    CommandSpec {
        name: "mapping.disable",
        usage: "droidwin mapping disable <package>",
        does: "Disarm mapping and release held touches",
    },
    CommandSpec {
        name: "capture.screenshot",
        usage: "droidwin capture screenshot [--out <folder>]",
        does: "Save a PNG of the phone's display",
    },
    CommandSpec {
        name: "capture.record.start",
        usage: "droidwin capture record start",
        does: "Record Android's screen with screenrecord",
    },
    CommandSpec {
        name: "capture.record.stop",
        usage: "droidwin capture record stop",
        does: "Finish the recording and save the MP4",
    },
    CommandSpec {
        name: "capture.record.status",
        usage: "droidwin capture record status",
        does: "Whether a recording is running",
    },
    CommandSpec {
        name: "nav.back",
        usage: "droidwin nav back [--package <package>] [--display <id>]",
        does: "Android Back on the phone or one app window",
    },
    CommandSpec {
        name: "nav.home",
        usage: "droidwin nav home [--package <package>] [--display <id>]",
        does: "Android Home on the phone or one app window",
    },
    CommandSpec {
        name: "nav.recents",
        usage: "droidwin nav recents [--package <package>] [--display <id>]",
        does: "Android recent apps on the phone or one app window",
    },
];

/// `droidwin tools` is answered locally: it is discovery, not a device action,
/// and it must work even when the app is not running.
fn command_table() -> Value {
    let commands: Vec<Value> = COMMANDS
        .iter()
        .map(|spec| {
            json!({
                "name": spec.name,
                "usage": spec.usage,
                "does": spec.does,
                "changes": if crate::agent::control::cli_command_is_control(spec.name) {
                    "control"
                } else {
                    "read"
                },
            })
        })
        .collect();
    json!({
        "ok": true,
        "commands": commands,
        "exit_codes": {
            "0": "the command succeeded",
            "1": "the command was refused or failed; error says why",
            "2": "DroidWin is not running with the agent endpoint enabled",
            "64": "the command line was not understood"
        }
    })
}

/// What `maybe_run` decided a set of arguments means.
enum Built {
    /// A local answer that needs no endpoint.
    Local(Value),
    /// A command for the app's `/cli` route.
    Call { command: String, arguments: Value },
    /// A raw MCP `tools/call`, for every tool the command table does not
    /// cover (screens, elements, waits, workflows, triggers, skills).
    Mcp { name: String, arguments: Value },
}

/// If these arguments are a CLI invocation, run it and return its exit code.
/// `None` means "not a CLI command": the process should start the window.
pub fn maybe_run(args: &[String]) -> Option<i32> {
    let first = args.first()?.as_str();
    let local = matches!(
        first,
        "help" | "--help" | "-h" | "version" | "--version" | "-V" | "tools"
    );
    let known = COMMANDS
        .iter()
        .any(|spec| spec.name.split('.').next() == Some(first));
    if !local && !known && first != "call" {
        return None;
    }
    #[cfg(windows)]
    attach_console();
    Some(run(args))
}

fn run(args: &[String]) -> i32 {
    let pretty = args.iter().any(|arg| arg == "--pretty");
    let filtered: Vec<String> = args
        .iter()
        .filter(|arg| *arg != "--pretty")
        .cloned()
        .collect();

    let built = match build(&filtered) {
        Ok(built) => built,
        Err(message) => {
            eprintln!(
                "{}",
                json!({
                    "ok": false,
                    "error": { "code": "usage", "summary": message }
                })
            );
            return EXIT_USAGE;
        }
    };

    let built = match built {
        Built::Local(value) => {
            emit(&value, pretty);
            return EXIT_OK;
        }
        other => other,
    };

    let endpoint = match discover() {
        Ok(endpoint) => endpoint,
        Err(detail) => {
            emit(
                &json!({
                    "ok": false,
                    "error": {
                        "code": "endpoint_not_running",
                        "summary": "DroidWin is not running with the agent endpoint enabled.",
                        "detail": detail
                    }
                }),
                pretty,
            );
            return EXIT_NO_ENDPOINT;
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            emit(
                &json!({
                    "ok": false,
                    "error": { "code": "backend_failure", "summary": "The command line could not start.", "detail": err.to_string() }
                }),
                pretty,
            );
            return EXIT_FAILED;
        }
    };

    match built {
        Built::Call { command, arguments } => {
            let body = json!({ "command": command, "args": arguments });
            match runtime.block_on(post(&endpoint.cli_url, &endpoint.token, &body)) {
                Ok(reply) => {
                    let ok = reply.get("ok").and_then(Value::as_bool).unwrap_or(false);
                    emit(&reply, pretty);
                    if ok {
                        EXIT_OK
                    } else {
                        EXIT_FAILED
                    }
                }
                Err(err) => {
                    emit(&transport_failure(err), pretty);
                    EXIT_NO_ENDPOINT
                }
            }
        }
        Built::Mcp { name, arguments } => {
            let body = json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": name, "arguments": arguments }
            });
            match runtime.block_on(post(&endpoint.mcp_url, &endpoint.token, &body)) {
                Ok(reply) => {
                    let failed = reply["result"]["isError"].as_bool().unwrap_or(true);
                    let answer = json!({
                        "ok": !failed,
                        "tool": name,
                        "summary": reply["result"]["content"][0]["text"],
                        "data": reply["result"]["structuredContent"],
                    });
                    emit(&answer, pretty);
                    if failed {
                        EXIT_FAILED
                    } else {
                        EXIT_OK
                    }
                }
                Err(err) => {
                    emit(&transport_failure(err), pretty);
                    EXIT_NO_ENDPOINT
                }
            }
        }
        Built::Local(_) => EXIT_OK,
    }
}

fn transport_failure(detail: String) -> Value {
    json!({
        "ok": false,
        "error": {
            "code": "endpoint_unreachable",
            "summary": "DroidWin's endpoint did not answer.",
            "detail": detail
        }
    })
}

/// One JSON object on stdout. A failed command still prints its answer, because
/// the answer carries the reason; only usage errors go to stderr.
fn emit(value: &Value, pretty: bool) {
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
    .unwrap_or_else(|_| "{\"ok\":false}".into());
    println!("{text}");
}

struct Parsed {
    positional: Vec<String>,
    flags: HashMap<String, String>,
    switches: Vec<String>,
}

impl Parsed {
    fn flag(&self, name: &str) -> Option<&str> {
        self.flags.get(name).map(String::as_str)
    }

    fn switch(&self, name: &str) -> bool {
        self.switches.iter().any(|set| set == name)
    }

    /// Refuse a flag this command does not read, instead of ignoring a typo and
    /// reporting success for something the caller did not ask for.
    fn only(&self, flags: &[&str], switches: &[&str]) -> Result<(), String> {
        for name in self.flags.keys() {
            if !flags.contains(&name.as_str()) {
                return Err(format!("--{name} is not an option for this command."));
            }
        }
        for name in &self.switches {
            if !switches.contains(&name.as_str()) {
                return Err(format!("--{name} is not an option for this command."));
            }
        }
        Ok(())
    }
}

fn parse(tokens: &[String], switches: &[&str]) -> Result<Parsed, String> {
    let mut positional = Vec::new();
    let mut flags = HashMap::new();
    let mut taken = Vec::new();
    let mut index = 0;
    while index < tokens.len() {
        let token = &tokens[index];
        if let Some(name) = token.strip_prefix("--") {
            if switches.contains(&name) {
                taken.push(name.to_string());
                index += 1;
                continue;
            }
            let value = tokens
                .get(index + 1)
                .ok_or_else(|| format!("--{name} needs a value."))?;
            if value.starts_with("--") {
                return Err(format!("--{name} needs a value."));
            }
            flags.insert(name.to_string(), value.clone());
            index += 2;
        } else {
            positional.push(token.clone());
            index += 1;
        }
    }
    Ok(Parsed {
        positional,
        flags,
        switches: taken,
    })
}

fn build(tokens: &[String]) -> Result<Built, String> {
    let first = tokens
        .first()
        .ok_or_else(|| "Run `droidwin help` for the command list.".to_string())?
        .as_str();
    match first {
        "help" | "--help" | "-h" => Ok(Built::Local(json!({
            "ok": true,
            "usage": "droidwin <command> [arguments]",
            "commands": command_table()["commands"],
            "notes": [
                "Requires DroidWin running with the agent endpoint enabled (Agent screen).",
                "Every answer is one JSON object; --pretty indents it.",
                "`droidwin call <tool> '<json>'` reaches every MCP tool by name."
            ]
        }))),
        "version" | "--version" | "-V" => Ok(Built::Local(json!({
            "ok": true,
            "name": "droidwin",
            "version": env!("CARGO_PKG_VERSION")
        }))),
        "tools" => Ok(Built::Local(command_table())),
        "call" => build_call(&tokens[1..]),
        "status" => simple(tokens, "status", "status"),
        "devices" => simple(tokens, "devices", "devices"),
        "runtime" => {
            let action = tokens.get(1).map(String::as_str).unwrap_or("");
            let command = match action {
                "start" => "runtime.start",
                "stop" => "runtime.stop",
                "restart" => "runtime.restart",
                other => {
                    return Err(format!(
                        "`droidwin runtime {other}` is not a command. Use start, stop or restart."
                    ))
                }
            };
            if tokens.len() > 2 {
                return Err("`droidwin runtime` takes no arguments.".into());
            }
            Ok(call(command, json!({})))
        }
        "apps" => build_apps(&tokens[1..]),
        "perf" => build_perf(&tokens[1..]),
        "mapping" => build_mapping(&tokens[1..]),
        "capture" => build_capture(&tokens[1..]),
        "nav" => build_nav(&tokens[1..]),
        other => Err(format!(
            "`droidwin {other}` is not a command. Run `droidwin tools` for the list."
        )),
    }
}

fn call(command: &str, arguments: Value) -> Built {
    Built::Call {
        command: command.to_string(),
        arguments,
    }
}

/// A command that takes nothing at all. Extra arguments are refused instead of
/// ignored, so a typo cannot look like a successful command.
fn simple(tokens: &[String], command: &str, usage: &str) -> Result<Built, String> {
    if tokens.len() > 1 {
        return Err(format!("`droidwin {usage}` takes no arguments."));
    }
    Ok(call(command, json!({})))
}

fn package_argument(value: Option<&String>, usage: &str) -> Result<String, String> {
    let package =
        value.ok_or_else(|| format!("This command needs a package id. Usage: {usage}"))?;
    if package.trim().is_empty() || package.starts_with("--") {
        return Err(format!("That is not a package id. Usage: {usage}"));
    }
    Ok(package.clone())
}

fn build_apps(tokens: &[String]) -> Result<Built, String> {
    let action = tokens.first().map(String::as_str).unwrap_or("");
    match action {
        "list" => simple(tokens, "apps.list", "apps list"),
        "info" => {
            let parsed = parse(&tokens[1..], &[])?;
            parsed.only(&[], &[])?;
            let package =
                package_argument(parsed.positional.first(), "droidwin apps info <package>")?;
            Ok(call("apps.info", json!({ "package_id": package })))
        }
        "install" => {
            let parsed = parse(&tokens[1..], &["launch", "allow-old-target"])?;
            parsed.only(&[], &["launch", "allow-old-target"])?;
            let path = parsed
                .positional
                .first()
                .ok_or("Usage: droidwin apps install <file.apk> [--launch]")?;
            // The app resolves the path in its own process, so a relative path
            // from the caller's shell must be made absolute here.
            Ok(call(
                "apps.install",
                json!({
                    "path": absolute(path),
                    "launch": parsed.switch("launch"),
                    "allow_old_target": parsed.switch("allow-old-target"),
                }),
            ))
        }
        "open" | "close" | "stop" => {
            let parsed = parse(&tokens[1..], &[])?;
            parsed.only(&[], &[])?;
            let usage = format!("droidwin apps {action} <package>");
            let package = package_argument(parsed.positional.first(), &usage)?;
            let command = match action {
                "open" => "apps.open",
                "close" => "apps.close",
                _ => "apps.stop",
            };
            Ok(call(command, json!({ "package_id": package })))
        }
        other => Err(format!(
            "`droidwin apps {other}` is not a command. Use list, info, install, open, close or stop."
        )),
    }
}

fn build_perf(tokens: &[String]) -> Result<Built, String> {
    let action = tokens.first().map(String::as_str).unwrap_or("get");
    match action {
        "get" | "" => simple(tokens, "perf.get", "perf get"),
        "preset" => {
            let parsed = parse(&tokens[1..], &[])?;
            parsed.only(&[], &[])?;
            let preset = parsed
                .positional
                .first()
                .ok_or("Usage: droidwin perf preset balanced|performance|compatibility")?;
            if parsed.positional.len() > 1 {
                return Err("`droidwin perf preset` takes one preset name.".into());
            }
            Ok(call("perf.preset", json!({ "preset": preset })))
        }
        "set" => {
            let parsed = parse(&tokens[1..], &[])?;
            parsed.only(&["mode", "cores", "ram", "gpu"], &[])?;
            let mut arguments = serde_json::Map::new();
            if let Some(mode) = parsed.flag("mode") {
                if mode != "auto" && mode != "custom" {
                    return Err("--mode must be auto or custom.".into());
                }
                arguments.insert("mode".into(), json!(mode));
            }
            if let Some(cores) = parsed.flag("cores") {
                let cores: u32 = cores
                    .parse()
                    .map_err(|_| "--cores needs a whole number (2, 4, 6 or 8).".to_string())?;
                arguments.insert("cores".into(), json!(cores));
            }
            if let Some(ram) = parsed.flag("ram") {
                let ram: u32 = ram
                    .parse()
                    .map_err(|_| "--ram needs megabytes (2048 to 8192).".to_string())?;
                arguments.insert("ram_mb".into(), json!(ram));
            }
            if let Some(gpu) = parsed.flag("gpu") {
                arguments.insert("gpu".into(), json!(gpu));
            }
            if arguments.is_empty() {
                return Err(
                    "Usage: droidwin perf set [--mode auto|custom] [--cores N] [--ram MB] [--gpu host|software|auto]"
                        .into(),
                );
            }
            // The automatic plan derives its own numbers; accepting values it
            // would ignore would report a change that did not happen.
            if parsed.flag("mode") == Some("auto")
                && ["cores", "ram", "gpu"]
                    .iter()
                    .any(|name| parsed.flag(name).is_some())
            {
                return Err(
                    "--mode auto chooses its own vCPUs, memory and GPU; remove --cores, --ram and --gpu or use --mode custom."
                        .into(),
                );
            }
            Ok(call("perf.set", Value::Object(arguments)))
        }
        other => Err(format!(
            "`droidwin perf {other}` is not a command. Use get, preset or set."
        )),
    }
}

fn build_mapping(tokens: &[String]) -> Result<Built, String> {
    // `droidwin mapping <package>` is the common read.
    let action = tokens.first().map(String::as_str).unwrap_or("");
    let (action, rest) = match action {
        "get" | "save" | "remove" | "reset" | "enable" | "disable" => (action, &tokens[1..]),
        _ => ("get", tokens),
    };
    let parsed = parse(rest, &[])?;
    let flags: Vec<&str> = match action {
        "save" => vec!["key", "tap", "hold", "keycode"],
        "remove" => vec!["key"],
        _ => Vec::new(),
    };
    parsed.only(&flags, &[])?;
    let usage = format!(
        "droidwin mapping {action} <package>{}",
        match action {
            "save" => " --key <key> (--tap x,y | --hold x,y | --keycode NAME)",
            "remove" => " --key <key>",
            _ => "",
        }
    );
    let package = package_argument(parsed.positional.first(), &usage)?;
    match action {
        "get" => Ok(call("mapping.get", json!({ "package_id": package }))),
        "reset" => Ok(call("mapping.reset", json!({ "package_id": package }))),
        "enable" => Ok(call("mapping.enable", json!({ "package_id": package }))),
        "disable" => Ok(call("mapping.disable", json!({ "package_id": package }))),
        "remove" => {
            let key = parsed
                .flag("key")
                .ok_or("Usage: droidwin mapping remove <package> --key W")?;
            let vk = crate::input::parse_key_spec(key)?;
            Ok(call(
                "mapping.remove",
                json!({ "package_id": package, "key": vk }),
            ))
        }
        _ => {
            let key = parsed
                .flag("key")
                .ok_or("Usage: droidwin mapping save <package> --key W --tap 50,80")?;
            let vk = crate::input::parse_key_spec(key)?;
            let touches = ["tap", "hold"]
                .iter()
                .filter(|name| parsed.flag(name).is_some())
                .count();
            if touches > 1 {
                return Err("Choose one action: --tap or --hold, not both.".into());
            }
            let action = if let Some(point) = parsed.flag("tap") {
                let (x, y) = parse_point(point)?;
                json!({ "kind": "touch_tap", "x": x, "y": y })
            } else if let Some(point) = parsed.flag("hold") {
                let (x, y) = parse_point(point)?;
                json!({ "kind": "touch_hold", "x": x, "y": y })
            } else if let Some(keycode) = parsed.flag("keycode") {
                json!({ "kind": "key_event", "keycode": keycode })
            } else {
                return Err(
                    "A mapping needs an action: --tap x,y, --hold x,y or --keycode KEYCODE_SPACE."
                        .into(),
                );
            };
            Ok(call(
                "mapping.save",
                json!({ "package_id": package, "key": vk, "action": action }),
            ))
        }
    }
}

/// A caller-relative path made absolute, because the app resolves it against
/// its own working directory, not the shell's.
fn absolute(path: &str) -> String {
    let target = std::path::Path::new(path);
    if target.is_absolute() {
        return path.to_string();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(target).display().to_string(),
        Err(_) => path.to_string(),
    }
}

/// `x,y` as percentages of the app's display, which is the coordinate space the
/// Controls editor and the hook already use.
fn parse_point(text: &str) -> Result<(f32, f32), String> {
    let (x, y) = text.split_once(',').ok_or_else(|| {
        format!("\"{text}\" is not a point. Use x,y as percentages, such as 50,80.")
    })?;
    let x: f32 = x
        .trim()
        .parse()
        .map_err(|_| format!("\"{text}\" has no numeric x."))?;
    let y: f32 = y
        .trim()
        .parse()
        .map_err(|_| format!("\"{text}\" has no numeric y."))?;
    if !(0.0..=100.0).contains(&x) || !(0.0..=100.0).contains(&y) {
        return Err(format!(
            "\"{text}\" is outside the display; x and y are percentages from 0 to 100."
        ));
    }
    Ok((x, y))
}

fn build_capture(tokens: &[String]) -> Result<Built, String> {
    match tokens.first().map(String::as_str).unwrap_or("") {
        "screenshot" => {
            let parsed = parse(&tokens[1..], &[])?;
            parsed.only(&["out"], &[])?;
            if !parsed.positional.is_empty() {
                return Err(
                    "`droidwin capture screenshot` takes no arguments besides --out.".into(),
                );
            }
            let mut arguments = serde_json::Map::new();
            if let Some(out) = parsed.flag("out") {
                arguments.insert("out".into(), json!(absolute(out)));
            }
            Ok(call("capture.screenshot", Value::Object(arguments)))
        }
        "record" => match tokens.get(1).map(String::as_str).unwrap_or("") {
            "start" => simple(
                tokens.get(1..).unwrap_or_default(),
                "capture.record.start",
                "capture record start",
            ),
            "stop" => simple(
                tokens.get(1..).unwrap_or_default(),
                "capture.record.stop",
                "capture record stop",
            ),
            "status" => simple(
                tokens.get(1..).unwrap_or_default(),
                "capture.record.status",
                "capture record status",
            ),
            other => Err(format!(
                "`droidwin capture record {other}` is not a command. Use start, stop or status."
            )),
        },
        other => Err(format!(
            "`droidwin capture {other}` is not a command. Use screenshot or record."
        )),
    }
}

fn build_nav(tokens: &[String]) -> Result<Built, String> {
    let action = tokens.first().map(String::as_str).unwrap_or("");
    let command = match action {
        "back" => "nav.back",
        "home" => "nav.home",
        "recents" => "nav.recents",
        other => {
            return Err(format!(
                "`droidwin nav {other}` is not a command. Use back, home or recents."
            ))
        }
    };
    let parsed = parse(&tokens[1..], &[])?;
    parsed.only(&["package", "display"], &[])?;
    if !parsed.positional.is_empty() {
        return Err(format!(
            "`droidwin nav {action}` takes --package or --display, not a package as a bare argument."
        ));
    }
    let mut arguments = serde_json::Map::new();
    if let Some(package) = parsed.flag("package") {
        arguments.insert("package_id".into(), json!(package));
    }
    if let Some(display) = parsed.flag("display") {
        let display: u32 = display
            .parse()
            .map_err(|_| "--display needs a display id.".to_string())?;
        arguments.insert("display_id".into(), json!(display));
    }
    Ok(call(command, Value::Object(arguments)))
}

fn build_call(tokens: &[String]) -> Result<Built, String> {
    let parsed = parse(tokens, &[])?;
    parsed.only(&["args"], &[])?;
    let name = parsed
        .positional
        .first()
        .ok_or("Usage: droidwin call <tool> ['{\"argument\": value}']")?;
    let raw = match parsed.flag("args") {
        Some(value) => value.to_string(),
        None => parsed
            .positional
            .get(1)
            .cloned()
            .unwrap_or_else(|| "{}".to_string()),
    };
    let arguments: Value =
        serde_json::from_str(&raw).map_err(|err| format!("The arguments are not JSON: {err}"))?;
    if !arguments.is_object() {
        return Err("A tool's arguments must be a JSON object.".into());
    }
    Ok(Built::Mcp {
        name: name.clone(),
        arguments,
    })
}

struct Endpoint {
    /// `.../mcp` for `tools/call`.
    mcp_url: String,
    /// `.../cli` for the commands in this table.
    cli_url: String,
    token: String,
}

fn data_dir() -> PathBuf {
    if let Ok(home) = std::env::var("DROIDWIN_HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home);
        }
    }
    dirs::data_dir()
        .map(|dir| dir.join("app.droidwin.desktop"))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Read the URL and the per-session token the running app wrote. A stale file
/// from a crashed process is reported as an unreachable endpoint rather than
/// being retried forever.
fn discover() -> Result<Endpoint, String> {
    let path = data_dir().join("agent").join("mcp.json");
    let text = std::fs::read_to_string(&path).map_err(|_| {
        format!(
            "No endpoint file at {}. Open DroidWin and press Start endpoint on the Agent screen.",
            path.display()
        )
    })?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|e| format!("The endpoint file is not valid JSON: {e}"))?;
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or("The endpoint file has no url.")?;
    let token = value
        .get("token")
        .and_then(Value::as_str)
        .ok_or("The endpoint file has no token.")?;
    if !url.starts_with("http://127.0.0.1:") && !url.starts_with("http://localhost:") {
        return Err(format!("The endpoint {url} is not a loopback URL."));
    }
    let base = url.trim_end_matches('/');
    let base = base.strip_suffix("/mcp").unwrap_or(base);
    Ok(Endpoint {
        mcp_url: url.to_string(),
        cli_url: format!("{base}/cli"),
        token: token.to_string(),
    })
}

/// One POST, one JSON answer. The server closes the connection after replying,
/// so reading to end is the framing.
async fn post(url: &str, token: &str, body: &Value) -> Result<Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    let without_scheme = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("{url} is not an http URL."))?;
    let (authority, path) = match without_scheme.find('/') {
        Some(index) => (&without_scheme[..index], &without_scheme[index..]),
        None => (without_scheme, "/"),
    };
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| format!("{url} has no port."))?;
    let port: u16 = port
        .parse()
        .map_err(|_| format!("{url} has an invalid port."))?;
    if host != "127.0.0.1" && host != "localhost" {
        return Err("DroidWin's endpoint is only reachable on loopback.".into());
    }
    let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAuthorization: Bearer {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        payload.len()
    );

    let work = async {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::ConnectionRefused {
                format!(
                    "Port {port} refused the connection, so the app that wrote the endpoint file is no longer running. Start DroidWin and press Start endpoint on the Agent screen. ({e})"
                )
            } else {
                format!("Could not reach 127.0.0.1:{port}: {e}")
            }
        })?;
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        stream
            .write_all(&payload)
            .await
            .map_err(|e| e.to_string())?;
        stream.flush().await.map_err(|e| e.to_string())?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&response).to_string();
        let (headers, content) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
        let status: u16 = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        if status == 401 {
            return Err(
                "The token in the endpoint file was refused; restart the endpoint in DroidWin."
                    .to_string(),
            );
        }
        if status == 404 {
            return Err(
                "This DroidWin build does not serve the command route; update the app.".to_string(),
            );
        }
        if status == 403 {
            return Err("The endpoint refused this request origin.".to_string());
        }
        if status != 200 {
            return Err(format!("The endpoint answered HTTP {status}: {content}"));
        }
        serde_json::from_str(content)
            .map_err(|e| format!("The endpoint's answer was not JSON: {e}"))
    };

    match tokio::time::timeout(COMMAND_TIMEOUT, work).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "The command did not answer within {} s.",
            COMMAND_TIMEOUT.as_secs()
        )),
    }
}

/// A release build is a windowed executable, so it has no console of its own.
/// Attach to the terminal that started it before the first line is printed;
/// output redirected to a pipe already has a handle.
#[cfg(windows)]
fn attach_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFF_FFFF;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_string).collect()
    }

    fn built(line: &str) -> (String, Value) {
        match build(&tokens(line)).expect("builds") {
            Built::Call { command, arguments } => (command, arguments),
            _ => panic!("{line} was not a command"),
        }
    }

    #[test]
    fn every_documented_command_is_reachable_from_the_parser() {
        let samples = [
            ("status", "status"),
            ("devices", "devices"),
            ("runtime.start", "runtime start"),
            ("runtime.stop", "runtime stop"),
            ("runtime.restart", "runtime restart"),
            ("apps.list", "apps list"),
            ("apps.info", "apps info app.pkg"),
            ("apps.install", "apps install game.apk --launch"),
            ("apps.open", "apps open app.pkg"),
            ("apps.close", "apps close app.pkg"),
            ("apps.stop", "apps stop app.pkg"),
            ("perf.get", "perf get"),
            ("perf.preset", "perf preset performance"),
            ("perf.set", "perf set --cores 8"),
            ("mapping.get", "mapping get app.pkg"),
            ("mapping.save", "mapping save app.pkg --key W --tap 50,80"),
            ("mapping.remove", "mapping remove app.pkg --key W"),
            ("mapping.reset", "mapping reset app.pkg"),
            ("mapping.enable", "mapping enable app.pkg"),
            ("mapping.disable", "mapping disable app.pkg"),
            ("capture.screenshot", "capture screenshot"),
            ("capture.record.start", "capture record start"),
            ("capture.record.stop", "capture record stop"),
            ("capture.record.status", "capture record status"),
            ("nav.back", "nav back"),
            ("nav.home", "nav home"),
            ("nav.recents", "nav recents"),
        ];
        // Every documented command has a sample that builds, and the table has
        // no command the samples miss.
        assert_eq!(samples.len(), COMMANDS.len());
        for (name, line) in samples {
            let (command, _) = built(line);
            assert_eq!(command, name, "{line}");
        }
    }

    #[test]
    fn the_commands_that_change_the_screen_are_marked_as_control() {
        let table = command_table();
        let changes = |name: &str| {
            table["commands"]
                .as_array()
                .unwrap()
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap()["changes"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(changes("status"), "read");
        assert_eq!(changes("mapping.get"), "read");
        assert_eq!(changes("mapping.save"), "control");
        assert_eq!(changes("runtime.restart"), "control");
        assert_eq!(changes("nav.back"), "control");
        assert_eq!(changes("capture.record.start"), "control");
    }

    #[test]
    fn a_key_flag_becomes_the_editors_virtual_key_code() {
        let (_, args) = built("mapping save com.example.game --key Ctrl+Shift+K --tap 12.5,80");
        assert_eq!(args["package_id"], "com.example.game");
        assert_eq!(args["key"], 0x4B | (5 << 8));
        assert_eq!(args["action"]["kind"], "touch_tap");
        assert_eq!(args["action"]["x"], 12.5);
        assert_eq!(args["action"]["y"], 80.0);

        let (_, args) = built("mapping save com.example.game --key Space --keycode KEYCODE_SPACE");
        assert_eq!(args["key"], 0x20);
        assert_eq!(args["action"]["kind"], "key_event");
        assert_eq!(args["action"]["keycode"], "KEYCODE_SPACE");
    }

    #[test]
    fn a_mapping_needs_exactly_one_action_and_an_inside_point() {
        assert!(build(&tokens("mapping save app.pkg --key W")).is_err());
        assert!(build(&tokens(
            "mapping save app.pkg --key W --tap 50,80 --hold 10,10"
        ))
        .is_err());
        assert!(build(&tokens("mapping save app.pkg --key W --tap 150,80")).is_err());
        assert!(build(&tokens("mapping save app.pkg --key W --tap 50")).is_err());
        assert!(build(&tokens("mapping save app.pkg --key Hyper --tap 50,80")).is_err());
    }

    #[test]
    fn an_unknown_flag_is_refused_instead_of_ignored() {
        assert!(build(&tokens("status --quick")).is_err());
        assert!(build(&tokens("apps install game.apk --launchh")).is_err());
        assert!(build(&tokens("nav back --pkg app")).is_err());
        assert!(build(&tokens("perf set --cores 6 --wat 2")).is_err());
    }

    #[test]
    fn a_relative_path_is_resolved_in_the_callers_directory() {
        // The app would resolve `./game.apk` against its own working directory,
        // which is not the shell's.
        let (command, args) = built("apps install ./game.apk");
        assert_eq!(command, "apps.install");
        let path = args["path"].as_str().unwrap();
        assert!(std::path::Path::new(path).is_absolute(), "{path}");
        assert!(path.ends_with("game.apk"), "{path}");

        let (_, args) = built("capture screenshot --out shots");
        assert!(std::path::Path::new(args["out"].as_str().unwrap()).is_absolute());
    }

    #[test]
    fn the_automatic_plan_does_not_accept_values_it_would_ignore() {
        assert!(build(&tokens("perf set --mode auto --cores 6")).is_err());
        assert!(build(&tokens("perf set --mode auto --gpu host")).is_err());
        assert!(build(&tokens("perf set --mode auto")).is_ok());
    }

    #[test]
    fn a_known_switch_is_parsed_not_mistaken_for_a_value() {
        let (command, args) = built("apps install a/Game.apk --launch");
        assert_eq!(command, "apps.install");
        assert!(
            args["path"].as_str().unwrap().ends_with("a/Game.apk")
                || args["path"].as_str().unwrap().ends_with("a\\Game.apk")
        );
        assert_eq!(args["launch"], true);
        assert_eq!(args["allow_old_target"], false);
    }

    #[test]
    fn navigation_targets_the_phone_unless_a_window_is_named() {
        let (command, args) = built("nav recents");
        assert_eq!(command, "nav.recents");
        assert!(args.get("package_id").is_none());

        let (_, args) = built("nav back --package com.example.game");
        assert_eq!(args["package_id"], "com.example.game");

        let (_, args) = built("nav home --display 4");
        assert_eq!(args["display_id"], 4);
    }

    #[test]
    fn a_package_id_is_required_where_it_changes_the_answer() {
        assert!(build(&tokens("apps info")).is_err());
        assert!(build(&tokens("apps open --launch")).is_err());
        assert!(build(&tokens("mapping enable")).is_err());
    }

    #[test]
    fn performance_values_are_numbers_not_strings() {
        let (command, args) = built("perf set --cores 8 --ram 5120 --gpu host");
        assert_eq!(command, "perf.set");
        assert_eq!(args["cores"], 8);
        assert_eq!(args["ram_mb"], 5120);
        assert_eq!(args["gpu"], "host");

        assert!(build(&tokens("perf set --cores eight")).is_err());
        assert!(build(&tokens("perf set --mode turbo")).is_err());
        assert!(build(&tokens("perf set")).is_err());
    }

    #[test]
    fn a_raw_tool_call_needs_a_json_object() {
        match build(&tokens("call get_runtime_status")).unwrap() {
            Built::Mcp { name, arguments } => {
                assert_eq!(name, "get_runtime_status");
                assert_eq!(arguments, json!({}));
            }
            _ => panic!("not an MCP call"),
        }
        match build(&tokens(r#"call tap {"x":10,"y":20}"#)).unwrap() {
            Built::Mcp { arguments, .. } => assert_eq!(arguments["x"], 10),
            _ => panic!("not an MCP call"),
        }
        assert!(build(&tokens(r#"call tap [1,2]"#)).is_err());
        assert!(build(&tokens("call tap {not json")).is_err());
        assert!(build(&tokens("call")).is_err());
    }

    #[test]
    fn discovery_is_local_and_works_without_a_running_app() {
        let table = command_table();
        let names: Vec<&str> = table["commands"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["name"].as_str().unwrap())
            .collect();
        for required in [
            "devices",
            "runtime.start",
            "apps.install",
            "mapping.save",
            "perf.set",
            "capture.screenshot",
            "nav.back",
        ] {
            assert!(names.contains(&required), "{required} is missing");
        }
        // A GUI launch with an APK path must stay a GUI launch.
        assert!(maybe_run(&["C:\\games\\thing.apk".to_string()]).is_none());
        assert!(maybe_run(&[]).is_none());
        assert!(maybe_run(&["--mcp-stdio".to_string()]).is_none());
    }
}
