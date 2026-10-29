# CLI and resource pass — 29 October 2026

Windows 11 (build 26200), Android 36 AVD `DroidWin_API36` (emulator 36, RTX 5060 Laptop, 24
logical processors, 16 GB). Everything below was run on this machine against this working tree.
Anything that did not run says so.

## 1. What changed

| Area | Change |
| --- | --- |
| Command line | `droidwin` is now also a CLI: one executable, one JSON answer per command, stable exit codes, no second runtime. New `src-tauri/src/cli.rs`, wired in `main.rs` before the window starts. |
| Command transport | The agent's loopback server gained `POST /cli` (`agent/server.rs`). Same token, same Host/Origin checks, and the same pause/read-only gate as `tools/call`; different envelope. MCP's 26-tool surface is unchanged. |
| Runtime operations | `ControlService::dispatch_cli` (`agent/control.rs`) reuses the existing runtime, store and input controller for: runtime stop/restart, app stop, performance get/set/preset, mapping get/save/remove/reset/enable/disable, screenshot, recording start/stop/status, and Back/Home/recents. |
| Capture ownership | The desktop's one `Capture` session is now shared with the agent layer (`Arc<Capture>`), so a recording started from the command line is the recording the Phone screen shows and stops — not a second `screenrecord`. |
| vCPU limit | The emulator caps vCPUs at 6 on this build, measured below. `ALLOWED_CORES` is now 2/4/6, `MAX_CORES` is documented from the measurement, the Performance preset stops at 6, `validate_cores(8)` refuses with the emulator's own reason, and Settings no longer offers 8. |
| Status honesty | `RuntimeStatus.limitations` said "Audio is not forwarded in this build", which stopped being true when audio was enabled. It now states what is true (one emulator stream, video-only recordings) and names the measured host-loopback exposure. |
| Process exit | The release binary now attaches to the parent console for CLI output while remaining a windowed app. |
| Live suite | `tests/cli_live.rs` (opt-in, real emulator) and a `cli` area in `scripts/live.mjs`. |

## 2. Measured findings

### 2.1 Why Android reported fewer vCPUs than requested

Started the AVD directly with `-cores 8 -memory 8192 -gpu host` on the 24-thread host:

```text
$ emulator -avd DroidWin_API36 -port 5554 -cores 8 -memory 8192 -gpu host -no-window -no-snapshot
WARNING | Emulator does not support more than 6 cores. Number of cores set to 6

$ adb -s emulator-5554 shell 'nproc; grep MemTotal /proc/meminfo'
6
MemTotal:        8135980 kB
```

So the earlier "8 requested, 6 reported" was not a DroidWin defect in passing the flag — the
emulator silently reduces it. Memory is separate and was honoured: 8 GB requested, 8,135,980 kB
reported (kernel overhead), against 3,937 MB requested in the automatic plan.

DroidWin now refuses to save 8 and says why, instead of storing a plan whose vCPU count Android will
never receive. A saved 8 from an older build is reduced to the recommended plan on load with the
note attached.

### 2.2 What the command line costs a client

Measured against the same running endpoint:

| Discovery surface | Bytes |
| --- | --- |
| `droidwin tools` (27 commands, usage, read/control) | **4,091** |
| MCP `tools/list` (26 tools with JSON schemas) | **27,915** |

The CLI is 6.8× smaller to discover and adds no tools to the MCP surface. This is a context-size
measurement, not a latency claim.

### 2.3 Host ADB reachable from the guest (unchanged, now measured in-app)

From the running guest, to the Windows ADB server through the emulator's host gateway:

```text
$ adb shell 'printf "000chost:version" | toybox nc -w 2 10.0.2.2 5037 | od -An -c'
   O   K   A   Y   0   0   0   4   0   0   2   9
$ adb shell 'printf "000chost:devices" | toybox nc -w 2 10.0.2.2 5037 | od -An -c'
   O   K   A   Y   0   0   1   5   e   m   u   l   a   t   o   r   -   5   5   5   4 ...
```

A guest with Android's Internet permission can speak the ADB smart socket and enumerate/control
Android transports. Section 5 says what can and cannot be done about it.

## 3. Tests executed

