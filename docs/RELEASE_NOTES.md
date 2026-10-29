# DroidWin 0.1.0 — release notes (29 October 2026)

DroidWin runs Android apps from APKs in their own Windows desktop windows. This
release adds the piece that made it usable by someone who is not an Android
developer: **DroidWin installs its own Android runtime on first run**.

## New in this release

- **First-run setup screen.** On a PC with no Android SDK, the app opens a setup
  screen instead of the shell. It lists every component it checked, what is
  already installed, and what is missing; nothing is downloaded until you press
  **Install Android runtime** and accept Google's Android SDK licence terms.
- **Runtime installed by DroidWin.** Command-line tools (verified against the
  SHA-1 Google publishes), a JRE only when the PC has none, platform-tools, the
  emulator, the Android 36 `google_apis;x86_64` system image, and the
  `DroidWin_API36` virtual device — about 2.3 GB, once, into a directory DroidWin
  owns. Android Studio is not required, and an SDK you already have is used as it
  is and never modified.
- **Real progress and honest states.** Percentages only where a transfer length is
  known; an indeterminate bar for the SDK manager. Cancel, retry, choose a folder,
  use DroidWin's folder, or check again. "Ready" means adb answered for API 36, not
  that files exist.
- **Settings → Android runtime.** Change the SDK directory, or keep the emulator
  running after DroidWin closes (default: stop the one DroidWin started).
- **Ownership that survives a crash.** A pid file in the data directory lets a
  relaunch recognise an emulator an earlier DroidWin run left behind and stop it
  on close, while an emulator you started yourself is left alone.
- **CI build.** `.github/workflows/windows.yml` builds the app and NSIS installer
  on `windows-latest` and uploads both; it needs no Android SDK.

## Requirements

- Windows 11 x64 (Windows on ARM is not supported: the image is x86_64).
- WebView2 runtime (present on Windows 11).
- ~6 GB free disk space and an internet connection for the one-time setup.
- Hardware virtualisation (WHPX/Hyper-V) for a usable emulator; without it the app
  installs but the emulator is slow and the Runtime screen says acceleration is
  unavailable.

## Install

1. Run `DroidWin_0.1.0_x64-setup.exe` (Start Menu shortcut, uninstaller included).
2. Launch DroidWin.
3. Press **Install Android runtime**, tick the licence checkbox, wait for
   "Android runtime is installed".
4. Drop an APK on the window (or **Browse APK**) to install and launch it.

## Verification

| Check | Result |
| --- | --- |
| `cargo test --lib` | 251 passed |
| `pnpm test` | 99 passed (10 files) |
| `cargo fmt --check` | clean |
| `pnpm tauri build` | installer built (12.87 MiB) |
| Fresh install into an empty directory (no SDK, no JRE, no AVD) | downloaded, verified, installed, AVD created, runtime Ready · API 36 |
| Reuse of an existing SDK | setup screen skipped, no download |
| First-run screen against an empty SDK | shown, install disabled until consent |
| After install | shell opens with RUNTIME READY · API 36 |
| Close with default preference | emulator DroidWin started stops |
| Close after a simulated crash | emulator adopted and stopped on close |
| Keep-running preference | emulator survives close |
| Live suite (`node scripts/live.mjs`) | see `docs/RUNTIME_SETUP.md` §8 |

Not verified: a clean Windows VM (the fresh-install test used a pristine
directory on this machine), network failure and out-of-disk-space handling
end-to-end, and Windows 10. Details and the remaining limits are in
[docs/RUNTIME_SETUP.md](RUNTIME_SETUP.md).

## Known limits

- The runtime is downloaded, never bundled; there is no offline installer.
- One Android user profile, one shared emulator, one virtual display per app.
- ARM-only packages cannot run; apps needing Google Play services usually fail.
- Untrusted APKs are contained against host loopback (see
  [HOST_ISOLATION.md](HOST_ISOLATION.md)) but still belong in a disposable
  Windows account or a VM.
