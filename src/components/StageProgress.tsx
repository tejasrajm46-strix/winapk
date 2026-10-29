import { useEffect, useState } from "react";
import type { Stage, StageEvent } from "../backend";

/**
 * Operation progress. The segments map one-to-one onto backend stages, and a
 * segment only fills after its stage was reported. There is no percentage and no
 * timer-driven simulation: if the backend has not reported a stage, it is still
 * pending.
 */

export type PhaseId = "prepare" | "start" | "install" | "launch" | "ready";

export interface Phase {
  id: PhaseId;
  label: string;
  detail: string;
  stages: Stage[];
}

export const PHASES: Phase[] = [
  {
    id: "prepare",
    label: "Preparing runtime",
    detail: "Package checks, inspection and SDK",
    stages: ["received", "validating", "inspecting", "checking_runtime"],
  },
  {
    id: "start",
    label: "Starting Android",
    detail: "Virtual device and boot",
    stages: ["preparing_runtime"],
  },
  {
    id: "install",
    label: "Installing and verifying",
    detail: "Android package manager",
    stages: ["installing", "verifying"],
  },
  {
    id: "launch",
    label: "Launching app",
    detail: "Virtual display and window",
    stages: ["launching"],
  },
  {
    id: "ready",
    label: "Ready",
    detail: "Window open",
    stages: ["completed"],
  },
];

export type PhaseStatus = "done" | "active" | "pending" | "failed";

export interface PhaseState extends Phase {
  index: number;
  status: PhaseStatus;
}

/** The phase a stage belongs to; terminal and waiting states belong to none. */
export function phaseFor(stage: Stage): PhaseId | null {
  if (stage === "completed") return "ready";
  return PHASES.find((phase) => phase.stages.includes(stage))?.id ?? null;
}

/**
 * Work out what is done, active, pending or failed from the reported events.
 * A failure marks the phase that was in progress, not the one after it.
 */
export function phaseStates(events: StageEvent[], running: boolean): PhaseState[] {
  const work = events.filter(
    (event) => event.stage !== "failed" && event.stage !== "cancelled",
  );
  const finished = events.some((event) => event.stage === "completed");
  const failed = events.some(
    (event) => event.stage === "failed" || event.stage === "cancelled",
  );
  const lastPhase = work.length > 0 ? phaseFor(work[work.length - 1].stage) : null;
  const reached = lastPhase ? PHASES.findIndex((phase) => phase.id === lastPhase) : -1;

  return PHASES.map((phase, index) => {
    let status: PhaseStatus = "pending";
    if (finished) {
      status = "done";
    } else if (index < reached) {
      status = "done";
    } else if (index === reached) {
      status = failed ? "failed" : running ? "active" : "done";
    }
    return { ...phase, index, status };
  });
}

function formatElapsed(seconds: number | null): string {
  if (seconds === null) return "—";
  const minutes = Math.floor(seconds / 60);
  const rest = seconds % 60;
  return `${minutes}:${rest.toString().padStart(2, "0")}`;
}

/**
 * Elapsed time between the first stage and the terminal stage (or now, while the
 * operation is still running). It measures the operation, not the render.
 */
function useElapsed(events: StageEvent[], running: boolean): number | null {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    if (!running) return;
    const timer = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(timer);
  }, [running]);

  if (events.length === 0) return null;
  const start = events[0].at_ms;
  const end =
    events.find(
      (event) =>
        event.stage === "completed" ||
        event.stage === "failed" ||
        event.stage === "cancelled",
    )?.at_ms ?? now;
  return Math.max(0, Math.round((end - start) / 1000));
}

export function StageProgress({
  events,
  running,
}: {
  events: StageEvent[];
  running: boolean;
}) {
  const elapsed = useElapsed(events, running);
  if (events.length === 0) return null;

  const phases = phaseStates(events, running);
  const done = phases.filter((phase) => phase.status === "done").length;
  const current =
    phases.find((phase) => phase.status === "failed") ??
    phases.find((phase) => phase.status === "active");

  return (
    <div className="progress">
      <div className="progress__head">
        <span className="progress__eyebrow">
          {current && current.status === "failed"
            ? `Stopped at ${current.label.toLowerCase()}`
            : current
              ? current.label
              : "Complete"}
        </span>
        <span className="progress__time" title="Elapsed time for this operation">
          {formatElapsed(elapsed)}
        </span>
      </div>

      <div
        className="progress__track"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={phases.length}
        aria-valuenow={done}
        aria-valuetext={current ? current.label : "Complete"}
        aria-live="polite"
      >
        {phases.map((phase) => (
          <div
            className={`progress__step progress__step--${phase.status}`}
            key={phase.id}
            data-status={phase.status}
          >
            <span className="progress__bar" aria-hidden="true" />
            <span className="progress__label">
              <span className="progress__index">{phase.index + 1}</span>
              {phase.label}
            </span>
            <span className="progress__detail">{phase.detail}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
