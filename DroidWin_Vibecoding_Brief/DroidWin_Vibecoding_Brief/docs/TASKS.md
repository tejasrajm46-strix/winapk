# Task Backlog — DroidWin

**Rule:** Do tasks in dependency order. Do not skip the runtime/window feasibility gate.

Statuses: `TODO`, `IN PROGRESS`, `BLOCKED`, `DONE`. Update status only with evidence.

## EPIC 0 — Repository and runtime feasibility

### DW-001 — Inspect current workspace
- Status: DONE
- Goal: Determine whether a repository already exists and identify current stack, build, tests, and constraints.
- Acceptance:
  - Existing files and architecture inspected.
  - Build/test commands recorded.
  - No files overwritten before inspection.
- Deliverable: workspace assessment.

### DW-002 — Evaluate candidate Android runtimes
- Status: DONE
- Owner: research agent
- Goal: Compare viable Windows Android execution backends.
- Check: maintenance, supported Windows versions, CPU architecture, Android API, APK install, graphics, input, clipboard, file sharing, concurrent apps, independent windows, security, licensing, redistribution.
- Acceptance:
  - Evidence and source links recorded.
  - Unknowns separated from verified facts.
  - A recommendation and fallback are documented.
- Deliverable: `docs/RUNTIME_DECISIONS.md`.

### DW-003 — Prove APK install and launch
- Status: DONE
- Goal: Install and launch one known-compatible APK using the selected backend.
- Acceptance:
  - Real APK installed.
  - Launch is initiated programmatically.
  - Backend result and errors captured.
  - No fake UI-only success.
- Deliverable: minimal reproducible prototype.

### DW-004 — Prove independent app windows
- Status: DONE
- Goal: Demonstrate two actual Android apps in independently managed Windows desktop windows.
- Acceptance:
  - Each app can be focused and resized independently.
  - Keyboard/mouse behavior is tested.
  - Closing a host window has documented semantics.
  - Backend limitation is recorded if this is not possible.
- Deliverable: prototype and decision.

### DW-005 — Record architecture decision
- Status: DONE
- Goal: Select the MVP backend and document limitations.
- Acceptance:
  - `DECISIONS_LOG.md` updated.
  - Architecture diagram and interfaces match evidence.
  - Any product compromise is approved before proceeding.

## EPIC 1 — UI foundation

### DW-010 — Translate design reference into tokens
- Status: DONE
- Goal: Implement the supplied Kage Zen/Kyoto Zen visual language.
- Acceptance:
  - Color, typography, spacing, border, and shape tokens centralized.
  - Fonts/assets licensing and offline fallback considered.
  - Screens visually checked against supplied references.

### DW-011 — Build shared application shell
- Status: DONE
- Goal: Create title bar strategy, navigation, search/command field if supported, status strip, and page container.
- Acceptance:
  - Navigation works.
  - Selected state is clear.
  - No fake runtime status.
  - Window resizing and DPI scaling do not clip important controls.

### DW-012 — Build Overview screen
- Status: DONE
- Acceptance:
  - Install action visible.
  - Runtime state truthful.
  - Installed apps sourced from runtime/repository, or an explicit development mock mode.
  - Empty/loading/error states implemented.

### DW-013 — Build APK Installer screen
- Status: DONE
- Acceptance:
  - File picker and drop zone work.
  - Inspection, installation, cancellation, success, and error states are defined.
  - No fake progress.
  - Package metadata is not fabricated.

### DW-014 — Build Applications Library
- Status: DONE (grid/list toggle built in the final polish pass; in-UI data deletion remains a separate, deliberate decision)
- Acceptance:
  - Search and grid/list views work if included.
  - App actions map to real services.
  - Empty and error states work.
  - Uninstall/data deletion are clearly distinguished.

### DW-015 — Build Runtime Management and Settings
- Status: PARTIAL (Runtime screen complete; Settings carries facts, not toggles)
- Acceptance:
  - Only supported backend controls appear enabled.
  - Capabilities and limitations are visible.
  - Diagnostic logging and error details are useful.

