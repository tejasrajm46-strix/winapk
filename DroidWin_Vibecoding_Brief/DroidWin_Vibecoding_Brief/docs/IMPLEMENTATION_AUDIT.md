# Implementation Audit — DroidWin

Audited on Windows 11 (build 26200), 25 October 2026. This document records what was **executed**
during this audit, not what the code appears to do. Anything that was not run stays out of
"verified".

Status vocabulary:

| Status | Meaning |
| --- | --- |
| **VERIFIED COMPLETE** | Implemented and exercised successfully in this environment, with the command or observation recorded below. |
| **IMPLEMENTED, NOT VERIFIED** | Code exists; this audit did not exercise it end to end. |
| **PARTIALLY COMPLETE** | Works, but a required part is missing or was only observed in one configuration. |
| **NOT IMPLEMENTED** | Absent. |
| **BROKEN** | Exists and fails its acceptance criteria. |
| **BLOCKED** | An external constraint (licence, platform policy, hardware, credentials) prevents it. |
| **NOT APPLICABLE** | Removed or replaced by a recorded decision. |

## 1. Environment actually used

| Component | Value | How |
| --- | --- | --- |
| OS | Windows 11 Home, 10.0.26200 | `systeminfo` |
| Rust | 1.99.1 (2026-10-13), MSVC | `rustc --version` |
| Node / pnpm | 24.19.0 / 11.23.0 | `node --version`, `pnpm --version` |
| Tauri CLI | 2.12.4 | `tauri --version` |
| adb | 1.0.41 / 37.0.1 | `adb version` |
| Android Emulator | 37.2.12.0 | `emulator -version` |
| System image | `system-images;android-36;google_apis;x86_64` | `sdkmanager --list`, runtime probe |
| AVD | `DroidWin_API36` (created by DroidWin) | probe + `emulator -list-avds` |
| Acceleration | WHPX usable (`accel: 0 — WHPX(10.0.26200) is installed and usable`) | `emulator -accel-check` via probe |
| Window host | scrcpy 5.0.1, bundled Apache-2.0 build | `scrcpy --version` |

The Android SDK, emulator and image are **user-installed**; DroidWin does not redistribute them.

## 2. Build, test and packaging checks

Each command below was run from the repository root during this audit.

| Check | Command | Result |
| --- | --- | --- |
| Formatting | `cargo fmt --check` (src-tauri) | exit 0 |
| Lints | `cargo clippy --all-targets -- -D warnings` | exit 0, no warnings |
| Rust tests | `cargo test` | **42 passed, 0 failed** |
| TypeScript | `tsc --noEmit` | exit 0 |
| Frontend tests | `vitest run` | **22 passed, 0 failed**, 2 files |
| Frontend build | `vite build` (part of `pnpm build`) | built, 266 kB main chunk |
| Release build | `pnpm tauri build` | `droidwin.exe` 14.1 MB, release profile |
| Installer | `pnpm tauri build` | `DroidWin_0.1.0_x64-setup.exe`, 11.63 MiB |
| File association packaged | grep of generated `target/release/nsis/x64/installer.nsi` | `APP_ASSOCIATE "apk" "Android package" …` plus matching `APP_UNASSOCIATE` |
| Window host packaged | same generated script | 34 scrcpy entries (exe, DLLs, `scrcpy-server`, `LICENSE.txt`) |
| Proof harness not shipped | same generated script | 0 matches for `proof.exe` |

Defects found and fixed by running these checks:

1. **`tauri build` failed outright** — `failed to find main binary`. Two `[[bin]]` targets suppressed
   autodiscovery of `src/main.rs`. Fixed with an explicit `droidwin` binary and
   `default-run = "droidwin"` in `src-tauri/Cargo.toml`.
2. **The installer shipped the proof harness** (`proof.exe`), a development tool that drives the
   emulator and kills process trees. Moved to `src-tauri/examples/proof.rs`; it is now run with
   `cargo run --example proof`.
