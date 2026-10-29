# Driving DroidWin from an AI agent

DroidWin runs a local [Model Context Protocol](https://modelcontextprotocol.io) endpoint. An MCP
client can install and open apps, inspect a screen, tap, type, swipe, press keys, wait for state and
verify the result — through the same Rust runtime the desktop window uses.

There is no second emulator, no second adb server and no separate window manager. Starting the
endpoint opens one loopback listener; closing it opens nothing.

## Enable it

1. Open **Agent** in DroidWin's sidebar.
2. **Start endpoint**. The URL, the stdio command and the token file appear; each has a copy button.
3. Grant the access level you want:
   - **Full control** — open apps, tap, type, install.
   - **Read-only** — inspect status and screens only.
4. **Pause automation** stops waits that are already running and refuses further input. The app
   windows keep working for you.

The endpoint comes back automatically the next time DroidWin starts, until you stop it.

## The same operations from a shell

The endpoint also serves a command line, for the operations that are tedious as tool schemas
(runtime start/stop/restart, performance plans, mapping edits, captures, navigation). One JSON
answer per command, same token, same pause/read-only gate:

```bash
droidwin status
droidwin perf preset performance
droidwin mapping save com.example.game --key W --tap 50,80
droidwin nav back
```

See [CLI.md](CLI.md). The MCP tool list is unchanged: the CLI adds no tools to a client's context.

## Transports

Both transports reach the same dispatcher, so a tool behaves identically either way.

### stdio (a client that launches a process)

```
"C:\path\to\DroidWin.exe" --mcp-stdio
```

`--mcp-stdio` starts no window and no webview. It forwards each newline-delimited JSON-RPC message
from stdin to the running app and writes the reply to stdout. Diagnostics go to stderr only.
DroidWin must already be running with the endpoint enabled.

Codex CLI and Claude Code both accept a command:

```bash
codex mcp add droidwin -- "C:\path\to\DroidWin.exe" --mcp-stdio
claude mcp add droidwin -- "C:\path\to\DroidWin.exe" --mcp-stdio
```

A JSON-configured client (Claude Desktop, Cursor, and others):

```json
{
  "mcpServers": {
    "droidwin": {
      "command": "C:\\path\\to\\DroidWin.exe",
      "args": ["--mcp-stdio"]
    }
  }
}
```

### Streamable HTTP (a client that connects to a server)

- URL: `http://127.0.0.1:<port>/mcp` (shown on the Agent screen)
- Header: `Authorization: Bearer <token>`

The token is written to the endpoint file together with the URL:

```
%APPDATA%\app.droidwin.desktop\agent\mcp.json      (installed app)
<DROIDWIN_HOME>\agent\mcp.json                     (development runs)
```

that file is removed when the endpoint stops. The port is chosen per session, so read it from the
Agent screen or the endpoint file instead of hard-coding it.

## Tool surface

Every input is a typed JSON object with a documented schema; `tools/list` is the authority.

**Read-only**

| Tool | What it answers |
| --- | --- |
| `get_runtime_status` | Runtime state, Android's report, and every open window with its display id |
| `list_devices` | Devices adb can see, marking the one DroidWin manages |
| `list_installed_apps` | Third-party packages with labels and window state |
| `get_app_info` | One package plus its window and display |
| `get_active_window` | Open windows and their display ids |
| `get_screen_state` | Screenshot + foreground package + elements in one observation |
| `get_screenshot` | PNG of one display (returned as an MCP image block) |
| `get_ui_hierarchy` | Accessibility elements with bounds |
| `find_ui_element` | First element matching a selector, with its centre point |
| `get_foreground_package` | What each display is showing |
| `wait_for_element` | Waits for a selector; returns `found: false` at the timeout |
| `wait_for_app_state` | Waits for running / stopped / foreground |
| `verify_action` | Checks the current state against your expectation |
| `get_recent_runtime_errors` | Failed actions and what Android said |
| `get_runtime_stats` | Measured host and guest CPU/memory and the reported renderer |
| `list_skills` / `read_skill` | Installed instructions for automating an app |

**Changes the Android screen**

`open_app`, `close_app_window`, `install_package`, `tap`, `long_press`, `swipe`, `scroll`,
`type_text`, `press_key`, `send_key_combination`.

### Display scoping is not optional

An app window is a **virtual Android display**, not a view on the main display. Input and
screenshots are display-scoped, so a tap without a target would land on whichever screen had focus.

- `open_app` returns the display id it resolved, and the window registry keeps the mapping.
- Every action accepts `package_id` (the display is looked up) or `display_id`.
- If neither can be resolved, the tool fails with a message telling you to open the app first. It
  does not guess display 0.
- Re-opening a closed app creates a new display with a new id; the old id is gone.

Coordinates are display-local: `(0, 0)` is the top-left of that app's display.

## Suggested workflow

1. `get_runtime_status` — is Android up, and which windows are open?
2. `open_app` — open the package if needed; keep the returned `display_id`.
3. `get_screen_state` — screenshot plus elements. The screenshot arrives as an image block.
4. Act with `tap` (by selector or by coordinates), `type_text`, `press_key`, `scroll`.
5. `wait_for_element` or `verify_action` — check it landed instead of assuming it did.
6. `get_screenshot` when you need to see the result of a visual change.

Real measurements from this machine (Android 36, 540×1170 virtual display, RTX 5060):

| Call | Time |
| --- | --- |
| `get_screenshot` | ~0.3 s |
| `get_screen_state` (screenshot + hierarchy + focus) | ~2.5 s |
| `get_ui_hierarchy` | ~2.2 s |
| `type_text` | ~0.16 s |
| `press_key` | ~0.09 s |
| Warm `open_app` | ~3.8 s |

Prefer `get_screen_state` over separate calls when you are about to make a decision; prefer a
selector over coordinates when the app exposes an accessibility tree.

## Honest limits

- **Text input is printable ASCII.** Android's `input text` maps characters to key events; it cannot
  type other scripts. `type_text` refuses the whole string rather than typing part of it.
- **`send_key_combination` needs Android 11+** (`input keycombination`). Older releases reject it
  with an explanation.
- **The accessibility tree depends on the app.** F-Droid exposed 27 actionable elements on its
  virtual display in the end-to-end test. Games and canvas-rendered apps often expose nothing; use
  the screenshot and coordinates there.
- **One Android user.** Apps share the emulator's user and are not isolated from each other.
- **DroidWin cannot tell which taps are consequential.** There is no purchase/message detection.
  Use read-only mode or stop the endpoint when you are not supervising.
- **No clipboard or file-drop tool.** Those paths are not verified in this build, so no tool claims
  them.

## Security model

- The listener binds `127.0.0.1` only and picks an ephemeral port.
- Every request needs the session token. A missing or wrong token is `401`.
- A `Host` header that is not loopback is refused (`403`), and a browser `Origin` that is not
  loopback is refused (`403`). The token, not the origin, is the authentication.
- Request bodies are capped at 8 MiB; JSON-RPC parse errors return `-32700`.
- Skills are validated and copied under the app data directory. Nothing in a skill is executed, and
  paths that could escape the skills directory are rejected.
- The action log records tool, result, duration and display — never screenshots or typed text.

Do not expose the port through a tunnel or reverse proxy. It is a local control channel by design.

## Troubleshooting

| Symptom | Cause |
| --- | --- |
| `DroidWin is not running, or its agent endpoint is stopped.` | Start the app and the endpoint; the stdio bridge has nothing to forward to. |
| `401 Unauthorized` | The token file changed (endpoint restarted) — re-read it. |
| `DroidWin does not know which display … is on` | Call `open_app` first, or pass `display_id`. |
| `Android did not return an image for display N` | The window was closed; the display no longer exists. |
| `permission_denied` | The Agent screen is in read-only mode. |
| `automation_paused` | Resume automation on the Agent screen. |
