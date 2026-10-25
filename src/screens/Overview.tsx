import { DropZone } from "../components/DropZone";
import { IconClock, IconRefresh } from "../components/icons";
import { Button, EmptyState, ErrorNote, Panel, StatusPill } from "../components/ui";
import { runtimeTone } from "../components/AppShell";
import { PageHead, AppRow, type ScreenProps } from "./common";

function relative(ms: number): string {
  const delta = Date.now() - ms;
  const minutes = Math.round(delta / 60000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours} h ago`;
  return new Date(ms).toLocaleDateString();
}

export function Overview(props: ScreenProps) {
  const { status, apps, appsError, activity, busy } = props;
  const runtime = runtimeTone(status);
  const installed = apps ?? [];
  const preview = installed.slice(0, 4);

  return (
    <div className="page">
      <PageHead
        eyebrow="Workspace"
        title="Your Android apps"
        lede="Install a package, and DroidWin opens the real Android app in a window of its own."
      >
        <Button variant="ghost" onClick={props.onRefresh} disabled={busy}>
          <IconRefresh size={14} />
          Refresh
        </Button>
      </PageHead>

      {props.error && <ErrorNote error={props.error} />}
      {props.notice && <div className="note note--info">{props.notice}</div>}

      <div className="grid grid--aside">
        <Panel title="Install a package" note="APK files only. Nothing runs during inspection.">
          <DropZone onBrowse={props.onBrowse} busy={busy} />
        </Panel>

        <Panel
          title="Runtime"
          action={<StatusPill tone={runtime.tone} label={runtime.label} />}
          note={status?.backend}
        >
          <p className="secondary">{status?.detail ?? "The runtime has not been probed yet."}</p>
          <dl className="fields">
            <div className="field">
              <dt className="field__key">Device</dt>
              <dd className="field__value">{status?.avd ?? "—"}</dd>
            </div>
            <div className="field">
              <dt className="field__key">Android</dt>
              <dd className="field__value">
                {status?.android_api ? `API ${status.android_api}` : "Not running"}
              </dd>
            </div>
            <div className="field">
              <dt className="field__key">Windows</dt>
              <dd className="field__value">
                {status?.capabilities.independent_windows
                  ? "Independent per app, verified"
                  : "Not verified on this machine"}
              </dd>
            </div>
          </dl>
          <div className="inline">
            {status?.state === "ready" ? (
              <Button variant="secondary" onClick={() => props.onNavigate("runtime")}>
                Runtime details
              </Button>
            ) : (
              <Button
                variant="primary"
                onClick={props.onStartRuntime}
                disabled={busy || status?.state === "not_configured"}
                disabledReason={
                  status?.state === "not_configured"
                    ? status.limitations.filter((l) => l.includes("sdkmanager") || l.includes("SDK")).join(" ")
                    : undefined
                }
              >
                {status?.state === "starting" ? "Starting…" : "Start runtime"}
              </Button>
            )}
            <Button variant="ghost" onClick={() => props.onNavigate("installer")}>
              Open installer
            </Button>
          </div>
        </Panel>
      </div>

      <Panel
        title="Installed applications"
        flush
        action={
          <Button size="sm" variant="ghost" onClick={() => props.onNavigate("applications")}>
            All applications
          </Button>
        }
      >
        {appsError ? (
          <div style={{ padding: "var(--space-lg)" }}>
            <ErrorNote error={appsError} />
          </div>
        ) : apps === null ? (
          <div style={{ padding: "var(--space-lg)" }} className="muted">
            Reading the runtime's package list…
          </div>
        ) : preview.length === 0 ? (
          <div style={{ padding: "var(--space-lg)" }}>
            <EmptyState
              title="No apps yet"
              body="Install an APK and it will appear here, with its icon, package name and window state."
              action={
                <Button variant="secondary" onClick={props.onBrowse}>
                  Choose an APK
                </Button>
              }
            />
          </div>
        ) : (
          <ul className="rows">
            {preview.map((app) => (
              <AppRow
                key={app.package_id}
                app={app}
                busy={busy}
                onLaunch={props.onLaunch}
                onStopApp={props.onStopApp}
                onUninstall={props.onUninstall}
                compact
              />
            ))}
          </ul>
        )}
      </Panel>

      <Panel title="Recent activity" flush note="Recorded operations, newest first.">
        {activity.length === 0 ? (
          <p className="muted" style={{ padding: "var(--space-lg)" }}>
            No operations recorded yet.
          </p>
        ) : (
          <ul className="rows">
            {activity.slice(0, 5).map((op) => (
              <li className="row" key={op.id}>
                <div className="row__icon" aria-hidden="true">
                  <IconClock size={15} />
                </div>
                <div className="row__main">
                  <div className="row__title">{op.summary}</div>
                  <div className="row__meta">
                    <span className="mono">{op.kind}</span>
                    <span>{op.package_id ?? "—"}</span>
                    <span>{relative(op.started_at)}</span>
                  </div>
                </div>
                <div className="row__actions">
                  <StatusPill
                    tone={op.status === "completed" ? "ok" : op.status === "failed" ? "error" : "idle"}
                    label={op.status}
                  />
                </div>
              </li>
            ))}
          </ul>
        )}
      </Panel>
    </div>
  );
}
