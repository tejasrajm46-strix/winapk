import type { Stage, StageEvent } from "../backend";

const LABEL: Record<Stage, string> = {
  received: "Received",
  validating: "Validating file",
  inspecting: "Inspecting package",
  checking_runtime: "Checking runtime",
  preparing_runtime: "Preparing runtime",
  installing: "Installing package",
  verifying: "Verifying installation",
  launching: "Launching app",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  needs_user_action: "Waiting for you",
};

function markFor(stage: Stage, isLast: boolean, running: boolean): string {
  if (stage === "failed") return "×";
  if (stage === "cancelled") return "—";
  if (stage === "completed") return "✓";
  if (isLast && running) return "▸";
  return "✓";
}

function classNameFor(stage: Stage, isLast: boolean, running: boolean): string {
  if (stage === "failed") return "stage stage--failed";
  if (stage === "cancelled") return "stage stage--failed";
  if (isLast && running) return "stage stage--active";
  return "stage stage--done";
}

function time(ms: number): string {
  return new Date(ms).toLocaleTimeString([], { hour12: false });
}

/**
 * Shows the stages the backend reported, in order. There is deliberately no
 * synthetic progress bar: if the backend has not reported a stage, it is not
 * shown as reached.
 */
export function StageList({ events, running }: { events: StageEvent[]; running: boolean }) {
  if (events.length === 0) {
    return <p className="muted">No operation has run yet.</p>;
  }
  return (
    <ol className="stages" aria-live="polite">
      {events.map((event, index) => {
        const isLast = index === events.length - 1;
        return (
          <li key={`${event.stage}-${event.at_ms}-${index}`} className={classNameFor(event.stage, isLast, running)}>
            <span className="stage__mark" aria-hidden="true">
              {markFor(event.stage, isLast, running)}
            </span>
            <span>
              <span className="stage__name">{LABEL[event.stage]}</span>
              <span className="stage__message">{event.message}</span>
            </span>
            <span className="stage__time">{time(event.at_ms)}</span>
          </li>
        );
      })}
    </ol>
  );
}
