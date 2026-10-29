# The `droidwin` command line

DroidWin ships one executable. Started with a subcommand, it is a command-line client for the
running DroidWin window; started with nothing (or an `.apk` path), it is the desktop app.

```bash
droidwin status
droidwin runtime restart
droidwin apps install ./game.apk --launch
droidwin perf preset performance
droidwin mapping save com.example.game --key W --tap 50,80
droidwin capture screenshot
```

Every answer is one JSON object on stdout:

```json
{"ok":true,"command":"nav.home","data":{"display_id":0,"key":"HOME","keycode":3},"summary":"Pressed HOME on display 0"}
```

`--pretty` indents it. `droidwin tools` prints the whole command table, locally, without contacting
DroidWin.

## Why it is a client, not a second runtime

The command is sent to the same loopback endpoint the MCP server uses, with the same per-session
token, and is dispatched through the same `ControlService` the MCP tools and the desktop UI use.
There is no second emulator, no second adb server and no second window registry. It also means the
desktop's permission gate applies unchanged: **Pause automation** and **Read-only** refuse a
command just as they refuse a tool call.

The MCP tool surface stays at 26 tools. The operations that are tedious as tool schemas — applying a
performance plan, writing one mapping, starting a recording — are commands instead, so a client does
not load them into context for every conversation. Measured on this machine: the command table is
**4,091 bytes**, against **27,915 bytes** for `tools/list` (26 tools with their schemas).

## Requirements

DroidWin must be running with the agent endpoint started (**Agent → Start endpoint**). The command
reads the URL and token the app writes to:

```
%APPDATA%\app.droidwin.desktop\agent\mcp.json     installed app
<DROIDWIN_HOME>\agent\mcp.json                    development run
```

If the app is not running, or the endpoint was stopped, the answer is:

```json
{"ok":false,"error":{"code":"endpoint_not_running","summary":"DroidWin is not running with the agent endpoint enabled.","detail":"No endpoint file at … . Open DroidWin and press Start endpoint on the Agent screen."}}
```

with exit status 2. It never starts an emulator of its own.

## Commands

### Device and runtime

| Command | What it does |
| --- | --- |
| `droidwin status` | Runtime state, Android's report, every open window and display id |
| `droidwin devices` | Devices adb can see, marking the managed emulator |
| `droidwin runtime start` | Start the runtime, or reuse it when it is healthy |
| `droidwin runtime stop` | Stop the emulator DroidWin started and its windows |
| `droidwin runtime restart` | Stop, then start; how a saved performance plan takes effect |

### Applications

| Command | What it does |
| --- | --- |
| `droidwin apps list` | Installed third-party packages |
| `droidwin apps info <package>` | One package, its window and its display |
| `droidwin apps install <file.apk> [--launch] [--allow-old-target]` | Validate and install |
| `droidwin apps open <package>` | Open the app's window; returns the display id |
| `droidwin apps close <package>` | Close the window; the process keeps running |
| `droidwin apps stop <package>` | Close the window and force-stop the app |

`--allow-old-target` is the same explicit override the app asks for: a package with a target SDK
below 23 is refused by Android 14+ unless the caller says it was told why.

### Performance

| Command | What it does |
| --- | --- |
| `droidwin perf` (or `perf get`) | The saved plan next to the values Android actually reports |
| `droidwin perf preset balanced\|performance\|compatibility` | Save a named plan |
| `droidwin perf set [--mode auto\|custom] [--cores 2\|4\|6] [--ram 2048..8192] [--gpu host\|software\|auto]` | Save a validated custom plan |

A saved plan applies on the next emulator start; the answer's `restart_required` says whether the
running runtime is still on the previous one. `effective` is what Android reported, never what was
requested.

**vCPUs stop at 6, because the emulator stops at 6.** Starting this AVD with `-cores 8` on a
24-thread host produced a guest with `nproc` 6, and the emulator's own log said
`Emulator does not support more than 6 cores. Number of cores set to 6`. DroidWin therefore offers
2, 4 or 6 and refuses a larger request with that reason, instead of saving a promise Android never
receives. Memory is separate and is applied: `-memory 8192` produced `MemTotal: 8135980 kB`
(~7.94 GB) inside Android.

### Key mapping

| Command | What it does |
| --- | --- |
| `droidwin mapping get <package>` | Mappings, whether mapping is armed, and the display geometry |
| `droidwin mapping save <package> --key <key> --tap x,y \| --hold x,y \| --keycode KEYCODE_X` | Add or replace one mapping |
| `droidwin mapping remove <package> --key <key>` | Delete one mapping |
| `droidwin mapping reset <package>` | Delete every mapping and switch mapping off |
| `droidwin mapping enable\|disable <package>` | Arm or disarm the package's mapping |

Keys are written the way the Controls editor labels them: `W`, `Space`, `F5`, `Ctrl+Shift+K`,
`Left arrow`, or a raw `0x2D`. Touch targets are percentages of that app's own display, so resizing
its window does not move them.

Mapping is per package and off until enabled; a mapping written here is the same row the Controls
dialog reads, and the same one the Windows keyboard hook dispatches.

### Capture and navigation

| Command | What it does |
| --- | --- |
| `droidwin capture screenshot [--out <folder>]` | Save a PNG of the phone's display; default `Pictures\DroidWin` |
| `droidwin capture record start\|stop\|status` | Android `screenrecord`, saved to `Videos\DroidWin` |
| `droidwin nav back\|home\|recents [--package <package>] [--display <id>]` | Android's own navigation keys |

Without `--package` or `--display`, navigation goes to the phone's display 0. With `--package`, it
goes to that app window's display — the same path the phone toolbar uses.

### Escape hatches

| Command | What it does |
| --- | --- |
| `droidwin tools` | The command table, answered locally |
| `droidwin call <tool> ['{"json": "arguments"}']` | Any MCP tool by name |

`call` covers the tools the command table does not: screenshots as image data, UI hierarchy, element
searches, waits, verification, workflows, triggers and skills.

## Exit status

| Status | Meaning |
| --- | --- |
| 0 | the command succeeded |
| 1 | the command was refused or failed; `error.code` says why |
| 2 | DroidWin is not running with the endpoint enabled, or it stopped answering |
| 64 | the command line was not understood (usage errors go to stderr) |

A refused command is still a JSON answer, so a script can read `error.code` instead of parsing
prose: `not_installed`, `permission_denied`, `automation_paused`, `invalid_config`, `timeout`,
`runtime_unavailable`, …

## What it does not do

- It does not start DroidWin, the emulator or an adb server.
- It does not bypass the Agent screen's permission mode or the pause switch.
- It does not connect physical phones. `list_devices` reports what adb sees (including a USB or
  wireless phone), but installing to and mirroring a non-managed device is not implemented.

## Verification

```bash
cargo test --manifest-path src-tauri/Cargo.toml --test cli_live -- --ignored --nocapture
```

`cli_live` starts the real endpoint with the app's wiring, then runs the real binary as a child
process against the machine's emulator: discovery, status, devices, app listing, performance report,
mapping save/remove/enable/disable, navigation, a PNG screenshot checked against its reported size, a
recording checked for an MP4 header, opening and closing a real window, a raw MCP call, a failed
command's exit status, and the read-only refusal. It reuses a running emulator or boots one, and
stops only an emulator it started. It is part of `node scripts/live.mjs` as the `cli` area.
