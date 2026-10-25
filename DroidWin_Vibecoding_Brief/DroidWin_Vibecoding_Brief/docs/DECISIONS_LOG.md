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
