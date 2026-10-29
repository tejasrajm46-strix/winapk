/**
 * Test-only fixtures. These values exist to exercise rendering states and are
 * never imported by application code.
 */

import type {
  AgentStatus,
  ApkInspection,
  EnvReport,
  InstalledApp,
  OperationRecord,
  RuntimeStatus,
} from "../backend";
import type { ScreenProps } from "../screens/common";

export const envMissing: EnvReport = {
  sdk_root: null,
  adb: null,
  emulator: null,
  avdmanager: null,
  sdkmanager: null,
  aapt2: null,
  scrcpy: null,
  system_image: null,
  avds: [],
  accel: null,
  blocking: ["The Android Emulator is not installed: sdkmanager \"emulator\""],
};

export const envReady: EnvReport = {
  sdk_root: "C:/Android/Sdk",
  adb: "C:/Android/Sdk/platform-tools/adb.exe",
  emulator: "C:/Android/Sdk/emulator/emulator.exe",
  avdmanager: "C:/Android/Sdk/cmdline-tools/latest/bin/avdmanager.bat",
  sdkmanager: "C:/Android/Sdk/cmdline-tools/latest/bin/sdkmanager.bat",
  aapt2: "C:/Android/Sdk/build-tools/36.0.0/aapt2.exe",
  scrcpy: "C:/DroidWin/scrcpy.exe",
  system_image: "system-images;android-36;google_apis;x86_64",
  avds: ["DroidWin_API36"],
  accel: "accel:\n0\nWHPX(10.0.26200) is installed and usable.\naccel",
  blocking: [],
};

export const statusNotConfigured: RuntimeStatus = {
  backend: "Android Emulator + scrcpy",
  state: "not_configured",
  detail: "The Android Emulator is not installed: sdkmanager \"emulator\"",
  avd: "DroidWin_API36",
  serial: null,
  android_api: null,
  capabilities: {
    install: true,
    uninstall: true,
    enumerate: true,
    launch: true,
    independent_windows: false,
    concurrent_apps: false,
    clipboard: true,
    file_drop: true,
    audio: true,
    hardware_accel: false,
  },
  limitations: [
    "Independent app windows have not been verified on this machine yet.",
    "The Android Emulator is not installed: sdkmanager \"emulator\"",
  ],
};

export const statusReady: RuntimeStatus = {
  backend: "Android Emulator + scrcpy",
  state: "ready",
  detail: "Android API 36 ready on emulator-5554.",
  avd: "DroidWin_API36",
  serial: "emulator-5554",
  android_api: 36,
  capabilities: {
    install: true,
    uninstall: true,
    enumerate: true,
    launch: true,
    independent_windows: true,
    concurrent_apps: true,
    clipboard: false,
    file_drop: false,
    audio: true,
    hardware_accel: true,
  },
  limitations: ["All apps share one Android user, so they are not isolated from each other."],
};

export const appRunning: InstalledApp = {
  package_id: "org.fdroid.fdroid",
  label: "F-Droid",
  version_name: "2.0.2",
  version_code: 2002,
  running: true,
  window_title: "DroidWin — F-Droid",
  icon: null,
  user_installed: true,
  target_sdk: 36,
};

export const appLegacyTarget: InstalledApp = {
  ...appRunning,
  package_id: "dudeofx.eval",
  label: "Expression Evaluator",
  version_name: "1.1",
  version_code: 101,
  running: false,
  window_title: null,
  target_sdk: 17,
};

export const appWithoutLabel: InstalledApp = {
  package_id: "com.example.unlabelled",
  label: null,
  version_name: null,
  version_code: null,
  running: false,
  window_title: null,
  icon: null,
  user_installed: true,
  target_sdk: null,
};

export const inspectionPartial: ApkInspection = {
  path: "C:/Users/someone/Downloads/demo.apk",
  file_name: "demo.apk",
  package_id: "com.example.demo",
  label: null,
  version_name: null,
  version_code: null,
  min_sdk: null,
  target_sdk: null,
  abis: [],
  size_bytes: 1048576,
  icon: null,
  signature: { v1: false, v2_plus: true },
  warnings: ["No native libraries are bundled (pure Java/Kotlin, or libraries loaded at runtime)."],
  unavailable: [
    "App name — declared as a resource reference; needs the SDK's aapt2 to resolve.",
    "Minimum SDK — not declared in the manifest.",
  ],
};

export const activityRow: OperationRecord = {
  id: "op-1",
  kind: "install",
  status: "completed",
  started_at: 1_700_000_000_000,
  finished_at: 1_700_000_004_000,
  summary: "org.fdroid.fdroid installed",
  package_id: "org.fdroid.fdroid",
  detail: null,
};

export const inspectionOldTarget: ApkInspection = {
  ...inspectionPartial,
  path: "C:\Downloads\Expression Evaluator.apk",
  file_name: "dudeofx.eval_101.apk",
  package_id: "dudeofx.eval",
  label: "Expression Evaluator",
  version_name: "1.1",
  version_code: 101,
  min_sdk: 10,
  target_sdk: 17,
};

export const agentRunning: AgentStatus = {
  running: true,
  endpoint: "http://127.0.0.1:51234/mcp",
  token_file: "C:/Users/someone/AppData/Roaming/app.droidwin.desktop/agent/mcp.json",
  stdio_command: '"C:/DroidWin/droidwin.exe" --mcp-stdio',
  permission: "control",
  paused: false,
  tool_count: 27,
  actions: [
    {
      id: "act-1",
      at_ms: Date.now() - 4_000,
      tool: "tap",
      ok: true,
      summary: "Tapped 270,170 on display 2",
      duration_ms: 84,
      display_id: 2,
      package_id: "org.fdroid.fdroid",
    },
    {
      id: "act-0",
      at_ms: Date.now() - 9_000,
      tool: "type_text",
      ok: false,
      summary: "Android rejected that input.",
      duration_ms: 41,
      display_id: 2,
    },
  ],
  error_count: 1,
  skills: [
    {
      id: "ui-flow",
      name: "UI flow",
      description: "Launch an app and verify its screen.",
      path: "C:/Users/someone/AppData/Roaming/app.droidwin.desktop/skills/ui-flow",
      enabled: true,
      files: 2,
      bytes: 1200,
    },
  ],
};

export const agentStopped: AgentStatus = {
  ...agentRunning,
  running: false,
  endpoint: null,
  token_file: null,
  actions: [],
  skills: [],
  error_count: 0,
};

export function props(overrides: Partial<ScreenProps> = {}): ScreenProps {
  return {
    status: statusReady,
    env: envReady,
    apps: [appRunning],
    appsError: null,
    activity: [activityRow],
    busy: false,
    error: null,
    inspection: null,
    stages: [],
    opRunning: false,
    notice: null,
    appQuery: "",
    onAppQueryChange: () => {},
    onNavigate: () => {},
    onRefresh: () => {},
    onStartRuntime: () => {},
    onStopRuntime: () => {},
    onBrowse: () => {},
    onInstall: () => {},
    onLaunch: () => {},
    onStopApp: () => {},
    onUninstall: () => {},
    onOpenLogs: () => {},
    attention: null,
    attentionBusy: false,
    onAttentionCheck: () => {},
    onRecover: () => {},
    agent: agentStopped,
    onAgentStart: () => {},
    onAgentStop: () => {},
    onAgentPause: () => {},
    onAgentPermission: () => {},
    onAgentRefresh: () => {},
    onAgentInstallSkill: () => {},
    onAgentToggleSkill: () => {},
    onAgentRemoveSkill: () => {},
    ...overrides,
  };
}
