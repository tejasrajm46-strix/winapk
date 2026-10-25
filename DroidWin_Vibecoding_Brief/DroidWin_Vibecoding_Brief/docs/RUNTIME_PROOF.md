# Runtime Feasibility Proof — Gate A and Gate B

**Date:** 2026-10-25
**Verdict:** Gate A **PASS**, Gate B **PASS** on the machine below.
**Harness:** `src-tauri/src/bin/proof.rs` — it calls the same `apk`, `runtime` and `store` modules the
product uses, so a pass is evidence about the product rather than about a demo.

## 1. Environment

| Item | Value |
| --- | --- |
| Host | Windows 11 build 26200, x64, 24 logical cores |
| Virtualisation | Windows hypervisor present; emulator reports `WHPX(10.0.26200) is installed and usable` |
| Android SDK | `C:\Users\<user>\AppData\Local\Android\Sdk` (user-provided) |
| Emulator | SDK `emulator` package |
| System image | `system-images;android-36;google_apis;x86_64` |
| AVD | `DroidWin_API36`, auto-created by DroidWin via `avdmanager` (device profile `pixel_6`), booted headless |
| Window host | scrcpy 5.0.1 (win64, Apache-2.0), bundled into the app data directory |
| Android API at runtime | 36 |

## 2. What was exercised, in order

1. Probe SDK, emulator, system image, scrcpy and acceleration.
2. Create the AVD if absent, spawn the emulator headless (`-no-window -no-audio -no-boot-anim -gpu auto`), wait for `sys.boot_completed`.
3. Inspect two real FOSS APKs (archive guard → binary manifest parse → `aapt2` enrichment for label/icon).
4. `adb install -r` each package, then verify with `pm path` and re-read `pm list packages -3 --show-versioncode`.
5. Launch each package with `scrcpy --serial … --new-display=540x1170/420 --start-app=<pkg> --no-vd-system-decorations --window-title=… --no-audio`.
6. Read the desktop's real windows from the OS (PowerShell `Get-Process scrcpy` → `MainWindowHandle`, `MainWindowTitle`).
7. Move and resize one window through `user32!MoveWindow`, then read the rectangle back with `GetWindowRect`.
8. Close one window through the adapter; re-enumerate the desktop.
9. Open a second runtime instance and store against the same state and confirm the packages are still installed.
10. Shut the runtime down and confirm no emulator, qemu or scrcpy process survives.

## 3. Results

```
[pass] environment — SDK, emulator, image and scrcpy found
[pass] runtime ready — 24554 ms, API 36        (cold boot; emulator logged "booted after 24.3s")
[pass] inspect — fdroid.apk → org.fdroid.fdroid (label "F-Droid", min SDK 24, abis arm64-v8a/armeabi-v7a/x86_64/x86)
[pass] install — 7201 ms
[pass] inspect — newpipe.apk → org.schabi.newpipe (label "NewPipe", min SDK 23, abis arm64-v8a/armeabi-v7a/x86_64/x86)
[pass] install — 3449 ms
[pass] enumerate — 2 listed, 2 installed by this run
[pass] independent windows — 2 windows, 2 distinct titles
[pass] move/resize — 70,47 291x637 → 120,140 520x760
[pass] close isolation — 1 window remains
[pass] persists across manager restart — 2 listed

host windows:
  hwnd 1116944 · "DroidWin — F-Droid" · pid …
  hwnd  983760 · "DroidWin — NewPipe" · pid …

Gate A (install + launch): PASS
Gate B (independent windows): PASS
```

After the run: `tasklist` reports **0** emulator, qemu and scrcpy processes.

The full JSON report is written next to the app's data (`$DROIDWIN_HOME/proof-report.json`).

## 4. What this proves — and what it does not

Proven:

- APK inspection works against real, signed packages without executing them.
- `adb install` through the adapter installs and verifies real packages, twice, in seconds.
- Two Android apps can run at the same time, each in its **own** Win32 window with its own title.
- Those windows are ordinary desktop windows: the OS moved and resized one and reported the new
  rectangle, so they are not a mock or a screenshot.
- Closing one app window does not affect the other.
- Installed state lives in the runtime, not in the manager: a fresh manager instance still lists both.
- The runtime can be stopped completely (launcher, qemu child and window host).

Not proven here, and therefore not claimed in the product:

- Clipboard sharing, file drop onto an app window, and audio forwarding.
- Per-app isolation: every app shares one Android user.
- Apps that refuse secondary displays. `scrcpy --start-app` placed both test apps correctly; an app
  that ignores virtual displays will fail to open and DroidWin reports the failure rather than
  showing an empty window.
- ARM-only packages on the x86_64 image; Google Play services dependent apps.
- DPI scaling behaviour across 100/125/150 %, resume after reboot, and the installer lifecycle.

## 5. Licence notes

| Component | Licence / constraint |
| --- | --- |
| Android SDK and Emulator | Android SDK Licence — development use only, clause 3.4 forbids copying/redistributing the SDK. DroidWin **requires a user-provided SDK** and never bundles or downloads it. |
| scrcpy 5.0.1 | Apache-2.0 — redistribution permitted with notice; licence text ships with the binary. |
| Instrument Serif, Onest | SIL OFL 1.1 — bundled with `OFL.txt`. |
| Test APKs | F-Droid (GPLv3) and NewPipe (GPLv3), downloaded for local testing only and excluded from version control. |

## 6. How to reproduce

```bash
# once: emulator + system image into the SDK you already have
sdkmanager "emulator" "system-images;android-36;google_apis;x86_64"

# scrcpy 5.0.1 win64 into $DROIDWIN_HOME/runtime/bin/scrcpy/
# two APKs into .droidwin/testdata/

cargo run --manifest-path src-tauri/Cargo.toml --bin proof
```
