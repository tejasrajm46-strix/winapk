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
- `.apk` activation works: cold start from argv queues the path, a second launch forwards to the running window (verified from the release binary and its log).
- Keeping app data was verified with `cmd package uninstall -k` (data still listed by `pm list packages -u`).
- The emulator must boot with `-no-snapshot`: quickboot was reverting installs between runs (D-008).
- A 2012-era API-17 app installs with the adb low-target flag. An earlier run showed a black virtual display while mirroring worked (D-011); on a phone-sized surface it now renders, so the blank is recorded as unconfirmed rather than as a rendering defect. Installed legacy packages are labelled from their manifest (`target_sdk`, schema v2).
- Display geometry is per app (DW-046): `runtime::display` builds a 1080×2400 / 2400×1080 @420 dpi surface (411×914 dp) from the launchable activity's `android:screenOrientation`, records the orientation per package (schema v3), and exposes `EmulatorRuntime::geometry_of` as the one source of truth for input coordinates and screenshots. Block'Buster (landscape) now renders and plays in its own window; Accelerace (portrait) is unchanged in shape.
- The old 540×1170 surface was 205×445 dp — less than half a phone — which is why a landscape layout was clipped by its own display, not by the window.
- Launch success requires the package's own process (`pidof`), not just an open scrcpy window.
- Cold boot of `DroidWin_API36` takes roughly 25–45 s; a warm run reuses the running device.
- Stopping the runtime requires stopping the emulator's qemu child process, not just the launcher (fixed with `adb emu kill` plus a process-tree kill).

## Repository map
- `src/` — React UI. `backend.ts` is the only module that invokes Rust commands.
- `src-tauri/src/domain/` — typed models, error codes, operation stage machine.
- `src-tauri/src/apk/` — archive guards and a binary AndroidManifest (AXML) reader.
- `src-tauri/src/runtime/` — SDK locator, adb wrapper, headless emulator, scrcpy windows.
- `src-tauri/src/store/` — SQLite schema and migrations.
- `src-tauri/examples/proof.rs` — Gate A/B harness (`cargo run --example proof`); reuses the production modules and is not bundled in the app.
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
- Built since the first pass: `.apk` association (packaged; cold start and forwarding verified), keep/delete app-data choice, NSIS installer with a bundled scrcpy window host, old-target-SDK opt-in, durable installs across runtime restarts, status polling.
- Per-app keyboard mapping (D-015, DW-055): a Controls dialog binds a physical key to an Android key event or a touch point on that app's display, stored per package in schema v4 and dispatched as display-scoped adb input. Proven against Block'Buster: a mapped key started the game, and with mapping switched off the same key did nothing (`screenshots/input-0*.png`).
- Agent automation (D-016, DW-056): the MCP endpoint offers 32 tools. Triggers (explicit, app lifecycle, state-based, runtime failure) and multi-step workflows run through the same action layer, and every step reports whether it was dispatched, what was observed, and whether that observation verified it. Proven over HTTP JSON-RPC, including mid-run cancellation and a dropped client, with no leftover windows or window hosts.
- Attention Needed, verified on the emulator: killing one app's window host while two apps ran produced one confirmed blocking problem (`… lost its window while still running`, exit code 1, pidof still alive, no crash block) and "Reopen this window" brought that display back in place — the other app's host pid was unchanged and the section dropped back to notice-only. Recorded failures are now re-checked against the current app list instead of being repeated as present-tense facts (D-017).
- Keyboard mapping on Subway Surfers: `J` → `KEYCODE_BACK` dismissed the seasonal popup and `K` → tap at 96 % / 2 % opened the game's Settings dialog; with the profile switched off the same `J` press changed nothing (`screenshots/input-05..07`). A first launch right after a cold boot was recorded as `started and then crashed` with no crash block and succeeded on the retry — that cold-boot path is the one open stability question.
- Not built yet: automatic SDK provisioning, audio, clipboard/file-drop verification, blank-virtual-display detection, DPI matrix, clean-machine installer test, compatibility matrix, code signing.

## Update protocol
When new durable facts or decisions are established:
1. Add the date and decision to `DECISIONS_LOG.md`.
2. Update this file if the project baseline changed.
3. Update `TASKS.md` to reflect the new state.
4. Do not preserve contradicted assumptions as active requirements.
