# Project Memory — DroidWin

This file stores durable project context for future coding sessions and agents. Update it when a decision or verified fact changes.

## Product identity
- Working name: DroidWin.
- Platform: Windows desktop.
- Product goal: make compatible Android APK installation and launch feel simple and native.
- Primary workflow: double-click APK → inspect → install/update → launch the real Android app in an independently managed desktop window.
- Users should not need to manually operate an emulator or issue terminal commands for ordinary workflows.

## Design source of truth
The user supplied `stitch_droidwin_android_desktop_engine.zip`. It contains:
- `droidwin_kage_zen_edition_home/`
- `droidwin_apk_installer_kage_zen_edition/`
- `droidwin_applications_library_kage_zen_edition/`
- `droidwin_runtime_management_kage_zen_edition/`
- `kyoto_zen/DESIGN.md`

Use these screens and the Kyoto Zen design description as the design inspiration. Preserve the restrained dark technical aesthetic, vermilion accent, sage/stone secondary colors, editorial serif headings, readable sans-serif UI text, thin borders, and dense but legible panels.

The reference includes example WSA labels, sample apps, synthetic metrics, and example logs. Those are visual placeholders, not product facts. Do not ship them as live status.

## Current architecture stance
- Desktop stack (D-003): Rust + Tauri 2, React + TypeScript + hand-written CSS, SQLite via `rusqlite`. The earlier C#/.NET + WinUI 3 preference is superseded.
- Runtime backend (D-004): Android Emulator from a **user-provided** Android SDK, booted headless, plus scrcpy virtual displays — one window per app.
- Use a replaceable runtime adapter: `src-tauri/src/runtime/` is the only place that knows adb, the emulator or scrcpy.
- Windows Subsystem for Android is discontinued and is not used.
- Independent app windows are verified on this machine (`docs/RUNTIME_PROOF.md`), not assumed.
- Capabilities are measured per machine (D-005); clipboard sharing, file drop and audio are reported as not verified.
- A custom Android compatibility layer remains out of scope; Mac.apk is a product reference only (D-006).

## Verified facts (2026-10-25)
- Host: Windows 11 build 26200, x64, 24 cores, Hypervisor present; WHPX acceleration reported usable by the emulator.
- Toolchain: Rust 1.99.1 (MSVC), MSVC 14.44, WebView2 155, Node 24.19, pnpm 11.23, JDK 25.
- SDK: `%LOCALAPPDATA%\Android\Sdk` with platform-tools, emulator, `system-images;android-36;google_apis;x86_64`.
- scrcpy 5.0.1 (Apache-2.0) bundled into the app data directory; `--new-display` + `--start-app` work against the emulator.
- Gate A and Gate B pass: two FOSS APKs installed and launched into two distinct HWNDs, moved/resized, one closed independently, both still installed after a manager restart.
- Cold boot of `DroidWin_API36` takes roughly 25–45 s; a warm run reuses the running device.
- Stopping the runtime requires stopping the emulator's qemu child process, not just the launcher (fixed with `adb emu kill` plus a process-tree kill).

## Repository map
- `src/` — React UI. `backend.ts` is the only module that invokes Rust commands.
- `src-tauri/src/domain/` — typed models, error codes, operation stage machine.
- `src-tauri/src/apk/` — archive guards and a binary AndroidManifest (AXML) reader.
- `src-tauri/src/runtime/` — SDK locator, adb wrapper, headless emulator, scrcpy windows.
- `src-tauri/src/store/` — SQLite schema and migrations.
- `src-tauri/src/bin/proof.rs` — Gate A/B harness; reuses the production modules.
- Build/verify commands are listed in the repository `README.md`.

## Core requirements
- APK file association/double-click handling.
- Drag-and-drop and file picker.
- Package inspection without executing APK code.
- Runtime readiness/provisioning.
- Install/update/uninstall.
- App library and persistent metadata.
- Launch real Android apps.
- Independent app windows, if supported by selected backend.
- Keyboard/mouse/clipboard and supported file integration.
- Honest status, meaningful diagnostics, recovery.
- Security boundaries and explicit data deletion.
- Compatibility claims backed by tests.

## Agent working model
- One agent/owner per module or file set.
- Architecture and interface changes require review.
- Do not parallel-edit the same core files without coordination.
- Each task needs acceptance criteria and tests.
- Report actual test/build outcomes; never claim unverified success.

## Current status
- Design archive extracted and read: `kyoto_zen/DESIGN.md` provides the authoritative tokens; the four screen HTML files provide layout intent. Their WSA labels, percentages and telemetry are placeholders and are not implemented.
- Product and architecture documents drafted, then updated with D-003…D-006.
- Runtime feasibility resolved and proven on this machine (see `docs/RUNTIME_PROOF.md`).
- Implemented and verified: workspace/git, Rust core (domain, AXML inspector, SQLite store, diagnostics), emulator+scrcpy adapter, proof harness, Tauri shell, and the Overview / Installer / Applications / Runtime / Activity / Settings screens on real backend state.
- Not built yet: `.apk` file association, in-UI app-data deletion, automatic SDK provisioning, audio, clipboard/file-drop verification, packaging, compatibility matrix.

## Update protocol
When new durable facts or decisions are established:
1. Add the date and decision to `DECISIONS_LOG.md`.
2. Update this file if the project baseline changed.
3. Update `TASKS.md` to reflect the new state.
4. Do not preserve contradicted assumptions as active requirements.
