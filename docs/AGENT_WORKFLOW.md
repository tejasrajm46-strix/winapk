# Agent workflow — OMC evaluation and what was used instead

## What was asked

Load `oh-my-claudecode` (OMC) into Freebuff for multi-agent work, then finish WINAPK.

## What OMC actually is (checked, not assumed)

From the project's own README (github.com/Yeachan-Heo/oh-my-claudecode, read 2026-10-25):

- It is **multi-agent orchestration for Claude Code**. The install paths are the Claude Code
  marketplace (`/plugin marketplace add …`, `/plugin install oh-my-claudecode`) or the npm CLI
  package `oh-my-claude-sisyphus`, which installs `omc`.
- Its in-session features (`/autopilot`, `/team`, `/execute`, `/ralph`) are **Claude Code slash
  commands** and require an active Claude Code session. The CLI surface (`omc setup`, `omc ask`,
  `omc team`) launches **tmux** panes running Claude/Codex/Gemini/Cursor CLIs.
- The README explicitly says the npm package exports helpers for programs built on
  `@anthropic-ai/claude-agent-sdk`; it does not expose a generic orchestrator API for a different
  host application.

## State of this machine

| Piece | Present? |
| --- | --- |
| `claude` CLI | yes (`~/AppData/Roaming/npm/claude`) |
| `omc` | no |
| `tmux` | no |
| Freebuff host application | running this session; its source is not in this repository |

## Conclusion: it cannot be loaded *into* Freebuff

Freebuff is the host process, not a Claude Code instance. OMC's agents, skills and hooks execute
inside Claude Code (or inside tmux-hosted CLI panes); there is no supported interface through which
this workspace can register OMC as Freebuff's engine, and copying OMC's skill files into the repo
would not reproduce its hooks, tools or session state — it would only look like integration.

Installing `omc` globally and authenticating Claude would still leave two separate tools: OMC in a
terminal, Freebuff here. That is a valid side-by-side setup, not a plugin load, and it was not done
because it changes the user's global toolchain without adding capability to this build.

## What was used instead — the same pipeline, Freebuff's own agents

OMC's durable idea is a staged pipeline: plan → interface first → parallel execution → integration →
verification → fix loop. That was applied directly:

1. **Plan and contracts before parallel work.** Shared types and signatures (`AppState`,
   `pending_activation`, `install_apk(..., allow_old_target)`, `uninstall_app` returning the
   runtime's message, `AppRow` props) were settled by the coordinator first.
2. **Parallel file-owned slices.** Two worker agents ran on disjoint files:
   - slice 1: uninstall keep/delete dialog (`UninstallDialog.tsx`, `Applications.tsx`,
     `common.tsx`, `base.css`, `uninstall.test.tsx`);
   - slice 2: APK archive-guard tests (`apk/mod.rs` test module only).
   Each worker was told which files it owned and which names it must use. Their reports named every
   edited file.
3. **Integration by one owner.** The coordinator reconciled the slices, fixed cross-slice mismatches
   (the `onInstall` signature, the runtime's new uninstall return type) and wrote the backend fixes
   itself.
4. **Verification, then repair, then re-verify.** `cargo fmt`, `cargo clippy -D warnings`,
   `cargo test`, `tsc`, `vitest`, `tauri build`, live emulator runs, and native window captures.
   Failures found by those checks were fixed rather than waived (see `IMPLEMENTATION_AUDIT.md` §2).

This is the behaviour the OMC request was aiming at: task-routed agent work with a single integration
owner and evidence-based completion — without pretending a Claude Code plugin is loaded in Freebuff.

## Project memory convention

The repository already carries the equivalent of OMC's persistent state, and it was kept updated
rather than duplicated: `AGENTS.md`, `docs/MEMORY.md`, `docs/DECISIONS_LOG.md`, `docs/TASKS.md`,
`docs/IMPLEMENTATION_AUDIT.md`. No `.claude/` or `omc.jsonc` files were added, because nothing here
would read them.

## If OMC is wanted alongside Freebuff

```bash
npm i -g oh-my-claude-sisyphus@latest     # installs omc
omc setup                                  # writes ~/.config/claude-omc
omc team 2:claude "review src-tauri/src/runtime"   # needs tmux + Claude auth
```

That runs in a terminal of its own; it does not become Freebuff's engine, and its workers cannot see
this conversation's context unless given it explicitly.
