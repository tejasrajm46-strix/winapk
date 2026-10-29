# Release verification — final pass

Windows 11 (build 26200), Android 36 AVD `DroidWin_API36`, RTX 5060 Laptop, 26 October 2026.

Everything below was run on this machine against this working tree. Anything that did not run, or
ran and failed, says so. Nothing here is inferred from reading code.

## 1. What this pass changed

| Change | Why |
| --- | --- |
| The supplied brand mark is now the executable, installer, window and in-app icon (`src-tauri/icons/{icon.ico,32x32.png,128x128.png,256x256.png}`, `src/assets/branding/droidwin-mark.png`) | The packaged icon was still the earlier geometric placeholder |
| `src-tauri/build.rs` watches `icons/` | tauri-build embeds the icon into the exe but does not watch the icon files, so a replaced icon left the **old** resource in the cached object file: the installer had the new mark while `droidwin.exe` still had the old one. Caught by extracting the icon from both binaries. |
| A launch whose window never gets a display is now closed and reported | Hill Climb Racing's activity is non-resizable and landscape-only, so Android keeps it on display 0. scrcpy's virtual display then has `mCurrentFocus=null` and the window is black. DroidWin used to leave that window open and only return "Android has not reported its display yet". |
| A restarted-after-EGL-crash app is treated as an EGL failure | Evidence in §4.4: the app died in `eglChooseConfig` under pid N, Android restarted it as pid N+1, so the "process is gone" check passed and the one-time software-renderer recovery never ran. **This branch did not reproduce after it was added — see §5.** |
| `mcp_proof` aims at a real element instead of `(1200, 870)` | That coordinate was written for a 2400×1080 game surface; a portrait app is 1080×2400, so the tap was refused as out of bounds and the proof stopped at step 4 |
| `mcp_proof` tears the runtime down on failure | The error path called `process::exit(1)` and skipped `shutdown()`, leaving an orphaned scrcpy window that then polluted the next run's "before" sample |
| `agent_live` expects the 1080×2400 surface | It still asserted the retired 540-wide display and would have failed on the first live run |

## 2. Automated checks

| Command | Result |
| --- | --- |
| `cargo test --lib` | **196 passed**, 0 failed |
| `cargo test --test mcp_transport` | **9 passed**, 0 failed |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean (two spots in `emulator_runtime.rs` were reformatted) |
| `pnpm exec tsc --noEmit` | clean |
| `pnpm test` | **58 passed** (7 files) |
| `pnpm tauri build` | NSIS installer built, 12.47 MiB |

Two live tests are `#[ignore]`d because they boot the emulator; both were run explicitly (§3, §4).

## 3. Live end-to-end: MCP client drives the runtime

`cargo test --test agent_live -- --ignored --nocapture` — **passed in 29.9 s** (re-run after the
launch-path change; the first run took 53.98 s with a cold boot).

It installs F-Droid, opens it on its own display, and then: screenshot (image block, 1080×2400) →
UI hierarchy (non-empty) → tap by selector → `type_text` ("droidwin", 8 characters) → `press_key` →
`wait_for_app_state` → `verify_action` → runtime stats → install and open a second package →
two windows with different displays → close one, the other survives → re-open (fresh display id) →
cancel a 30 s wait (169 ms / 191 ms on the two runs) → the refused Japanese input appears in the
error trail. The emulator pid is unchanged throughout: one emulator, one adb server.

Runtime stats reported by the guest: 6 cores, 1548 MB free, renderer
`Android Emulator OpenGL ES Translator (NVIDIA GeForce RTX 5060 Laptop GPU)`.

## 4. Live end-to-end: triggers and workflows

`cargo run --example mcp_proof -- .droidwin/testdata/fdroid.apk` — **passed**.

| Fact | Measured |
| --- | --- |
| Tool surface | 32 tools |
| Workflow (open → wait → look → tap → verify) | 5/5 dispatched, `verified=true`, 6.52 s |
| `app_opened` trigger | fired on its own when the window opened, `fire_count=1` |
| Explicit `fire_trigger` | 1/1 dispatched, verified |
| Cancellation mid-run | stopped in 2.32 s, `dispatched_count=0`, step reported `cancelled` |
| Client dropped mid-call | endpoint still answered the next request |
| After teardown | `windows:0, scrcpy:0`, adb list unchanged |

### 4.1 Black window on an app Android keeps off its own display

