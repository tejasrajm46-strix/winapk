import { useEffect, useMemo, useState } from "react";
import type { InstalledApp } from "../backend";
import { IconInstall, IconRefresh, IconSearch } from "../components/icons";
import { Button, EmptyState, ErrorNote, Panel, StatusPill } from "../components/ui";
import { UninstallDialog } from "../components/UninstallDialog";
import { AppRow, PageHead, type ScreenProps } from "./common";

export function Applications(props: ScreenProps) {
  const { apps, appsError, busy, status } = props;
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<InstalledApp | null>(null);

  // If the package leaves the runtime while the dialog is open (another window,
  // or a refresh), the question no longer applies.
  useEffect(() => {
    const target = pending;
    if (!target || !apps) return;
    if (!apps.some((app) => app.package_id === target.package_id)) setPending(null);
  }, [apps, pending]);

  const filtered = useMemo(() => {
    if (!apps) return [];
    const q = query.trim().toLowerCase();
    if (!q) return apps;
    return apps.filter(
      (app) =>
        app.package_id.toLowerCase().includes(q) || (app.label ?? "").toLowerCase().includes(q),
    );
  }, [apps, query]);

  const running = apps?.filter((app) => app.running).length ?? 0;

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
            <span className="panel__note">
              {apps === null ? "…" : `${filtered.length} of ${apps.length}`}
            </span>
          </div>
        }
      >
        <div style={{ padding: "var(--space-md) var(--space-lg)" }}>
          <div className="search">
            <IconSearch size={14} />
            <input
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Search by name or package"
              aria-label="Search installed applications"
            />
          </div>
        </div>

        {appsError ? (
          <div style={{ padding: "0 var(--space-lg) var(--space-lg)" }}>
            <ErrorNote error={appsError} />
            {status?.state !== "ready" && (
              <div style={{ marginTop: "var(--space-md)" }}>
                <Button variant="primary" onClick={props.onStartRuntime} disabled={busy}>
                  Start runtime
                </Button>
              </div>
            )}
          </div>
        ) : apps === null ? (
          <p className="muted" style={{ padding: "0 var(--space-lg) var(--space-lg)" }}>
            Reading packages from the runtime…
          </p>
        ) : filtered.length === 0 ? (
          <div style={{ padding: "0 var(--space-lg) var(--space-lg)" }}>
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
                  <Button variant="secondary" onClick={() => setQuery("")}>
                    Clear search
                  </Button>
                )
              }
            />
          </div>
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
