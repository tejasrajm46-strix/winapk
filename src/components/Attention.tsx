import type { AttentionProblem, AttentionRecovery, AttentionReport } from "../backend";
import { IconAlert, IconRefresh } from "./icons";
import { Button, Panel, StatusPill } from "./ui";

const SEVERITY_LABEL: Record<AttentionProblem["severity"], string> = {
  blocking: "Blocking",
  degraded: "Degraded",
  notice: "Notice",
};

const SOURCE_LABEL: Record<string, string> = {
  probe: "probe",
  log: "log",
  event: "event",
  screenshot: "screenshot",
  state: "state",
};

/** A recovery button's text names exactly what the action touches. */
export function recoveryLabel(recovery: AttentionRecovery): string | null {
  switch (recovery.kind) {
    case "start_runtime":
      return "Start runtime";
    case "reopen_window":
      return "Reopen this window";
    case "retry_launch":
      return "Retry launch";
    case "reinstall_package":
      return "Reinstall package";
    case "restart_runtime":
      return "Restart the runtime (last resort — closes every app)";
    case "recheck":
      return "Re-check";
    case "none":
      return null;
  }
}

function RecoveryButton({
  recovery,
  busy,
  onRecover,
}: {
  recovery: AttentionRecovery;
  busy: boolean;
  onRecover: (recovery: AttentionRecovery) => void;
}) {
  const label = recoveryLabel(recovery);
  if (!label) return null;
  return (
    <Button
      size="sm"
      variant={recovery.kind === "restart_runtime" ? "danger" : "secondary"}
      disabled={busy}
      onClick={() => onRecover(recovery)}
    >
      {label}
    </Button>
  );
}

function ProblemCard({
  problem,
  busy,
  onRecover,
}: {
  problem: AttentionProblem;
  busy: boolean;
  onRecover: (recovery: AttentionRecovery) => void;
}) {
  const actions = [problem.recovery, ...(problem.alternatives ?? [])].filter(
    (recovery) => recovery.kind !== "recheck" && recovery.kind !== "none",
  );
  return (
    <li className={`attention attention--${problem.severity}`}>
      <div className="attention__top">
        <StatusPill
          tone={problem.confidence === "confirmed" ? "error" : "warn"}
          label={problem.confidence === "confirmed" ? "Confirmed" : "Suspected"}
        />
        <StatusPill tone="idle" label={SEVERITY_LABEL[problem.severity]} />
        <span className="attention__scope mono">
          {problem.package_id ?? (problem.scope === "environment" ? "environment" : "runtime")}
        </span>
      </div>
      <h3 className="attention__title">{problem.title}</h3>
      <p className="attention__observed">{problem.observed}</p>
      <details className="attention__evidence">
        <summary>
          {problem.evidence.length} piece{problem.evidence.length === 1 ? "" : "s"} of evidence
        </summary>
        <ul>
          {problem.evidence.map((item, index) => (
            <li key={`${item.source}-${index}`}>
              <span className="attention__source mono">{SOURCE_LABEL[item.source] ?? item.source}</span>
              <pre>{item.detail}</pre>
            </li>
          ))}
        </ul>
      </details>
      <p className="attention__recommend">
        <strong>Recommended:</strong> {problem.recommendation}
      </p>
      {actions.length > 0 && (
        <div className="inline attention__actions">
          {actions.map((recovery, index) => (
            <RecoveryButton
              key={`${recovery.kind}-${index}`}
              recovery={recovery}
              busy={busy}
              onRecover={onRecover}
            />
          ))}
        </div>
      )}
    </li>
  );
}

function Group({
  title,
  note,
  problems,
  busy,
  onRecover,
}: {
  title: string;
  note: string;
  problems: AttentionProblem[];
  busy: boolean;
  onRecover: (recovery: AttentionRecovery) => void;
}) {
  if (problems.length === 0) return null;
  return (
    <section className="attention__group" aria-label={title}>
      <div className="attention__group-head">
        <h3>{title}</h3>
        <span className="muted">{note}</span>
      </div>
      <ul className="attention__list">
        {problems.map((problem) => (
          <ProblemCard key={problem.id} problem={problem} busy={busy} onRecover={onRecover} />
        ))}
      </ul>
    </section>
  );
}

/**
 * "Attention Needed": problems WINAPK observed itself, split into what is
 * confirmed and what is only a suspicion, each with its evidence and the most
 * targeted recovery available. A full restart is only ever offered as a labelled
 * last resort.
 */
export function AttentionPanel({
  report,
  busy,
  onCheck,
  onRecover,
}: {
  report: AttentionReport | null;
  busy: boolean;
  onCheck: () => void;
  onRecover: (recovery: AttentionRecovery) => void;
}) {
  const confirmed = report?.problems.filter((problem) => problem.confidence === "confirmed") ?? [];
  const suspected = report?.problems.filter((problem) => problem.confidence === "suspected") ?? [];
  const checkedAt = report ? new Date(report.checked_at_ms) : null;

  return (
    <Panel
      title="Attention Needed"
      note={
        report
          ? `Checked ${checkedAt?.toLocaleTimeString() ?? ""}${report.deep ? " · full check with screenshots" : " · quick check"}`
          : "WINAPK has not run a check in this session yet."
      }
      action={
        <div className="inline">
          <Button size="sm" variant="secondary" onClick={onCheck} disabled={busy}>
            <IconRefresh size={13} />
            {busy ? "Checking…" : "Check now"}
          </Button>
        </div>
      }
    >
      {report === null ? (
        <p className="muted">
          A check looks at the environment, the runtime, each open window, the app events the
          runtime observed and the last failed operation. It reports only what it saw.
        </p>
      ) : report.problems.length === 0 ? (
        <p className="attention__clear">
          <StatusPill tone="ok" label="All clear" />
          Nothing needs attention. {report.checked.length} signal
          {report.checked.length === 1 ? "" : "s"} looked at.
        </p>
      ) : (
        <>
          <div className="attention__summary">
            <span>
              <strong>{report.confirmed}</strong> confirmed
            </span>
            <span>
              <strong>{report.suspected}</strong> suspected
            </span>
            <span className="muted">
              Confirmed means WINAPK measured it. Suspected means it could not observe enough to
              be sure.
            </span>
          </div>

          <Group
            title="Confirmed"
            note="Backed by a direct observation."
            problems={confirmed}
            busy={busy}
            onRecover={onRecover}
          />
          <Group
            title="Suspected"
            note="A clue WINAPK could not confirm — not a diagnosis yet."
            problems={suspected}
            busy={busy}
            onRecover={onRecover}
          />
        </>
      )}

      {report !== null && (
        <details className="attention__checked">
          <summary>What this check looked at</summary>
          <ul>
            {report.checked.map((item, index) => (
              <li key={`${item}-${index}`} className="mono">
                {item}
              </li>
            ))}
          </ul>
        </details>
      )}

      {report !== null && report.problems.length > 0 && (
        <p className="attention__footnote">
          <IconAlert size={13} />
          Reopening, retrying and reinstalling touch only this app. Restarting the runtime is
          offered only when the failure cannot be narrowed to one app.
        </p>
      )}
    </Panel>
  );
}
