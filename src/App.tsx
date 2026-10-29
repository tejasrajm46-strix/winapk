import { useCallback, useEffect, useRef, useState } from "react";
import * as backend from "./backend";
import type {
  AgentPermission,
  AgentStatus,
  ApkInspection,
  AttentionRecovery,
  AttentionReport,
  DwError,
  OperationRecord,
  StageEvent,
} from "./backend";
import { AppShell, type ScreenId } from "./components/AppShell";
import { ControlsDialog } from "./components/ControlsDialog";
import { Activity } from "./screens/Activity";
import { Agent } from "./screens/Agent";
import { Applications } from "./screens/Applications";
import { Installer } from "./screens/Installer";
import { Overview } from "./screens/Overview";
import { Phone } from "./screens/Phone";
import { RuntimeScreen } from "./screens/Runtime";
import { Settings } from "./screens/Settings";
import { Setup } from "./screens/Setup";
import type { ScreenProps } from "./screens/common";
import { useAppLibrary } from "./state/apps";
import { useRuntimeState } from "./state/runtime";
import { useSetupState } from "./state/setup";

/** The leaf of a Windows path, for a queue notice that must stay one line. */
function fileName(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/**
 * The shell: navigation, the banner, and the installer's own workflow. The facts
 * it renders have one owner each and are read from there — `useRuntimeState`
 * (runtime status and environment), `useAppLibrary` (installed packages) and
 * `useSetupState` (the runtime installation) — so a change to one of them lands
 * in that owner rather than in a conditional here.
 */
export function App() {
  const [screen, setScreen] = useState<ScreenId>("overview");
  const [activity, setActivity] = useState<OperationRecord[]>([]);
  const [inspection, setInspection] = useState<ApkInspection | null>(null);
  const [stages, setStages] = useState<StageEvent[]>([]);
  const [opRunning, setOpRunning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<DwError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [appQuery, setAppQuery] = useState("");
  const [agent, setAgent] = useState<AgentStatus | null>(null);
  const [attention, setAttention] = useState<AttentionReport | null>(null);
  const [attentionBusy, setAttentionBusy] = useState(false);
  // The nav bar's Controls button asks for the editor of the app it is over. The
  // package is looked up in the library rather than reconstructed, so the dialog
  // always describes a real installed app.
  const [overlayControlsId, setOverlayControlsId] = useState<string | null>(null);
  const mounted = useRef(true);
  // Diagnostics has its own single-flight gate, but the UI must not queue
  // checks either: an apps-changed event and a manual refresh often arrive
  // together, and only one of them needs to ask.
  const attentionInFlight = useRef(false);
  // An install owns the inspection screen; a package requested meanwhile waits
  // instead of replacing the one being installed.
  const installRunning = useRef(false);
  const activationBusy = useRef(false);
  const activationQueue = useRef<string[]>([]);

  const fail = useCallback((err: unknown) => setError(backend.toDwError(err)), []);

  const refreshAgent = useCallback(async () => {
    if (!backend.isDesktop()) return;
    try {
      const next = await backend.getAgentStatus();
      if (mounted.current) setAgent(next);
    } catch {
      // The agent panel is not the reason the app exists; a failed read leaves
      // the last known status instead of an error banner on every screen.
    }
  }, []);

  const refreshAttention = useCallback(async (deep = false) => {
    if (!backend.isDesktop() || attentionInFlight.current) return;
    attentionInFlight.current = true;
    if (deep) setAttentionBusy(true);
    try {
      const next = await backend.getAttention(deep);
      if (mounted.current) setAttention(next);
    } catch {
      // The last report stays on screen; a failed check is not a new problem.
    } finally {
      attentionInFlight.current = false;
      if (mounted.current) setAttentionBusy(false);
    }
  }, []);

  const refreshActivity = useCallback(async () => {
    try {
      const records = await backend.getActivity(40);
      if (mounted.current) setActivity(records);
    } catch {
      /* history is not worth an error banner; the panel shows an empty state */
    }
  }, []);

  const runtime = useRuntimeState({
    onError: fail,
    // The agent endpoint is re-read on the runtime's heartbeat instead of having
    // a poll of its own.
    onPoll: () => void refreshAgent(),
  });
  const setup = useSetupState({
    status: runtime.status,
    onError: fail,
    onRuntimeChanged: () => void runtime.refresh(),
  });
  const apps = useAppLibrary({ status: runtime.status });

  /** The Refresh button: every owner re-reads its own fact. */
  const refresh = useCallback(async () => {
    setBusy(true);
    setError(null);
    await runtime.refresh();
    await apps.refresh();
    await refreshActivity();
    await refreshAgent();
    await refreshAttention();
    await setup.refresh();
    if (mounted.current) setBusy(false);
  }, [apps.refresh, refreshActivity, refreshAgent, refreshAttention, runtime.refresh, setup.refresh]);

  const inspectPath = useCallback(async (path: string) => {
    setScreen("installer");
    setError(null);
    setNotice(null);
    setStages([]);
    setInspection(null);
    setBusy(true);
    try {
      const next = await backend.inspectApk(path);
      if (mounted.current) setInspection(next);
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, []);

  /**
   * Packages arrive from Explorer, drops and the picker, sometimes two at once.
   * They are inspected one at a time: a second inspection must not swap the
   * package out from under an install that is already running.
   */
  const openPath = useCallback(
    (path: string) => {
      const next = path.trim();
      if (!next) return;
      const queue = activationQueue.current;
      if (!queue.includes(next)) queue.push(next);
      if (queue.length > 6) queue.length = 6;
      if (activationBusy.current) {
        setNotice(`Queued ${fileName(next)} until the current package is done.`);
        return;
      }
      activationBusy.current = true;
      void (async () => {
        try {
          while (activationQueue.current.length > 0) {
            while (installRunning.current) {
              await new Promise((resolve) => window.setTimeout(resolve, 250));
            }
            const target = activationQueue.current.shift();
            if (!target) break;
            await inspectPath(target);
          }
        } finally {
          activationBusy.current = false;
        }
      })();
    },
    [inspectPath],
  );

  useEffect(() => {
    mounted.current = true;
    void refresh();

    const stops: Promise<() => void>[] = [
      backend.onStage((event) => {
        // The Operation panel belongs to an install this window started. The
        // startup boot emits its own stages through the same feed, and they used
        // to sit there as a finished "operation" the user never began.
        if (!installRunning.current) return;
        setStages((current) => [...current, event]);
        if (
          event.stage === "completed" ||
          event.stage === "failed" ||
          event.stage === "cancelled"
        ) {
          setOpRunning(false);
        }
      }),
      backend.onApkActivation((path) => openPath(path)),
      backend.onAppsChanged(() => {
        // The library reads the list from its own subscription to this event; a
        // window opening or closing also makes the previous diagnosis history.
        void refreshAttention();
      }),
      backend.onOverlayControls((packageId) => {
        setOverlayControlsId(packageId);
        void apps.refresh();
      }),
      backend.onFileDrop((paths) => {
        const apk = paths.find((p) => p.toLowerCase().endsWith(".apk"));
        if (apk) openPath(apk);
      }),
    ];

    // A package opened from Explorer before this webview existed is waiting in
    // the backend queue; the event listener above only covers later launches.
    void (async () => {
      try {
        const pending = await backend.takePendingActivation();
        if (!mounted.current || pending.length === 0) return;
        openPath(pending[0]);
        if (mounted.current && pending.length > 1) {
          setNotice(
            `${pending.length - 1} other package${pending.length > 2 ? "s were" : " was"} requested; opened the first.`,
          );
        }
      } catch (err) {
        if (mounted.current) setError(backend.toDwError(err));
      }
    })();

    return () => {
      mounted.current = false;
      for (const stop of stops) void stop.then((fn) => fn());
    };
  }, [apps.refresh, openPath, refresh, refreshAttention]);

  const browse = async () => {
    setError(null);
    try {
      const path = await backend.pickApk();
      if (path) openPath(path);
      else setScreen("installer");
    } catch (err) {
      setError(backend.toDwError(err));
    }
  };

  const install = async (launch: boolean, allowOldTarget: boolean) => {
    if (!inspection) return;
    installRunning.current = true;
    setBusy(true);
    setOpRunning(true);
    setStages([]);
    setError(null);
    setNotice(null);
    try {
      const outcome = await backend.installApk(inspection.path, launch, allowOldTarget);
      if (!mounted.current) return;
      setNotice(
        `${outcome.message}${outcome.version_name ? ` (v${outcome.version_name})` : ""}. ${
          launch ? "Its window is open." : "Open it from Applications."
        }`,
      );
      await apps.refresh();
      await refreshActivity();
      await refreshAttention();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
      await refreshActivity();
    } finally {
      installRunning.current = false;
      if (mounted.current) {
        setBusy(false);
        setOpRunning(false);
      }
    }
  };

  const installRuntime = async (acceptLicenses: boolean) => {
    setError(null);
    if (!(await setup.install(acceptLicenses))) return;
    await runtime.refresh();
    await apps.refresh();
    await refreshActivity();
    await refreshAttention();
  };

  const startRuntime = async () => {
    setScreen("runtime");
    setBusy(true);
    setError(null);
    setNotice(null);
    if (await runtime.start()) await apps.refresh();
    if (mounted.current) setBusy(false);
  };

  const stopRuntime = async () => {
    setBusy(true);
    if (await runtime.stop()) await apps.refresh();
    if (mounted.current) setBusy(false);
  };

  const launch = async (packageId: string) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const window = await backend.launchApp(packageId);
      if (mounted.current) setNotice(`Opened “${window.title}”.`);
      await apps.refresh();
      await refreshActivity();
      await refreshAttention();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const stopApp = async (packageId: string) => {
    setBusy(true);
    setError(null);
    try {
      await backend.stopApp(packageId);
      await apps.refresh();
      await refreshAttention();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const uninstall = async (packageId: string, deleteData: boolean) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const message = await backend.uninstallApp(packageId, deleteData);
      if (mounted.current) setNotice(message);
      await apps.refresh();
      await refreshActivity();
      await refreshAttention();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  /** Run one targeted recovery and show the state it left behind. */
  const recover = async (recovery: AttentionRecovery) => {
    setAttentionBusy(true);
    setError(null);
    setNotice(null);
    try {
      const outcome = await backend.recoverProblem(recovery);
      if (!mounted.current) return;
      setAttention(outcome.report);
      if (outcome.ok) {
        setNotice(outcome.summary);
      } else {
        setError({
          code: "backend_failure",
          summary: outcome.summary,
          detail: outcome.detail,
        });
      }
      await apps.refresh();
      await refreshActivity();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setAttentionBusy(false);
    }
  };

  /** Run one agent command and take the status it returns as the truth. */
  const agentCommand = async (run: () => Promise<AgentStatus>) => {
    setBusy(true);
    setError(null);
    try {
      const next = await run();
      if (mounted.current) setAgent(next);
      await refreshAgent();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const installSkill = async () => {
    setError(null);
    try {
      const path = await backend.pickSkillFolder();
      if (!path) return;
      await agentCommand(async () => {
        await backend.installAgentSkill(path);
        return backend.getAgentStatus();
      });
      if (mounted.current) setNotice("Skill installed. Enable it for agents on the Agent screen.");
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    }
  };

  const props: ScreenProps = {
    status: runtime.status,
    env: runtime.env,
    apps: apps.apps,
    appsError: apps.appsError,
    activity,
    busy,
    error,
    inspection,
    stages,
    opRunning,
    notice,
    appQuery,
    onAppQueryChange: (value) => {
      setAppQuery(value);
      if (value.trim()) setScreen("applications");
    },
    onNavigate: setScreen,
    onRefresh: () => void refresh(),
    onStartRuntime: () => void startRuntime(),
    onStopRuntime: () => void stopRuntime(),
    onBrowse: () => void browse(),
    onInstall: (launchAfter, allowOldTarget) => void install(launchAfter, allowOldTarget),
    onLaunch: (packageId) => void launch(packageId),
    onStopApp: (packageId) => void stopApp(packageId),
    onUninstall: (packageId, deleteData) => void uninstall(packageId, deleteData),
    onOpenLogs: () => {
      void backend.openLogs().catch((err) => setError(backend.toDwError(err)));
    },
    attention,
    attentionBusy,
    onAttentionCheck: () => void refreshAttention(true),
    onRecover: (recovery) => void recover(recovery),
    agent,
    onAgentStart: () => void agentCommand(backend.startAgent),
    onAgentStop: () => void agentCommand(backend.stopAgent),
    onAgentPause: (paused: boolean) => void agentCommand(() => backend.setAgentPaused(paused)),
    onAgentPermission: (permission: AgentPermission) =>
      void agentCommand(() => backend.setAgentPermission(permission)),
    onAgentRefresh: () => {
      void refreshAgent();
      void refreshActivity();
    },
    onAgentInstallSkill: () => void installSkill(),
    onAgentToggleSkill: (id: string, enabled: boolean) =>
      void agentCommand(async () => {
        await backend.setAgentSkill(id, enabled);
        return backend.getAgentStatus();
      }),
    onAgentRemoveSkill: (id: string) =>
      void agentCommand(async () => {
        await backend.removeAgentSkill(id);
        return backend.getAgentStatus();
      }),
  };

  // The setup screen comes before the shell only while something has to be
  // installed. `starting` means the files are all there and the emulator is
  // booting, which is the shell's job to show — not an installation prompt.
  const setupBlocksShell =
    setup.report !== null &&
    (setup.report.state === "needs_install" ||
      setup.report.state === "failed" ||
      setup.report.state === "installing");
  if (backend.isDesktop() && setupBlocksShell && !setup.dismissed) {
    return (
      <Setup
        report={setup.report}
        progress={setup.progress}
        busy={setup.busy}
        error={error}
        onInstall={(accept) => void installRuntime(accept)}
        onCancel={() => void setup.cancel()}
        onChangeLocation={() => void setup.chooseFolder()}
        onUseManaged={() => void setup.useManaged()}
        onRefresh={() => {
          setup.clearProgress();
          void setup.refresh();
        }}
        onSkip={setup.dismiss}
      />
    );
  }

  return (
    <AppShell
      screen={screen}
      onNavigate={setScreen}
      status={runtime.status}
      env={runtime.env}
      apps={apps.apps}
      agent={agent}
      searchValue={appQuery}
      onSearch={(value) => {
        setAppQuery(value);
        if (value.trim()) setScreen("applications");
      }}
    >
      {screen === "phone" && <Phone {...props} />}
      {screen === "overview" && <Overview {...props} />}
      {screen === "installer" && <Installer {...props} />}
      {screen === "applications" && <Applications {...props} />}
      {screen === "runtime" && <RuntimeScreen {...props} />}
      {screen === "agent" && <Agent {...props} />}
      {screen === "activity" && <Activity {...props} />}
      {screen === "settings" && <Settings {...props} />}

      {(() => {
        if (!overlayControlsId) return null;
        const target = apps.apps?.find((app) => app.package_id === overlayControlsId);
        if (!target) return null;
        return <ControlsDialog app={target} onClose={() => setOverlayControlsId(null)} />;
      })()}
    </AppShell>
  );
}
