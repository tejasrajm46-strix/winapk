# Final control and capture pass — 28 October 2026

Windows 11 (build 26200), Android 36 AVD `DroidWin_API36`, RTX 5060 Laptop. Everything below was
run against this working tree; anything that did not run says so.

## What changed

| Area | Change |
| --- | --- |
| Android controls | The phone screen has a compact toolbar: Back, Home and recent apps, sent as Android key events on display 0 (the same `input` path the phone's keyboard already used). |
| Screenshot | `phone_screenshot` captures Android's own display with `screencap` and writes the PNG into `Pictures\DroidWin` (or the data directory when Windows reports no Pictures folder). The phone screen shows the file name and opens it in Explorer. |
| Screen recording | `recording_start` / `recording_stop` / `recording_status` run Android's `screenrecord` on the device, finalise it with SIGINT, pull it into `Videos\DroidWin`, and verify the MP4 header before reporting a path. A recorder that Android rejects is detected in 350 ms and reports its own stderr. |
| Performance presets | Balanced, Performance and Compatibility are real, validated plans (`hw.preset`), on top of the existing automatic/custom plan. Compatibility asks for SwiftShader and says so. |
| Phone display | Size and density are Android's own `wm size` / `wm density` overrides on display 0: applied immediately, no emulator restart, reset available, validated in the backend. `apply` waits until `screencap` really returns the new size before reporting success. |
| Audio | The emulator is no longer started with `-no-audio`. Android's audio reaches the Windows output device once per runtime. App windows stay `--no-audio` so N windows cannot play N copies. |

## Automated checks

| Command | Result |
| --- | --- |
| `cargo test --lib` | **216 passed**, 0 failed |
| `cargo clippy --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `pnpm exec tsc --noEmit` | clean |
| `pnpm test` | **81 passed** (8 files) |
| `cargo test --test capture_live -- --ignored --nocapture` | **passed** (36.7 s, emulator already booted) |
| `cargo test --test hook_live -- --ignored --nocapture` | **passed** (20.6 s, added by the hook pass below) |

## One command for every live check

[`scripts/live.mjs`](../scripts/live.mjs) runs the live checks one after another (they all need the
emulator, so they cannot run together), keeps each run's full output in `.droidwin/live/<area>.log`, and
prints one line per area with its result, its time and the test's own summary line:

```text
$ node scripts/live.mjs
capture   PASS   35.6 s  test result: ok. 1 passed; 0 failed; … finished in 35.23s
phone     PASS    3.9 s  test result: ok. 1 passed; 0 failed; … finished in 3.57s
mapping   PASS   17.1 s  test result: ok. 1 passed; 0 failed; 1 filtered out; …
hook      PASS   23.3 s  test result: ok. 1 passed; 0 failed; … finished in 22.84s

4/4 passed
not run: agent, hcr (add --all or name them)
```

`--all` adds `agent` (installs a real package) and `hcr` (needs a game installed in the AVD); naming an
area runs just that one. The exit status is 1 if any area failed, and an area that hangs is killed after
its timeout and reported as a failure instead of stopping the run silently. The GUI sweeps — the frame
rate and input latency measurements, screenshots of the running desktop app — are not areas: they drive
the app, not a test target.

**It found two things on its first full run.** `phone` failed: its swipe asserted that the screen
changed, but the phone had been left on the launcher's app drawer scrolled to its last row, where an
up-swipe moves nothing. The test now presses Home first, so it starts from a known screen; re-run with
the drawer parked at its end (the exact failing state) it passes. And the runner itself first crashed
closing a log handle it had already closed — it now streams into the log file instead.

## Release artifact

`src-tauri/target/release/bundle/nsis/DroidWin_0.1.0_x64-setup.exe` — 12.67 MiB, built from this
working tree after the audio change. The installer is not signed and was not installed on a clean
machine.

**The app you had open while testing was the previous build, which still passed `-no-audio` to the
emulator.** Restart DroidWin (or install this setup) before judging audio.

## Live verification (`capture_live`)

One test drives a real emulator through Android's own state, not through DroidWin's bookkeeping:

| Fact | Measured |
| --- | --- |
| Home | foreground becomes `com.google.android.apps.nexuslauncher` |
| Back | Settings opened (`com.android.settings`), Back leaves it, the captured pixels change |
| Recent apps | `uiautomator` hierarchy contains `overview_panel` / `task_view_single` |
| Screenshot | 1 398 027-byte PNG, 1080×2400, decodes as 8-bit colour, header size matches the report |
| Landscape | 2400×1080 capture after `user_rotation 1` |
| Display override | `wm size` reports `720x1600` / `280 dpi`; the capture is 720×1600; reset returns 1080×2400 |
| Invalid display values | `12×12` and `9000 dpi` refused before reaching the device |
| Recording | 2 669 044-byte MP4; `ffprobe`: H.264 1080×2400, 85 frames, 7.34 s, decodes cleanly |
| Live view during recording | a second `screenrecord` produced 132 KB while the recorder ran |
| Leftovers | `0` recording files left in `/data/local/tmp` |
| Preset | Performance saved as 8 vCPUs / 5120 MB; Android reports 6 vCPUs and the host GPU renderer |

`ffmpeg -f null -` warned about non-monotonic DTS on the null muxer. That is `screenrecord`'s
variable-frame-rate timestamps, not a malformed file: the same command decoded every frame.

## Audio

- Android `screenrecord` v1.4 has **no audio option** (`screenrecord --help` on this image), so a
  recording is video-only. That is Android's limitation, not a setting DroidWin hides.
- scrcpy 5.0.1 has no `--audio` flag (the command fails with `ambiguous option -- audio`); audio
  forwarding is on by default. Its own path was already measured as dropping 6–19 chunks per run
  (see the table further down), which is why the runtime does not depend on it.
- The emulator was the problem: `launch_args` passed `-no-audio`, silencing the whole runtime. The
  emulator now starts with audio enabled. During this pass the user reported hearing the guest's alarm
  and volume-key sounds on the host speakers; that is a person listening, not a measured capture, and
  it is recorded as such.
- App windows still get `--no-audio`: the sound belongs to the runtime, not to each window.

## Resource observations

Measured on this machine (24 logical processors, RTX 5060 Laptop) with the emulator running the
launcher, using the same `screenrecord` path the recorder uses:

| State | Emulator CPU | Emulator RAM | GPU utilisation |
| --- | --- | --- | --- |
| Idle (launcher, no capture) | 106.6 % of one core | 2 882 MB | 7 % |
| Recording 1080×2400 at 8 Mbit/s, screen changing | 281.6 % of one core | 2 907 MB | 14 % |

These are observations, not a before/after benchmark: no earlier build with identical settings was
measured in the same session, so no speed-up is claimed.

## Not verified here

- Host speaker output is confirmed by a person listening, not by a capture of the Windows mixer.
- Keyboard mapping's held-key/release behaviour is covered by its existing Rust and React tests; this
  pass did not re-drive it against a live app. **Superseded by the mapping pass below.**
- Recording a completely static screen produces very few frames (Android sends none until something
  changes); the MP4 is valid but short. A screen with motion records normally.
- No before/after performance benchmark was run, so the preset and display controls are reported as
  available and measured, not as a measured speed-up.

## Mapping pass, same day: mapped keys were reaching nobody

The report above called keyboard mapping "implemented and unit-tested". Driving it against a real app
window found that every mapped key event was discarded by Android before it reached the app.

**The defect.** `Adb::input` prefixed every command with `-d <display>` so app-window input could be
display-scoped. Motion events honour that. Key events do not: Android delivers an injected key to the
display that currently holds the focused window and drops it when `-d` names another display. Its own
word for it, from `logcat` on the emulator while injecting `input -d 4 keyevent 221`:

```text
W InputDispatcher: Focused display #0 does not have a focused window.
E InputDispatcher: But another display has a focused window
E InputDispatcher:   FocusedWindows:
E InputDispatcher:     displayId=4, name='b1f8c31 com.android.settings/com.android.settings.Settings'
```

Measured: `input -d 4 keyevent 221` left `screen_brightness` at 10; `input keyevent 221` on the focused
display moved it to 16. Touch was never affected — `input -d 4 motionevent DOWN 540 1200` shows up in
`dumpsys input` under `TouchStatesByDisplay → 4`.

**The fix.** [adb.rs](../src-tauri/src/runtime/adb.rs) scopes a key event only for the device's own
display. An app window's mapped key is unscoped and lands on that window's display, which is correct
because a mapping only fires while that window is the foreground one. The phone keeps `-d 0`, so a click
on the phone cannot type into an app window.

**Live evidence** ([mapping_live.rs](../src-tauri/tests/mapping_live.rs), `--ignored`, real emulator,
Settings in a real window on Android display 5). `screen_brightness` is reset to 10 before each case:

| Case | Android's value afterwards |
| --- | --- |
| one press and release of mapped K | 16 — exactly one step |
| five key-downs (one hold, four auto-repeats) then release | 16 — one step, repeats not forwarded |
| two presses back to back, no pause | 25 — both delivered, in order |
| Ctrl held, K pressed, no Ctrl+K mapping | 10 — an unmapped combo does not trigger plain K |

Held touches are read from Android, not assumed: `dumpsys input` lists the windows a display has a
pointer on. Holding a key mapped to a touch shows
`f516282 com.android.settings/com.android.settings.SubSettings` under display 5; after the release rule
runs the list is empty. Three cases were driven for real: Ctrl+K released with Ctrl first (the release
still found the combo's point), focus leaving the window (`reconcile_focus(None)`, what the 250 ms poll
does on Alt-Tab), and `release_all` + `flush` — what the app runs on exit — with a key still down. All
three left no pointer behind. A key-up arriving after the focus release is not treated as a second
release.

Frontend input is now ordered instead of fired in parallel
([Phone.tsx](../src/screens/Phone.tsx)): typed text, taps, swipes and named keys go through one queue, so
a word lands before the Enter that submits it and two taps keep the user's order. Held Escape/Home no
longer repeat Back one screen per auto-repeat, and losing pointer capture mid-drag drops the stale drag
origin instead of turning the next release into a swipe from nowhere (three React tests, one of which
fails on the previous build).

## The hook itself, driven by real keystrokes

`mapping_live` drives `on_key_for`, so the `WH_KEYBOARD_LL` hook — the part that decides *whose* window
has the focus — was still unproven. It is proven now, twice, with genuine desktop input.

**In the suite** ([hook_live.rs](../src-tauri/tests/hook_live.rs), `--ignored`, ~21 s):

```text
cargo test --test hook_live -- --ignored --nocapture
```

It installs the real hook through the app's own `arm()` path, gives a real `scrcpy` window the keyboard
focus with `SetForegroundWindow` (falling back to a title-bar click, because Windows refuses a
foreground change from a process the user is not using), and injects keys with `keybd_event`, which
enters the same system input queue a physical key does. No part of the app is called directly: the test
never mentions `on_key_for`, so a hook that is never installed, or one that resolves the wrong window,
fails on the first assertion. Everything is read back out of Android:

| Real key event | Android's answer |
| --- | --- |
| one press of mapped K | `screen_brightness` 10 → **16**, one step |
| held: four key-downs, one key-up | **16** — auto-repeat is swallowed, not delivered |
| two presses 80 ms apart | **25** — both arrived, in order |
| Ctrl down, K down, **Ctrl up first**, K up | the point under `TouchStatesByDisplay → display` appears, then is gone |
| Ctrl down, Shift down, K down, **Ctrl up**, **Shift up**, K up | Ctrl+Shift+K's own point appears, then is gone — the release finds its binding with no modifier left down |
| L held, then a real **Alt-Tab** | the focus really left (`GetForegroundWindow` checked), the held point was lifted **260–291 ms** later (the 250 ms poll plus one adb round trip), and the late key-up pressed nothing |
| focus returned, L held and released | works again — the earlier release left no stale ownership |

Four consecutive runs passed in 20.5–23.0 s. The test was then checked for the failure it
exists to catch: with the profile switched off (so the hook is never installed) the first real press
leaves brightness at 10 and the run **fails in 9.9 s** with
`a key typed into the app window reaches Android exactly once: brightness 10, expected 16`. That
mutation also found a defect in this test: a panic mid-run left the harness runtime unable to shut
down, so a failing check hung instead of reporting. The matrix now returns its first failure, restores
Android's brightness and closes the window, and only then fails.

**Against the shipped binary too.** A throwaway harness ran the same matrix through `droidwin.exe`
itself — its own hook, an app window opened over MCP, `keybd_event` from outside, `dumpsys input` and
`screen_brightness` read back. The final matrix was 19 checks and all 19 passed; an earlier 15-check run
of the same harness passed too. It covered two cases the suite test cannot: a mapped key typed while
another package's window has the focus does nothing (brightness stayed at 10), and the same key in
DroidWin's own window does nothing. It also held a key while closing the window: the app's log shows `motionevent UP` delivered for that display before the
display was destroyed. Nothing of the user's was left changed: the harness puts back the brightness,
the brightness mode, `agent.enabled` and every input mapping it found for that package (there were none;
the mappings for the machine's installed game were never touched). It is not kept in the tree —
`hook_live.rs` is the durable equivalent.

**One ordering defect that was real, found earlier and covered here:** a key-up is resolved through the
binding that consumed the press, not through the modifiers still down. Ctrl+K with Ctrl released first
lifts the point, and the Alt-Tab case above is the same rule reached through the focus poll. Two
modifier keys of the same kind held at once (left and right Shift) both normalise to the same code, so
releasing one lifts a touch the other was still holding; nothing latches.

## Rendering pass: measured, and the one slow path that was real

Item 3 of the brief ("fix slow screen rendering and improve responsiveness") had never been measured.
This pass measured it in the shipped pipeline — Android's `screenrecord` H.264 over adb, forwarded to
the webview, decoded with WebCodecs and painted on a canvas — and then fixed what the numbers showed.

**How it was measured.** The real app, not a harness: WebView2's debugging port
(`additionalBrowserArgs` in `tauri.conf.json`, added only for measurement), the real emulator, and the
real IPC channel. [scripts/perf-live-view.mjs](../scripts/perf-live-view.mjs) attaches over CDP, opens
the Phone screen, wraps `drawImage` on the phone canvas to timestamp every painted frame, and drives
Android with real `input swipe`s. Every rate is measured while the screen is actually moving, because
`screenrecord` only emits a picture when something changes: a still screen reports about 1 fps no matter
how healthy the pipeline is, and the script fails rather than report that.

| Measurement | Device / view | Before | After the fix |
| --- | --- | --- | --- |
| Sustained painted frames | 1080×2400 stream, 261×582 CSS box | **59.8 fps** (median gap 16.5 ms, p95 22.5 ms) | **60.1 fps** (median 16.5 ms, p95 21 ms) |
| Main-thread long tasks in 10 s | webview | 0 | 0 |
| Android Home click → changed pixel | phone toolbar | 282 ms | 291 ms |
| Tap on the phone screen → changed pixel | canvas | 484 ms | 349 ms |
| Stream ends mid-session | live view | **falls back to still captures for good** | **back to 60 fps, 262 ms gap** |

Upstream, for completeness, from a separate probe of the same command the app runs: the encoder
sustains ~51 fps at 1080p, 720p and 540p alike, but at **17.8 Mbit/s** at 1080p against 2.2 Mbit/s at
720p; a visible change reaches the host in **114–117 ms** at either size. The codec is not the limit and
the resolution is not the limit — so neither was changed.

**The defect that was real.** Android's `screenrecord` has a three-minute lifetime (`--time-limit 180`),
so the capture ends in ordinary use. DroidWin treated that as fatal: the canvas disappeared, the panel
switched to still captures at one every 750 ms, and the stream was never restarted. Reproduced by
killing the device-side `screenrecord` under a running live view:

```text
3 s after the drop:  {"canvasVisible":false,"imageVisible":true,"frames":0,
                      "error":"Live phone video is unavailable; showing still captures instead.
                               … Android video capture ended unexpectedly."}
15 s after the drop: same, still captures, no screenrecord process on the device
```

That is the slow rendering: 60 fps to 1.3 fps, permanently, a few minutes into any session.

**The fix** ([Phone.tsx](../src/screens/Phone.tsx)). An ended capture is no longer an error: if the
stream had produced a picture, the view restarts it, keeping the last frame on screen instead of
blanking the panel. Only a stream that never delivered a frame falls back to still captures, since that
is a real failure rather than a hiccup. Two React tests cover both branches (they run the real
`PhoneVideo` parser against a stubbed decoder; the recovery test fails on the previous build).

After the fix, the same kill leaves the canvas live with a new `screenrecord` process:
`fpsBeforeDrop 59.9`, `fpsAfterDrop 60.0`, longest gap **262 ms**, no error, no still captures.

**Regression checks.** Screenshot and recording share Android's capture path but not this code; the
`capture_live` suite was re-run against the same emulator and passes (Home, Back, screenshot at the
device's size, a playable recording with the live view running beside it, display override and reset).
Its Settings step now force-stops the package first: Android restores the page it last showed, so a
sub-screen left open by an earlier run made Back go up a level inside Settings instead of out, and the
test asserted the wrong thing. Audio is emulator output and was not touched.

**What this did not measure.** The other half of the request — a weak machine where decode or the
17.8 Mbit/s byte rate might genuinely bind — is not tested here; the notes above are this machine
(RTX 5060 laptop). The stream still asks for `--bit-rate 4M` and gets 17.8 Mbit/s at 1080p, so there is
clear headroom to cut bytes if a slower machine ever shows the frame rate dropping. The 750 ms
still-capture fallback (for browsers without WebCodecs) was left alone: it is a fallback, and nothing
measured pointed at it.
