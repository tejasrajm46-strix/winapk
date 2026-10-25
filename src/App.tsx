import { useCallback, useEffect, useRef, useState } from "react";
import * as backend from "./backend";
import type {
  ApkInspection,
  DwError,
  EnvReport,
  InstalledApp,
  OperationRecord,
  RuntimeStatus,
  StageEvent,
} from "./backend";
import { AppShell, type ScreenId } from "./components/AppShell";
import { Activity } from "./screens/Activity";
import { Applications } from "./screens/Applications";
import { Installer } from "./screens/Installer";
import { Overview } from "./screens/Overview";
import { RuntimeScreen } from "./screens/Runtime";
import { Settings } from "./screens/Settings";
import type { ScreenProps } from "./screens/common";

/**
 * One owner for every piece of screen state: this component. Screens are pure
 * functions of these values plus the callbacks, so nothing can show state the
 * backend did not report.
 */
export function App() {
  const [screen, setScreen] = useState<ScreenId>("overview");
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [env, setEnv] = useState<EnvReport | null>(null);
  const [apps, setApps] = useState<InstalledApp[] | null>(null);
  const [appsError, setAppsError] = useState<DwError | null>(null);
  const [activity, setActivity] = useState<OperationRecord[]>([]);
  const [inspection, setInspection] = useState<ApkInspection | null>(null);
  const [stages, setStages] = useState<StageEvent[]>([]);
  const [opRunning, setOpRunning] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<DwError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const mounted = useRef(true);

  const refreshApps = useCallback(async () => {
    try {
      const list = await backend.listApps();
      if (!mounted.current) return;
      setApps(list);
      setAppsError(null);
    } catch (err) {
      if (!mounted.current) return;
      setApps(null);
      setAppsError(backend.toDwError(err));
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

  const refresh = useCallback(async () => {
    setBusy(true);
    try {
      const probe = await backend.probeRuntime();
      if (mounted.current) {
        setStatus(probe.status);
        setEnv(probe.env);
      }
      await refreshApps();
      await refreshActivity();
      setError(null);
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  }, [refreshActivity, refreshApps]);

  // The first probe can run before adb has reported a booting emulator, which
  // would leave the status strip stale. Re-read the runtime's own state quietly.
  useEffect(() => {
    if (!backend.isDesktop()) return;
    let last = -1;
    const timer = window.setInterval(() => {
      void backend
        .getRuntimeStatus()
        .then((next) => {
          if (!mounted.current) return;
          setStatus(next);
          if (next.state === "ready" && last !== 1) void refreshApps();
          last = next.state === "ready" ? 1 : 0;
        })
        .catch(() => {
          /* keep the last reported state; a failed poll is not a state change */
        });
    }, 8000);
    return () => window.clearInterval(timer);
  }, [refreshApps]);

  useEffect(() => {
    mounted.current = true;
    void refresh();

    const stops: Promise<() => void>[] = [
      backend.onStage((event) => {
        setStages((current) => [...current, event]);
        if (event.stage === "completed" || event.stage === "failed" || event.stage === "cancelled") {
          setOpRunning(false);
        }
      }),
      backend.onRuntimeStatus((next) => setStatus(next)),
      backend.onAppsChanged(() => {
        void refreshApps();
      }),
      backend.onApkActivation((path) => {
        void openPath(path);
      }),
      backend.onFileDrop((paths) => {
        const apk = paths.find((p) => p.toLowerCase().endsWith(".apk"));
        if (apk) void openPath(apk);
      }),
    ];

    // A package opened from Explorer before this webview existed is waiting in
    // the backend queue; the event listener above only covers later launches.
    void (async () => {
      try {
        const pending = await backend.takePendingActivation();
        if (!mounted.current || pending.length === 0) return;
        await openPath(pending[0]);
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
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refresh]);

  const openPath = async (path: string) => {
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
  };

  const browse = async () => {
    setError(null);
    try {
      const path = await backend.pickApk();
      if (path) await openPath(path);
      else setScreen("installer");
    } catch (err) {
      setError(backend.toDwError(err));
    }
  };

  const install = async (launch: boolean, allowOldTarget: boolean) => {
    if (!inspection) return;
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
      await refreshApps();
      await refreshActivity();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
      await refreshActivity();
    } finally {
      if (mounted.current) {
        setBusy(false);
        setOpRunning(false);
      }
    }
  };

  const startRuntime = async () => {
    setBusy(true);
    setError(null);
    setNotice(null);
    setScreen("runtime");
    try {
      const next = await backend.startRuntime();
      if (mounted.current) setStatus(next);
      await refreshApps();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const stopRuntime = async () => {
    setBusy(true);
    try {
      await backend.stopRuntime();
      const probe = await backend.probeRuntime();
      if (mounted.current) setStatus(probe.status);
      await refreshApps();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const launch = async (packageId: string) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const window = await backend.launchApp(packageId);
      if (mounted.current) setNotice(`Opened “${window.title}”.`);
      await refreshApps();
      await refreshActivity();
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
      await refreshApps();
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
      await refreshApps();
      await refreshActivity();
    } catch (err) {
      if (mounted.current) setError(backend.toDwError(err));
    } finally {
      if (mounted.current) setBusy(false);
    }
  };

  const props: ScreenProps = {
    status,
    env,
    apps,
    appsError,
    activity,
    busy,
    error,
    inspection,
    stages,
    opRunning,
    notice,
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
  };

  return (
    <AppShell screen={screen} onNavigate={setScreen} status={status} env={env} apps={apps}>
      {screen === "overview" && <Overview {...props} />}
      {screen === "installer" && <Installer {...props} />}
      {screen === "applications" && <Applications {...props} />}
      {screen === "runtime" && <RuntimeScreen {...props} />}
      {screen === "activity" && <Activity {...props} />}
      {screen === "settings" && <Settings {...props} />}
    </AppShell>
  );
}
