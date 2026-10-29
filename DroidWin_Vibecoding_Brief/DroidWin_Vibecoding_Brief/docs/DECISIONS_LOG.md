# Architecture and Product Decisions Log

Record important decisions in reverse chronological order. Do not erase rejected alternatives; mark them superseded.

## D-000 — Initial project baseline
- Status: Proposed baseline
- Date: 2026-10-25
- Decision: Use the name DroidWin as a working name and preserve the supplied Kage Zen/Kyoto Zen design direction.
- Context: User provided a four-screen Stitch design archive for a Windows Android desktop engine.
- Consequences: UI implementation must follow `docs/UI_UX_SPEC.md`; the screenshots are inspiration and must not be treated as proof of backend capability.

## D-001 — Runtime selection is an explicit gate
- Status: Open
- Date: 2026-10-25
- Decision: Do not approve an Android execution backend until install/launch and independent-window feasibility are evaluated.
- Context: Product requires double-click APK installation and separate desktop windows; not all runtimes support that experience.
- Consequences: Complete `docs/RUNTIME_DECISIONS.md` and prototype tasks before committing to runtime-specific architecture.

## D-002 — Separate desktop product from runtime
- Status: Proposed
- Date: 2026-10-25
- Decision: Keep UI and application services independent of runtime-specific commands through a typed adapter boundary.
- Consequences: Backend can be replaced without rewriting the UI, provided actual capabilities match the abstraction.

## D-003 — Desktop stack is Rust + Tauri 2 + React
- Status: Accepted
- Date: 2026-10-25
- Decision: Build DroidWin as a Tauri 2 desktop application: Rust for the application core, APK inspection, runtime adapter, SQLite persistence and Windows integration; React + TypeScript + hand-written CSS for the Kage Zen interface.
- Context: `MEMORY.md` originally preferred C#/.NET + WinUI 3 "subject to repository and environment inspection". The environment has no .NET SDK; the product owner supplied an explicit stack decision favouring Rust + Tauri, and `AGENTS.md` requires an architecture decision rather than silent drift.
- Alternatives: C# + WinUI 3 (no SDK installed, and the supplied visual language is CSS-shaped); Electron (heavier runtime, no Rust core).
- Evidence: Rust 1.99.1 with MSVC 14.44 and WebView2 155 present; Tauri 2.11.6 builds and launches on this machine (`droidwin.exe`, window title `DroidWin`); the webview successfully invoked backend commands (an `adb.exe` probe process was spawned by the running app).
- Consequences: Presentation stays in `src/` and never builds backend commands; all system work is in `src-tauri/src`. WinUI-specific guidance in the brief is superseded; UI rules in `UI_UX_SPEC.md` still apply verbatim.
- Follow-up tasks: DW-010, DW-011 (tokens and shell) are implemented against this stack.

## D-004 — Runtime backend is the Android Emulator plus scrcpy virtual displays
- Status: Accepted, verified on this machine
- Date: 2026-10-25
- Decision: Use the Android Emulator (from a user-provided Android SDK, booted headless) as the Android execution environment, and `scrcpy --new-display --start-app` to give each launched app its own virtual display and its own desktop window.
- Context: `RUNTIME_DECISIONS.md` required evidence before committing to a backend. WSA is discontinued; every commercial emulator examined licenses itself for personal use and cannot be embedded; a custom Android compatibility engine is explicitly out of MVP scope.
- Alternatives considered and rejected: WSA and community WSA builds (unsupported, provenance and licensing risk); BlueStacks, Genymotion, LDPlayer, MEmu, Nox, MuMu (consumer licences prohibit embedding; MuMu does ship per-app windows but not for third parties); Waydroid/redroid (Linux host or non-standard kernel); Android-x86/Bliss in a VM (no per-app host windows, licensing caveats).
- Evidence: see `docs/RUNTIME_PROOF.md` — two real APKs installed and launched into two distinct HWNDs, moved and resized at the OS level, one closed without disturbing the other, both still installed after a manager restart.
- Consequences: The SDK licence (cl. 3.4) forbids redistribution, so DroidWin requires a user-provided SDK and never bundles the emulator; `scrcpy` 5.0.1 (Apache-2.0) is the only bundled runtime component. All apps share one Android user, so isolation claims must stay modest.
- Follow-up tasks: DW-023/024/025 (adapter, install, enumeration) are implemented; DW-031 window integration is proven but not yet polished (no host-side back control).