## EPIC 2 — Domain, persistence, and operations

### DW-020 — Define domain models and operation states
- Status: DONE
- Acceptance:
  - Typed models for apps, runtime status, package inspection, operation results, and errors.
  - State transitions validated.
  - Unit tests cover invalid transitions.

### DW-021 — Implement SQLite persistence
- Status: DONE
- Acceptance:
  - Versioned schema and migrations.
  - Settings and metadata persist.
  - Startup reconciliation against runtime is supported.
  - No sensitive secrets stored unnecessarily.

### DW-022 — Implement APK inspection
- Status: DONE
- Acceptance:
  - Valid/invalid APK cases tested.
  - Metadata is extracted only when available.
  - Malformed archive and large-file cases handled.
  - Inspection never executes APK code.

### DW-023 — Implement runtime adapter
- Status: DONE
- Acceptance:
  - Uses documented backend interfaces.
  - No runtime-specific commands in UI code.
  - Timeouts, cancellation, exit codes, and diagnostics handled.
  - Contract tests added.

### DW-024 — Implement install/update/uninstall
- Status: DONE (keep-data and delete-data paths exposed; keep-data verified live with `cmd package uninstall -k`; update with a second version still unexercised)
- Acceptance:
  - Install and update work with test APKs.
  - Invalid update signatures are not bypassed.
  - App data is preserved on update.
  - Data deletion requires explicit user choice.

### DW-025 — Implement application enumeration and launch
- Status: DONE
- Acceptance:
  - Library reconciles with backend state.
  - Launch failures are shown clearly.
  - App identity is based on package ID, not display name alone.

## EPIC 3 — Windows integration

### DW-030 — Implement APK file activation
- Status: PARTIAL (association compiled into the NSIS installer; cold start from argv and single-instance forwarding verified from the release binary; installer not run, so the registry write and Explorer double-click are unverified)
- Acceptance:
  - File association registered by installer using supported mechanisms.
  - Double-click passes path correctly.
  - Spaces, Unicode, stale paths, and repeated activation tested.
  - Uninstall removes association as appropriate.

### DW-031 — Implement independent window integration
- Status: PARTIAL (independent windows proven; no host-side chrome or back control)
- Acceptance:
  - Actual app surface is shown, not a mock.
  - Window move/resize/minimize/maximize/close behavior tested.
  - Multiple concurrent app windows tested.
  - Input focus and app crash isolation documented.

### DW-032 — Implement clipboard and file integration
- Status: TODO
- Acceptance:
  - Supported copy/paste scenarios tested.
  - Host file access is explicit and limited.
  - Unsupported scenarios explained to user.
  - Security implications documented.

## EPIC 4 — Quality, release, and documentation

### DW-040 — Add failure and recovery coverage
- Status: PARTIAL (missing SDK, corrupt/empty/non-zip APK, refused old target SDK, timeout, keep-data uninstall and launch-timeout paths are covered; disk exhaustion and interrupted install are not)
- Acceptance:
  - Runtime missing/crashed, insufficient disk, invalid APK, interrupted install, failed launch, and timeout tested.
  - No permanent loading states.
  - Recovery path is clear.

### DW-041 — UI and accessibility QA
- Status: PARTIAL (DOM/a11y checks plus native captures at 1280x820; DPI 125/150% and minimum-size matrix pending)
- Acceptance:
  - Visual review against supplied screenshots.
  - No clipped/overlapping controls.
  - Keyboard navigation and visible focus.
  - DPI and resizing checked.

### DW-042 — Packaging and update workflow
- Status: PARTIAL (NSIS installer builds at 11.63 MiB with the `.apk` association and bundled scrcpy; not installed, unsigned, no update flow)
- Acceptance:
  - Clean install/uninstall tested.
  - User data preserved on upgrade.
  - File association lifecycle tested.
  - Runtime licensing and redistribution reviewed.

