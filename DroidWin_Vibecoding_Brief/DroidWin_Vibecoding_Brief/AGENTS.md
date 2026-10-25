# AGENTS.md — Mandatory Instructions for All Coding Agents

## Mission

Build DroidWin: a polished Windows desktop application that installs and launches compatible Android APKs with as little manual setup as possible. The target user should not need to operate an emulator manually or issue terminal commands for ordinary app installation and launch.

The desired workflow is:

1. User double-clicks an APK associated with DroidWin, or drops an APK into the app.
2. Windows passes the APK path to DroidWin.
3. DroidWin validates and inspects the package without executing it.
4. DroidWin checks runtime readiness and explains setup if needed.
5. DroidWin installs or updates the package using the selected runtime.
6. DroidWin launches the actual Android application in an independently managed desktop window, if supported by the chosen backend.
7. App metadata and user settings persist. App data is not silently deleted.

## Read before acting

Before every task, read:
- `docs/MEMORY.md`
- `docs/PRD.md`
- `docs/ARCHITECTURE.md`
- `docs/UI_UX_SPEC.md` for UI work
- `docs/RUNTIME_DECISIONS.md` for runtime work
- `docs/RULES.md`
- `docs/TASKS.md`

Inspect the existing repository and current implementation before editing. Do not assume a blank repository or invent files, APIs, runtime capabilities, test results, or package behavior.

## Non-negotiable product truths

- A UI mock, screenshot, embedded image, web page, or placeholder process is not an Android execution engine.
- “Installed”, “Running”, “Runtime Ready”, progress percentages, compatibility status, and acceleration status must be derived from actual backend responses.
- A conventional emulator may not provide independent app windows. Prove this before designing around it.
- Do not promise all APKs will work. Compatibility depends on Android API level, CPU ABI, native libraries, Google Play Services, DRM, graphics, permissions, and the runtime.
- The supplied Stitch design is inspiration and a UI specification source, not authorization to copy fake telemetry or unsupported controls.
- Do not start a custom Android compatibility engine as a side effect of ordinary UI work. It requires an explicit research decision.

## Architecture constraints

- Keep presentation, application/domain services, persistence, Windows integration, and runtime-specific implementation separated.
- UI code must not contain raw runtime commands or package-manager shell strings.
- Route runtime operations through a typed adapter interface.
- The runtime is authoritative for installed package state. Reconcile local metadata with runtime state.
- Long-running work must be asynchronous, cancellable where safe, and expose explicit state transitions.
- Do not overengineer: add abstractions only for real boundaries or demonstrated testability needs.
- Do not add dependencies without checking compatibility, maintenance status, licensing, and necessity.
- Preserve existing contracts unless an approved architecture decision changes them.

## Coding rules

- Use clear, maintainable C#/.NET code and the agreed Windows UI framework.
- Prefer small cohesive services and explicit types over giant view models or god classes.
- Validate file paths, package metadata, command arguments, and external process results.
- Never concatenate untrusted user input into shell commands. Prefer argument-list APIs and strict allowlists.
- Never swallow exceptions silently. Show a useful user message and record diagnostic detail without leaking secrets.
- Avoid arbitrary sleeps and fake progress. Use real process/event state, polling with bounded timeouts, or supported backend APIs.
- Dispose processes, streams, subscriptions, and cancellation sources correctly.
- Avoid blocking the UI thread.
- Do not delete app data during updates or ordinary uninstall unless the user explicitly chooses data deletion.
- Do not silently elevate privileges or weaken package-signature validation.
- Do not bundle proprietary components without verifying redistribution rights.

## UI rules

- Follow `docs/UI_UX_SPEC.md`; do not redesign the product on a whim.
- Preserve the supplied Kyoto Zen / Kage Zen visual direction: deep charcoal surfaces, restrained vermilion accents, muted sage/stone secondary tones, thin borders, editorial serif headings paired with readable sans-serif interface text, deliberate spacing, and dense but legible technical panels.
- Do not use generic glowing gradients, gratuitous glassmorphism, random rounded-card grids, oversized hero panels, or decorative charts.
- Ensure important buttons and options are visible at normal desktop sizes. Avoid clipped labels, overlapping content, tiny low-contrast metadata, and hidden hover-only actions.
- Use real icons/assets where licensed and available. Do not hotlink reference image URLs in production.
- Support keyboard navigation, visible focus, accessible labels, sensible minimum window sizes, and DPI scaling.
- All visible controls must work or be visibly disabled with a reason. No dead buttons.

## Testing and verification

For every change:
1. Identify affected requirements and modules.
2. Implement the smallest coherent change.
3. Add/update tests for behavior and regressions.
4. Run relevant tests and build commands where available.
5. Inspect the result at the UI level for spacing, alignment, contrast, overflow, and state handling.
6. Report actual commands and outcomes. If a test was not run, say so.

Never claim “tested”, “working”, “fixed”, or “complete” without evidence. A successful build is not proof of runtime compatibility.

## Agent collaboration

- Assign one owner per file/module per task. Parallelize only across clear boundaries.
- Do not let multiple agents rewrite the same core files simultaneously.
- Make changes in separate branches/worktrees when available.
- Define task input, allowed files, interfaces, acceptance criteria, and tests before delegation.
- Review integrations and resolve conflicts intentionally; do not blindly accept generated changes.
- Do not introduce unrelated features while fixing a bug.
- Update `docs/MEMORY.md`, `docs/DECISIONS_LOG.md`, and `docs/TASKS.md` when durable facts, decisions, or task status change.

## Definition of done

A task is done only when:
- Its acceptance criteria are met.
- Relevant tests were run and results are reported.
- Errors and edge cases are handled.
- No unrelated requirements regressed.
- Documentation is updated where needed.
- Any remaining limitations are explicit.