## D-005 — Capabilities are measured, never declared
- Status: Accepted
- Date: 2026-10-25
- Decision: A capability is reported as supported only after it was verified on this machine. Unverified capabilities (clipboard sharing, file drop, audio) are reported as not verified even where the underlying component is known to support them.
- Context: The brief forbids fake telemetry and capability claims inferred from a backend's name.
- Evidence: `runtime.independent_windows_verified` is written by the proof harness only when two independent windows were observed; the Runtime screen renders every capability from that status payload.
- Consequences: The first run of the product honestly understates clipboard/file features. Turning them on requires a verification step, not a copy change.
- Follow-up tasks: verify clipboard and file drop in a later pass.

## D-006 — Product model borrowed from Mac.apk, architecture not
- Status: Accepted
- Date: 2026-10-25
- Decision: Adopt Mac.apk's product behaviours (one ordinary window per app, `.apk` association, per-app data directory, uninstall that keeps data by default, hand-tested compatibility list). Do not attempt its architecture.
- Context: The product owner supplied Mac.apk as a reference for "how it is done".
- Evidence: Mac.apk is an AGPLv3 Android-framework reimplementation for Apple Silicon (Bionic shim, ART, recompiler); `PRD.md` §8 puts exactly that out of MVP scope.
- Consequences: The experience targets per-app windows, while the implementation stays behind the runtime adapter so a future backend can replace the emulator without touching the UI.
- Follow-up tasks: file association and a tested compatibility matrix remain open (DW-030, DW-043).

## D-007 — Keeping app data uses the package manager, not `adb uninstall -k`
- Status: Accepted, verified on this machine
- Date: 2026-10-25
- Decision: Uninstall-with-keep-data runs `adb shell cmd package uninstall -k <pkg>`.
- Context: The audit tested the documented `adb uninstall -k` on Android 16 and it was refused: "The -k option uninstalls the application while retaining the data/cache… execute 'adb shell cmd package uninstall -k'."
- Evidence: `cmd package uninstall -k org.fdroid.fdroid` → `Success`; the package disappeared from `pm list packages -3` and remained in `pm list packages -u -3`, which is how DroidWin now confirms retention before telling the user data was kept.
- Consequences: The runtime reports what Android actually said; when retention cannot be read, the message says so instead of asserting it.
- Follow-up: DW-024.

## D-008 — The emulator boots with `-no-snapshot`, so installs survive a restart
- Status: Accepted, verified
- Date: 2026-10-25
- Decision: Spawn the emulator with `-no-snapshot` (no quickboot load or save).
- Context: With `-no-snapshot-save` the device still loaded an older quickboot snapshot on start, which silently reverted packages installed after that snapshot. The audit observed F-Droid missing after a restart.
- Evidence: after `adb emu kill` and a cold boot with `-no-snapshot`, `pm list packages -3` still contained the package installed before the restart.
- Consequences: Cold boot is slower (≈20–25 s here) but installation state is durable, which the product promises.

## D-009 — `.apk` activation: packaged association, queued cold start, single-instance forwarding
- Status: Accepted, partially verified
- Date: 2026-10-25
- Decision: Declare the `.apk` association in the Tauri bundle; on startup queue package paths from argv and let the webview collect them with `take_pending_activation`; forward later launches through `tauri-plugin-single-instance`.
- Evidence: release binary started with a package path logged `started with 1 package path(s)` and `activation: 1 package path(s) queued` and showed that package's inspection screen; a second launch exited 0 and the first logged `forwarded 1 package path(s)`. The NSIS script contains `APP_ASSOCIATE "apk" "Android package"`.
- Remaining: the installer was not run, so the registry write and Explorer double-click are still unverified.