3. **The installer contained no window host.** A clean install would report "scrcpy missing;
   reinstall DroidWin" forever. The Apache-2.0 scrcpy build is now a Tauri resource
   (`src-tauri/resources/scrcpy`), copied once into the data directory by
   `provision_window_host()` at startup.

The installer was **built but not executed** on this machine: running it would install DroidWin and
claim the `.apk` association in the current user's registry, which is the user's decision. The
generated NSIS script is the evidence that both the association and the window host are inside it.
A clean-machine install, the registry write, and uninstaller behaviour remain **UNVERIFIED**.

## 3. Requirement status

Requirement IDs follow `docs/PRD.md` (FR-xx) and the MVP gates in `docs/TASKS.md`.

### APK entry points and inspection

| Req | Status | Evidence |
| --- | --- | --- |
| FR-01 Browse for an APK | VERIFIED COMPLETE | Native dialog wired in `backend.pickApk`; used to inspect F-Droid and the supplied test package from the release build. |
| FR-01 Drag and drop | IMPLEMENTED, NOT VERIFIED | `onFileDrop` resolves absolute paths; not exercised with a physical drag in this audit. |
| FR-01 Open an `.apk` from Explorer (cold start) | VERIFIED COMPLETE | Release exe started with the package path: log `started with 1 package path(s) on the command line`, then `activation: 1 package path(s) queued`; the running window showed the package's inspection screen (screenshot `screenshots/droidwin-installer.png`). |
| FR-01 Forward a second open to the running instance | VERIFIED COMPLETE | Second process exited 0 and the first logged `forwarded 1 package path(s) to the running window`. |
| FR-01 Association registered by the installer | PARTIALLY COMPLETE | Compiled into the NSIS script (see §2); the installer was not run, so the registry write and Explorer double-click were not observed. |
| FR-02 Package identity, label, versions, SDK levels, ABIs, size, icon | VERIFIED COMPLETE | Real runs: `org.fdroid.fdroid` (four ABIs, min SDK 24), `org.schabi.newpipe`, and `dudeofx.eval` (label "Expression Evaluator", min 10, target 17, no native libs, v1 signature only). |
| FR-02 Signature presence is not reported as validity | VERIFIED COMPLETE | UI reads "APK signing block present (v2+)"/"v1 signature present" with an explicit note; a unit test asserts the wording. |
| FR-02 Unavailable metadata is named | VERIFIED COMPLETE | Resource-reference label, missing version, min/target SDK are reported as "Not declared"/"name unavailable" plus reasons; unit and DOM tests. |
| FR-02 Hostile archive handling | VERIFIED COMPLETE | 8 new tests cover missing path, directory, wrong extension, empty file, non-zip, missing manifest, garbage manifest, and a Unicode path with spaces. Guards: 4 GiB cap, 200 000 entries, bounds-checked AXML, 8 MiB manifest cap. |
| FR-02 No code execution during inspection | VERIFIED COMPLETE | Inspection reads the ZIP directory and `AndroidManifest.xml` only; no dex load, no install during inspect. |

### Runtime

