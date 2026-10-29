import { useCallback, useEffect, useRef, useState } from "react";
import * as backend from "../backend";
import type { DwError, InstalledApp, RuntimeStatus } from "../backend";
import { useAlive } from "./alive";

/** How many times a failed list read is retried, and how far apart. */
const RETRIES = 5;
const RETRY_DELAY_MS = 5_000;

/**
 * The installed-app library as one fact: the list, the read error, and the rules
 * for when to read it again.
 *
 * Two of those rules are subtle enough that they used to live in the shell's
 * event handlers, where they were easy to lose: a list read can lose the race
 * with a boot (the package manager is still settling), so a failure is retried a
 * bounded number of times instead of leaving "apps unknown" under a READY header;
 * and a runtime that becomes ready re-reads the list once, because the startup
 * read ran against an emulator that was not up yet.
 */
export function useAppLibrary({ status }: { status: RuntimeStatus | null }) {
  const [apps, setApps] = useState<InstalledApp[] | null>(null);
  const [appsError, setAppsError] = useState<DwError | null>(null);
  const alive = useAlive();
  // A list read costs adb work, and several callers ask at once (an event, the
  // ready transition, an action). Only one read runs.
  const inFlight = useRef(false);
  const retries = useRef(0);
  const retryTimer = useRef<number | null>(null);
  const refreshRef = useRef<() => Promise<void>>(async () => {});
  const seenReady = useRef(false);

  const refresh = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const list = await backend.listApps();
      if (!alive.current) return;
      setApps(list);
      setAppsError(null);
      retries.current = 0;
    } catch (err) {
      if (!alive.current) return;
      setApps(null);
      setAppsError(backend.toDwError(err));
      if (retries.current < RETRIES && retryTimer.current === null) {
        retries.current += 1;
        retryTimer.current = window.setTimeout(() => {
          retryTimer.current = null;
          void refreshRef.current();
        }, RETRY_DELAY_MS);
      }
    } finally {
      inFlight.current = false;
    }
  }, [alive]);

  refreshRef.current = refresh;

  useEffect(
    () => () => {
      if (retryTimer.current !== null) window.clearTimeout(retryTimer.current);
    },
    [],
  );

  // The backend reports apps changed by something other than this window (a
  // dropped package, a collapsed window host).
  useEffect(() => {
    if (!backend.isDesktop()) return;
    let stop: (() => void) | undefined;
    void backend
      .onAppsChanged(() => void refreshRef.current())
      .then((fn) => {
        if (alive.current) stop = fn;
        else fn();
      });
    return () => stop?.();
  }, [alive]);

  // The startup read raced the boot. Once the runtime is ready, read once more;
  // polls repeat the same state object only in value, so the transition is what
  // triggers this, not every poll.
  useEffect(() => {
    if (!status) return;
    const ready = status.state === "ready";
    if (ready && !seenReady.current) void refreshRef.current();
    seenReady.current = ready;
  }, [status]);

  return { apps, appsError, refresh };
}
