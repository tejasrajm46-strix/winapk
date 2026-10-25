# Stability and Performance Audit — P0 stabilization pass

Windows 11 (build 26200), 25 October 2026. Every number below was measured on this machine;
nothing here is an estimate from reading code. Where a measurement was contaminated or could not
be taken, it says so instead of guessing.

## 1. Reported symptoms, traced to causes

| Reported symptom | Root cause | Where |
| --- | --- | --- |
| Command windows / terminal tabs opening constantly | No child process set `CREATE_NO_WINDOW`. DroidWin is a GUI-subsystem program, so every console child (`adb`, `cmd /c avdmanager.bat`, `taskkill`, `scrcpy`, `aapt2`, `emulator -accel-check`) made Windows allocate a fresh console — a Windows Terminal surface on this machine. | 8 spawn sites across `runtime/adb.rs`, `runtime/emulator.rs`, `runtime/scrcpy.rs`, `runtime/sdk.rs`, `apk/mod.rs` |
| Same, while idle | The UI polled `runtime_status` every 8 s. Each call ran the full environment probe, including a **blocking** `emulator -accel-check` (a process launch), plus three adb invocations (`devices`, `getprop sys.boot_completed`, `getprop ro.build.version.sdk`). | `emulator_runtime.rs::probe/status`, `App.tsx` interval |
| Same, while booting | Boot readiness was polled once a second with **two to three** adb processes per attempt, for up to 420 s. | `emulator.rs::wait_boot` |
| Slow / stuttering actions | `std::process::Command::output()` (accel check, `aapt2 dump badging`) blocked the async worker; the emulator's stderr was piped and never read, so a chatty boot could fill the pipe and stall the VM. | `sdk.rs`, `apk/mod.rs`, `emulator.rs::spawn` |
| Concurrent actions booting twice | No single-flight guard: a start request racing an install could both pass the readiness check and both spawn an emulator on the same port. Two launches of one package could both find no window and both open one. | `emulator_runtime.rs::ensure_ready/launch` |
| Windows left behind after closing DroidWin | Nothing ran on exit: scrcpy children (`kill_on_drop(false)`) and the emulator survived the manager. | `lib.rs`, `emulator_runtime.rs::shutdown` |
| Runtime killed unexpectedly | `shutdown()` called `adb emu kill` unconditionally, even for an emulator DroidWin never started. | `emulator_runtime.rs::shutdown` |

## 2. Fixes

1. **Central process policy** — new `src-tauri/src/proc.rs`. `proc::no_console` / `no_console_async`
   apply `CREATE_NO_WINDOW` (0x08000000) and are used by **every** subprocess DroidWin starts.
   The flag suppresses the console, not the process: output, exit codes and scrcpy's own SDL window
   are unaffected, which `proc::tests` asserts by capturing a hidden child's stdout and exit code.
2. **Environment probe cached** (`ENV_TTL` 30 s) with `probe_now()` for explicit refreshes. The
   acceleration check therefore runs once per half minute at most, not once per poll, and the
   blocking call sites are off the hot path.
3. **One readiness call** — `Adb::readiness()` reads a single `getprop` dump and parses
   `sys.boot_completed` and `ro.build.version.sdk` from it (`parse_props`, unit-tested). Status,
   boot wait and package listing use it instead of two or three adb calls.
4. **Boot polling halved to one adb spawn per second** (`wait_boot`).
5. **Single-flight boot** (`boot` mutex) and **deduplicated launch** (`launch` mutex around the
   check-then-spawn), with a fast path so an already-open app returns its existing window.
6. **Emulator stderr to `logs/emulator.log`** instead of an unread pipe, so a full pipe can no
   longer stall the VM and boot failures stay diagnosable.
7. **Exit owns the runtime** — `RunEvent::Exit` runs `shutdown()` under an 8 s bound. Only an
   emulator this process spawned is stopped; otherwise it logs
   `the runtime was not started by DroidWin; leaving it running`.
8. **Frontend** — `refreshApps` is single-flight, so event, poll and post-action refreshes no
   longer stack adb work.

