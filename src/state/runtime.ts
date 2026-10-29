import { useCallback, useEffect, useRef, useState } from "react";
import * as backend from "../backend";
import type { EnvReport, RuntimeStatus } from "../backend";
import { nextPollDelay } from "../polling";
import { useAlive } from "./alive";

/**
 * The Android runtime as one fact: the status the shell renders, the environment
 * it was measured in, and the three ways it can change (a poll, a status event
 * from the backend, an explicit start or stop).
 *
 * Nothing here knows about apps, setup or screens. Screens read `status` and
 * `env`; the app library and the setup report watch `status` for the transitions
 * that concern them; `refresh` is what a user action or a post-install step asks
 * for when it wants the truth again.
 */
export interface RuntimeOptions {
  /** Report a failed action or probe. Poll failures stay quiet: a poll that
   *  did not answer is not a new runtime state. */
  onError: (err: unknown) => void;
  /** Runs after every successful poll, for readers that ride on the heartbeat
   *  rather than having a poll of their own (the agent endpoint). */
  onPoll?: () => void;
}

export function useRuntimeState({ onError, onPoll }: RuntimeOptions) {
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [env, setEnv] = useState<EnvReport | null>(null);
  const alive = useAlive();
  const report = useRef(onError);
  report.current = onError;
  const polled = useRef(onPoll);
  polled.current = onPoll;
  /** The last status any source produced, so a boot sends one re-probe rather
   *  than one per event. */
  const known = useRef<RuntimeStatus | null>(null);

  const accept = useCallback((next: RuntimeStatus) => {
    known.current = next;
    setStatus(next);
  }, []);

  /** Probe status and environment together. `quiet` is for the re-probe after a
   *  boot: it is housekeeping, not something the user asked for. */
  const probe = useCallback(
    async (quiet: boolean) => {
      try {
        const next = await backend.probeRuntime();
        if (!alive.current) return;
        accept(next.status);
        setEnv(next.env);
      } catch (err) {
        if (!quiet) report.current(err);
      }
    },
    [accept, alive],
  );

  /** A full read: what the Refresh button and every post-install step want. */
  const refresh = useCallback(() => probe(false), [probe]);

  /** True when the runtime answered with a new status. */
  const start = useCallback(async () => {
    try {
      const next = await backend.startRuntime();
      if (!alive.current) return true;
      accept(next);
      return true;
    } catch (err) {
      report.current(err);
      return false;
    }
  }, [accept, alive]);

  const stop = useCallback(async () => {
    try {
      await backend.stopRuntime();
      const next = await backend.getRuntimeStatus();
      if (!alive.current) return true;
      accept(next);
      return true;
    } catch (err) {
      report.current(err);
      return false;
    }
  }, [accept, alive]);

  // A steady runtime is re-read slowly, a booting one quickly, and a window
  // nobody is looking at almost never: each poll is an adb process on the Rust
  // side. The timer pauses while the window is hidden.
  useEffect(() => {
    if (!backend.isDesktop()) return;
    let timer: number | undefined;
    let cancelled = false;

    const schedule = (delay: number) => {
      timer = window.setTimeout(() => void tick(), delay);
    };

    const tick = async () => {
      if (cancelled) return;
      if (document.visibilityState === "hidden") {
        schedule(nextPollDelay(null, true));
        return;
      }
      let delay = nextPollDelay(null, false);
      try {
        const next = await backend.getRuntimeStatus();
        if (cancelled || !alive.current) return;
        accept(next);
        delay = nextPollDelay(next.state, false);
        polled.current?.();
      } catch {
        // Keep the last reported state; a failed poll is not a state change.
        delay = 20_000;
      }
      if (!cancelled) schedule(delay);
    };

    schedule(8_000);
    const onVisibility = () => {
      if (cancelled || document.visibilityState !== "visible") return;
      window.clearTimeout(timer);
      void tick();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [accept, alive]);

  // The backend announces state changes it did not wait for (a boot finishing in
  // the background). A runtime that just reached Ready has tools on disk that
  // were not there at the first probe, so the environment is read once on that
  // transition — that is what keeps "missing pieces" off the installer screen
  // after a successful setup.
  useEffect(() => {
    if (!backend.isDesktop()) return;
    let stop: (() => void) | undefined;
    void backend
      .onRuntimeStatus((next) => {
        const wasReady = known.current?.state === "ready";
        known.current = next;
        if (!alive.current) return;
        setStatus(next);
        if (next.state === "ready" && !wasReady) void probe(true);
      })
      .then((fn) => {
        if (alive.current) stop = fn;
        else fn();
      });
    return () => stop?.();
  }, [alive, probe]);

  return { status, env, refresh, start, stop };
}