`cargo test --test hcr_recovery -- --ignored --nocapture` — **passed in 25.2 s**.

Before the change, opening Hill Climb Racing produced a window with no picture, `display_id: null`
and a launcher foreground. Now:

```
com.fingersoft.hillclimb opened, but Android drew nothing on its window.
  The app has a running process but no display of its own, so taps and screenshots could not be
  aimed at it; the empty window was closed instead of being left open. …
```

The window is closed, the runtime keeps running, and the test asserts both. Evidence for the cause:
`dumpsys window displays` shows the scrcpy display with `mCurrentFocus=null` while the launcher owns
display 0, and `dumpsys activity` shows the game's task as `nonResizable …
SCREEN_ORIENTATION_SENSOR_LANDSCAPE`.

### 4.2 The EGL crash this was originally about

`logcat -b crash` from the same package, before the fix:

```
Process: com.fingersoft.hillclimb, PID: 8431
java.lang.IllegalArgumentException: No config chosen
    at com.fingersoft.game.FSGLSurfaceView$BaseConfigChooser.chooseConfig(FSGLSurfaceView.java:884)
```

Android then restarted the package under pid 8963, which is why the process check passed. This is the
case §1's third fix addresses; on re-run the app did not crash (see §5).

## 5. Remaining defects and limits

- **The EGL-recovery-on-restart branch is unverified.** It was added from the crash evidence in §4.2,
  but the crash did not recur on the next two runs, so no test has driven that branch to success.
  It is guarded (display unresolved + a crash block naming the launching pid + an EGL signature) and
  it closes the window before restarting the runtime, so a wrong guess cannot kill a working window.
- **Hill Climb Racing does not run in its own window on this image.** DroidWin now says so instead of
  showing a black window. A mirror-on-default-display fallback is still not built (DW-045).
- **A failed launch takes ~25 s to give up** (15 s process wait + 10 s display wait). The display wait
  exists so a slow first composition is not mistaken for a black window; checking display 0's focus
  while waiting would shorten the common failure.
- **`sidecar` cleanup on a force kill**: a killed (not closed) DroidWin cannot run its exit handler;
  the emulator it started may survive until the next launch reconciles it.
- **Untested by design, unchanged from before**: audio, clipboard, file drop, code signing, clean-machine
  install, DPI matrix, `.apk` association behaviour under Explorer (built, not exercised here).
- **Workspace clutter**: the untracked `src.zip`/`src-tauri.zip` source archives, a stale
  `DroidWin-Final` bundle and the `.droidwin` probe sandboxes (~38 GB) were removed after this
  verification; the installer below is the current build output.

## 6. Release package

`src-tauri/target/release/bundle/nsis/DroidWin_0.1.0_x64-setup.exe` (12.47 MiB), built from this
working tree after every change above, with the new mark in the installer and in
`src-tauri/target/release/droidwin.exe`. The icon was verified by extracting it from both binaries
(`.droidwin/icon-exe2.png`, `.droidwin/icon-setup.png`), not by trusting the build output.

The installer is **not signed** and was **not installed on a clean machine**; those two remain the
gaps between "release package builds and runs" and "release ready to publish".

## 7. Second pass: the virtual phone (27 October 2026)

Same machine, same AVD. This pass added the integrated phone screen — the mission's largest missing
objective — and touched nothing the earlier pass verified except through the runtime methods below.

### 7.1 What changed

| Change | Why |
| --- | --- |
| `EmulatorRuntime::device_screen` / `device_tap` / `device_swipe` | The phone is Android's **own** display, so its capture and its input belong with the runtime that already owns adb, the serial and every display's geometry — not in a second pipeline. |
| `display::DEVICE_DISPLAY` (`0`) | Every other display DroidWin touches it created itself. Naming the one it did not means no caller passes a bare `0` whose meaning is only in the caller's head. |
| `png_size` moved from `agent/control.rs` to `runtime/display.rs` | It reads a capture's size, which is geometry. The control API now re-exports it, so its call sites are unchanged. |
| `phone_screen` / `phone_tap` / `phone_swipe` commands + `PhoneScreen` | Thin command layer over the runtime methods: base64 data URL, one sanity check on a touch point. |
| `src/screens/Phone.tsx` and a **Phone** navigation item | The screen itself: the mirror, an app launcher with search, the installation stages and the connection panel. |

### 7.2 The design decisions worth recording

- **Stills, not a stream.** A live video mirror would be a second scrcpy pipeline competing with the
  app windows for the same display server. The frame takes one capture per second, single-flight,
  only while the runtime is ready and the window is visible. A hidden window captures nothing.
- **The frame is the coordinate space.** Its aspect ratio is set from the capture, so the image fills
  it exactly, a click converts with the image's own scale, and there are no letterbox bars for a
  click to be measured against. A drag under 8 device pixels is a tap; a longer one is a swipe.
- **The phone does not own an emulator.** It renders the runtime DroidWin already runs. Launching a
  tile opens the app's own Windows window, which is what DroidWin is.

### 7.3 Checks actually run

| Command | Result |
| --- | --- |
| `cargo test --lib` | **198 passed**, 0 failed (was 196; `png_size` and the touch-point check each gained a case) |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `pnpm exec tsc --noEmit` | clean |
| `pnpm test` | **69 passed**, 8 files (was 58; 11 new cases for the phone screen and its coordinate conversion) |
| `cargo test --test phone_live -- --ignored --nocapture` | **passed in 27.5 s** |
| `pnpm tauri build` | NSIS installer rebuilt (see §7.5) |

Live phone check, measured lines from `.droidwin/phone-live.log`:

```
runtime ready in 24265 ms
capture: 1080x2400, 610380 bytes, 730 ms
flat colour: None
swipe dispatched in 345 ms
tap dispatched at 540, 1200
```

That is the contract, checked against a real device rather than a mock: the capture's header size
matches the decoded frame (so the screen is scaled by the same numbers it is clicked through), the
frame is not one flat colour (a black window would be), and a swipe sent through DroidWin's own input
path changed what Android drew — `.droidwin/phone-before.png` → `.droidwin/phone-after.png` have
different hashes, and the pictures show the phone before and after the lock screen was dismissed.

### 7.4 What this pass did not verify

- **The Tauri command layer was not exercised in the running window at the time of this pass.**
  That was fixed in the next pass: §8 records `phone_screen`, `phone_tap`, `phone_swipe`, `phone_key`
  and `phone_text` driven through the packaged executable, with Android's own responses as evidence.
- **No app catalogue and no download browser.** Mission §3's second half — discovering a package,
  obtaining its APK from a legitimate source and installing it without leaving DroidWin — is not
  built. The screen shows the install pipeline that exists (choose an APK, watch its stages), and the
  panel is titled "Installations" rather than advertising a download feature that is not there.
- **Gesture fidelity.** One swipe is forwarded as one Android swipe; there is no pinch, no
  long-press, and no keyboard input to the phone display.

### 7.5 Release package

`src-tauri/target/release/bundle/nsis/DroidWin_0.1.0_x64-setup.exe`, rebuilt from this working tree
after the changes above. Still unsigned and still not installed on a clean machine.

## 8. Third pass: the phone driven through the packaged executable (27 October 2026)

Same machine, same AVD. Nothing here was measured through the Vite dev shell: every number below came
from the **release executable** (`src-tauri/target/release/droidwin.exe`, the same binary the installer
ships), driving its real runtime.

### 8.1 How it was driven

```bash
DROIDWIN_HOME=<checkout>/.droidwin/home \
WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 \
  ./src-tauri/target/release/droidwin.exe

