# System Architecture — DroidWin

**Status:** Proposed architecture; backend choice is gated by runtime feasibility.

## 1. Architectural principles

- Separate Windows product experience from Android execution.
- Keep the runtime replaceable behind an adapter.
- Treat the backend as the source of truth for installed packages and live runtime state.
- Build only abstractions that represent real boundaries.
- Never let UI code invoke runtime-specific commands directly.
- Keep the UI responsive and operations observable.
- Design for explicit limitations rather than claiming universal compatibility.

## 2. Logical components

### Desktop UI
Responsibilities:
- Navigation and screen presentation.
- User interaction and accessible controls.
- View models and state rendering.
- Displaying operation progress and errors.
- No raw process orchestration or package-manager command construction.

### Application/Core
Responsibilities:
- Domain models and application use cases.
- Operation coordination.
- Validation and state transitions.
- Interfaces for runtime, persistence, package inspection, and Windows integration.

### APK inspection service
Responsibilities:
- Validate file existence, extension, archive structure, and reasonable size limits.
- Parse available package metadata using a maintained library or trusted tool.
- Treat archive contents and metadata as untrusted.
- Never execute package code during inspection.
- Avoid inventing fields that cannot be determined.

### Runtime manager
Responsibilities:
- Detect backend installation and health.
- Provision/repair/update the runtime if supported.
- Start/stop/restart lifecycle operations.
- Expose status and capabilities.
- Handle operation timeouts and cancellation.

### Runtime adapter
Responsibilities:
- Translate typed product operations into backend-specific operations.
- Normalize backend results and errors.
- Enumerate installed packages.
- Install/update/uninstall packages.
- Launch/close apps and expose process/window information where supported.
- Report capabilities so UI does not show unsupported controls.

### Windows integration
Responsibilities:
- APK file association and command-line/activation argument handling.
- Single-instance or multi-instance activation policy.
- File picker and drag/drop.
- Native window behavior.
- Clipboard and supported file integration.
- Installer/uninstaller integration.
- Respect Windows security boundaries.

### Persistence
Responsibilities:
- Store user preferences, app metadata cache, recent activity, and operation history.
- Versioned SQLite schema and migrations.
- Reconcile cached records against the backend.
- Never treat cached “running” or “installed” values as authoritative without reconciliation.

### Diagnostics
Responsibilities:
- Structured logs and operation IDs.
- Safe capture of backend output and exit status.
- Redaction of secrets.
- User-readable error summaries plus optional technical details.

## 3. Proposed solution structure

```text
src/
  DroidWin.App/                 # WinUI 3 views, view models, assets
  DroidWin.Core/                # Domain models, use cases, interfaces
  DroidWin.ApkInspection/        # APK validation and metadata inspection
  DroidWin.Runtime/              # Runtime abstractions, lifecycle, capabilities
  DroidWin.Runtime.Adapter/      # Selected backend implementation
  DroidWin.Storage/              # SQLite repositories and migrations
  DroidWin.WindowsIntegration/   # File association, activation, clipboard/files
  DroidWin.Diagnostics/          # Logging and diagnostics
tests/
  DroidWin.Core.Tests/
  DroidWin.ApkInspection.Tests/
  DroidWin.Storage.Tests/
  DroidWin.Runtime.ContractTests/
  DroidWin.Integration.Tests/
docs/
```

Adjust the structure to the actual solution and chosen backend; do not create empty projects without an implementation need.

## 4. Runtime adapter contract

Illustrative contract; refine after backend selection:

```csharp
public interface IAndroidRuntime
{
    Task<RuntimeSnapshot> GetSnapshotAsync(CancellationToken ct);
    Task<RuntimeCapabilities> GetCapabilitiesAsync(CancellationToken ct);
    Task<IReadOnlyList<InstalledAndroidApp>> ListAppsAsync(CancellationToken ct);
    Task<PackageInspection> InspectApkAsync(string path, CancellationToken ct);
    Task<InstallResult> InstallAsync(string path, InstallOptions options, CancellationToken ct);
    Task<LaunchResult> LaunchAsync(string packageId, LaunchOptions options, CancellationToken ct);
    Task<UninstallResult> UninstallAsync(string packageId, UninstallOptions options, CancellationToken ct);
}
```

