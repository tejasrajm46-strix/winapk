# DroidWin

Run Android apps from APKs in their own Windows desktop windows.

DroidWin inspects a package, installs it into an Android runtime, and opens **one desktop window
per app**. It is a manager, not an emulator: the Android runtime is the Android Emulator from the
Android SDK that **you** install, and each app window is a `scrcpy` window attached to its own
virtual Android display.

> **Status: early.** The install → launch → window workflow is proven on this machine (see
> [docs/RUNTIME_PROOF.md](DroidWin_Vibecoding_Brief/DroidWin_Vibecoding_Brief/docs/RUNTIME_PROOF.md)).
> Compatibility is not universal, audio is not forwarded, and `.apk` file association is not built
> yet. Nothing in the UI shows a state the backend did not report.

## Why the runtime is what it is

- **Windows Subsystem for Android is discontinued** (no Store distribution since 5 March 2025), so it
  is not a foundation. Community builds of it are unsupported.
- **Commercial emulators** (BlueStacks, Genymotion, LDPlayer, MEmu, Nox, MuMu) license themselves for
  personal use; none of them may be embedded in a product like this.
- **Android Emulator + scrcpy** is the open path that actually delivers per-app windows: scrcpy
  creates a virtual Android display per window and launches the package onto it.
- **The Android SDK is development-licensed and must not be redistributed**, so DroidWin locates the
  SDK you installed and tells you which packages are missing instead of bundling Google's emulator.

## Requirements

| Need | Why |
| --- | --- |
| Windows 11 x64 | target platform |
| Visual Studio 2022 Build Tools (C++ workload) | Rust MSVC toolchain |
| WebView2 runtime | Tauri shell (present on Windows 11 by default) |
| Rust (stable, MSVC) + Node 24 + pnpm | build the app |
| Android SDK with `platform-tools`, `emulator`, and an x86_64 system image | the Android runtime |

Install the runtime pieces once:

```bat
set SDK=%LOCALAPPDATA%\Android\Sdk
"%SDK%\cmdline-tools\latest\bin\sdkmanager.bat" --sdk_root="%SDK%" "platform-tools" "emulator" "system-images;android-36;google_apis;x86_64"
```

DroidWin creates its own AVD (`DroidWin_API36`) headless on first run; you never touch the emulator
by hand.

## Run it

```bash
pnpm install
cargo build --manifest-path src-tauri/Cargo.toml
pnpm tauri dev
```

`DROIDWIN_HOME` overrides where DroidWin keeps its database, logs and bundled window host — useful
so a development run stays inside the checkout:

```bash
DROIDWIN_HOME="$PWD/.droidwin/home" ./src-tauri/target/debug/droidwin.exe
```

## Verify it

```bash
cargo test  --manifest-path src-tauri/Cargo.toml          # domain, AXML, store, adb, sdk
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
pnpm exec tsc --noEmit
pnpm test                                                 # honest-state rendering tests
cargo run --manifest-path src-tauri/Cargo.toml --bin proof # Gate A + B against the real runtime
```

The `proof` binary uses the same modules the app uses — it boots the runtime, installs two real
FOSS APKs, launches both, checks two distinct HWNDs from the OS, moves and resizes one, closes one,
and confirms the other survives and the packages are still installed after a manager restart. Its
report lands in `$DROIDWIN_HOME/proof-report.json` and the evidence is transcribed into
`docs/RUNTIME_PROOF.md`.

## Layout

```
src/                    React + TypeScript UI (presentation only)
  backend.ts            the only module that talks to Rust
  screens/              Overview, Installer, Applications, Runtime, Activity, Settings
  components/           shell, panels, pills, stage list, drop zone, icons
  styles/               Kyoto Zen tokens and layout
  assets/fonts/         bundled Instrument Serif + Onest (OFL 1.1)
src-tauri/src/
  domain/               typed models, error codes, operation state machine
  apk/                  archive guards + binary AndroidManifest (AXML) reader
  runtime/              SDK locator, adb, headless emulator, scrcpy windows
  store/                SQLite with migrations
  diag/                 tracing to a daily log file, path redaction
  bin/proof.rs          Gate A/B feasibility harness
DroidWin_Vibecoding_Brief/  the product brief this build follows (docs/ inside)
```

Rules the code keeps: no `adb`/`scrcpy` string outside `runtime/`, no `invoke()` outside
`backend.ts`, no SQL outside `store/`, and no capability reported as supported until it was verified
on this machine.

## Known limits

- Every app shares one Android user; there is no per-app sandbox.
- App windows are streamed virtual displays: Android's back key works, there is no host-side back button.
- Audio, clipboard sharing and file drop are not verified, so they are reported as unsupported.
- ARM-only packages cannot run on the x86_64 image; apps needing Google Play services usually fail.
- `.apk` association, deleting app data from the UI, and automatic SDK provisioning are not built yet.
- No telemetry, no accounts, no network calls at runtime.