| Command | Result |
| --- | --- |
| `cargo test --lib` | **232 passed**, 0 failed |
| `cargo test` (all non-ignored targets) | **247 passed** overall; 0 failed |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `pnpm exec tsc --noEmit` | clean |
| `pnpm test` | **86 passed** (8 files) |
| `cargo test --test cli_live -- --ignored --nocapture` | **passed** twice: 13.19 s warm, 51.89 s on the final run including runtime readiness |
| `cargo build --release` | built; CLI smoke-tested (see §3.2) |

New automated coverage: 13 CLI parsing/table tests, the `/cli` route test in `mcp_transport`
(auth, error shape, unknown command, read-only refusal, pause refusal, read commands still allowed),
`parse_key_spec` round-trips, and the adjusted vCPU tests.

### 3.1 Live command-line run (real emulator, real binary as a child process)

`cli_live` starts the real endpoint with the app's wiring, then runs `target/…/droidwin.exe`:

| Command | Observable result |
| --- | --- |
| `tools` | 27 commands, no endpoint needed |
| `status` | `state: ready`, `android_api: 36`, `emulator-5554`, window list |
| `devices` | the managed serial is listed |
| `apps list` | 9 installed packages |
| `perf get` | plan `6 vCPU / 3937 MB / host`; Android reports `6 vCPU / 7945 MB`, `restart_required: true` because the running emulator predates the plan |
| `mapping save … --key Ctrl+Shift+K --tap 50,80` | one mapping, label and percentages round-trip |
| `mapping remove …` | back to the previous count |
| `mapping enable` / `disable` | the flag flips; the Windows hook installs on enable |
| `nav home` | Android keycode 3 accepted on display 0 |
| `capture screenshot --out …` | 1,395,877-byte PNG, 1080×2400; header and reported byte count match the file |
| `capture record start/status/stop` | `active: true` at 3,025 ms; MP4 with `ftyp`, 63,054 bytes, 3,773 ms |
| `apps open org.fdroid.fdroid` | real scrcpy window, Android display 3, `foreground: org.fdroid.fdroid` |
| `status` again | the window is reported with its geometry |
| `apps close` | window closed; the other state untouched |
| `call get_runtime_status` | raw MCP tool by name, `ok: true` |
| `apps info not.installed.anywhere` | exit **1**, `error.code: not_installed` |
| `nav home` in read-only mode | exit **1**, `error.code: permission_denied` |
| `status` in read-only mode | exit **0** — reads still work |

The mapping rows the test wrote were removed, and the package's enable flag was restored to off.

### 3.2 Release binary and the window

| Check | Result |
| --- | --- |
| `droidwin.exe version` | `{"name":"droidwin","ok":true,"version":"0.1.0"}` |
| `droidwin.exe tools` | JSON on a redirected stdout |
| `droidwin.exe status` with a stale endpoint file | exit **2**, `endpoint_unreachable`, names the refused port and the fix |
| `droidwin.exe nav sideways` | exit **64**, usage error on stderr |
| Release GUI with a fresh `DROIDWIN_HOME` | window titled `DroidWin`, responding; `CloseMainWindow` exited cleanly and left no process |

## 4. Brief coverage

Implemented and verified in this pass:

- Command line (§6) for device/runtime, applications, mapping, performance, capture and navigation,
  with discovery, structured output, error codes and input validation.
- Agent control (§7) reaches the new operations without enlarging the MCP tool list, and cannot
  bypass pause/read-only.
- Performance reporting (§2): requested vs Android-reported values, plus the measured 6-vCPU cap and
  an honest refusal instead of an impossible option.
- Security (§11) was probed with real smart-socket requests; see §5.
- Clean runtime relationship: no second emulator, no second adb server, one capture session.

