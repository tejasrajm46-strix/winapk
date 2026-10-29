import { useCallback, useEffect, useRef, useState } from "react";
import * as backend from "../backend";
import type { RuntimeStatus, SetupProgress, SetupReport } from "../backend";
import { useAlive } from "./alive";

interface SetupOptions {
  /** The runtime's current status: the setup report follows it, because the SDK
   *  can appear while the setup screen is open. */
  status: RuntimeStatus | null;
  onError: (err: unknown) => void;
  /** The runtime changed behind the shell's back (setup finished), so the
   *  runtime owner should read it again. */
  onRuntimeChanged: () => void;
}

/**
 * The Android runtime installation as one fact: what is installed, an
 * installation running right now, and the user's choice to look at the app
 * before installing anything.
 *
 * This is the only place that can install, move the SDK root or cancel, so the
 * setup screen is a pure projection of `report`/`progress` and the shell only
 * has to ask whether the report blocks it.
 */
export function useSetupState({ status, onError, onRuntimeChanged }: SetupOptions) {
  const [report, setReport] = useState<SetupReport | null>(null);
  const [progress, setProgress] = useState<SetupProgress | null>(null);
  const [busy, setBusy] = useState(false);
  // "Open DroidWin anyway": a person who wants to look at the app before
  // installing anything is not trapped on the setup screen for the session.
  const [dismissed, setDismissed] = useState(false);
  const alive = useAlive();
  const reportError = useRef(onError);
  reportError.current = onError;
  const runtimeChanged = useRef(onRuntimeChanged);
  runtimeChanged.current = onRuntimeChanged;

  const refresh = useCallback(async () => {
    if (!backend.isDesktop()) return;
    try {
      const next = await backend.getSetupState();
      if (alive.current) setReport(next);
    } catch {
      // The last answer stays; a failed check is not a state change.
    }
  }, [alive]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    if (!backend.isDesktop()) return;
    let stop: (() => void) | undefined;
    void backend
      .onSetupProgress((event) => {
        setProgress(event);
        // A finished step changes what is installed, so both reports are
        // re-read rather than patched from the event.
        if (event.done) {
          void refreshRef.current();
          runtimeChanged.current();
        }
      })
      .then((fn) => {
        if (alive.current) stop = fn;
        else fn();
      });
    return () => stop?.();
  }, [alive]);

  useEffect(() => {
    if (status?.state === "not_configured") void refreshRef.current();
  }, [status]);

  /** Start an installation. Returns false when it could not be started, so the
   *  caller knows not to announce a result. */
  const install = useCallback(
    async (acceptLicenses: boolean) => {
      setBusy(true);
      setProgress(null);
      try {
        const next = await backend.startSetup(acceptLicenses);
        if (!alive.current) return false;
        setReport(next);
        return true;
      } catch (err) {
        if (alive.current) reportError.current(err);
        void refreshRef.current();
        return false;
      } finally {
        if (alive.current) setBusy(false);
      }
    },
    [alive],
  );

  const cancel = useCallback(async () => {
    try {
      await backend.cancelSetup();
    } catch (err) {
      reportError.current(err);
    }
  }, []);

  const setLocation = useCallback(
    async (path: string | null) => {
      try {
        const next = await backend.setSdkLocation(path);
        if (alive.current) setReport(next);
      } catch (err) {
        reportError.current(err);
      }
    },
    [alive],
  );

  const chooseFolder = useCallback(async () => {
    try {
      const path = await backend.pickSdkFolder();
      if (path) await setLocation(path);
    } catch (err) {
      reportError.current(err);
    }
  }, [setLocation]);

  return {
    report,
    progress,
    busy,
    dismissed,
    dismiss: () => setDismissed(true),
    /** A retry starts from a clean progress strip. */
    clearProgress: () => setProgress(null),
    refresh,
    install,
    cancel,
    chooseFolder,
    useManaged: () => setLocation(null),
  };
}