| Req | Status | Evidence |
| --- | --- | --- |
| FR-03 Detect SDK, adb, emulator, image, AVD, scrcpy, acceleration | VERIFIED COMPLETE | Probe output above; missing pieces are listed with the exact `sdkmanager` package. |
| FR-03 Boot the runtime and wait for real readiness | VERIFIED COMPLETE | `sys.boot_completed` polled; measured 20.3 s in the audit run, 24.5 s in the recorded full run. |
| FR-03 Stop the runtime without orphans | VERIFIED COMPLETE | `adb emu kill` + process-tree kill; earlier audit showed 0 emulator/qemu/scrcpy processes after shutdown. |
| FR-03 Runtime state is authoritative | VERIFIED COMPLETE | Library is read from `pm list packages -3`; cached labels only decorate rows. Status strip re-probes every 8 s so a booting emulator cannot leave a stale "stopped". |
| FR-03 Guided setup for missing components | PARTIALLY COMPLETE | Runtime screen lists each missing package and the command; it does not offer to run the SDK manager or accept licences, by design. |
| FR-04 Install and verify | VERIFIED COMPLETE | `adb install -r` then `pm path`; F-Droid installed in 5.16 s and was re-read from the runtime. |
| FR-04 Update an installed package | IMPLEMENTED, NOT VERIFIED | Same path with `-r` and a signature-conflict error mapping; not exercised with a second version in this audit. |
| FR-04 Failure messages name the Android cause | VERIFIED COMPLETE | Mapped codes for signature conflict, storage, newer SDK, missing ABI, parse failure, already exists, and deprecated target SDK, each with a unit test. |
| FR-04 Install packages targeting an old SDK | VERIFIED COMPLETE | The supplied package targets API 17; Android 16 refused it, and with the explicit "Install anyway" option (adb `--bypass-low-target-sdk-block`) the install returned `Success` and the process started. The flag is only offered when the manifest says `targetSdk < 23`, and is off by default. |
| FR-05 Library lists installed apps | VERIFIED COMPLETE | Real list after installs; system packages excluded. |
| FR-05 Search / empty / error states | VERIFIED COMPLETE | DOM tests; stopped-runtime state offers "Start runtime". |
| FR-05 Uninstall with a keep-or-delete choice | VERIFIED COMPLETE (keep-data mechanism) | The dialog offers keep/delete with Escape, focus and a test for each. Live check: `cmd package uninstall -k org.fdroid.fdroid` → `Success`; package gone from `pm list packages -3`, still listed by `pm list packages -u -3` (data retained). The runtime now uses that command because `adb uninstall -k` is refused by modern Android — a defect found by this audit and fixed. |
| FR-05 Delete app data | IMPLEMENTED, NOT VERIFIED | Plain `pm uninstall` path; no live deletion of a package's data was performed. |
| FR-06 Launch an app | VERIFIED COMPLETE | scrcpy `--new-display --start-app`; F-Droid window opened with real content. |
| FR-06 Independent window per app | VERIFIED COMPLETE | Recorded full run: two distinct HWNDs (`DroidWin — F-Droid`, `DroidWin — NewPipe`), OS geometry read back. |
| FR-06 Move and resize independently | VERIFIED COMPLETE | `MoveWindow` + `GetWindowRect`: `46,23 291x637 → 120,140 520x760`. |
| FR-06 Closing one window leaves the other | VERIFIED COMPLETE | Closed F-Droid's window, the second window and the installation survived. |
| FR-06 Per-app compatibility of virtual displays | PARTIALLY COMPLETE (limitation) | The supplied API-17 app renders **black** on a virtual display while its mirrored main-display window shows the working app (screenshots). DroidWin cannot yet detect a blank virtual display; see §5. |
| FR-06 Host-side back control | NOT IMPLEMENTED | Stated in the UI limitations: Android's own back key works, no host-side button. |
| FR-07 Clipboard / file drop / audio | IMPLEMENTED, NOT VERIFIED | Report as "Not verified" in the capability list; not exercised. |
| FR-08 Persistence across app restart | VERIFIED COMPLETE | Fresh manager read both packages from the runtime. |
| FR-08 Persistence across runtime restart | VERIFIED COMPLETE | Found and fixed: the emulator was launched with `-no-snapshot-save`, so quickboot restored an older snapshot and **installs disappeared on the next start**. Now `-no-snapshot`; after `emu kill` + cold boot, `dudeofx.eval` was still installed. |
| FR-08 SQLite migrations and history | VERIFIED COMPLETE | Idempotent-open, label retention, newest-first history tests; DB file created in the data directory. |
| FR-09 Diagnostics and log folder | VERIFIED COMPLETE | Daily tracing log; "Open log folder" opens Explorer at the log directory; paths in log messages are reduced to file names. |
| FR-10 Settings | PARTIALLY COMPLETE | Settings screen shows data/log paths, runtime target and licence facts; there are no editable preferences, and none are faked. |

### UI / UX