Already implemented before this pass, re-used rather than rebuilt (not re-verified here): Back/Home/
recents in the Phone toolbar and per-app windows (`Alt+B/H/S` are the window host's shortcuts),
per-app keyboard mapping with the Windows hook, performance presets and custom plans, display
size/density overrides, emulator audio, and the MCP tools.

**Not implemented in this pass, and not claimed:**

- A navigation button bar drawn at the bottom of each scrcpy app window (§1). Navigation exists on
  the Phone screen's toolbar and through scrcpy's own window shortcuts; DroidWin does not draw
  chrome into a window it does not own.
- A DroidWin-drawn key-mapping overlay over an app window, and the `Ctrl+Shift+R` show/hide shortcut
  (§4). scrcpy owns those windows (separate processes); an overlay needs a new always-on-top
  transparent window layer, and nothing was built or measured for it here.
- Automatic control detection / generated default mappings (§3). Android games do not expose control
  layouts; only the manual mapping path exists.
- Physical-phone install, mirroring and control (§8). `list_devices` reports what adb sees, but no
  managed operation targets a non-emulator device.
- `Ctrl+Shift+R`, free-resize behaviour and mapping alignment under resize (§5) were not re-tested;
  scrcpy windows are freely resizable and mapping points are stored as display percentages, which is
  a design property, not a measurement from this pass.
- `runtime stop`, `runtime restart`, `perf preset`/`perf set` and `apps install`/`apps stop` were not
  exercised live. They share the same runtime methods the UI already uses, and `perf set`/`preset`
  are covered by unit tests, but no end-to-end run in this pass.
- Audio was not measured in this pass. It was previously confirmed by a person listening; no capture
  of the Windows mixer exists, and `screenrecord` on this image is video-only.

## 5. Security: what is and is not achievable here

Measured: the guest can reach `10.0.2.2:5037` and speak the ADB smart socket (§2.3). That is the
emulator's documented host gateway (`10.0.2.2` is the host loopback), implemented inside qemu's
user-mode networking. Inspected mitigations and why they were not adopted:

| Idea | Result |
| --- | --- |
| Windows Firewall rule for the emulator process | The connection is made by the emulator process itself; slirp rewrites it to loopback, which Windows Firewall does not filter by default. Not a boundary. |
| A non-default adb port | Reduces the default-port hit, but a guest can scan loopback and the user's own adb server on 5037 stays exposed. It would also break every tool that expects the default port without a clear gain. |
| Disabling guest networking | Would break apps that need the internet, and is a per-run choice, not an isolation guarantee. |
| `restrict=on` slirp option | Not exposed by the Android Emulator's command line; passing raw qemu netdevs alongside its own is unsupported and unverified. |

Conclusion at the time: **the emulator cannot isolate untrusted APKs from host services listening on
loopback.** That conclusion was wrong, and a later pass in the same day replaced it: this image is
`ro.debuggable=1`, so `adb root` works, and two egress rules (`iptables -d 10.0.2.2` and
`ip6tables -d fec0::2`) refuse the guest the host gateway on both families while leaving DNS and the
internet alone. The IPv6 gateway matters: `fec0::2` reaches host loopback too, so a v4-only rule
would have been theatre. See [HOST_ISOLATION.md](HOST_ISOLATION.md) for the measurements and
`tests/isolation_live.rs` for the standing check.
DroidWin's own control channel is not exposed by this: the agent endpoint requires a per-session
bearer token from a file in the user's profile and refuses a browser Origin, and the CLI uses the
same token. A guest can, however, reach and use an adb server — disturbing DroidWin's session or
controlling other Android transports. This is now stated in the app's own runtime limits, and the
honest mitigation remains a separate Windows account/VM with no host ADB server, not a documentation
warning.

## 6. Files touched

| File | Change |
| --- | --- |
| `src-tauri/src/cli.rs` | new: parser, command table, endpoint discovery, HTTP client, console attach, caller-relative path resolution, tests |
| `src-tauri/src/main.rs` | subcommand dispatch before the window |
| `src-tauri/src/lib.rs` | `cli` module, shared capture session |
| `src-tauri/src/agent/control.rs` | CLI dispatch, performance/mapping/capture/stop/nav operations, control classifier |
| `src-tauri/src/agent/server.rs` | `POST /cli` with the same auth and permission gate |
| `src-tauri/src/agent/mod.rs` | agent wiring with input + capture |
| `src-tauri/src/input/mod.rs` | `parse_key_spec` and its tests |
| `src-tauri/src/runtime/hardware.rs` | 6-vCPU ceiling from measurement, refusal message, tests |
| `src-tauri/src/runtime/emulator_runtime.rs` | effective-limit note; corrected status limitations |
| `src-tauri/src/commands.rs` | `Arc<Capture>` in app state |
| `src-tauri/tests/cli_live.rs` | new live suite |
| `src-tauri/tests/support/mod.rs` | full-wiring harness and `/cli` helper |
| `src-tauri/tests/mcp_transport.rs` | `/cli` auth/gate/error tests |
| `scripts/live.mjs` | the `cli` area |
| `src/screens/Settings.tsx` | vCPU options 2/4/6 |
| `README.md`, `docs/CLI.md` | command reference and corrected limits |
