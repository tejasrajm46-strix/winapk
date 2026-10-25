# UI/UX Specification — DroidWin / Kage Zen Edition

**Source of truth:** The supplied `stitch_droidwin_android_desktop_engine.zip`, especially its four screen references and `kyoto_zen/DESIGN.md`.

## 1. Design intent

Preserve the supplied **Kyoto Zen / Kage Zen** direction: quiet, editorial, technical, dark, disciplined, and deliberately sparse. It is not a generic consumer dashboard and should not be replaced with a bright blue/purple gradient SaaS template.

The reference's strongest qualities:
- Near-black canvas and layered charcoal surfaces.
- Restrained vermilion/red for high-priority actions and system alerts.
- Muted sage/stone colors for secondary information.
- Hairline borders and limited elevation.
- Editorial serif headings paired with readable sans-serif UI text.
- Dense technical information arranged into deliberate panels.
- A persistent left navigation, top command/search bar, and status footer.
- Clear operational hierarchy: overview → install → applications → runtime/diagnostics.

The reference screenshots contain placeholder/synthetic values and references to WSA. These must not be presented as real production state. Retain the visual language, not false telemetry or a discontinued backend assumption.

## 2. Tokens

Treat these as initial design tokens to implement and refine against the supplied screenshots.

### Core colors
- `background`: `#05070A`
- `surface`: `#111417`
- `surface-low`: `#0B0D10`
- `surface-high`: `#1D2023`
- `surface-higher`: `#282A2E`
- `surface-highest`: `#323539`
- `text-primary`: `#E1E2E7`
- `text-secondary`: `#AEB5B2` (validate contrast)
- `text-muted`: `#8B928F` (only for nonessential metadata; validate contrast)
- `vermilion`: `#E0231C`
- `vermilion-soft`: `#FFB4A9`
- `sage`: `#BBCBBD`
- `sage-muted`: `#738276`
- `outline`: `#5D3F3B`
- `error`: `#FFB4AB`
- `warning`: a restrained amber with accessible contrast
- `success`: muted green/sage with a distinct icon/label, not color alone

### Typography
- Display/editorial headings: Instrument Serif or a bundled/licensed equivalent.
- UI/body: Onest or a bundled/licensed equivalent.
- Fallbacks must be available offline.
- Body default around 13–15 px with comfortable line height.
- Small technical labels may use 10–12 px only where readable at normal Windows scaling.
- Avoid using thin weights for dense technical copy.

### Shape and spacing
- Reference design calls for sharp, precise geometry; use square or subtly rounded containers consistently.
- Do not mix arbitrary corner radii.
- Base spacing rhythm: 4, 8, 12, 16, 24, 32 px.
- Desktop page gutters around 24 px; major section gaps around 24–32 px.
- Thin borders, no heavy generic drop shadows.
- Motion should be subtle and never required to discover a control.

## 3. Shared shell

### Top bar
- Brand mark and “DroidWin” at left.
- Actual runtime state indicator, only when status has been retrieved.
- Global search/command field with `Ctrl+K` if implemented.
- Optional profile/avatar only if the product has real user identity; otherwise omit the placeholder profile.
- Native Windows title-bar controls should remain native and accessible. Do not fake minimize/maximize/close buttons inside the page if the app uses a native title bar.

### Left navigation
Groups:
- WORKSPACE: Overview, Applications, Installer.
- MANAGEMENT: Runtime, Activity, Settings.
- Help/About near the bottom.

Active navigation uses a restrained vermilion accent and a clear selected state. Navigation must remain usable at the minimum supported window size.

### Main content
- Use the reference's compact technical dashboard composition.
- Ensure cards and panels have a clear purpose.
- Do not use arbitrary decorative widgets or fake statistics.
- Content must scroll without clipping, overlap, or hidden primary actions.

### Footer/status strip
Show only actual runtime/app counts and truthful status. If the runtime is not detected, display “Not configured” or “Unavailable”, not “Ready”.

## 4. Screen: Overview

Reference: `droidwin_kage_zen_edition_home/screen.png` and `code.html`.

Structure:
1. Shared shell.
2. Editorial heading: “Your Android apps” and concise description.
3. Primary install panel with drag/drop and Browse APK action.
4. Runtime status panel with actual state and an appropriate next action.
5. Optional compatibility/backend summary, only if sourced from real capabilities.
6. Installed applications preview with app icon/name/status/actions.
7. Recent activity/diagnostics list, only for real events.

