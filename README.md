<div align="center">

<img src="docs/images/hero.svg" alt="WINAPK: an Android game running in its own Windows window, with the screenshot/rotate/More rail on the right, Back · Home · Recent Apps along the bottom, and Shift+A, Shift+H and Ctrl+Shift+R keycaps beside it" width="100%" />

# WINAPK

<p><strong>Your apps. Your windows. Your control.</strong></p>

Run Android apps and games in their own real Windows windows.

Not a phone-shaped rectangle inside an emulator. One window per app, on its own Android display —
with the controls anchored to the window they belong to, and keyboard mapping that starts from the
real control you click.

<br />

[![Download the installer](https://img.shields.io/badge/Download_Setup.exe-0.1.0-e0231c?style=for-the-badge&logo=windows11&logoColor=white)](release/WINAPK/Setup.exe)
[![Read the build report](https://img.shields.io/badge/Build_report-verified-62d0e8?style=for-the-badge&logo=readthedocs&logoColor=white)](release/WINAPK/BUILD_REPORT.txt)

<br />

![Windows 10 / 11](https://img.shields.io/badge/Windows-10_%7C_11-e0231c?style=flat-square&logo=windows11&logoColor=white)
![Tauri 2](https://img.shields.io/badge/Tauri-2-24c8db?style=flat-square&logo=tauri&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-1.80+-b7410e?style=flat-square&logo=rust&logoColor=white)
![React 19](https://img.shields.io/badge/React-19-149eca?style=flat-square&logo=react&logoColor=white)
![Tests](https://img.shields.io/badge/tests-122_frontend_%C2%B7_271_rust-2ea043?style=flat-square)
![MCP](https://img.shields.io/badge/MCP-34_tools-62d0e8?style=flat-square)
[![License](https://img.shields.io/badge/license-MIT-6b7280?style=flat-square)](LICENSE)

<sub><b>One shared Android runtime · one virtual display per app · one window per app</b></sub>

</div>

---

## Why it exists

Running an Android game inside a phone emulator gives you a phone-shaped rectangle and a mouse that
pretends to be a finger. WINAPK treats **the window itself** as the product:

<table>
<tr>
<td width="33%" valign="top">

### 🪟 One window per app
Each app runs on its own virtual display, so four apps can be open at once without sharing a screen.
Move, resize or rotate any of them independently.

</td>
<td width="33%" valign="top">

### 🎯 Controls where the app is
The rail and the navigation bar are anchored to the window they belong to, and follow it when it
moves, resizes or rotates.

</td>
<td width="33%" valign="top">

### ⌨️ Mapping from the app itself
Click the real control on the real screen, press a key. The point is stored as a percentage of the
Android display, so it survives resize and rotation.

</td>
</tr>
</table>

---

## Screenshots

<div align="center">

| Click-to-place key mapping | Universal shortcuts |
| :---: | :---: |
| <img src="docs/images/key-mapping.png" alt="Click-to-place key mapping over a running game" width="470" /> | <img src="docs/images/universal-keys.png" alt="Universal keyboard shortcut reference" width="470" /> |
| Click a control on the game, press a key — the point, the key and the profile are saved per Android package. | `Shift+A/H/R/M/P/O` work in every Android app, before any game profile. |

| Control rail |
| :---: |
| <img src="docs/images/window-controls.png" alt="Vertical control rail: screenshot, rotate, More" width="156" /> |

</div>

---

## What changes

| | Traditional emulator workflow | **WINAPK** |
| :--- | :--- | :--- |
| **Where apps run** | One emulator window, shared by everything | Each app on its own virtual display, in its own Windows window |
| **Controls** | Configured against a phone screen, inside emulator chrome | Anchored to the app window; mapped by clicking the real control |
| **Navigation** | Emulator-level | Back · Home · Recents at the bottom of *that* window |
| **Automation** | Not part of the app | MCP tools and a CLI over one local, token-protected endpoint |

<sub>This describes WINAPK's intended experience — not an audit of every other emulator.</sub>

---

## Features

<table>
<tr>
<td width="50%" valign="top">

#### 🪟 Desktop experience

- **One window per app** — each app on its own virtual display, so four can be open at once
- **Free resize** — portrait and landscape, independently per window
- **Anchored controls** — the rail and navigation bar follow the window they belong to
- **Compact by default** — 900×560 minimum; below 1040 px the sidebar becomes an icon rail

</td>
<td width="50%" valign="top">

#### 🎮 Controls

- **Click-to-place mapping** — click the real control, press the key; stored as a percentage of the Android display
- **Universal chords** — `Shift+A/H/R/M/P/O` decided before any game profile, so a game cannot swallow navigation
- **Overlay labels** — `Ctrl+Shift+R` hides them without losing the profile
- **Per-package profiles** — saved with that Android package, reloaded with the app

</td>
</tr>
<tr>
<td valign="top">

#### ⚡ Runtime and performance

- **Own Android runtime** — installed on first run from Google's distribution, then reused
- **Honest resource reporting** — the plan you requested is shown apart from what Android reports
- **Phone display control** — size and density through `wm`, without restarting the emulator
- **Real cleanup** — closing WINAPK stops the emulator it started and leaves yours alone

</td>
<td valign="top">

#### 🤖 Automation and developer

- **34 MCP tools** — runtime state, packages, taps, screenshots, workflows and triggers
- **CLI on the same runtime** — 27 commands; it never starts a second emulator
- **Loopback + token** — per-session token, with Host and Origin checks
- **Your name** — rename the app; the title bar, header and app windows follow

</td>
</tr>
</table>

---

## Quick start

### Install

```powershell
.\release\WINAPK\Setup.exe
```

First launch sets up its own Android runtime (SDK, system image, AVD) from Google's distribution, then
reuses it. Nothing else has to be installed beforehand.

> The download button above points at the installer committed in this repository — the exact binary
> whose SHA-256 is in the build report. Tagged releases publish the same artifact under **Releases**.

### Run the portable build

```powershell
.\release\WINAPK\WINAPK.exe
```

### Build from source

```powershell
pnpm install
pnpm dev            # frontend only
pnpm tauri dev      # full app
pnpm tauri build    # release EXE + NSIS installer
```

### Requirements

| | |
| :--- | :--- |
| **OS** | Windows x64. Built and run on this machine's Windows; a clean-VM install has not been tested, and Windows on ARM is not supported (the system image is x86_64). |
| **Virtualisation** | Required for a usable emulator. Without it WINAPK still runs and says acceleration is unavailable. |
| **Disk** | ~6 GB free for the one-time Android runtime download. |
| **Memory** | The emulator uses 2–8 GB depending on the plan you choose (2 GB is the floor). |
| **WebView2** | Required; present on Windows 11. |
| **From source** | Node 20+, pnpm, and Rust with the MSVC toolchain. |

---

## Keyboard

**Universal — the same in every Android app** (editable in *Key mapping*):

| Action | Default | Action | Default |
| :--- | :---: | :--- | :---: |
| Back | `Shift+A` | Menu | `Shift+M` |
| Home | `Shift+H` | Screenshot | `Shift+P` |
| Recent Apps | `Shift+R` | Rotate | `Shift+O` |
| Show / hide mapping | `Ctrl+Shift+R` | | |

Universal chords are decided **before** any per-app profile, so a game cannot swallow Android's own
navigation. A chord needs a modifier, so plain typing is never captured, and a chord already used by
another action or app mapping is refused with a reason instead of being silently shadowed.

### Per-app mapping, in three steps

| | Step | What happens |
| :---: | :--- | :--- |
| **1** | *More → Key mapping → Place on the game* | The overlay waits for your next click on the running app |
| **2** | Click the control, then press a key | The point is captured as a percentage of the Android display, so resize and rotation keep it on the same control |
| **3** | **Add** | The mapping is saved for that Android package and reloaded when you open it again |

---

## 🤖 Agent control (MCP + CLI)

WINAPK ships an MCP server bound to loopback with a per-session token — **34 tools**, from runtime
status and package management to screenshots, taps and workflows. The `WINAPK` CLI is a client of
the same endpoint, so it never starts a second emulator.

```powershell
WINAPK status
WINAPK apps list
WINAPK apps open com.fingersoft.hillclimb
WINAPK mapping save com.fingersoft.hillclimb --key W --tap 49,61
WINAPK capture screenshot
WINAPK tools              # 27 commands, 4 KB — far smaller than tools/list
```

Point an MCP client at the stdio bridge:

```json
{
  "mcpServers": {
    "winapk": {
      "command": "C:\\path\\to\\WINAPK.exe",
      "args": ["--mcp-stdio"]
    }
  }
}
```

<sub>Transport, tool schemas, examples and troubleshooting: [docs/MCP_AGENT.md](docs/MCP_AGENT.md) · CLI: [docs/CLI.md](docs/CLI.md)</sub>

---

## 🏷️ Make it yours

*Settings → Application name* renames the title bar, the top bar, the About panel and every app
window opened afterwards. The value is trimmed, limited to 40 characters, refused when empty or
multi-line, saved locally, and read back at startup — so the name is already in effect next launch.

This is **display branding only**. The executable, the installer's product name and the bundle
identifier are build-time values and are left alone, which is what keeps shortcuts and updates
working.

---

## Architecture

<div align="center">
  <img src="docs/images/architecture.svg" alt="Three layers: the Tauri desktop shell (main window plus four overlay windows), the Rust core (runtime, input, overlay, capture, agent), and the Android runtime (one emulator with one virtual display per app)" width="100%" />
</div>

One owner per concern: the runtime owns processes, `InputController` owns profiles and key decisions,
`Overlay` owns window geometry and visibility, and the React layer renders whatever the backend
pushes — it never mirrors state.

---

## Testing

```powershell
pnpm test                                              # 122 frontend tests
cargo test --manifest-path src-tauri/Cargo.toml --lib  # 271 Rust tests
```

| Check | Result |
| :--- | :--- |
| `tsc --noEmit` | clean |
| `pnpm test` | **122 passed** |
| `cargo test --lib` | **271 passed, 0 failed** |
| `cargo fmt --check` | clean |
| `pnpm tauri build` | EXE + NSIS installer |

**What this build verified** against the packaged executable: rename, persistence across a restart,
reset, window-title updates, clean shutdown with no emulator or scrcpy left behind, and the compact
layout at 900×560.

**What was verified in earlier passes and is unchanged here**: APK install and launch, one window per
app, universal `Shift+A/H/R/M/P/O` chords, click-to-place mapping saved and delivered, screenshots and
rotation. The two lists are kept apart on purpose — see
[release/WINAPK/BUILD_REPORT.txt](release/WINAPK/BUILD_REPORT.txt), which also names every check that
was **not** run (installer install/uninstall on a clean machine, signing, MCP `tools/list` size,
performance numbers).

---

## Compatibility & known limits

- **Windows 11 x64** (Windows on ARM is not supported: the system image is x86_64). Hardware
  virtualisation is required for a usable emulator; without it WINAPK still runs, and the Runtime
  screen says acceleration is unavailable rather than pretending otherwise.
- ARM-only packages cannot run on this image, and apps that insist on Google Play services usually
  fail. WINAPK reports what Android reports rather than predicting compatibility.
- Recording an app's own virtual display is not supported by `screenrecord`; the button is therefore
  not offered.
- Rotating reopens the window on the other orientation surface; scrcpy ignores a requested window
  size for a new display, so the frame can keep the old shape.
- `Ctrl+Shift+R` cannot be rebound yet; the other six can.
- No control detection: mapping a game is place → key, never a guessed profile.
- The AVD WINAPK creates is still named `DroidWin_API36` on disk. That identifier is kept on purpose:
  renaming it would orphan the system image and settings of anyone upgrading from an earlier build.

---

## Troubleshooting

| Symptom | What to do |
| :--- | :--- |
| The setup screen appears on first launch | Expected on a PC with no Android SDK. Press **Install Android runtime** and accept Google's licence terms — or point WINAPK at an SDK you already have: *Settings → Android runtime → Choose SDK folder*. |
| The Runtime screen says acceleration is unavailable | Virtualisation is off in firmware or Hyper-V is not enabled. WINAPK still runs, but the emulator will be slow; the screen reports what the emulator itself answered rather than guessing. |
| A window opens black or empty | Some apps refuse a secondary display or draw only on display 0. Open the **Attention** panel: it names the app, shows the evidence it collected, and offers *Retry launch* or *Reopen window*. |
| A mapped key does nothing | Mapping is per Android package and is **off** until you enable it: *Applications → Controls* on that app's row. |
| A shortcut chord is refused | A chord needs a modifier, and one already used by another action or app mapping is refused with a reason instead of being silently shadowed. Pick another chord. |
| `WINAPK status` reports an unreachable endpoint | The agent endpoint exists only while WINAPK runs with it enabled (*Agent* screen). The endpoint file is rewritten per session, so a stale one is reported as unreachable rather than retried forever. |
| An emulator outlives the app | *Settings → Keep the emulator running after WINAPK closes* is on. Turn it off to stop the emulator WINAPK started. An emulator you started yourself is never touched. |

---

## Security

The agent endpoint binds to **loopback only** and requires a per-session token, with Host and Origin
checks. The guest-to-host boundary is documented and tested separately — see
[docs/HOST_ISOLATION.md](docs/HOST_ISOLATION.md) for what is contained, what is not, and why running
untrusted APKs still deserves a dedicated Windows account.

---

## Roadmap

#### Shipped

- [x] One window per app, on its own virtual display
- [x] Click-to-place key mapping, saved per Android package
- [x] Universal `Shift+A/H/R/M/P/O` chords, rebindable
- [x] MCP server + CLI over one runtime, loopback-only with a token
- [x] Customisable application name
- [x] Compact layout down to 900×560

#### Next

These are deliberately not shipped yet, rather than half-working:

- [ ] **Recording an app's own virtual display** — `screenrecord` refuses a virtual display, so the
  button is not offered rather than pretending to record
- [ ] **Rebinding `Ctrl+Shift+R`** — the other six universal chords can be changed
- [ ] **Control detection** — mapping a game is click → key; WINAPK does not guess a profile from a screenshot
- [ ] **Frame shape after rotation** — scrcpy ignores a requested window size for a new display
- [ ] **Installer test on a clean machine** — the current build was not installed or uninstalled on one

---

## Repository layout

```text
src/                 React + TypeScript UI (main window, overlays, screens)
src-tauri/src/       Rust backend: runtime, input, overlay, capture, agent, cli
src-tauri/tests/     live harnesses (mapping, capture, isolation, phones)
release/WINAPK/      shipped EXE, installer, README and build report
docs/                runtime setup, host isolation, MCP, CLI, decisions, images
```

---

## License

[MIT](LICENSE). scrcpy is bundled under Apache-2.0. The Android SDK and emulator are provided by you
and are not redistributed with WINAPK.

<div align="center">
<br />
<sub>Built with Tauri 2 · Rust · React 19 — and scrcpy, which does the hard part</sub>
</div>
