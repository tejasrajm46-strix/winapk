<div align="center">

# WINAPK

**Run Android apps and games in their own real Windows windows.**

A Tauri + Rust desktop shell over the Android Emulator, ADB and scrcpy — with an on-window control rail, universal keyboard shortcuts, and click-to-place keyboard mapping.

[![Windows](https://img.shields.io/badge/platform-Windows%2010%2F11-0a7cff?style=flat-square&logo=windows)](https://www.microsoft.com/windows)
[![Tauri 2](https://img.shields.io/badge/Tauri-2.x-24c8db?style=flat-square&logo=tauri)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-1.8x-b7410e?style=flat-square&logo=rust)](https://www.rust-lang.org)
[![Tests](https://img.shields.io/badge/tests-113%20frontend%20%C2%B7%20256%20rust-2ea043?style=flat-square)](#testing)
[![License](https://img.shields.io/badge/license-MIT-6b7280?style=flat-square)](#license)

</div>

<div align="center">
  <img src="docs/images/key-mapping.png" alt="Click-to-place key mapping over a running game" width="720">
  <br>
  <sub>Click a control on the real game, press a key — the point, the key and the profile are saved per Android package.</sub>
</div>

---

## Why it exists

Running an Android game inside a phone emulator window gives you a phone-shaped
rectangle and a mouse that pretends to be a finger. WINAPK gives every installed
app its own desktop window and treats that window as the product:

- **One window per app** — each runs on its own virtual display, so four apps can be
  open at once without sharing a screen.
- **Controls where the game is** — the rail and the navigation bar are anchored to
  the window they belong to, and follow it when it moves, resizes or rotates.
- **Keyboard mapping that starts from the game itself** — not a percentage pad.
- **Universal shortcuts** that work in every Android app, before any game profile.

## Features

| Area | What you get |
| --- | --- |
| **Library** | Install APKs from the UI, see what is installed, open or stop an app |
| **App windows** | Per-app virtual display, own title, free resize, portrait & landscape |
| **Control rail** | Screenshot of that window's display, rotate, More → key mapping |
| **Navigation** | Back · Home · Recent Apps, always at the bottom of the window |
| **Universal keys** | `Shift+A/H/R/M` nav, `Shift+P` screenshot, `Shift+O` rotate, all rebindable |
| **Key mapping** | Click the real control → press a key → saved per package and reloaded |
| **Overlay** | Labels on the game, `Ctrl+Shift+R` hides everything without losing the profile |
| **Performance** | vCPU / RAM / GPU plan with requested-vs-applied reporting and restart notice |
| **Agent control** | MCP endpoint (JSON-RPC over loopback) and a `droidwin` CLI over the same runtime |
| **Cleanup** | Closing WINAPK releases held keys, closes its windows and stops the emulator it started |
| **Branding** | Settings renames the app; the title bar, header and app windows follow, and the name is remembered |

## Screenshots

| Window controls | Universal keys |
| --- | --- |
| ![](docs/images/window-controls.png) | ![](docs/images/universal-keys.png) |

## Quick start

```powershell
# 1. Run the built app (no toolchain needed)
.\release\WINAPK\WINAPK.exe

# 2. or install it
.\release\WINAPK\Setup.exe
```

First launch sets up its Android runtime (SDK, system image, AVD) from Google's
distribution, then reuses it. Nothing else has to be installed beforehand.

### From source

```powershell
pnpm install
pnpm dev            # frontend only
pnpm tauri dev      # full app
pnpm tauri build    # release EXE + NSIS installer
```

Requirements: Node 20+, pnpm, Rust (MSVC toolchain), WebView2, and a Windows
machine with virtualization enabled for the emulator.

## Keyboard

**Universal — the same in every Android app** (editable in *Key mapping*):

| Action | Default | Action | Default |
| --- | --- | --- | --- |
| Back | `Shift+A` | Menu | `Shift+M` |
| Home | `Shift+H` | Screenshot | `Shift+P` |
| Recent Apps | `Shift+R` | Rotate | `Shift+O` |
| Show / hide mapping | `Ctrl+Shift+R` | | |

Universal chords are decided **before** any per-app profile, so a game cannot
swallow Android's own navigation. A chord needs a modifier, so plain typing is
never captured, and a chord already used by another action or app mapping is
refused with a reason instead of being silently shadowed.

**Per-app mapping:** *More → Key mapping → Place on the game*, click the control,
press the key, **Add**. The point is stored as a percentage of the Android
display, so resizing, moving or rotating the window keeps it on the same control.

## Architecture

```text
┌──────────────── Desktop (Tauri) ────────────────┐
│  main window      library, installer, settings   │
│  overlay-labels   transparent label layer        │
│  overlay-nav      Back · Home · Recents          │
│  overlay-rail     Screenshot · Rotate · More     │
│  overlay-mapper   key-mapping panel              │
└───────────────┬─────────────────────────────────┘
                │ one runtime, one adb server
┌───────────────▼─────────────────────────────────┐
│ Rust core                                        │
│  runtime     emulator + adb + scrcpy lifecycle   │
│  input       keyboard hook, profiles, dispatch   │
│  overlay     anchoring, visibility, placement    │
│  capture     screenshots, recordings             │
│  agent       MCP server, CLI client, control API │
└───────────────┬─────────────────────────────────┘
                │ adb shell input on one display id
┌───────────────▼─────────────────────────────────┐
│ Android Emulator  ·  one virtual display per app │
└──────────────────────────────────────────────────┘
```

One owner per concern: the runtime owns processes, `InputController` owns
profiles and key decisions, `Overlay` owns window geometry and visibility, and
the React layer renders whatever the backend pushes — it never mirrors state.

## CLI and agents

The `WINAPK` command is a client of the running window (loopback endpoint,
token auth, same permission gate as MCP). It never starts a second emulator.

```powershell
WINAPK status
WINAPK apps list
WINAPK apps open com.fingersoft.hillclimb
WINAPK mapping save com.fingersoft.hillclimb --key W --tap 49,61
WINAPK perf preset performance
WINAPK capture screenshot
WINAPK tools              # 27 commands, 4.1 KB — 6.8x smaller than tools/list
```

`WINAPK.exe --mcp-stdio` bridges an MCP client to the same endpoint.

## Security

The agent endpoint binds to loopback only and requires a per-session token. The
guest-to-host boundary is documented and tested separately — see
[docs/HOST_ISOLATION.md](docs/HOST_ISOLATION.md) for what is contained, what is
not, and why running untrusted APKs still deserves a dedicated Windows account.

## Testing

```powershell
pnpm test                                   # 122 frontend tests
cargo test --manifest-path src-tauri/Cargo.toml --lib   # Rust unit tests
```

Live checks were run against the built executable: app launch, emulator boot,
window per app, click-to-place mapping saved and delivered (`tap 531 1474` on
display 2), universal `Shift+A/H/R/M/P/O`, screenshot file, rotate, and clean
shutdown. Details and known limits: [release/DroidWin/BUILD_REPORT.txt](release/DroidWin/BUILD_REPORT.txt).

## Repository layout

```text
src/                 React + TypeScript UI (main window, overlays, screens)
src-tauri/src/       Rust backend: runtime, input, overlay, capture, agent, cli
src-tauri/tests/     live harnesses (mapping, capture, isolation, phones)
release/WINAPK/      shipped EXE, installer, README and build report
docs/                runtime setup, host isolation, decisions, images
```

## Known limitations

- Recording an app's own virtual display is not supported by `screenrecord`; the
  button is therefore not offered.
- Rotating reopens the window on the other orientation surface; scrcpy ignores a
  requested window size for a new display, so the frame can keep the old shape.
- `Ctrl+Shift+R` cannot be rebound yet; the other six can.
- No control detection: mapping a game is place → key, never a guessed profile.
- The application name is display branding only. The executable, the installer's
  product name and the bundle identifier are build-time values; renaming the
  display name does not move or rename them.
- The AVD WINAPK creates is still named `DroidWin_API36` on disk. That is an
  internal identifier kept on purpose: renaming it would orphan the system image
  and settings of anyone upgrading from an earlier build.

## License

MIT. See [LICENSE](LICENSE) if present in this repository.