## D-010 — scrcpy ships as a resource; the proof harness does not ship at all
- Status: Accepted, verified in the built installer
- Date: 2026-10-25
- Decision: Bundle the Apache-2.0 scrcpy build under `src-tauri/resources/scrcpy` and copy it into the data directory on first run; move the proof harness from `src/bin` to `examples/`.
- Context: A clean install had no window host, so every launch would have failed with a message that told the user to reinstall. At the same time the installer contained `proof.exe`.
- Evidence: generated `installer.nsi` has 34 scrcpy entries and zero `proof.exe` entries; `cargo run --example proof` is the documented harness command.
- Consequences: Installer is 11.63 MiB and self-contained apart from the user's SDK.

## D-011 — Old-target packages need an explicit opt-in, and virtual-display rendering is not assumed
- Status: Accepted, partially verified
- Date: 2026-10-25
- Decision: Offer `--bypass-low-target-sdk-block` only when the manifest reports `targetSdk < 23`, off by default, labelled with its consequences. Do not treat "process started on the display" as proof that the window renders app content.
- Context: The supplied test package targets API 17. Android 16 refused it (`INSTALL_FAILED_DEPRECATED_SDK_VERSION`); with the flag it installs and its process starts on a new display, but scrcpy shows a black window for it while the mirrored main display shows the app working.
- Consequences: The audit's limitation list says old packages may render blank on a virtual display; a rendering probe or mirror fallback is the next fix.

## D-012 — Child processes never allocate a console
- Status: Accepted, verified on this machine
- Date: 2026-10-25
- Decision: Every subprocess DroidWin starts is created with `CREATE_NO_WINDOW` through one module (`src-tauri/src/proc.rs`). No call site constructs a process without it.
- Context: DroidWin is a GUI-subsystem program. A console child spawned without the flag makes Windows allocate a console, which on this machine appears as a Windows Terminal tab/window per command — reported as "many unnecessary things opening".
- Alternatives: `STARTF_USESHOWWINDOW`/`SW_HIDE` (hides the window but still allocates a console and can leak a brief window); running commands through a shell (worse: no argument safety).
- Evidence: `docs/STABILITY_PERFORMANCE_AUDIT.md` §3.1 — a GUI-parent probe produced a visible console window with no flags and none with the policy; the policy does not disturb captured output or scrcpy's own window (`proc::tests`).
- Consequences: diagnostics rely on captured stdout/stderr and `logs/emulator.log`, not on a visible window.

## D-013 — A manager exit stops only the runtime it started
- Status: Accepted, verified on this machine
- Date: 2026-10-25
- Decision: On `RunEvent::Exit`, DroidWin closes its app windows and shuts down its emulator under an 8 s bound. An emulator DroidWin attached to but did not spawn is left running.
- Context: scrcpy children were created with `kill_on_drop(false)` and nothing ran on exit, so closing the manager left windows and an emulator behind; conversely `shutdown()` used to kill any emulator at the AVD's port, including one the user started.
- Evidence: proof runs in both directions (`terminated emulator process tree …` versus `leaving it running`), followed by no qemu/emulator/scrcpy/droidwin process and an empty `adb devices`.
- Consequences: stopping the runtime is explicit per session; the UI shows the boot honestly. A force kill cannot run the handler, so a killed process may leave the emulator for the next launch to reconcile.