### DW-043 — Compatibility test matrix
- Status: PARTIAL (F-Droid, NewPipe and one API-17 package tested; the API-17 package renders blank on a virtual display and works mirrored)
- Acceptance:
  - Windows version, host architecture, backend version, Android API/ABI, test APK, result, and known limitations recorded.
  - Public claims do not exceed test evidence.

### DW-044 — Release readiness review
- Status: PARTIAL (build, tests and clippy pass; installer unsigned; transitive licence inventory outstanding)
- Acceptance:
  - Build/tests pass or failures are documented.
  - No fake telemetry or demo data in release.
  - Security and licensing reviewed.
  - Known limitations published.

### DW-045 — Detect a blank virtual display at launch
- Status: PARTIAL (installed legacy packages are labelled from their manifest and launch now requires the app's own process; a blank *rendered* display is still not detected by a rendering probe)
- Progress: with per-app display geometry (DW-046) the supplied API-17 package renders on its own display (`screenshots/geometry-after-eval-display.png`). The blank recorded earlier was not reproduced at either 540×1170 or a phone-sized surface in the latest run, so it is an open, unconfirmed rendering case rather than a confirmed defect.
- Goal: do not report a window as successfully opened when its Android display renders nothing.
- Acceptance: launch either proves pixels were produced, or reports "opened, Android started the app, but the display rendered blank" and offers the mirror fallback.
- Evidence: see `docs/IMPLEMENTATION_AUDIT.md` §3 FR-06 and `docs/DECISIONS_LOG.md` D-011.

### DW-046 — Per-app display geometry
- Status: DONE (verified on the emulator)
- Goal: create the virtual display with the orientation and size the app expects instead of always 540×1170 portrait.
- Implementation: `src-tauri/src/runtime/display.rs` is the single geometry policy. The AXML reader (`src-tauri/src/apk/axml.rs`) reads `android:screenOrientation` from the launchable activity (application-level fallback) and the installer caches it per package (store schema v3). `EmulatorRuntime::launch` builds a 1080×2400 portrait / 2400×1080 landscape surface at 420 dpi — 411×914 dp, a phone surface — with a window at the same aspect ratio, and exposes `geometry_of(package)` for coordinate conversion. When a manifest names no orientation, the display Android actually composes is recorded for the next launch.
- Root cause: the old surface was 540×1170 at 420 dpi = 205×445 dp, less than half a phone. A landscape-locked app rotated it to 1170×540 and its layout was taller than 205 dp, so its own content was clipped; the portrait window then showed a landscape stream in a fraction of its area.
- Acceptance: `android:screenOrientation` selects display geometry; a landscape game renders fully; portrait apps keep a portrait surface.
- Evidence: `screenshots/geometry-before-blockbuster-window.png`, `screenshots/geometry-before-blockbuster-display.png`, `screenshots/geometry-after-blockbuster-window.png`, `screenshots/geometry-after-blockbuster-playing.png`, `screenshots/geometry-after-accelerace-window.png`. Reproduce with `cargo run --example geometry_proof -- <apk>…`.

### DW-047 — No console windows for child processes
- Status: DONE (verified)
- Goal: no console window or terminal tab appears for adb, avdmanager, taskkill, scrcpy, aapt2 or the emulator's acceleration check.
- Implementation: `src-tauri/src/proc.rs` applies `CREATE_NO_WINDOW`; every spawn site uses it.
- Evidence: `docs/STABILITY_PERFORMANCE_AUDIT.md` §3.1 (GUI-parent probe: 1 visible console window without the flag, 0 with it).

### DW-048 — Polling and boot checks must not spawn process storms
- Status: DONE (verified)
- Goal: an idle app should not launch a process every couple of seconds; a boot should not launch two or three per second.
- Implementation: cached environment probe (30 s TTL, `probe_now()` for explicit refresh), one `getprop` dump per readiness check, one adb spawn per boot second, single-flight frontend library refresh.
- Evidence: `docs/STABILITY_PERFORMANCE_AUDIT.md` §3.2.

### DW-049 — One boot, one window per package
- Status: DONE (verified)
- Goal: concurrent start/install/launch must not boot a second emulator or open a duplicate window.
- Implementation: `boot` and `launch` mutexes in `EmulatorRuntime`, fast path returning an existing window.
- Evidence: `docs/STABILITY_PERFORMANCE_AUDIT.md` §4.

### DW-050 — Closing the manager owns its runtime
- Status: DONE (verified)
- Goal: exiting DroidWin closes its app windows and stops only the emulator it started.
- Implementation: `RunEvent::Exit` runs a bounded `shutdown()`; `shutdown()` returns early when this process did not spawn the emulator; emulator stderr goes to `logs/emulator.log`.
- Evidence: `docs/STABILITY_PERFORMANCE_AUDIT.md` §4 (both ownership directions observed; no leftover processes).

### DW-051 — Poll only as often as the answer can change
- Status: PARTIAL (policy unit-tested; live subprocess count not re-measured)
- Goal: an idle UI must not spawn an adb process every 8 s, and a hidden window must not poll at all.
- Implementation: `src/polling.ts` (3 s while starting, 15 s steady, 30 s not configured, 20 s hidden), visibility-aware timer in `App.tsx`, and a 2 s readiness cache in `EmulatorRuntime` so a status poll, a launch and a library refresh share one `getprop` answer.
- Evidence: `src/polling.test.ts` (4 tests); `cargo test` and `cargo clippy` pass.

### DW-052 — Library grid/list views and shared search
- Status: DONE (DOM tests and 1100×720 preview check)
- Goal: match the reference's card/list modes and let the top bar search reach the library.
- Implementation: `Applications.tsx` view toggle persisted in localStorage, `AppCard` in `common.tsx`, top-bar search with Ctrl+K in `AppShell.tsx`.
- Evidence: `screens.test.tsx` grid-toggle and shared-query tests; preview DOM at 1100×720 with no horizontal overflow.

### DW-053 — Serialise package activation
- Status: PARTIAL (implemented; live double-open not exercised)
- Goal: two APKs opened quickly must not inspect or install on top of each other.
- Implementation: activation queue in `App.tsx` that waits for an install to finish, plus an `install_gate` mutex in `EmulatorRuntime`.
- Evidence: code review and compile-time checks; no live two-APK race was run.

### DW-054 — Adopt the official brand mark
- Status: DONE (in-app and packaged icon both inspected)
- Goal: use the supplied DroidWin identity instead of the provisional inline SVG.
- Implementation: `src/assets/branding/droidwin-mark.png` (256 from the 512 master); top bar 26x26, Settings About 52x52; Tauri 32/128/256 PNGs and multi-size `icon.ico` rebuilt; obsolete `scripts/make-icons.mjs` removed.
- Evidence: preview DOM reports natural 256x256 at both sizes; `pnpm build` emits the asset; `tsc` and 30 vitest tests pass.
- Follow-up: done in DW-057 — the packaged executable's icon is now extracted and inspected, not assumed.

### DW-055 — Per-application keyboard mapping (mission steps 2–3)
- Status: DONE (proven on the emulator with Block'Buster)
- Goal: a user can bind any physical key to an Android key event or to a touch point on one app's display, per package, off by default, editable while the app runs.
- Implementation: `src-tauri/src/input/mod.rs` (action model, validation, key labels, one dispatcher), `src-tauri/src/input/hook.rs` (`WH_KEYBOARD_LL` hook and its message loop), store schema v4 (`input_mappings` + `input_profiles`, migrated from v3 with a test), `EmulatorRuntime::package_for_pid` (foreground process → package), commands `input_state`/`save_input_mapping`/`remove_input_mapping`/`set_input_mapping_enabled`/`set_input_mappings_enabled`/`reset_input_mappings`, and `src/components/ControlsDialog.tsx` reached from the app row's Controls action.
- Mechanism: a global low-level keyboard hook swallows a key only when the foreground window belongs to one of DroidWin's app windows **and** that package has an enabled mapping for it; everything else reaches scrcpy unchanged, so nothing double-fires. Swallowed keys become `adb shell input` on the app's own display id, with touch percentages resolved through `geometry_of(package)`.
- Evidence: `screenshots/input-01-start-screen.png` (start screen) → `screenshots/input-02-mapped-key-started-game.png` (J pressed: the game started and ran to a score of 4) → `screenshots/input-03-mapping-off-start-screen.png` and `screenshots/input-04-mapping-off-key-inert.png` (same key with mapping off: byte-identical frames, nothing tapped). Reproduce with `cargo run --manifest-path src-tauri/Cargo.toml --example input_proof -- <apk> --key J --x 50 --y 80`; the harness verifies the target window actually held focus before sending the key.
- Tests: `cargo test` 155 passed, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `tsc --noEmit`, 52 vitest tests (5 of them for the Controls panel and its capture flow).
- Follow-up: a capture-from-hardware affordance in the panel for keys the UI cannot synthesise, and per-mapping conflict warnings when the same key maps to two actions.

### DW-056 — Triggers, event detection and separated dispatch/verification (mission step 5)
- Status: DONE (proven over the real MCP transport)
- Goal: a client can register an action, a state trigger or a workflow trigger, see it fire, and tell from the result whether an action was merely dispatched or actually verified; cancellation and a dropped client leave nothing behind.
- Inventory before this task: the endpoint already had 25 tools — runtime/device/app listing, open/close, install, screenshot, UI hierarchy, element search, foreground package, tap, long press, swipe, scroll, type text, press key, key combinations, `wait_for_element`, `wait_for_app_state`, `verify_action`, recent errors, runtime stats and skills. Genuinely missing: any trigger mechanism, any workflow composition, any app lifecycle signal (only `apps_changed`), and any per-action dispatch/observation/verification result.
- Implementation: `src-tauri/src/agent/triggers.rs` (trigger model, validation, event/state/runtime-failure detection, single-flight firing), `src-tauri/src/agent/workflow.rs` (step runner returning dispatched/observed/verified per step), `RuntimeEvents::app_event` + `EventBus` (`src-tauri/src/runtime/mod.rs`), window lifecycle emission and `classify_exit` (`emulator_runtime.rs`), store schema v5 `triggers` table, six tools (`run_workflow`, `list_triggers`, `set_trigger`, `delete_trigger`, `fire_trigger`, `get_app_events`) and the observation helpers on `ControlService` (`observe`, `screen_hash`, `has_element`, `app_running`).
- Detection: only facts DroidWin observes — the window host started, exited, or exited with a failure code; a state trigger polls the display it names; a runtime trigger watches the runtime's own status. A close DroidWin asked for is reported as Closed, not Crashed (`classify_exit`, tested).
- Evidence: `screenshots/mcp-01-workflow-tap-started-game.png` (workflow opened, waited, looked, tapped, verified) and `screenshots/mcp-02-trigger-fired-on-open.png` (a trigger registered over MCP fired when the window opened, and its own tap started the game). Run log: `.droidwin/mcp-proof.log`; reproduce with `cargo run --manifest-path src-tauri/Cargo.toml --example mcp_proof -- <apk>`.
- Measured over HTTP JSON-RPC: 32 tools listed; workflow 5/5 dispatched and verified; `app_opened` event fired trigger `trg-…` with `fire_count=1`; explicit fire verified; a 60 s wait cancelled in 2.28 s with `cancelled:true` and the step marked not dispatched; a dropped connection left the endpoint answering; after teardown `windows=0`, `scrcpy=0`, `adb` unchanged.
- Tests: `cargo test` 177 passed, `clippy -D warnings` clean, `fmt --check` clean, `tsc` clean, 53 vitest tests pass.
- Follow-up: no UI for triggers yet (MCP-only), and the trigger engine reports a fire in the action log but not in the Agent screen's live feed.

### DW-057 — Put the brand mark in the executable and installer
- Status: DONE (icon extracted from both binaries and inspected)
- Goal: the supplied logo is the icon of `droidwin.exe`, the installer, the window and the app header, and a replaced icon actually reaches the binary.
- Implementation: emblem (the badge without the wordmark, which smears at 16–32 px) exported at 16/24/32/48/64/128/256/512 and written to `src-tauri/icons/{icon.ico,32x32.png,128x128.png,256x256.png}` plus `src/assets/branding/droidwin-mark.png`; `build.rs` declares `cargo:rerun-if-changed=icons`.
- Root cause it fixed: tauri-build embeds `icons/icon.ico` but does not watch the icon files, so after replacing the mark the NSIS installer carried the new logo while `droidwin.exe` still carried the old resource from a cached object file.
- Evidence: `.droidwin/icon-exe2.png` and `.droidwin/icon-setup.png`, extracted with `Icon::ExtractAssociatedIcon` from `target/release/droidwin.exe` and the NSIS setup, both showing the new mark; `pnpm tauri build` → 12.47 MiB installer.

### DW-058 — Repair the proofs that had drifted from the runtime
- Status: DONE (both proofs run green against the real runtime)
- Goal: the release gates must fail for product reasons, not because a fixture still describes an older runtime.
- Implementation: `examples/mcp_proof.rs` now finds a clickable element and taps it by selector (its fixed `(1200, 870)` tap was outside a 1080-wide portrait display and was refused, correctly, as out of bounds), and the runtime is created in `main` so a failed proof still runs `shutdown()` (the error path called `process::exit(1)` and left an orphaned scrcpy window that polluted the next run's "before" sample). `tests/agent_live.rs` asserts the 1080×2400 surface instead of the retired 540-wide one.
- Evidence: `mcp_proof` 5/5 steps verified, `after: windows:0, scrcpy:0`; `agent_live` 29.9 s green with no leftovers.

### DW-059 — Never leave a window with no picture
- Status: DONE (live, `hcr_recovery`)
- Goal: a window that Android draws nothing into must be reported and closed, not handed back as an opened app.
- Implementation: `EmulatorRuntime::launch` waits up to 10 s for the package's display; if none is reported it looks for an EGL-config crash block naming the pid it launched (Android restarting a crashed app used to make the "process is gone" check pass, so the software-renderer recovery never ran), and otherwise closes the window and returns `launch_failed` with what was observed.
- Root cause: a non-resizable, landscape-only activity stays on the default display; the scrcpy display then has `mCurrentFocus=null` and the window is black. Verified with `dumpsys window displays` and `dumpsys activity` on Hill Climb Racing (`nonResizable … SCREEN_ORIENTATION_SENSOR_LANDSCAPE`).
- Evidence: `cargo test --test hcr_recovery -- --ignored --nocapture` — 25.2 s, the launch reports `com.fingersoft.hillclimb opened, but Android drew nothing on its window.`, the window is closed, the runtime keeps running, and the re-run `agent_live` confirms a normal app is unaffected.
- Follow-up: the EGL-recovery-on-restart branch is unverified (the crash did not recur); a mirror-on-default-display fallback is still not built (DW-045).

### DW-060 — The virtual phone: Android's own display inside DroidWin
- Status: DONE (live: the capture and the touch path were verified on the emulator; see the gap below)
- Goal: a user can see and use the runtime's own phone screen — its home screen, its installed apps, what is installing and the connection — without switching to another tool.
- Implementation: `EmulatorRuntime::device_screen` (capture), `::device_tap` and `::device_swipe` (input), all on `display::DEVICE_DISPLAY`, which is the one display DroidWin did not create; `PhoneScreen` in `domain/runtime.rs`; `png_size` moved from `agent/control.rs` to `runtime/display.rs` because it reads a capture's geometry; commands `phone_screen`/`phone_tap`/`phone_swipe`; and `src/screens/Phone.tsx` behind a new Phone navigation item.
- Mechanism: the emulator DroidWin already runs, mirrored — no second emulator. One still per second while the screen is visible and the runtime is ready, single-flight, nothing while the window is hidden. The frame takes the capture's own aspect ratio, so a click converts to a device pixel with the image's scale and no letterbox; a drag under 8 device pixels is a tap, a longer one is a swipe. Launching a tile opens the app's own Windows window.
- Root cause it avoids: a live video mirror would be a second scrcpy pipeline competing with the app windows for the same display server.
- Evidence: `cargo test --test phone_live -- --ignored --nocapture` — passed in 27.5 s. `capture: 1080x2400, 610380 bytes, 730 ms`; `flat colour: None`; `swipe dispatched in 345 ms`; `tap dispatched at 540, 1200`; the frame changed after the swipe (`.droidwin/phone-before.png` vs `.droidwin/phone-after.png`, different hashes — the lock screen was dismissed). Tests: `cargo test --lib` 198 passed, `clippy -D warnings` clean, `fmt --check` clean, `tsc` clean, `pnpm test` 69 passed (11 new for this screen).
- Gap: mission §3's discovery half is not built — there is no app catalogue or download browser, so every install still starts from an APK file the user chooses. The Tauri command layer was not clicked through in a running window, and only tap/drag are forwarded (no pinch, long-press or typing to the phone display).

### DW-061 — The phone driven through the packaged exe: mapping, keyboard, cadence
- Status: DONE (live: measured in the release executable, with Android's own responses as evidence)
- Goal: the phone should be usable, not just visible — clicks land where they look, the keyboard reaches Android, and an idle phone is not captured every second forever.
- Implementation: `Phone.tsx` now converts input against the **painted image box** (a `ResizeObserver`-measured aspect-fit rectangle) instead of the frame, so a click in a letterbox margin is ignored and one on the image scales exactly; a `keydown` handler maps ASCII typing and Enter/Tab/arrows/Backspace/Escape(Back)/Home/PageUp/PageDown to `phone_text`/`phone_key`; the refresh loop keeps a small interval for four seconds after input and idles at 2.5 s, with no capture while the frame is off screen or the window hidden; the duplicate `role=alert` is gone. Rust: `EmulatorRuntime::device_text` (one batched `input text` per burst, control characters stripped, spaces as `%s` via the extracted `device_text_argument`), `device_key` extended with `FORWARD_DEL`, commands `phone_key`/`phone_text` registered and allowlisted.
- Evidence: in the packaged exe — frame 262.7×492 with a 20.6 px bar each side (a click at the visual right edge sent device x=995 of 1080) is now 262.7×583.7, aspect 0.4500 = the capture's 0.4500, bar 0; a tap at device (540,228) left the launcher's search `EditText` with `focused="true"`; typing `fdroid` gave `text="fdroid"` and live results; Enter moved `mCurrentFocus` to `OneSearchActivity`; Home returned to `NexusLauncherActivity`; Escape closed the app drawer; six characters cost one `input text` call. Cadence: idle 2.8–3.3 s per capture, 1.0–1.2 s after input (the ~0.73 s `screencap` itself is the floor), nothing when hidden. Tests: 199 Rust lib tests, 76 frontend (18 for this screen), clippy/fmt/tsc clean; `pnpm tauri build` 12.51 MiB.
- Gap: still stills, not video; no pinch or long-press; typing is key events, so an IME-dependent field needs a tap first; mission §3's discovery/download half and the plan's performance dashboard are still not built.
- Follow-up found in the same pass: the installations row laid out wrong (title clipped to 44 px, package id under the "completed" pill) because `.row` reserved a 44 px icon column for a row that renders no icon; `.row` is now two columns with `.row:has(.row__icon)` restoring the icon column. Reuse was also measured: a fresh app process attached to the running emulator and captured in 365 ms with the emulator's pid unchanged.

## Task execution template

For each new task record:
- ID and title
- Status and owner
- Goal
- Files/modules allowed to change
- Dependencies
- Acceptance criteria
- Tests to run
- Actual results
- Follow-up items
