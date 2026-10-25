# Runtime Research and Decision Record

**Status: DECIDED (2026-10-25) — Android Emulator from a user-provided SDK + scrcpy virtual displays.**
See §6 for the evaluation record and `docs/RUNTIME_PROOF.md` for the prototype evidence.

## 1. Decision to make

Select an Android execution backend that can support the product's primary workflow:

- APK installation through a supported interface.
- Programmatic launch.
- Minimal manual runtime management.
- Multiple concurrent apps if possible.
- Independent Windows desktop windows, or a technically credible path to them.
- Keyboard/mouse and clipboard integration.
- Supported file sharing.
- Graphics acceleration where possible.
- Maintainable security and licensing.

Do not select a runtime solely because it can boot Android or launch an app in a single shared emulator window.

## 2. Candidate evaluation checklist

For every candidate, record:
- Project and official repository.
- Last meaningful release/commit and maintenance signals.
- Supported Windows versions.
- x64/ARM64 support.
- Android version/API coverage.
- APK install/uninstall interfaces.
- Programmatic launch and app enumeration.
- Independent per-app window support.
- Multiple simultaneous applications.
- Input focus and keyboard mapping.
- Clipboard.
- Host file sharing.
- Graphics and audio.
- Runtime startup/shutdown and repair.
- Isolation/security model.
- Dependencies and system requirements.
- License and redistribution conditions.
- Known compatibility limitations.
- Evidence URL and date checked.
- Prototype outcome.

## 3. Mandatory prototype tests

1. Install a small, known-compatible APK.
2. Launch it programmatically.
3. Restart the manager app and verify the Android app remains installed.
4. Run two apps concurrently.
5. Determine whether each has an independent host window.
6. Move and resize each window.
7. Test keyboard, mouse, and text entry.
8. Test clipboard copy/paste.
9. Test a supported host-to-Android file operation.
10. Test an app requiring graphics acceleration.
11. Test runtime crash/restart behavior.
12. Confirm how app data is stored and removed.
13. Inspect runtime license and redistribution terms.

## 4. Current constraints

- Do not rely on Windows Subsystem for Android as the default foundation without confirming current availability and suitability.
- Do not assume emulator display bridging equals true independent app windows.
- Do not bundle proprietary Google components without authorization.
- Do not claim that an untested candidate supports a feature.
- Do not implement a custom compatibility engine as an unplanned workaround.

## 5. Decision template

### Candidate
Name and repository:

### Evidence
Sources and dates checked:

### Verified capabilities
List only tested/documented capabilities.

### Missing or uncertain capabilities
List separately.

### Security and licensing
Isolation model, license, redistribution constraints:

### Prototype results
Commands, environment, test APKs, actual results:

### Decision
Adopt / reject / keep researching:

### Consequences
Product limitations, implementation changes, risks:

### Approver
Record who approved the decision and when.

---

## 6. Evaluation record — 2026-10-25

Sources checked on this date: Microsoft's WSA support FAQ and the archived `microsoft/WSA`
announcement; vendor terms for BlueStacks, Genymotion, LDPlayer, MEmu, Nox and MuMuPlayer; scrcpy's
README, release notes and virtual-display guide; Android's emulator CLI, acceleration and multi-display
documentation; the Android SDK Licence Agreement; AOSP multi-window documentation; Mac.apk's README
and licensing.

| Candidate | Install / launch interface | Per-app host windows | Embedding / redistribution | Verdict |
| --- | --- | --- | --- | --- |
| **Android Emulator + scrcpy** | `adb install`, `am`/`monkey`; fully scriptable headless | **Yes** — scrcpy 3.0+ creates a virtual display per window and starts the package on it; verified here with two apps | SDK is development-licensed: cannot bundle or redistribute. Must be user-provided. scrcpy is Apache-2.0 | **Adopted** |
| Windows Subsystem for Android | adb | Discontinued; no Store distribution since 2025-03-05 | Unsupported, provenance risk, community builds only | Rejected |
| BlueStacks 5 | UI; adb not a supported management API | Multiple instances only | Terms limit use to personal, non-commercial | Rejected |
| Genymotion Desktop | `gmtool` + adb | Window per virtual device | Free tier personal use; no redistribution right | Rejected |
| LDPlayer / MEmu / Nox | `ldconsole` / `memuc` / multi-instance | Window per instance | Consumer EULAs: personal use, no embedding | Rejected |
| MuMuPlayer | adb documented | **Yes** (split-screen app window) | Terms forbid third-party access/redistribution without agreement | Rejected (would need a commercial agreement) |
| Google Play Games (developer emulator) | adb | No general APK runtime | Limited to authorised game development/testing | Rejected |
| Waydroid | `waydroid app install/launch` | Linux desktop integration | GPLv3 with commercial option; no Windows host | Rejected |
| redroid | adb inside a container | Streamed display | Needs a customised Linux kernel; not supported under Docker Desktop | Rejected |
| Android-x86 / Bliss OS in a VM | adb | VM window only | Component-level licensing caveats; no app–window bridge | Rejected |
| Mac.apk (reference) | n/a | Native window per app | AGPLv3, Apple Silicon only, reimplements the Android framework | Reference for product behaviour only (D-006) |

### Decision

Adopt **Android Emulator (user-provided SDK) + scrcpy 5.0.1 virtual displays** as the MVP backend.

- Verified: install, programmatic launch, two concurrent apps in independent host windows, move/resize
  through the OS, independent close, state surviving a manager restart, complete shutdown.
- Unknown, therefore not claimed: clipboard sharing, file drop onto app windows, audio forwarding,
  per-app isolation, ARM-only packages, Play-services dependent apps.
- Recorded as D-004 in `DECISIONS_LOG.md`; evidence in `docs/RUNTIME_PROOF.md`.

### Consequences carried into the product

1. DroidWin must never bundle or fetch the Android SDK; first run explains which SDK packages are
   missing and why DroidWin does not install them for you.
2. `scrcpy` is the only redistributed runtime component and ships with its Apache-2.0 notice.
3. App windows are streamed virtual displays: Android's back key works inside the window, but there is
   no host-side toolbar yet. Adding one means either a host-side key bridge or embedding the
   scrcpy-server video stream in a DroidWin window — both are follow-up work.
4. Compatibility claims stay per-package and per-machine; `docs/TEST_PLAN.md` gains the matrix entries
   as more APKs are tried.

### Still open

- Verify clipboard, file drop and audio, then flip those capability flags (D-005).
- Test a package that refuses secondary displays and confirm the failure message is actionable.
- Test 100/125/150 % DPI scaling and multi-monitor placement.
