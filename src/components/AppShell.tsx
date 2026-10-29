import { useEffect, useRef, type ReactNode } from "react";
import type { AgentStatus, EnvReport, InstalledApp, RuntimeStatus } from "../backend";
import mark from "../assets/branding/droidwin-mark.png";
import {
  IconActivity,
  IconAgent,
  IconApplications,
  IconInstall,
  IconOverview,
  IconPhone,
  IconRuntime,
  IconSearch,
  IconSettings,
} from "./icons";
import { StatusPill, type Tone } from "./ui";

export type ScreenId =
  | "phone"
  | "overview"
  | "applications"
  | "installer"
  | "runtime"
  | "agent"
  | "activity"
  | "settings";

const NAV: { group: string; items: { id: ScreenId; label: string; icon: ReactNode }[] }[] = [
  {
    group: "Workspace",
    items: [
      { id: "phone", label: "Phone", icon: <IconPhone /> },
      { id: "overview", label: "Overview", icon: <IconOverview /> },
      { id: "applications", label: "Applications", icon: <IconApplications /> },
      { id: "installer", label: "Installer", icon: <IconInstall /> },
    ],
  },
  {
    group: "Management",
    items: [
      { id: "runtime", label: "Runtime", icon: <IconRuntime /> },
      { id: "agent", label: "Agent", icon: <IconAgent /> },
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
  return <img className="topbar__mark" src={mark} alt="" aria-hidden="true" />;
}

export function AppShell({
  screen,
  onNavigate,
  status,
  env,
  apps,
  agent,
  searchValue,
  onSearch,
  children,
}: {
  screen: ScreenId;
  onNavigate: (id: ScreenId) => void;
  status: RuntimeStatus | null;
  env: EnvReport | null;
  apps: InstalledApp[] | null;
  agent: AgentStatus | null;
  searchValue: string;
  onSearch: (value: string) => void;
  children: ReactNode;
}) {
  const runtime = runtimeTone(status);
  // Recent evidence, not the endpoint being reachable: an idle endpoint is not
  // "controlling", and a paused one is not either.
  const latestAction = agent?.actions[0];
  const agentActive = Boolean(
    agent?.running && !agent.paused && latestAction && Date.now() - latestAction.at_ms < 20_000,
  );
  const running = apps?.filter((app) => app.running).length ?? 0;
  const installed = apps?.length ?? null;
  const search = useRef<HTMLInputElement>(null);

  // Ctrl+K focuses the one search field, wherever the user is in the app.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
        event.preventDefault();
        search.current?.focus();
        search.current?.select();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  return (
    <div className="shell">
      <header className="topbar">
        <div className="topbar__brand">
          <Mark />
          <span className="topbar__wordmark">DroidWin</span>
        </div>
        <div className="topbar__context">
          <StatusPill tone={runtime.tone} label={runtime.label} />
          {(agentActive || agent?.paused) && (
            <StatusPill
              tone={agent?.paused ? "warn" : "accent"}
              label={agent?.paused ? "Agent paused" : "Agent acting"}
            />
          )}
        </div>
        <div className="topbar__spacer" />
        <div className="topbar__search">
          <IconSearch size={14} />
          <input
            ref={search}
            type="search"
            value={searchValue}
            onChange={(event) => onSearch(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                onSearch("");
                event.currentTarget.blur();
              }
            }}
            placeholder="Search apps"
            aria-label="Search installed applications"
          />
          <kbd className="topbar__kbd">Ctrl K</kbd>
        </div>
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
          {`${running} window${running === 1 ? "" : "s"} open`}
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