Do not treat this example as a final API. Add operations for lifecycle, update, process/window tracking, data handling, and diagnostics based on the selected runtime's actual capabilities. Avoid a bloated “do everything” interface if capabilities need separate interfaces.

Every operation should return a typed result containing success/failure, a stable error code, a user-facing summary, diagnostic details/reference, and backend metadata where useful.

## 5. Runtime capabilities

Represent capabilities explicitly, for example:
- package installation
- update/reinstall
- application enumeration
- application launch
- multiple concurrent apps
- independent app windows
- clipboard
- file sharing
- hardware acceleration
- runtime lifecycle management
- app data backup/restore, if genuinely supported

A capability is enabled only after verified backend support. Do not infer support from a backend name.

## 6. Operation state machine

Example installation flow:

`Received → Validating → Inspecting → AwaitingConsent (if required) → CheckingRuntime → PreparingRuntime (if required) → Installing → VerifyingInstall → Launching (if requested) → Completed`

Terminal alternatives:
- `Failed`
- `Cancelled`
- `NeedsUserAction`

Requirements:
- Every transition has a defined owner and valid predecessor.
- Progress is based on real backend events or bounded, truthful stage reporting.
- Failed operations retain diagnostic context.
- UI cannot leave an operation permanently active after backend termination.
- Duplicate operations for the same package are coordinated or rejected safely.

## 7. Persistence model

Suggested tables:
- `InstalledAppCache`
- `RuntimeProfile`
- `UserSetting`
- `OperationHistory`
- `RecentLaunch`
- `SchemaVersion`

Keep the schema minimal. Store app icons/cache files on disk with stable references rather than embedding large binary payloads in SQLite unless there is a clear reason.

Use migrations. Back up or preserve state during upgrades. Avoid storing secrets or unnecessary personal data.

## 8. Windows file activation

- Register `.apk` association through a supported installer/registration mechanism.
- Support the path passed through the activation arguments.
- Validate every path; do not assume it is trusted because Windows launched the app.
- Handle spaces, Unicode, multiple files, stale paths, and repeated activation.
- Choose and document single-instance behavior.
- If a second activation reaches an existing instance, forward the request safely and show the relevant installer view.
- Do not promise file association behavior until tested on a clean install and after upgrade/uninstall.

## 9. Process and window integration

The architecture must distinguish:
1. DroidWin manager window.
2. Android app execution process/container/guest.
3. Host window or surface used to present the Android app.

Do not equate “app launched in the guest” with “independent Windows window created.”

The backend feasibility prototype must prove window ownership, resizing, input focus, close semantics, multiple simultaneous apps, and crash isolation.

If the backend provides only one shared emulator display, record that as a product gap rather than faking independent windows.

## 10. Security boundaries

- APK metadata parsing is untrusted-input processing.
- Runtime execution is a separate trust boundary.
- Host file sharing must be explicit and limited.
- Process invocation must avoid shell interpolation.
- Runtime downloads must use trusted endpoints and verification.
- Logs must redact secrets.
- Elevation must be avoided unless absolutely required and justified.
- Do not weaken package-signature checks.
- Document whether the backend provides a VM, container, sandbox, or ordinary host processes and what that means for isolation.

## 11. Failure handling

Normalize failures into categories:
- invalid package
- unsupported package/ABI/API
- runtime unavailable
- runtime startup failed
- insufficient disk
- install/update failed
- launch failed
- app crashed
- host integration unsupported
- operation timed out
- cancelled
- unknown backend failure

Keep user-facing text concise and diagnostic details actionable.

## 12. Build and release

- Pin dependencies and document versions.
- Separate app installer from runtime distribution decisions.
- Respect backend licenses and redistribution rules.
- Provide a clean uninstall path.
- Preserve user data unless deletion is explicit.
- Sign release artifacts when a suitable signing process is established.
- Maintain a compatibility matrix by Windows version, CPU architecture, backend version, and test APK.