Requirements:
- Do not display fake counts, fake CPU/RAM, fake package data, fake compatibility, or fake “sync” status.
- Primary APK action must be immediately visible without scrolling at the default window size.
- Empty state should explain the next step.
- Error states should show an actionable summary and “View details”.

## 5. Screen: APK Installer

Reference: `droidwin_apk_installer_kage_zen_edition/screen.png` and `code.html`.

Structure:
- Title “APK Installer” with restrained editorial heading.
- Target/runtime information from the selected backend.
- Main drag-and-drop/file selection area.
- Package inspection panel.
- Operation sequence panel with real state transitions.
- Diagnostics/log view that can be collapsed.
- Status footer.

Workflow states:
`Idle → Selecting → Inspecting → AwaitingConfirmation (when needed) → PreparingRuntime → Installing/Updating → LaunchReady → Completed`
Failure and cancellation must be explicit states.

Requirements:
- Progress reflects actual operations. Never fabricate percentage values.
- Show package name, ID, version, min SDK, ABI, signature information only when reliably available.
- Do not execute APK code during inspection.
- Logs should be readable and copyable, with technical detail separated from the user-facing error.
- “Launch” is enabled only when install and launch prerequisites are met.
- Do not claim signature verification or “verified payload” unless a real check was performed and its meaning is defined.

## 6. Screen: Applications Library

Reference: `droidwin_applications_library_kage_zen_edition/screen.png` and `code.html`.

Structure:
- Title “Applications Library”.
- Install APK primary action.
- Search/filter and grid/list view.
- App cards with icon, display name, package ID, version, runtime-derived state, and actions.
- Optional filters: All, User, System, Running—only if backend supports those categories and states.
- Context menu: Open, Details, Update/Reinstall, Uninstall.

Requirements:
- Never hardcode sample app records as production apps.
- Running state comes from runtime/process information.
- Distinguish app uninstall from deleting app data.
- Make all primary actions visible; avoid tiny action buttons and clipped labels.
- Provide a useful empty state.

## 7. Screen: Runtime Management

Reference: `droidwin_runtime_management_kage_zen_edition/screen.png` and `code.html`.

Structure:
- Backend identity and version.
- Runtime health/lifecycle status.
- System requirements and detected host architecture where known.
- Capabilities and limitations.
- Start/stop/restart/repair/update actions only where supported.
- Diagnostics and logs.
- Advanced settings grouped separately.

Requirements:
- Do not label the backend “WSA” unless that is actually the selected backend and supported.
- Do not ship fake performance charts, fake subsystem health, fake GPU acceleration claims, or pretend service toggles.
- Every setting must map to a real capability.
- Dangerous actions need confirmation and clear consequences.
- Runtime operations should expose progress, cancellation where safe, and recovery guidance.

## 8. Application windows

The Android app window is a real runtime surface, not a mock. Window chrome should use Windows conventions. The backend must provide or enable independent app surfaces; if not, do not simulate independence by showing static views.

Requirements:
- Move, resize, minimize, maximize, close.
- Correct keyboard/mouse focus and input.
- Clipboard behavior clearly documented.
- Support app orientation and DPI behavior as backend allows.
- App crashes must not crash the manager UI.
- Closing a window must have defined behavior for the underlying app process.

## 9. Interaction and accessibility

- Every button has a meaningful action.
- No hover-only essential controls.
- Visible keyboard focus and logical tab order.
- Tooltips for icon-only buttons.
- Accessible names for icons and controls.
- Do not rely on color alone for status.
- Confirm destructive actions.
- Prevent double-submit and conflicting operations.
- Disable conflicting controls while an operation is active, with a clear state.
- Ensure normal scaling and common DPI settings do not clip labels or actions.

## 10. Visual QA checklist

Before accepting a screen:
- Compare against the supplied reference screenshots.
- Check spacing consistency and alignment.
- Check that labels and actions are not clipped.
- Check that options remain visible at ordinary window sizes.
- Check that contrast is sufficient for all important text.
- Check loading, empty, success, warning, error, and disabled states.
- Check keyboard navigation.
- Check 100%, 125%, 150% Windows scaling if test environments allow.
- Check resize behavior and long package names/logs.
- Remove fake data and placeholder states from production paths.