## 3. Measurements

### 3.1 Console windows (controlled, GUI parent)

`.droidwin/perf/gui_parent_probe.py`, run under `pythonw.exe` (no console of its own, matching
DroidWin's subsystem), 5 spawns per batch of `cmd.exe /c ping -n 2 127.0.0.1`:

| Batch | Distinct visible console windows observed |
| --- | --- |
| No creation flags (previous behaviour) | **1** `CASCADIA_HOSTING_WINDOW_CLASS` (Windows Terminal reuses one window; each console arrives as another tab) |
| `CREATE_NO_WINDOW` (current policy) | **0** |

A second control (`console-policy.ps1`, spawns from a console parent) shows the same flag removes
the console while the process still runs; the GUI-parent probe is the case that matches DroidWin.

### 3.2 Subprocess launches while the app is open

Harness `measure.ps1` (samples `adb.exe` every 10 ms and console hosts every ~40 ms for 40 s,
starting the real packaged build with its own `DROIDWIN_HOME`):

| Build | adb launches in 40 s (idle UI) | Console-host processes seen |
| --- | --- | --- |
| Before (`201a200`, tauri-built) | 7 | 49 |
| After | 4 | 13, of which 12 were pre-existing ambient hosts (same PIDs as the control); one new |

The "after" sample opened with a real user click on **Start runtime**, so its 40 s includes a cold
boot; that is why the launch count is not lower still. Boot polling alone used to cost 2–3 adb
processes per second — 40–60 launches and as many console allocations in a 20 s boot; it is now one.

### 3.3 Boot and install timings (cold, this machine)

`cargo run --release --example proof -- .droidwin/testdata`, emulator stopped beforehand:

| Step | Result |
| --- | --- |
| Runtime ready (cold boot, API 36) | **19.1 s** |
| Install F-Droid | **5.6 s** |
| Install NewPipe | **2.4 s** |
| Runtime ready (warm, emulator already attached) | **107 ms** |

## 4. End-to-end runtime proof after the fixes

Same harness, two packages, real emulator:

```
[pass] environment — SDK, emulator, image and scrcpy found
[pass] runtime ready — 19090 ms, API Some(36)
[pass] install — fdroid.apk → org.fdroid.fdroid
[pass] install — newpipe.apk → org.schabi.newpipe
[pass] independent windows — 2 window(s), 2 distinct title(s)
[pass] move/resize — 70,47 291x637 → 120,140 520x760
[pass] close isolation — 1 window(s) remain
[pass] persists across manager restart — 5 listed
Gate A (install + launch): PASS
Gate B (independent windows): PASS
```

Shutdown ownership was checked in both directions: a proof run that started the emulator ended with
`terminated emulator process tree at pid …`, and one that attached to an external emulator logged
`leaving it running`. Afterwards: no `qemu-system-x86_64`, `emulator`, `scrcpy` or `droidwin`
process, and `adb devices` empty.

The packaged UI was also launched from the built binary and captured
(`screenshots/ui-after-fixes.png`): the Kage Zen shell loads and reports `RUNTIME READY · API 36`.
A `cargo build --release` binary without the `custom-protocol` feature shows the dev-server error
page — run `pnpm tauri build` for a real build.

## 5. Automated checks

| Command | Result |
| --- | --- |
| `cargo fmt --check` | clean |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo test` | 50 passed |
| `tsc --noEmit` | clean |
| `vitest run` | 24 passed |
| `pnpm tauri build` | installer, 11.64 MiB |

## 6. Still open

- **Dev mode differs**: `pnpm tauri dev` runs a console-subsystem binary, so its own console is
  expected; child consoles are already suppressed.
- **Display geometry per app** (DW-046): landscape packages such as Block'Buster still draw wider
  than their portrait virtual display.
- **Force kill**: a killed process cannot run the exit handler; the emulator may survive until the
  next launch reconciles it. A graceful close is clean.
- Clipboard, file drop, audio, a DPI matrix, and a signed installer remain unverified as recorded in
  `IMPLEMENTATION_AUDIT.md`.
