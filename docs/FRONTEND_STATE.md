# Where the shell's state lives

`src/App.tsx` used to own every fact the app shows: runtime status, the installed
packages and their retry policy, the runtime installation, the installer's
progress feed, the agent and the attention report. Each new rule for one of them
landed as another conditional in the shell, and the interface between them was
whoever happened to call whom.

The facts that more than one screen reads now have a single owner under
`src/state/`. A screen reads a fact; it never decides when the fact changes.

| Owner | Owns | Reads it from | Re-reads when |
|---|---|---|---|
| `state/runtime.ts` — `useRuntimeState` | `RuntimeStatus`, `EnvReport` | `probeRuntime`, `getRuntimeStatus`, `start_runtime` / `stop_runtime` | its own poll (`nextPollDelay`: 3 s booting, 15 s steady, 30 s unconfigured, 20 s hidden), `runtime://status` events, and an explicit `refresh()` |
| `state/apps.ts` — `useAppLibrary` | `InstalledApp[]`, read error | `list_apps` | `apps://changed`, a transition of the runtime to `ready`, an explicit `refresh()`; a failed read retries five times at 5 s and then stops |
| `state/setup.ts` — `useSetupState` | `SetupReport`, `SetupProgress`, install busy, "open anyway" | `setup_state`, `start_setup`, `cancel_setup`, `set_sdk_location` | `runtime://setup` progress, the runtime reporting `not_configured`, and explicit calls |

`App.tsx` keeps what is genuinely the shell's: navigation, the banner
(`error`/`notice`), the installer's own workflow (inspection, stage feed gated by
`installRunning`, the activation queue), and the agent/attention panels. It
composes the owners and passes their values into `ScreenProps`; it holds no copy
of their facts.

## Rules this shape preserves

- **The shell gate.** `setup.report.state` is the only thing that can replace the
  shell with the setup screen, and `starting` is not one of the three blocking
  states — a booting runtime belongs to the shell, not to an installation prompt.
- **Ready means re-read, not infer.** A transition into `ready` re-reads the
  environment once (the SDK's adb and image are on disk only after a setup) and
  the app list once (the startup read raced the boot). Both are triggered by the
  status value changing; a poll repeating the same status does not re-read.
- **Bounded, never permanent, "apps unknown".** Retries are capped in the owner,
  so no screen has to know the policy.
- **The installer's stage feed belongs to an install this window started.** The
  startup boot emits stages through the same channel; `installRunning` filters
  them at the source, not in a screen.

## Adding to it

A new fact goes in its own module under `src/state/` with an owner, one
`refresh()` and the events that invalidate it. If two owners need each other,
whichever depends on the other takes it as an argument (the app library reads the
runtime's `status`) rather than reaching across. `src/state/alive.ts` is the
shared mount-lifetime guard; nothing else is shared between owners.
