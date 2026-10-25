import type { ReactNode } from "react";
import type { EnvReport, InstalledApp, RuntimeStatus } from "../backend";
import {
  IconActivity,
  IconApplications,
  IconInstall,
  IconOverview,
  IconRuntime,
  IconSettings,
} from "./icons";
import { StatusPill, type Tone } from "./ui";

export type ScreenId =
  | "overview"
  | "applications"
  | "installer"
  | "runtime"
  | "activity"
  | "settings";

const NAV: { group: string; items: { id: ScreenId; label: string; icon: ReactNode }[] }[] = [
  {
    group: "Workspace",
    items: [
      { id: "overview", label: "Overview", icon: <IconOverview /> },
      { id: "applications", label: "Applications", icon: <IconApplications /> },
      { id: "installer", label: "Installer", icon: <IconInstall /> },
    ],
  },
  {
    group: "Management",
    items: [
      { id: "runtime", label: "Runtime", icon: <IconRuntime /> },
      { id: "activity", label: "Activity", icon: <IconActivity /> },
      { id: "settings", label: "Settings", icon: <IconSettings /> },
    ],
  },
];

export function runtimeTone(status: RuntimeStatus | null): { tone: Tone; label: string } {
  if (!status) return { tone: "idle", label: "Runtime unknown" };
  switch (status.state) {
    case "ready":
      return { tone: "ok", label: `Runtime ready · API ${status.android_api ?? "?"}` };
    case "starting":
      return { tone: "warn", label: "Runtime starting" };
    case "unhealthy":
      return { tone: "error", label: "Runtime unhealthy" };
    case "not_running":
      return { tone: "idle", label: "Runtime stopped" };
    default:
      return { tone: "warn", label: "Runtime not configured" };
  }
}

function Mark() {
  return (
    <svg className="topbar__mark" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      <rect width="24" height="24" fill="#05070a" />
      <rect x="4" y="4" width="16" height="11" fill="#e0231c" />
      <rect x="4" y="18" width="16" height="1.6" fill="#bbcbbd" />
    </svg>
  );
}

export function AppShell({
  screen,
  onNavigate,
  status,
  env,
  apps,
  children,
}: {
  screen: ScreenId;
  onNavigate: (id: ScreenId) => void;
  status: RuntimeStatus | null;
  env: EnvReport | null;
  apps: InstalledApp[] | null;
  children: ReactNode;
}) {
  const runtime = runtimeTone(status);
  const running = apps?.filter((app) => app.running).length ?? 0;
  const installed = apps?.length ?? null;

  return (
    <div className="shell">
      <header className="topbar">
        <div className="topbar__brand">
          <Mark />
          <span className="topbar__wordmark">DroidWin</span>
        </div>
        <div className="topbar__context">
          <StatusPill tone={runtime.tone} label={runtime.label} />
        </div>
        <div className="topbar__spacer" />
        <div className="topbar__env" title={env?.sdk_root ?? undefined}>
          {env?.sdk_root ? `SDK ${env.sdk_root}` : "No Android SDK detected"}
        </div>
      </header>

      <nav className="nav" aria-label="Primary">
        <div className="nav__groups">
          {NAV.map((group) => (
            <div key={group.group}>
              <span className="nav__label">{group.group}</span>
              <ul className="nav__list">
                {group.items.map((item) => (
                  <li key={item.id}>
                    <button
                      type="button"
                      className="nav__item"
                      aria-current={screen === item.id ? "page" : undefined}
                      onClick={() => onNavigate(item.id)}
                    >
                      {item.icon}
                      <span>{item.label}</span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </div>
        <div className="nav__foot">
          <span>DroidWin 0.1.0</span>
          <span>{status?.backend ?? "Backend not probed"}</span>
        </div>
      </nav>

      <main className="main" id="main">
        {children}
      </main>

      <footer className="statusbar">
        <span className="statusbar__item">{runtime.label}</span>
        <span className="statusbar__item">
          {installed === null ? "Apps unknown" : `${installed} app${installed === 1 ? "" : "s"}`}
        </span>
        <span className="statusbar__item">
          {running} window{running === 1 ? "" : "s"} open
        </span>
        <span className="statusbar__spacer" />
        <span className="statusbar__item">{status?.avd ?? "No AVD"}</span>
        <span className="statusbar__item">
          {status?.serial ?? (env?.adb ? "adb ready" : "adb missing")}
        </span>
      </footer>
    </div>
  );
}
