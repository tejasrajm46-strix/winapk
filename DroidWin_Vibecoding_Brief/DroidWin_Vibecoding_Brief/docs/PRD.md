# Product Requirements Document — DroidWin

**Status:** Draft / runtime feasibility not yet proven  
**Product type:** Windows desktop application  
**Working name:** DroidWin

## 1. Vision

DroidWin makes compatible Android apps feel approachable on Windows. A user double-clicks an APK, DroidWin handles inspection and runtime setup, installs or updates the app, and launches the real Android app in an independently managed desktop window where the selected backend supports it.

The app should feel like a carefully crafted Windows utility—not an Android emulator control panel.

## 2. Problem

Running Android APKs on Windows often requires users to understand and manually operate a runtime, emulator, package installer, command-line tools, or virtual device. The product should hide unnecessary operational complexity while remaining honest about compatibility and security.

## 3. Target users

- Windows users who have a legitimate APK and want to run it with minimal setup.
- Developers/testers who need a convenient app library and diagnostics.
- Technical users who want runtime details available without forcing those details on everyone.

## 4. Product principles

1. Install and run first.
2. Native-feeling Windows experience.
3. Runtime truth over optimistic UI.
4. Clear compatibility boundaries.
5. Persistent app data and settings.
6. Secure handling of untrusted APKs.
7. Technical details available on demand.
8. No fake progress, fake runtime states, or fake compatibility claims.

## 5. Primary user journey

1. User double-clicks an APK or selects/drops it into DroidWin.
2. Windows passes the absolute path to the application.
3. DroidWin verifies that the file exists and is a supported package archive.
4. DroidWin extracts available metadata and displays relevant warnings.
5. DroidWin checks runtime availability and compatibility.
6. If setup is required, DroidWin explains what will be installed and requests consent where appropriate.
7. DroidWin installs or updates the APK through a supported runtime interface.
8. DroidWin launches the installed package.
9. The real app appears in its own desktop window if supported by the backend.
10. DroidWin updates the library and diagnostic history.

## 6. Functional requirements

### FR-01 — APK entry points
- Register an appropriate Windows file association through the installer and OS-supported mechanisms.
- Accept APK paths passed on launch, including multiple paths if supported.
- Support drag-and-drop and file picker selection.
- Handle missing, moved, inaccessible, and malformed paths gracefully.
- Never assume file association registration succeeded without checking.

### FR-02 — Package inspection
- Read archive/package metadata without executing app code.
- Show app name, package identifier, version, minimum SDK and ABI information when available.
- Show signing/signature information only when it can be determined reliably.
- Explain unavailable metadata instead of inventing values.
- Treat metadata as untrusted input and guard against malformed archives and decompression abuse.

### FR-03 — Runtime provisioning and status
- Detect whether the chosen backend is installed and healthy.
- Explain requirements before installing dependencies.
- Download runtime artifacts only from configured trusted sources.
- Verify downloads where trusted hashes/signatures are available.
- Display truthful progress and recoverable failure states.
- Do not rely on discontinued Windows Subsystem for Android as the default backend.

### FR-04 — Package install/update
- Install packages through supported backend interfaces.
- Detect existing package identifiers and present update/reinstall behavior.
- Let the backend enforce package-signature/update rules.
- Preserve app data during a valid update.
- Handle insufficient storage, incompatible packages, runtime loss, and interrupted operations.

### FR-05 — Application library
- Enumerate installed apps from the runtime.
- Search and sort apps.
- Show icon/name/package/version where available.
- Launch, inspect, update, reinstall, and uninstall apps.
- Reconcile database records with the runtime's actual package list.
- Never show stale database data as authoritative runtime state.

### FR-06 — Independent app windows
- The long-term UX target is one independently managed desktop window per running Android app.
- Windows must be movable, resizable, minimizable, maximizable, and closable.
- Multiple apps must be able to run concurrently if the backend supports it.
- This requirement is a runtime feasibility gate, not a UI-only task.
- If the selected backend cannot provide this, document the gap and propose alternatives before claiming support.

### FR-07 — Input and integration
- Support keyboard and mouse input through the runtime/window bridge.
- Support clipboard operations where technically and securely supported.
- Support explicit, documented file-sharing locations.
- Respect DPI scaling and app orientation where possible.
- Clearly disclose backend limitations.

### FR-08 — Persistence
- Persist user preferences, installed app metadata, recent launches, and diagnostic references.
- Treat the runtime as the source of truth for installed packages.
- Use database migrations.
- Never delete user app data during routine updates or uninstall without explicit confirmation.

### FR-09 — Diagnostics
- Record operation IDs, timestamps, state transitions, exit codes, and sanitized backend output.
- Provide a readable error summary and optional technical detail.
- Redact credentials, tokens, and sensitive paths where appropriate.
- Provide log export only with user consent and clear destination.

### FR-10 — Settings
- General preferences.
- Runtime status and repair.
- App launch/window defaults.
- Clipboard/file integration preferences.
- Security warnings and diagnostic logging.
- Updates and about information.
- Do not show unsupported toggles.

## 7. Non-functional requirements

- UI remains responsive during runtime operations.
- Long-running work is asynchronous and cancellable where safe.
- Common errors produce actionable messages.
- Database and settings updates are resilient to application restarts.
- App state and runtime state are reconciled at startup.
- UI supports keyboard navigation, accessible names, visible focus, and reasonable contrast.
- App supports the agreed Windows versions and CPU architectures, explicitly documented after runtime research.
- The app installer and runtime redistribution comply with relevant licenses.
- Security claims must match the actual isolation model.

## 8. Out of scope for initial MVP

- Guaranteeing that every APK works.
- Reimplementing the full Android framework.
- Reimplementing ART, Binder, Android system services, native ABI, and graphics stack.
- Bundling Google Play Services or proprietary app stores without authorization.
- Bypassing DRM, app integrity checks, or package signing.
- Cloud sync/accounts unless a later product decision justifies them.
- A custom Android compatibility engine before a separate feasibility/research decision.

## 9. MVP release gates

### Gate A — Runtime feasibility
Demonstrate actual APK installation and launch using the selected backend.

### Gate B — Windowing feasibility
Demonstrate at least two real Android apps in independently managed desktop windows, or document a revised product compromise before full implementation.

### Gate C — Core workflow
Double-click APK → inspect → install/update → launch → manage app.

### Gate D — Quality
Test failures, persistence, keyboard/mouse, clipboard, supported file access, DPI, packaging, and recovery.

## 10. Success criteria

- A first-time user can understand how to install an APK without reading developer documentation.
- APK paths from file association are handled correctly.
- Runtime operations show real states and useful failures.
- Installed apps are discoverable and manageable.
- The core workflow does not require the user to issue terminal commands.
- The selected backend's windowing limitations are clearly documented.
- No data deletion occurs without clear user intent.
- The release's compatibility claims are supported by a test matrix.
