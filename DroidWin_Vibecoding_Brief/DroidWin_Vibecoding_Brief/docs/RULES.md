# Vibecoding Rules — DroidWin

## A. Understand before editing
1. Read project memory and the relevant specs first.
2. Inspect the existing code, project files, package versions, and tests before making changes.
3. Identify the root cause and affected modules.
4. State assumptions when a fact is not known.
5. Ask a concise question only when a missing decision blocks safe progress.

## B. Work in small, verifiable increments
1. Prefer one coherent task per change set.
2. Do not generate the entire application in one prompt.
3. Define acceptance criteria before implementation.
4. Build after each meaningful integration.
5. Run focused tests before broad tests.
6. Inspect UI output, not just source code.
7. Record limitations and follow-up work.

## C. Preserve architecture
1. Do not put runtime commands in view models or code-behind.
2. Do not add new frameworks without a reason.
3. Do not duplicate existing services.
4. Do not rewrite stable modules to satisfy personal style preferences.
5. Do not silently change public interfaces or data formats.
6. Use dependency injection where it improves replaceability and testing.
7. Keep runtime-specific behavior inside the adapter.
8. Keep cached metadata distinct from authoritative runtime state.

## D. Truthfulness
1. No fake runtime status.
2. No fake progress percentages.
3. No fake app records in production.
4. No fake “verified” package badges.
5. No fake hardware-acceleration claims.
6. No claiming a feature is implemented because its button exists.
7. No claiming tests passed without running them.
8. Distinguish implemented, mocked, unsupported, planned, and unverified behavior.

## E. UI quality
1. Follow the supplied Kage Zen/Kyoto Zen design reference.
2. Preserve the deep charcoal, restrained vermilion, sage/stone palette and editorial typography.
3. Maintain consistent spacing and alignment.
4. Keep important options visible at normal desktop sizes.
5. Prevent clipping, overflow, overlap, tiny low-contrast text, and hidden primary actions.
6. Provide loading, empty, success, error, and disabled states.
7. Ensure controls are keyboard accessible.
8. Do not use decorative metrics or meaningless cards.
9. Every visible control must work or explain why it is disabled.
10. Bundle fonts/assets appropriately; avoid production dependence on temporary remote image URLs.

## F. Security
1. Treat APKs and their metadata as untrusted.
2. Never execute APK code during inspection.
3. Avoid shell interpolation with untrusted paths.
4. Validate and normalize file paths.
5. Do not silently elevate privileges.
6. Do not bypass package signing, DRM, or app integrity checks.
7. Verify runtime downloads.
8. Respect runtime and dependency licenses.
9. Make app-data deletion explicit.
10. Document the actual isolation model.

## G. Error handling
1. Use typed results or well-defined exceptions at boundaries.
2. Never swallow exceptions.
3. Give the user a concise explanation and a next step.
4. Preserve diagnostic details without leaking secrets.
5. Avoid indefinite loading states.
6. Handle cancellation and timeouts.
7. Make repeated requests safe or reject duplicates explicitly.

## H. Git and agent coordination
1. Keep work in version control.
2. Use branches/worktrees for parallel work when available.
3. Define file ownership per task.
4. Review changes before integration.
5. Do not blindly accept generated diffs.
6. Keep commits focused and descriptive.
7. Update docs with architecture and requirement changes.

## I. Completion report
Every coding task ends with:
- Summary of behavior changed.
- Files changed.
- Acceptance criteria addressed.
- Tests/build commands actually run and results.
- Manual checks performed.
- Known limitations and unverified behavior.
- Next task recommendation.
