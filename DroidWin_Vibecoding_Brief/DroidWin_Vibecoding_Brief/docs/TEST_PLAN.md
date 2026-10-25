# Test Plan — DroidWin

## 1. Test layers

- Unit: domain logic, validation, state transitions, settings, repositories.
- Contract: runtime adapter behavior and error normalization.
- Integration: real runtime operations and APK workflows.
- UI/manual: layout, accessibility, focus, resizing, dialogs, DPI.
- Release: clean install, upgrade, uninstall, file association, data preservation.

## 2. APK inspection cases

- Valid APK.
- Empty file renamed to `.apk`.
- Corrupted ZIP.
- Missing manifest.
- Malformed manifest/metadata.
- Very large archive or suspicious expansion ratio.
- Unicode and long file path.
- Path with spaces.
- File moved/deleted after selection.
- Unsupported package metadata.
- Package with multiple ABIs.
- Package whose signature metadata cannot be verified.

Expected: no code execution during inspection; clear errors; no crash; no invented metadata.

## 3. Installation and update cases

- Runtime ready; install valid APK.
- Runtime missing.
- Runtime unavailable or unhealthy.
- Insufficient disk space.
- Installation cancelled.
- Backend process fails.
- Operation times out.
- Same package already installed.
- Valid update with compatible signing identity.
- Update with incompatible signature.
- Downgrade attempt.
- Interrupted update.
- Repeated click/double-submit.
- Uninstall with data preserved.
- Uninstall with explicit data deletion.

Expected: accurate state, recoverable failure, no silent data deletion, backend signing checks respected.

## 4. Launch and window cases

- Launch installed app.
- Launch unknown package ID.
- App crashes at startup.
- App becomes unresponsive.
- Close window.
- Minimize/maximize/resize.
- Two apps open concurrently.
- Keyboard input and shortcuts.
- Mouse click/scroll.
- Text entry.
- Portrait/landscape.
- DPI 100%, 125%, 150% where available.
- Manager UI remains responsive during app execution.

Expected: actual app execution, no fake windows, clear failure behavior, no manager crash when app crashes.

## 5. Clipboard and file cases

- Copy text from host and paste into Android app.
- Copy text from Android app and paste into host.
- Unsupported clipboard content.
- Share a permitted file.
- Attempt access outside permitted shared location.
- File unavailable or moved.
- Large file transfer.
- Runtime restarted during transfer.

Expected: supported operations work; unsupported operations are explained; file boundaries remain enforced.

## 6. Persistence and recovery

- Close/reopen DroidWin.
- Reboot Windows.
- Runtime stopped unexpectedly.
- Reconcile local cache with backend package list.
- Upgrade DroidWin.
- Uninstall DroidWin.
- Restore after interrupted operation.
- Database migration from prior schema.
- Corrupted settings/database recovery.

Expected: app metadata and preferences persist; runtime state is reconciled; data is not silently deleted.

## 7. UI quality checks

- Compare screens to supplied design reference.
- Verify typography and color tokens.
- Check alignment and spacing.
- Check clipping, overlap, scroll behavior, and minimum window size.
- Check long app names and package IDs.
- Check long logs and error text.
- Check empty/loading/success/error/disabled states.
- Check keyboard navigation and visible focus.
- Check contrast and status icons.
- Verify no fake production telemetry.
- Verify every visible control works or is explicitly disabled.

## 8. Test reporting

For each run record:
- Date/environment.
- Windows version and CPU architecture.
- Runtime/backend version.
- APK name, package ID, version, API/ABI if known.
- Test steps.
- Expected result.
- Actual result.
- Logs/artifact references.
- Pass/fail/blocked.
- Known limitations.

## 9. Automated checks in this repository

| Check | Command | Covers |
| --- | --- | --- |
| Rust unit tests | `cargo test --manifest-path src-tauri/Cargo.toml` | operation stage transitions, AXML manifest parsing (valid, resource-reference label, truncated, wrong chunk, absurd sizes), store migrations and label retention, adb typed errors and timeouts, SDK root validation, redaction |
| Rust lint | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` | no warnings allowed in the tree |
| UI type check | `pnpm exec tsc --noEmit` | the typed backend contract matches every screen |
| UI rendering tests | `pnpm test` | honest states: missing SDK never shows "ready", undetermined manifest fields say so, signature presence is not validity, running state comes from the window registry, empty library is empty |
| Runtime feasibility | `cargo run --manifest-path src-tauri/Cargo.toml --bin proof` | Gate A/B against a real emulator: inspect, install, launch, two independent windows, OS-level move/resize, independent close, survival across a manager restart, full shutdown |

Cases in §2–§7 that are **not** yet automated: corrupted-APK fuzzing, decompression-bomb inputs,
DPI scaling, clipboard and file transfer, reboot recovery, installer lifecycle. They remain manual
until the features they cover exist.
