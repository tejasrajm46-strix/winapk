import { useEffect, useMemo, useState } from "react";
import type { InstalledApp } from "../backend";
import { IconGrid, IconInstall, IconList, IconRefresh, IconSearch } from "../components/icons";
import { Button, EmptyState, ErrorNote, Panel, StatusPill } from "../components/ui";
import { ControlsDialog } from "../components/ControlsDialog";
import { UninstallDialog } from "../components/UninstallDialog";
import { AppCard, AppRow, PageHead, SkeletonRows, type ScreenProps } from "./common";

type View = "list" | "grid";

function initialView(): View {
  try {
    return window.localStorage.getItem("droidwin.appView") === "grid" ? "grid" : "list";
  } catch {
    return "list";
  }
}

export function Applications(props: ScreenProps) {
  const { apps, appsError, busy, status, appQuery, onAppQueryChange } = props;
  const [view, setView] = useState<View>(initialView);
  const [pending, setPending] = useState<InstalledApp | null>(null);
  // Controls is opened per app and never rendered otherwise: mapping is off by
  // default and the panel is the only place it can be turned on.
  const [controls, setControls] = useState<InstalledApp | null>(null);

  // If the package leaves the runtime while the dialog is open (another window,
  // or a refresh), the question no longer applies.
  useEffect(() => {
    const target = pending;
    if (!target || !apps) return;
    if (!apps.some((app) => app.package_id === target.package_id)) setPending(null);
  }, [apps, pending]);

  const filtered = useMemo(() => {
    if (!apps) return [];
    const q = appQuery.trim().toLowerCase();
    if (!q) return apps;
    return apps.filter(
      (app) =>
        app.package_id.toLowerCase().includes(q) || (app.label ?? "").toLowerCase().includes(q),
    );
  }, [apps, appQuery]);

  const running = apps?.filter((app) => app.running).length ?? 0;

  const chooseView = (next: View) => {
    setView(next);
    try {
      window.localStorage.setItem("droidwin.appView", next);
    } catch {
      /* a view preference is not worth an error */
    }
  };

  return (
    <div className="page">
      <PageHead
        eyebrow="Workspace"
        title="Applications Library"
        lede="What the runtime reports as installed. DroidWin does not cache install state as truth."
      >
        <Button variant="secondary" onClick={props.onBrowse} disabled={busy}>
          <IconInstall size={14} />
          Install APK
        </Button>
        <Button variant="ghost" onClick={props.onRefresh} disabled={busy}>
          <IconRefresh size={14} />
          Refresh
        </Button>
      </PageHead>

      {props.error && <ErrorNote error={props.error} />}

      <Panel
        title="Installed"
        flush
        action={
          <div className="inline">
            {running > 0 && <StatusPill tone="accent" label={`${running} running`} />}
            <span className="panel__note" aria-live="polite">
              {apps === null
                ? appsError
                  ? "Unavailable"
                  : "Reading…"
                : `${filtered.length} of ${apps.length}`}
            </span>
            <div className="segmented" role="group" aria-label="Application view">
              <button
                type="button"
                className={`segmented__item${view === "grid" ? " segmented__item--active" : ""}`}
                aria-pressed={view === "grid"}
                aria-label="Grid view"
                title="Grid view"
                onClick={() => chooseView("grid")}
              >
                <IconGrid size={14} />
              </button>
              <button
                type="button"
                className={`segmented__item${view === "list" ? " segmented__item--active" : ""}`}
                aria-pressed={view === "list"}
                aria-label="List view"
                title="List view"
                onClick={() => chooseView("list")}
              >
                <IconList size={14} />
              </button>
            </div>
          </div>
        }
      >
        <div className="toolbar">
          <div className="search">
            <IconSearch size={14} />
            <input
              type="search"
              value={appQuery}
              onChange={(event) => onAppQueryChange(event.target.value)}
              placeholder="Search by name or package"
              aria-label="Filter installed applications"
            />
          </div>
        </div>

        {appsError ? (
          <div className="panel__inset">
            {appsError.code === props.error?.code ? (
              <p className="muted">
                The package list is unavailable until the runtime answers.
              </p>
            ) : (
              <ErrorNote error={appsError} />
            )}
            {status?.state !== "ready" && (
              <div style={{ marginTop: "var(--space-md)" }}>
                <Button variant="primary" onClick={props.onStartRuntime} disabled={busy}>
                  Start runtime
                </Button>
              </div>
            )}
          </div>
        ) : apps === null ? (
          <div aria-busy="true" aria-label="Reading packages from the runtime">
            <SkeletonRows rows={4} />
          </div>
        ) : filtered.length === 0 ? (
          <div className="panel__inset">
            <EmptyState
              title={apps.length === 0 ? "No apps installed yet" : "No match"}
              body={
                apps.length === 0
                  ? "Packages you install into the runtime appear here. System apps are not listed."
                  : "Nothing matches that search. Clear it to see every installed package."
              }
              action={
                apps.length === 0 ? (
                  <Button variant="primary" onClick={props.onBrowse}>
                    Choose an APK
                  </Button>
                ) : (
                  <Button variant="secondary" onClick={() => onAppQueryChange("")}>
                    Clear search
                  </Button>
                )
              }
            />
          </div>
        ) : view === "grid" ? (
          <ul className="app-grid">
            {filtered.map((app) => (
              <AppCard
                key={app.package_id}
                app={app}
                busy={busy}
                onLaunch={props.onLaunch}
                onStopApp={props.onStopApp}
                onRequestUninstall={setPending}
                onControls={setControls}
              />
            ))}
          </ul>
        ) : (
          <ul className="rows">
            {filtered.map((app) => (
              <AppRow
                key={app.package_id}
                app={app}
                busy={busy}
                onLaunch={props.onLaunch}
                onStopApp={props.onStopApp}
                onUninstall={props.onUninstall}
                onRequestUninstall={setPending}
                onControls={setControls}
              />
            ))}
          </ul>
        )}
      </Panel>

      <div className="note note--info">
        <div className="note__head">
          <IconInstall size={14} />
          <span>Uninstalling asks before touching app data</span>
        </div>
        <p className="note__body">
          Android keeps an app&apos;s data after the package is removed, so reinstalling restores it.
          Deleting that data is a separate choice and is never made for you.
        </p>
      </div>

      {controls && (
        <ControlsDialog app={controls} onClose={() => setControls(null)} />
      )}

      {pending && (
        <UninstallDialog
          app={pending}
          busy={busy}
          onCancel={() => setPending(null)}
          onConfirm={(deleteData) => {
            const target = pending;
            if (!target) return;
            setPending(null);
            props.onUninstall(target.package_id, deleteData);
          }}
        />
      )}
    </div>
  );
}