| Req | Status | Evidence |
| --- | --- | --- |
| Six screens exist | VERIFIED COMPLETE | Overview, Installer, Applications, Runtime, Activity, Settings; DOM tests plus the running release app. |
| Kage Zen tokens and offline fonts | VERIFIED COMPLETE | Tokens file mirrors the supplied design; Instrument Serif + Onest bundled with OFL licence. |
| No fabricated telemetry or fake progress | VERIFIED COMPLETE | Every screen renders backend values or an explicit empty/unavailable state; tests assert the absence of "signature verified", invented SDK values and placeholder history. |
| Keyboard and focus | PARTIALLY COMPLETE | Dialogs trap focus and Escape closes; sidebar items are buttons. No full keyboard walk of all six screens was performed. |
| Visual QA at 1280×820 | PARTIALLY COMPLETE | Release window captured at 1280×820 (`screenshots/droidwin-overview.png`); no DPI (125/150 %) or minimum-size matrix. |
| Explain disabled controls | VERIFIED COMPLETE | "Start runtime" is disabled with the missing-pieces list when the SDK is incomplete; capability rows say "Not verified". |

### Security, licensing, distribution

| Req | Status | Evidence |
| --- | --- | --- |
| Argument-array process spawning | VERIFIED COMPLETE | `adb`/`scrcpy`/`emulator` all use `Command::args`; no shell string concatenation. |
| Bounded parsing of untrusted archives | VERIFIED COMPLETE | Caps and tests in §3; AXML parser rejects truncated/absurd chunks. |
| Tauri capability surface | VERIFIED COMPLETE | `core:default` + `dialog:default` only; no shell or filesystem plugin. |
| Log redaction | VERIFIED COMPLETE | `diag` keeps file names and drops directory paths. |
| SQL parameterisation | VERIFIED COMPLETE | All statements use `params!`. |
| `adb uninstall -k` → package-manager command | VERIFIED COMPLETE | Live: the legacy form is refused by Android 16; the replacement returns `Success` and data is retained. |
| Dependency licensing | PARTIALLY COMPLETE | scrcpy Apache-2.0 licence ships in `src-tauri/resources/scrcpy/LICENSE.txt`; fonts OFL. A full transitive licence inventory was not produced. |
| Code signing | NOT APPLICABLE (unsigned) | Installer is unsigned; Windows SmartScreen will warn. Reported as a release limitation. |
| Android SDK/emulator redistribution | VERIFIED COMPLETE (policy) | Not bundled; located from the user's SDK. |

## 4. Test suite shape

Rust (42): AXML reader (6), archive guards (8), operation state machine (6), store migrations and
history (3), adb argument/timeout behaviour (4), scrcpy argument construction (1), SDK discovery (2),
install-failure mapping (6), `.apk` argument parsing (5), plus supporting tests.

Frontend (22): honest runtime states, disabled start, empty library, unresolved labels, signature
wording, stage rendering, running state from the window registry, stopped-runtime recovery, verified
capability counts, empty activity, uninstall keep/delete/cancel/Escape/package-disappears/compact
rows, and the old-target-SDK option.

Not covered: any test that needs a live emulator is in the `proof` example rather than the test
suite, so `cargo test` stays hermetic and fast.

## 5. Remaining work, in priority order

1. **Detect a blank virtual display.** The supplied 2012-era app launches on display 3, is focused,
   owns a surface — and scrcpy renders black, while the same app mirrored from display 0 works. A
   launch currently counts as successful on process/display facts alone. Needs a rendering probe,
   or a documented "known blank display" warning for `targetSdk < 23`, or a mirror-mode fallback.
2. **Install and uninstall the built installer** on a disposable machine/profile to verify the
   `.apk` association, uninstaller cleanup, and first-run scrcpy provisioning.
3. **DPI matrix** at 100/125/150 % and the minimum window size; one visual pass per screen.
4. **Clipboard, file drop, audio**: verify or keep reported as unsupported.
5. **Update flow** with a real second version, including the signature-conflict path.
6. **Transitive licence inventory** and a signed installer.

## 6. Verified limitation statement (product copy)

> DroidWin runs each Android app in its own scrcpy window backed by a virtual Android display. Apps
> share one Android user, audio is not forwarded, and a package whose manifest targets an Android
> version below API 23 needs an explicit testing flag and may render a blank window on a virtual
> display.