## D-014 — Display geometry comes from the app's declared orientation, sized in dp
- Status: Accepted, verified on this machine
- Date: 2026-10-26
- Decision: One policy (`src-tauri/src/runtime/display.rs`) creates the Android surface an app window is drawn on: 411×914 dp at 420 dpi, i.e. 1080×2400 px portrait or 2400×1080 px landscape, with the window at the same aspect ratio (420×933 or 933×420). The orientation comes from `android:screenOrientation` on the launchable activity (application-level fallback), read by the existing AXML reader and cached per package.
- Context: Every display was created 540×1170 at 420 dpi — 205×445 dp, less than half a phone. A landscape-locked app (Block'Buster) forced Android to rotate that surface to 1170×540, and its layout was taller than 205 dp, so its own content was clipped (the Start button, the score column). The portrait window then showed a landscape stream inside a portrait frame. Measured: `init=540x1170 … cur=1170x540` on display 2 while the app was focused.
- Alternatives: Keep one size and let the user pick per app (more UI, still needs the same numbers); scale the window to crop (hides the defect and breaks input coordinates); use the device's 1080×2400 for every app including landscape (defeats the point).
- Evidence: `screenshots/geometry-before-blockbuster-display.png` (content clipped inside a 1170×540 surface) against `screenshots/geometry-after-blockbuster-window.png` and `screenshots/geometry-after-blockbuster-playing.png` (full layout, game started and scored via a display-scoped tap). Accelerace keeps a portrait surface (`geometry-after-accelerace-window.png`). Reproduce with `cargo run --example geometry_proof -- <apk>…`.
- Consequences: A virtual display costs ~10 MB of framebuffer at 1080p instead of ~2.5 MB at 540×1170. `EmulatorRuntime::geometry_of(package)` is the single source of truth for turning window coordinates into display coordinates (keyboard/touch mapping, MCP taps, screenshot sizing). Apps whose manifest locks no orientation get a portrait surface, and the orientation Android actually composed is recorded for the next launch.
- Follow-up tasks: DW-046 closed; DW-045 (blank-render detection) still open.

## D-015 — Keyboard mapping is a low-level hook in front of scrcpy, dispatched as adb input
- Status: Accepted, verified on this machine
- Date: 2026-10-26
- Decision: DroidWin installs one `WH_KEYBOARD_LL` hook for the process. It resolves the foreground window's process to an app package (`package_for_pid`), looks up that package's enabled mappings, and swallows a key only when one matches. A swallowed key becomes a display-scoped `adb shell input` (`keyevent`, `tap`, or `motionevent DOWN/UP`), never a second keyboard path. Ctrl/Alt/Win combinations always pass through; Shift does not block a mapping, because it is ordinary typing and a common game control.
- Context: scrcpy owns the app window and forwards the keyboard itself. Mapping inside DroidWin's own webview would never see those keys, and forwarding them a second time after scrcpy already had them would double-fire input. A hook is the only place that sees a key before scrcpy does, and returning `1` from it is the honest way to say "this key is mine now".
- Alternatives: scrcpy's `--no-keyboard` plus re-implementing all typing (throws away text entry); an injected keystroke through `SendInput` after interception (same duplicate problem, plus it would re-enter our own hook); asking the user to run DroidWin elevated and reading raw input (more privilege than the feature needs).
- Evidence: `screenshots/input-01-start-screen.png` … `input-04-mapping-off-key-inert.png`; the hook log shows `key 0x4A in org.asafonov.blockbuster: mapped to tap 50% / 80% (down)` for the mapped press and `not mapped, passed to the window` with mapping off, while the game started in the first case and stayed on its start screen in the second.
- Consequences: mapping is per package, not global; it is off until the user enables it; a key held while focus leaves, the window closes, or the profile is disabled is released (`release_package`), so a finger cannot stay pressed on a game. The dispatcher serialises adb calls on one queue, so a release can never overtake its press.
- Follow-up tasks: DW-055 records the panel and its tests.

## D-016 — Triggers compose existing tools; every step reports dispatched, observed and verified
- Status: Accepted, verified on this machine
- Date: 2026-10-26
- Decision: A trigger is a name, one watched subject (one package, or the runtime), a condition and a list of steps. A step is a tool name plus arguments plus an optional expectation. Steps run through `ControlService::dispatch`, the same path a single tool call takes. Each step result carries `dispatched`, an `observation` (app running, window open, display id, and — when the expectation is a comparison — the screenshot hash), and `verified` with a one-line reason. Triggers and the workflow runner live in the agent layer, not the Android action layer, because they compose actions rather than perform them.
- Context: Step 1–3 gave DroidWin display-scoped control and keyboard mapping, but an agent could only do one thing at a time and had no way to know whether a tap had any effect. Adding a second action layer or a second MCP server would have duplicated the 25 tools that already worked.
- Alternatives: A generic automation DSL (more surface, no new capability); trigger actions that call MCP over HTTP back into the same process (a loop through the network to reach a local function); per-action verification inside every existing tool (changes 25 tool results and couples them to package state).
- Detection: `RuntimeEvents::app_event` carries what DroidWin observes about its own window host — Opened when the display was resolved, Closed when it exits or its process disappears, Crashed only when the host exits with a failure code it was not asked for. `close_window` sets a shared `closing` flag first, so a requested close is never reported as a crash. State triggers poll the display they name at their own interval with edge detection; a runtime trigger watches a Ready → stopped/unhealthy transition. Nothing infers app state from a picture or a timer.
- Lifecycle: the engine starts with the endpoint and stops with it, aborting in-flight fires, so switching automation off leaves no worker. Firing is single-flight per trigger; cancellation flows through the same `Cancel` token the MCP request owns.
- Evidence: `screenshots/mcp-01-workflow-tap-started-game.png`, `screenshots/mcp-02-trigger-fired-on-open.png`, `.droidwin/mcp-proof.log` (5/5 steps verified; trigger fired on `app_opened` with `fire_count=1`; 60 s wait cancelled in 2.28 s; dropped connection left the endpoint healthy; `windows=0`, `scrcpy=0`, adb unchanged after teardown).
- Consequences: triggers persist in schema v5 and survive a restart, but their steps are only as capable as the tools that already exist. A verification that is merely "the screen changed" says the pixels moved, not that the change was the intended one; the result says so in `verify_detail`.
- Follow-up tasks: DW-056 records the surface; a trigger UI and a live-fire entry in the Agent screen are not built yet.

### D-017 — A recorded failure is history, re-checked before it is reported
- Status: Accepted, verified on this machine
- Date: 2026-10-26
- Decision: The operation history is evidence of what happened, never a claim about what is true now. A failed launch or install problem keeps the record's own sentence, prefixes it with the time it was recorded, and adds what a fresh look at the app list says today ("Re-checked now: … is installed (1.61.0) and has no window open", or "still not installed", or "no app list to check against"). The age is part of the title, so a nine-hour-old failure cannot read as a failure happening now.
- Context: The section reported `The last launch of com.fingersoft.hillclimb failed` with the recorded sentence `com.fingersoft.hillclimb is not installed in the runtime` as a present-tense, confirmed observation, while the Applications screen one click away listed that package as installed. The badge said confirmed and the sentence was no longer true: exactly the guess-as-fact the section exists to prevent.
- Alternatives: Drop old failures (loses a real, actionable retry); demote them to suspected (they are not suspected — they are recorded facts); re-probe the package over adb on every report (a device round trip per package, which is the polling this pass removed). The app list DroidWin already caches answers the question for free.
- Rule: a diagnosis may only state as fact what was measured now; anything remembered is labelled with when it was recorded and what has changed since.
- Evidence: Runtime screen with a live lost window and the stale record showing its age (`screenshots/attention-01-lost-window.png`); the report JSON carries `Recorded 9 h ago: … Re-checked now: com.fingersoft.hillclimb is installed (1.61.0) and has no window open.`
- Consequences: the section can now say "this failed then, here is the state now", which is what makes a retry offer honest.

## New decision template

### D-XXX — Short title
- Status: Proposed / Accepted / Rejected / Superseded
- Date:
- Decision:
- Context:
- Alternatives:
- Evidence:
- Consequences:
- Follow-up tasks:
