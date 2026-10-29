import type { ReactNode } from "react";
import type {
  AgentPermission,
  AgentStatus,
  ApkInspection,
  AttentionRecovery,
  AttentionReport,
  DwError,
  EnvReport,
  InstalledApp,
  OperationRecord,
  RuntimeStatus,
  StageEvent,
} from "../backend";
import type { ScreenId } from "../components/AppShell";
import { IconPackage, IconPlay, IconSettings, IconStop, IconTrash } from "../components/icons";
import { Button, StatusPill } from "../components/ui";

export interface ScreenProps {
  status: RuntimeStatus | null;
  env: EnvReport | null;
  apps: InstalledApp[] | null;
  appsError: DwError | null;
  activity: OperationRecord[];
  busy: boolean;
  error: DwError | null;
  inspection: ApkInspection | null;
  stages: StageEvent[];
  opRunning: boolean;
  notice: string | null;
  /** Shared with the top bar, so search survives a screen change. */
  appQuery: string;
  onAppQueryChange: (value: string) => void;
  onNavigate: (screen: ScreenId) => void;
  onRefresh: () => void;
  onStartRuntime: () => void;
  onStopRuntime: () => void;
  onBrowse: () => void;
  onInstall: (launch: boolean, allowOldTarget: boolean) => void;
  onLaunch: (packageId: string) => void;
  onStopApp: (packageId: string) => void;
  onUninstall: (packageId: string, deleteData: boolean) => void;
  onOpenLogs: () => void;
  /** "Attention Needed" report, or null before the first check answered. */
  attention: AttentionReport | null;
  attentionBusy: boolean;
  onAttentionCheck: () => void;
  onRecover: (recovery: AttentionRecovery) => void;
  /** Agent control surface: endpoint, permission, audit trail, skills. */
  agent: AgentStatus | null;
  onAgentStart: () => void;
  onAgentStop: () => void;
  onAgentPause: (paused: boolean) => void;
  onAgentPermission: (permission: AgentPermission) => void;
  onAgentRefresh: () => void;
  onAgentInstallSkill: () => void;
  onAgentToggleSkill: (id: string, enabled: boolean) => void;
  onAgentRemoveSkill: (id: string) => void;
}

export function PageHead({
  eyebrow,
  title,
  lede,
  children,
}: {
  eyebrow: string;
  title: string;
  lede?: string;
  children?: ReactNode;
}) {
  return (
    <div className="page__head">
      <div>
        <span className="page__eyebrow">{eyebrow}</span>
        <h1 className="page__title">{title}</h1>
        {lede && <p className="page__lede">{lede}</p>}
      </div>
      {children && <div className="page__actions">{children}</div>}
    </div>
  );
}

export function AppRow({
  app,
  busy,
  onLaunch,
  onStopApp,
  onUninstall,
  onRequestUninstall,
  onControls,
  compact = false,
}: {
  app: InstalledApp;
  busy: boolean;
  onLaunch: (packageId: string) => void;
  onStopApp: (packageId: string) => void;
  onUninstall: (packageId: string, deleteData: boolean) => void;
  /** Opens the keep-or-delete choice. Without it the row uninstalls keeping data. */
  onRequestUninstall?: (app: InstalledApp) => void;
  /** Opens Configure Controls for this app. Mapping stays off until chosen there. */
  onControls?: (app: InstalledApp) => void;
  compact?: boolean;
}) {
  const name = app.label;
  const version = app.version_name ? `v${app.version_name}` : null;
  const legacy = app.target_sdk !== null && app.target_sdk < 23;
  return (
    <li className="row">
      <div className="row__icon" aria-hidden="true">
        {app.icon ? <img src={app.icon} alt="" /> : <IconPackage size={16} />}
      </div>
      <div className="row__main">
        <div
          className="row__title"
          title={
            legacy
              ? `${name ?? app.package_id} — targets API ${app.target_sdk}; old apps can render a blank window on a separate display`
              : name ?? app.package_id
          }
        >
          {name ?? app.package_id}
        </div>
        <div className="row__meta">
          <span className="mono">{app.package_id}</span>
          {version && <span>{version}</span>}
          {!name && <span>name unresolved</span>}
          {app.target_sdk !== null && app.target_sdk < 23 && (
            <StatusPill
              tone="warn"
              label={`Legacy target · API ${app.target_sdk}`}
            />
          )}
          {app.running ? (
            <StatusPill tone="accent" label="Window open" />
          ) : (
            <StatusPill tone="idle" label="Not running" />
          )}
        </div>
      </div>
      <div className="row__actions">
        {app.running ? (
          <Button size="sm" variant="ghost" onClick={() => onStopApp(app.package_id)} disabled={busy}>
            <IconStop size={13} />
            Stop
          </Button>
        ) : (
          <Button size="sm" variant="secondary" onClick={() => onLaunch(app.package_id)} disabled={busy}>
            <IconPlay size={13} />
            Open
          </Button>
        )}
        {!compact && onControls && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onControls(app)}
            disabled={busy}
            title="Configure Controls — per-app keyboard and touch mapping"
            aria-label={`Configure controls for ${name ?? app.package_id}`}
          >
            <IconSettings size={13} />
            Controls
          </Button>
        )}
        {!compact && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              onRequestUninstall ? onRequestUninstall(app) : onUninstall(app.package_id, false)
            }
            disabled={busy}
            title="Uninstall — choose whether to keep app data"
            aria-label={`Uninstall ${name ?? app.package_id}`}
          >
            <IconTrash size={13} />
          </Button>
        )}
      </div>
    </li>
  );
}

/** The same app, composed as a card for the grid view. */
export function AppCard({
  app,
  busy,
  onLaunch,
  onStopApp,
  onRequestUninstall,
  onControls,
}: {
  app: InstalledApp;
  busy: boolean;
  onLaunch: (packageId: string) => void;
  onStopApp: (packageId: string) => void;
  onRequestUninstall?: (app: InstalledApp) => void;
  onControls?: (app: InstalledApp) => void;
}) {
  const name = app.label;
  const version = app.version_name ? `v${app.version_name}` : null;
  const legacy = app.target_sdk !== null && app.target_sdk < 23;
  return (
    <li className="app-card">
      <div className="app-card__head">
        <div className="row__icon app-card__icon" aria-hidden="true">
          {app.icon ? <img src={app.icon} alt="" /> : <IconPackage size={18} />}
        </div>
        <div className="app-card__id">
          <div className="row__title" title={name ?? app.package_id}>
            {name ?? app.package_id}
          </div>
          <div className="row__meta">
            <span className="mono">{app.package_id}</span>
            {version && <span>{version}</span>}
          </div>
        </div>
        <StatusPill tone={app.running ? "accent" : "idle"} label={app.running ? "Open" : "Idle"} />
      </div>

      <div className="app-card__meta">
        {legacy && <StatusPill tone="warn" label={`Legacy target · API ${app.target_sdk}`} />}
        {!name && <span className="muted">name unresolved</span>}
        {!legacy && name && <span className="muted">Third-party package</span>}
      </div>

      <div className="app-card__actions">
        {app.running ? (
          <Button size="sm" variant="ghost" onClick={() => onStopApp(app.package_id)} disabled={busy}>
            <IconStop size={13} />
            Stop
          </Button>
        ) : (
          <Button
            size="sm"
            variant="secondary"
            onClick={() => onLaunch(app.package_id)}
            disabled={busy}
          >
            <IconPlay size={13} />
            Open
          </Button>
        )}
        {onControls && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => onControls(app)}
            disabled={busy}
            title="Configure Controls — per-app keyboard and touch mapping"
            aria-label={`Configure controls for ${name ?? app.package_id}`}
          >
            <IconSettings size={13} />
            Controls
          </Button>
        )}
        <Button
          size="sm"
          variant="ghost"
          onClick={() => onRequestUninstall?.(app)}
          disabled={busy}
          title="Uninstall — choose whether to keep app data"
          aria-label={`Uninstall ${name ?? app.package_id}`}
        >
          <IconTrash size={13} />
        </Button>
      </div>
    </li>
  );
}

/** Placeholder rows while the runtime reports its package list. */
export function SkeletonRows({ rows = 3 }: { rows?: number }) {
  return (
    <div className="rows" aria-hidden="true">
      {Array.from({ length: rows }, (_, index) => (
        <div className="row skeleton-row" key={index}>
          <div className="skeleton skeleton--icon" />
          <div className="skeleton-row__main">
            <div className="skeleton skeleton--title" />
            <div className="skeleton skeleton--meta" />
          </div>
        </div>
      ))}
    </div>
  );
}
