# Task Backlog — DroidWin

**Rule:** Do tasks in dependency order. Do not skip the runtime/window feasibility gate.

Statuses: `TODO`, `IN PROGRESS`, `BLOCKED`, `DONE`. Update status only with evidence.

## EPIC 0 — Repository and runtime feasibility

### DW-001 — Inspect current workspace
- Status: DONE
- Goal: Determine whether a repository already exists and identify current stack, build, tests, and constraints.
- Acceptance:
  - Existing files and architecture inspected.
  - Build/test commands recorded.
  - No files overwritten before inspection.
- Deliverable: workspace assessment.

### DW-002 — Evaluate candidate Android runtimes
- Status: DONE
- Owner: research agent
- Goal: Compare viable Windows Android execution backends.
- Check: maintenance, supported Windows versions, CPU architecture, Android API, APK install, graphics, input, clipboard, file sharing, concurrent apps, independent windows, security, licensing, redistribution.
- Acceptance:
  - Evidence and source links recorded.
  - Unknowns separated from verified facts.
  - A recommendation and fallback are documented.
- Deliverable: `docs/RUNTIME_DECISIONS.md`.

### DW-003 — Prove APK install and launch
- Status: DONE
- Goal: Install and launch one known-compatible APK using the selected backend.
- Acceptance:
  - Real APK installed.
  - Launch is initiated programmatically.
  - Backend result and errors captured.
  - No fake UI-only success.
- Deliverable: minimal reproducible prototype.

### DW-004 — Prove independent app windows
- Status: DONE
- Goal: Demonstrate two actual Android apps in independently managed Windows desktop windows.
- Acceptance:
  - Each app can be focused and resized independently.
  - Keyboard/mouse behavior is tested.
  - Closing a host window has documented semantics.
  - Backend limitation is recorded if this is not possible.
- Deliverable: prototype and decision.

### DW-005 — Record architecture decision
- Status: DONE
- Goal: Select the MVP backend and document limitations.
- Acceptance:
  - `DECISIONS_LOG.md` updated.
  - Architecture diagram and interfaces match evidence.
  - Any product compromise is approved before proceeding.

## EPIC 1 — UI foundation

### DW-010 — Translate design reference into tokens
- Status: DONE
- Goal: Implement the supplied Kage Zen/Kyoto Zen visual language.
- Acceptance:
  - Color, typography, spacing, border, and shape tokens centralized.
  - Fonts/assets licensing and offline fallback considered.
  - Screens visually checked against supplied references.

### DW-011 — Build shared application shell
- Status: DONE
- Goal: Create title bar strategy, navigation, search/command field if supported, status strip, and page container.
- Acceptance:
  - Navigation works.
  - Selected state is clear.
  - No fake runtime status.
  - Window resizing and DPI scaling do not clip important controls.

### DW-012 — Build Overview screen
- Status: DONE
- Acceptance:
  - Install action visible.
  - Runtime state truthful.
  - Installed apps sourced from runtime/repository, or an explicit development mock mode.
  - Empty/loading/error states implemented.

### DW-013 — Build APK Installer screen
- Status: DONE
- Acceptance:
  - File picker and drop zone work.
  - Inspection, installation, cancellation, success, and error states are defined.
  - No fake progress.
  - Package metadata is not fabricated.

### DW-014 — Build Applications Library
- Status: DONE (grid/list toggle and in-UI data deletion not built)
- Acceptance:
  - Search and grid/list views work if included.
  - App actions map to real services.
  - Empty and error states work.
  - Uninstall/data deletion are clearly distinguished.

### DW-015 — Build Runtime Management and Settings
- Status: PARTIAL (Runtime screen complete; Settings carries facts, not toggles)
- Acceptance:
  - Only supported backend controls appear enabled.
  - Capabilities and limitations are visible.
  - Diagnostic logging and error details are useful.

## EPIC 2 — Domain, persistence, and operations

