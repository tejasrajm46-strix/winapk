import { IconClock, IconRefresh } from "../components/icons";
import { Button, EmptyState, Panel, StatusPill } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";

function stamp(ms: number): string {
  return new Date(ms).toLocaleString([], {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

function tone(status: string) {
  if (status === "completed") return "ok" as const;
  if (status === "failed" || status === "launch_failed") return "error" as const;
  return "idle" as const;
}

export function Activity(props: ScreenProps) {
  const { activity, busy } = props;
  return (
    <div className="page">
      <PageHead
        eyebrow="Management"
        title="Activity"
        lede="Every operation DroidWin ran, with its own outcome and diagnostic detail."
      >
        <Button variant="ghost" onClick={props.onRefresh} disabled={busy}>
          <IconRefresh size={14} />
          Refresh
        </Button>
      </PageHead>

      <Panel title="Operation history" flush note={`${activity.length} recorded`}>
        {activity.length === 0 ? (
          <div style={{ padding: "var(--space-lg)" }}>
            <EmptyState
              title="Nothing recorded yet"
              body="Installing, launching and uninstalling an app each leave a record here — with the runtime's own result, not a hopeful one."
            />
          </div>
        ) : (
          <ul className="rows">
            {activity.map((op) => (
              <li className="row" key={op.id} style={{ alignItems: "start" }}>
                <div className="row__icon" aria-hidden="true">
                  <IconClock size={15} />
                </div>
                <div className="row__main">
                  <div className="row__title">{op.summary}</div>
                  <div className="row__meta">
                    <span className="mono">{op.id}</span>
                    <span className="mono">{op.kind}</span>
                    <span>{stamp(op.started_at)}</span>
                    {op.finished_at && <span>{Math.max(0, op.finished_at - op.started_at)} ms</span>}
                  </div>
                  {op.detail && (
                    <details style={{ marginTop: 6 }}>
                      <summary className="muted" style={{ cursor: "pointer", fontSize: 13 }}>
                        Technical detail
                      </summary>
                      <pre className="note__detail">{op.detail}</pre>
                    </details>
                  )}
                </div>
                <div className="row__actions">
                  <StatusPill tone={tone(op.status)} label={op.status.replace("_", " ")} />
                </div>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}