node scripts/ui-shot.mjs --click "Phone"          # real clicks in the shipped renderer
node scripts/ui-shot.mjs --eval "<js>"            # real gestures, keys and measurements
node scripts/ui-shot.mjs --shot .droidwin/exe-phone.png
powershell -File scripts/capture-window.ps1 -Match "DroidWin" -Out .droidwin/exe-window.png
```

Android's own answers came from `uiautomator dump`, `dumpsys window | grep mCurrentFocus` and
`input -d 0 …` through the SDK's adb. `__TAURI_INTERNALS__.invoke` is defined `writable:false,
configurable:false`, so calls cannot be wrapped from the page; the phone's own state was used as the
evidence instead.

### 8.2 Defects this pass fixed, with the measurement that found them

| Defect | Measured before | After |
| --- | --- | --- |
| Click mapping | Frame 262.7×492 (aspect 0.534) for a 1080×2400 capture: `object-fit: contain` drew the image 221.4 wide with a **20.6 px bar each side**, and the click was scaled by the frame, so the image's visual right edge sent device **x=995 instead of 1080 (7.9 % short)** and a click in the margin sent **x=42** | Frame 262.7×583.7, aspect **0.4500 = capture 0.4500**, bar **0**. Clicks are measured against the box the capture is painted in, and a click in any margin is ignored rather than tapped |
| Second identical alert | With the backend down the screen rendered two `role=alert` blocks for one failure | One alert; the library panel says "unavailable until the backend answers", as Overview already did |
| Unbounded capture polling | One `adb screencap` + 1080×2400 PNG **every second, forever**, whether or not anything changed | Idle **2.8–3.3 s** per capture, **1.0–1.2 s** for 4 s after input, and nothing while the frame is scrolled out of view or the window is hidden |
| No keyboard on the phone | Typing, Enter, Back, Home and Tab did nothing on the phone's display | Typing, Enter/Tab/arrows/Backspace/Escape(Back)/Home/PageUp/PageDown reach Android through the same `input` path an app window's mapped key uses |

### 8.3 Evidence, in Android's words

| Action in the app UI | Android's answer |
| --- | --- |
| `phone_screen` | `display_id 0`, 1080×2400, `data:image/png;base64,…`, 396 KB |
| `phone_swipe` (540,2150 → 540,900) | next capture differs → the phone moved |
| `phone_tap` (999999, 10) | `invalid_request` — "That point is not on the phone's screen." |
| Drag on the frame | the phone's screen changed (the app drawer opened) |
| Tap at device (540,228) | launcher search `EditText` → `focused="true"` |
| Typing `fdroid` | `EditText` → `text="fdroid"`; results "f droid", "fdroid termux", "fdroid termux download" |
| Enter | `mCurrentFocus` moved from `nexuslauncher/NexusLauncherActivity` to `googlequicksearchbox/OneSearchActivity` |
| Home | foreground returned to `nexuslauncher/NexusLauncherActivity` (from a Chrome CustomTab) |
| Escape (Back) | first press dismissed the drawer's search state, second closed the drawer |
| Typing six characters | one `input text` call, not six (the burst is batched) |

Runtime facts read on this machine: WHPX **installed and usable** (`emulator -accel-check`);
guest renderer **`Android Emulator OpenGL ES Translator (NVIDIA GeForce RTX 5060 Laptop GPU)`** —
hardware accelerated, not SwiftShader; guest **6 cores**; guest memory **3 858 580 kB total,
1 858 148 kB available**.

### 8.4 Checks run

| Command | Result |
| --- | --- |
| `cargo test --lib` | **199 passed**, 0 failed |
| `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check` | clean |
| `pnpm exec tsc --noEmit` | clean |
| `pnpm test` | **76 passed**, 8 files (18 for the phone screen) |
| `pnpm tauri build` | NSIS installer **12.51 MiB**; `phone_key`/`phone_text` present in the binary |

### 8.5 Still open

- **The mirror is stills, not video.** One capture costs about **0.73 s** on this emulator, and the loop
  is sequential, so the mirror is roughly a capture behind input. That cost is `screencap` of
  1080×2400, not the delay between captures.
- **No pinch, no long-press, no IME text.** Typing sends key events, so a field that needs the soft
  keyboard must be tapped first; non-ASCII text is not covered.
- **No app discovery or download manager** (mission §3's second half).
- **No performance dashboard** (the plan's §5): CPU, per-process RAM, launch-time and input-latency
  numbers are not surfaced anywhere yet. `hardware_report` already carries host facts, the plan, the
  verified renderer and the acceleration answer, and operation records already carry start/finish
  times, so it is a UI-and-aggregation job rather than new measurement machinery.
### 8.6 Runtime reuse, orientation, and one layout defect found by looking

**Reuse (mission §5 "verify runtime startup and reuse").** With the emulator already up, a *fresh*
app process was started and asked for a capture: **365 ms**, and the emulator's pid was unchanged
(35 760 before and after) — it attached to the running runtime instead of cold-booting it (a cold boot
here takes about 40 s).

**Orientation.** Android's own screen follows whatever has focus, so the phone frame must follow the
capture. Hill Climb Racing — landscape-only and non-resizable, the DW-045 case — was still resumed on
display 0 and the capture was **2400×1080**; the frame carried `aspect-ratio: 2400 / 1080` and measured
**263×118 (2.229 against the capture's 2.222)**, the difference being its 1 px border. After
`am force-stop` the capture returned to **1080×2400** and the frame to **263×584 (0.4500 against
0.4500)**. No distortion and no letterbox in either direction.

**Layout defect fixed.** A screenshot of the shipped window showed the package id running under the
"completed" pill. Measured: the id (`.mono`) spanned x 697→915 while the pill spanned x 757→846, and the
row's title was clipped to 44 px. Cause: `.row` is a 3-column grid (`44px minmax(0,1fr) auto`), and the
installations list renders no icon, so the title took the 44 px icon column, the id overflowed it, and
the pill started exactly at 697 + 44 + 16 = **757**. Fixed in CSS rather than in the screen: the base
rule is now two columns and `.row:has(.row__icon)` restores the icon column, so any icon-less row lays
out correctly. Long package ids also wrap (`overflow-wrap: anywhere`) instead of overflowing.

After the rebuild, measured in the packaged exe: the row's computed grid is `371.562px 89.1042px`
(two tracks), the title has **372 px instead of 44**, and the pill moved to **1084→1173**, clear of the
id that ends at 915. `:has()` is available in the renderer that ships (`CSS.supports` true; the window
reports Chrome/155). The rest of the rows in the app keep the icon column and were unchanged;
`pnpm test` re-run after the change: **76 passed**.

- Unchanged: installer unsigned, not clean-machine tested, no audio/clipboard/file-drop.

### 8.7 Core-count benchmark (real cold restarts, this host: 24 logical CPUs, 15.4 GB RAM)

Each row is a cold boot (`-no-snapshot`, the app's flags, `-gpu host`, `-memory 3858`), timed to
`sys.boot_completed=1`, then one phone frame. Read from `.droidwin/bench-cores.log`.

| Requested `-cores` | Guest `nproc` | Cold boot | First capture |
| --- | --- | --- | --- |
| 2 | 2 | 19.1 s | 923 ms |
| 4 | 4 | 17.8 s | 649 ms |
| 6 | 6 | 20.1 s | 665 ms |
| 8 | **6** | 17.9 s | 701 ms |

- Core counts up to 6 are applied exactly. **8 is not applied on this host**: the guest reports 6.
  The planner's `ALLOWED_CORES` offers 8 on machines with more than 16 logical CPUs; on this one that
  value is silently capped, so the Settings view should not present it as effective.
- Boot time does not move with core count beyond run-to-run noise (about ±2 s). The default stays
  at 6 for this host, because 4 is no faster and 6 keeps the same boot time.
- Single runs only; the spread between runs is about 2 s, so small differences are not meaningful.

### 8.8 Re-audit: measured phone path (packaged exe, warm runtime)

| Measurement | Result |
| --- | --- |
| `phone_screen` (6 calls, median) | **594 ms** (581–601) |
| `phone_key` HOME dispatch | **52 ms** |
| Frame requested right after the key | **606 ms** |
| Guest raw `screencap` (no PNG on guest) | 424–451 ms for 10.4 MB |
| Guest `screencap -p` | 574–581 ms for 1.4 MB |

Conclusion: a key is acknowledged in about 50 ms, but its visible result lags one full capture
(~0.6 s). Most of the cost is the guest capturing the whole screen, not PNG encoding — the raw
capture is still ~440 ms. Shrinking the payload therefore cannot remove most of the lag. The
only route to a smooth phone is a continuous video stream (scrcpy-style), which the app's own
windows already use. That is a large new pipeline, so it is recorded as the next step rather
than built in this pass without a benchmark to justify it.

### 8.9 Audio forwarding: investigated, not enabled

Measured with the bundled scrcpy (5.0.1) against the warm emulator (API 36):

| Probe | Result |
| --- | --- |
| `--audio-source=playback`, default buffer, 20 s with media | Opus encoder and decoder start; **19** "buffering threshold exceeded, skipping" chunks |
| `--audio-buffer=100` / `200` (same run) | **0** skips in each of two runs |
| `--audio-buffer=100`, final rerun with media | **16** skips |
| `--audio-buffer=100`, settled 15 s before media | **6** skips before media, **6** after (no new drops from playback) |

Conclusion: the decoder drops audio chunks even with no media playing, and the buffer setting does not
reliably remove them. Sound audibility through Windows was **not** measured. Audio is therefore kept
off (`--no-audio`) and the capability is reported as not verified. The argument pair is kept as
`scrcpy::AUDIO_ARGUMENTS` for the next pass.
