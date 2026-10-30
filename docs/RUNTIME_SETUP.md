# Automatic Android runtime setup

WINAPK needs an Android runtime and does not ship one. Before this change a new
user had to install Android Studio (or the SDK command-line tools), accept
Google's licences, install `platform-tools`, `emulator` and a system image, and
set `ANDROID_SDK_ROOT` — a development setup, asked of someone who wants to play
a game.

This document describes what WINAPK installs itself, where it puts it, what it
refuses to do, and exactly which parts were exercised on the development machine.

---

## 1. What gets installed, and from where

| Step | Source | Size (measured) | Verified how |
| --- | --- | --- | --- |
| Java runtime (only when the PC has no usable JRE) | Eclipse Adoptium Temurin 21 JRE, `api.adoptium.net` | 47 MB | zip opens, `bin/java.exe` exists |
| Android command-line tools | newest Windows archive for `cmdline-tools;X.Y` in Google's `repository2-3.xml` | 148 MB (23.0) | archive **SHA-1** compared with the checksum in that XML, then `bin/sdkmanager.bat` must exist after extraction |
| `platform-tools` (adb) | `sdkmanager` / native `android sdk install` | 15 MB | `platform-tools/adb.exe` on disk |
| `emulator` | same | 450 MB | `emulator/emulator.exe` on disk |
| `system-images;android-36;google_apis;x86_64` | same | 1.7 GB | directory present under `system-images/android-36/google_apis/x86_64` |
| AVD `DroidWin_API36` | `avdmanager`, from that image | ~1 GB on disk (sparse) | `~/.android/avd/DroidWin_API36.avd` |
| Window host (scrcpy) | bundled in the installer, copied to the data directory | 26 MB | `runtime/bin/scrcpy/scrcpy.exe` |

Nothing is bundled that Google licenses for development use. The installer stays
~13 MB; the runtime is downloaded once, by the app, after the user accepts the
SDK licence terms.

Final footprint measured on this machine after a complete fresh install:
**5.7 GB** in the SDK directory (including the image and the unpacked AVD data
lives in the AVD home).

## 2. Where it goes

`runtime::sdk` picks a root in this order:

1. a directory the user chose on the setup screen (stored as `runtime.sdk_root`),
2. `DROIDWIN_ANDROID_SDK` (an explicit choice for a portable run or a test),
3. an existing SDK: `ANDROID_SDK_ROOT`, `ANDROID_HOME`, `%LOCALAPPDATA%\Android\Sdk`,
   `%USERPROFILE%\AppData\Local\Android\Sdk`, `C:\Android\Sdk` — the most complete
   one wins,
4. WINAPK's own directory: `%APPDATA%\app.droidwin.desktop\android-sdk`, or
   `$DROIDWIN_HOME/android-sdk`.

A directory named in (1) or (2) is used even when it is empty, because it is a
decision rather than a hint — that is how a first run on a fresh PC looks. A
complete SDK discovered in (3) is adopted as it is; if it is missing a required
component, the setup screen names only that component and installs it into the
same SDK, so there is never a second copy.

The setup screen, the installer and the runtime all resolve this root through
`sdk::active_root()`, so they cannot disagree about which SDK the PC has.

## 3. The first-run screen

`src/screens/Setup.tsx` replaces the shell while the runtime is not installed
(`needs_install` or `failed`). It shows:

- every component WINAPK checked, whether it is present, where it is, and the
  download size of a missing one (sizes are the published archive sizes, and the
  screen calls the total "about");
- the SDK directory and the free space on that volume;
- a licence checkbox — **the install button stays disabled until it is ticked**,
  unless the licences were already accepted by a previous run;
- real progress: a percentage when a transfer length is known (the JRE and the
  command-line tools are downloaded by WINAPK itself), an indeterminate bar for
  the SDK manager, whose output is only forwarded as text;
- Cancel (a cooperative flag the downloader and every spawned tool check), Retry,
  **Choose folder**, **Use WINAPK's folder**, Check again, and *Open WINAPK
  anyway* so nobody is trapped on the screen.

States are `needs_install → installing → starting | failed → ready`. The gate
lifts once nothing is left to install; `starting` means every file is present but
no boot has been verified, and only a boot the runtime observed through adb turns
the state into `ready`. The app also starts an installed runtime at launch, so a
stopped emulator comes up without the user pressing Start.

## 4. Licences

The setup screen states that the Android SDK licence terms are being accepted, and
the checkbox is required. The install then runs `sdkmanager --licenses` with `y`
answers on stdin and records the result in `<sdk>/licenses/android-sdk-license`.
If the prompt does not exit 0, the install fails with that reason and
`runtime.licenses_accepted` is not set. WINAPK never writes a licence hash
itself.

## 5. Security of the download path

- HTTPS to `dl.google.com` and `api.adoptium.net` only. No third-party mirror, no
  GitHub release, no user-supplied URL; the package names are constants.
- The command-line tools archive is verified against the **SHA-1 from the same
  official repository XML** before anything is extracted, and the download is
  removed on mismatch.
- The zip is opened in-process (no external unzip), entries with `..`, a drive
  letter or an absolute path are skipped, and only the `cmdline-tools/` prefix is
  stripped.