### DW-020 — Define domain models and operation states
- Status: DONE
- Acceptance:
  - Typed models for apps, runtime status, package inspection, operation results, and errors.
  - State transitions validated.
  - Unit tests cover invalid transitions.

### DW-021 — Implement SQLite persistence
- Status: DONE
- Acceptance:
  - Versioned schema and migrations.
  - Settings and metadata persist.
  - Startup reconciliation against runtime is supported.
  - No sensitive secrets stored unnecessarily.

### DW-022 — Implement APK inspection
- Status: DONE
- Acceptance:
  - Valid/invalid APK cases tested.
  - Metadata is extracted only when available.
  - Malformed archive and large-file cases handled.
  - Inspection never executes APK code.

### DW-023 — Implement runtime adapter
- Status: DONE
- Acceptance:
  - Uses documented backend interfaces.
  - No runtime-specific commands in UI code.
  - Timeouts, cancellation, exit codes, and diagnostics handled.
  - Contract tests added.

### DW-024 — Implement install/update/uninstall
- Status: PARTIAL (install/update/uninstall proven; in-UI data deletion not exposed)
- Acceptance:
  - Install and update work with test APKs.
  - Invalid update signatures are not bypassed.
  - App data is preserved on update.
  - Data deletion requires explicit user choice.

### DW-025 — Implement application enumeration and launch
- Status: DONE
- Acceptance:
  - Library reconciles with backend state.
  - Launch failures are shown clearly.
  - App identity is based on package ID, not display name alone.

## EPIC 3 — Windows integration

### DW-030 — Implement APK file activation
- Status: TODO
- Acceptance:
  - File association registered by installer using supported mechanisms.
  - Double-click passes path correctly.
  - Spaces, Unicode, stale paths, and repeated activation tested.
  - Uninstall removes association as appropriate.

### DW-031 — Implement independent window integration
- Status: PARTIAL (independent windows proven; no host-side chrome or back control)
- Acceptance:
  - Actual app surface is shown, not a mock.
  - Window move/resize/minimize/maximize/close behavior tested.
  - Multiple concurrent app windows tested.
  - Input focus and app crash isolation documented.

### DW-032 — Implement clipboard and file integration
- Status: TODO
- Acceptance:
  - Supported copy/paste scenarios tested.
  - Host file access is explicit and limited.
  - Unsupported scenarios explained to user.
  - Security implications documented.

## EPIC 4 — Quality, release, and documentation

### DW-040 — Add failure and recovery coverage
- Status: PARTIAL
- Acceptance:
  - Runtime missing/crashed, insufficient disk, invalid APK, interrupted install, failed launch, and timeout tested.
  - No permanent loading states.
  - Recovery path is clear.

### DW-041 — UI and accessibility QA
- Status: PARTIAL (DOM/a11y checks done; DPI and visual QA pending)
- Acceptance:
  - Visual review against supplied screenshots.
  - No clipped/overlapping controls.
  - Keyboard navigation and visible focus.
  - DPI and resizing checked.

### DW-042 — Packaging and update workflow
- Status: TODO
- Acceptance:
  - Clean install/uninstall tested.
  - User data preserved on upgrade.
  - File association lifecycle tested.
  - Runtime licensing and redistribution reviewed.

### DW-043 — Compatibility test matrix
- Status: TODO
- Acceptance:
  - Windows version, host architecture, backend version, Android API/ABI, test APK, result, and known limitations recorded.
  - Public claims do not exceed test evidence.

### DW-044 — Release readiness review
- Status: TODO
- Acceptance:
  - Build/tests pass or failures are documented.
  - No fake telemetry or demo data in release.
  - Security and licensing reviewed.
  - Known limitations published.

## Task execution template

For each new task record:
- ID and title
- Status and owner
- Goal
- Files/modules allowed to change
- Dependencies
- Acceptance criteria
- Tests to run
- Actual results
- Follow-up items