- The install root must be absolute, must have a parent, and may not be inside
  `%SystemRoot%`. A drive root is refused.
- `sdkmanager`/`avdmanager` are batch files invoked through `cmd /c` with a
  relative script name and separate arguments: no shell interpolation, no string
  built from user input, and paths with spaces work.
- No administrator rights are requested anywhere in this flow.

## 6. Ownership and shutdown

`EmulatorRuntime` tracks whether *it* started the emulator:

- starting one writes its pid to `<data>/runtime/emulator.pid`;
- a later run that finds that pid and confirms the process is still
  `emulator.exe` adopts it as owned (a recycled pid is never acted on);
- closing WINAPK closes every app window and stops an emulator **only if it is
  owned**; an emulator the user started themselves is left running;
- **Settings → Android runtime → Keep the emulator running after WINAPK closes**
  skips the stop and deletes the marker, so the next run treats it as external;
- **Stop runtime** on the Runtime screen always stops an owned emulator,
  regardless of that preference.

## 7. Failure and recovery

| Failure | What happens |
| --- | --- |
| Network drop mid-download | `curl` is retried once, resuming the partial file with `-C -`; if the server ignores the range the partial file is discarded and the download restarts once |
| Checksum mismatch | download deleted, install fails with the expected/actual digest |
| Disk too small | refused before any download, with free/needed space in the message |
| SDK manager fails or hangs | non-zero exit or the per-package timeout (30 min) is reported with the tail of its output; the package is verified on disk afterwards, so a "success" that installed nothing is caught |
| Cancelled | the tool is killed, installed components stay, the report returns to `needs_install` (not `failed`) |
| Interrupted install | every step re-checks the disk; running the install again continues from what is there |

## 8. What was actually run

All of this was executed on the development machine (Windows 11 x64, WHPX
available, ~300 GB free).

**Fresh install into an empty directory** — `cargo run --example setup_proof` with
`DROIDWIN_HOME`, `DROIDWIN_ANDROID_SDK` and `ANDROID_AVD_HOME` pointing at a new
directory and `JAVA_HOME` unset (so the JRE path was exercised):

```
java: None
[prepare] Checking disk space.
[tools] Downloading a Java runtime for the Android SDK tools.
[tools] Downloading Java runtime: 45 of 47 MB.
[tools] Downloading Android command-line tools 23.0.
[tools] Verifying the download.
[tools] Unpacking the command-line tools.
[licenses] Recording Android SDK licence acceptance.
[packages] Installing platform-tools.
[packages] Installing emulator.
[packages] Installing system-images;android-36;google_apis;x86_64.
[avd] Creating the Android virtual device.
[validate] Checking the installed runtime.
[done] The Android runtime is installed.
install: ok
runtime: Ready · Android API 36 on emulator-5554 (DroidWin_API36).
```

**Reuse** — the same harness against an SDK where only the AVD was missing:
`state before: NeedsInstall · missing: ["Virtual device"]`, then licences, AVD
creation, validation and boot, with no download.

**First-run screen** — the release build launched with an empty SDK directory
shows *"Set up Android"*, `NOT INSTALLED`, the five components with their sizes,
the licence checkbox, a disabled **Install Android runtime** button and *Open
WINAPK anyway* (`.droidwin/setup-screen.png`).

**After the install** — the same build launched against the freshly installed SDK
opens the normal shell with **RUNTIME READY · API 36**, boots the emulator and
lists the device as `emulator-5554` (`.droidwin/after-setup.png`).

**Shutdown** — closing the window stopped both `droidwin.exe` and the emulator it
had started; the proof harness's own `shutdown()` also left no emulator process.

**Regression** — `node scripts/live.mjs` (see the run recorded in
[docs/RELEASE_VERIFICATION.md](RELEASE_VERIFICATION.md)) and the unit/frontend
suites are green after these changes.

## 9. Limits that remain

- **The clean-Windows test was simulated, not virtualised.** The install ran into
  a directory with no SDK, no JRE and no AVD, which is the state a fresh PC is in,
  but it ran on this machine's Windows installation. A Windows VM without the
  Visual C++ runtime, WebView2 or WHPX-enabled virtualization has not been tried.
- Downloads need the internet and about 6 GB of free space. There is no offline
  installer and no bundled image.
- The system image is fixed to `android-36;google_apis;x86_64`. ARM Windows and
  32-bit Windows are not supported, and a Google Play image is not used (WINAPK
  needs `adb root` for its loopback containment, which a Play image refuses).
- The command-line tools version is whatever Google's repository lists; the
  installer parses that XML rather than pinning a filename. If Google removes the
  Windows archive from that document the install fails with "did not list Windows
  command-line tools", which is honest but not self-healing.
- The SDK manager's own progress is text, not a percentage, so the bar is
  indeterminate for those steps.
- Package downloads are not resumed inside a package; a retry re-downloads that
  package. Downloads WINAPK makes itself (JRE, tools) do resume once.
- A JRE is installed only when none is found. An unsupported Java version already
  on the machine (for example a JRE 8 in `JAVA_HOME`) would make `sdkmanager` fail;
  the error is reported, but WINAPK does not detect the version and fall back to
  its own JRE.
