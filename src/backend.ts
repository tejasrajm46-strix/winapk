/**
 * The only module that talks to the Rust backend. Screens import these
 * functions and never build a command name or a backend invocation themselves.
 *
 * Types here mirror the Rust serde shapes one-for-one; if one side changes the
 * other stops compiling, which is the point.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ErrorCode =
  | "invalid_path"
  | "invalid_package"
  | "package_too_large"
  | "corrupt_archive"
  | "missing_manifest"
  | "unsupported_package"
  | "runtime_not_configured"
  | "runtime_unavailable"
  | "runtime_start_failed"
  | "install_failed"
  | "uninstall_failed"
  | "launch_failed"
  | "not_installed"
  | "timeout"
  | "cancelled"
  | "backend_failure"
  | "storage_failure"
  // Frontend-only: the webview is running without the Rust side (browser dev).
  | "backend_unavailable";

export interface DwError {
  code: ErrorCode;
  summary: string;
  detail?: string;
}

export type RuntimeState = "not_configured" | "not_running" | "starting" | "ready" | "unhealthy";

export interface Capabilities {
  install: boolean;
  uninstall: boolean;
  enumerate: boolean;
  launch: boolean;
  independent_windows: boolean;
  concurrent_apps: boolean;
  clipboard: boolean;
  file_drop: boolean;
  audio: boolean;
  hardware_accel: boolean;
}

export interface RuntimeStatus {
  backend: string;
  state: RuntimeState;
  detail: string;
  avd: string | null;
  serial: string | null;
  android_api: number | null;
  capabilities: Capabilities;
  limitations: string[];
}

export interface EnvReport {
  sdk_root: string | null;
  adb: string | null;
  emulator: string | null;
  avdmanager: string | null;
  sdkmanager: string | null;
  aapt2: string | null;
  scrcpy: string | null;
  system_image: string | null;
  avds: string[];
  accel: string | null;
  blocking: string[];
}

export interface RuntimeControl {
  status: RuntimeStatus;
  env: EnvReport;
}

export interface SignatureInfo {
  v1: boolean;
  v2_plus: boolean;
}

export interface ApkInspection {
  path: string;
  file_name: string;
  package_id: string;
  label: string | null;
  version_name: string | null;
  version_code: number | null;
  min_sdk: number | null;
  target_sdk: number | null;
  abis: string[];
  size_bytes: number;
  icon: string | null;
  signature: SignatureInfo;
  warnings: string[];
  unavailable: string[];
}

export interface InstalledApp {
  package_id: string;
  label: string | null;
  version_name: string | null;
  version_code: number | null;
  running: boolean;
  window_title: string | null;
  icon: string | null;
  user_installed: boolean;
}

export type Stage =
  | "received"
  | "validating"
  | "inspecting"
  | "checking_runtime"
  | "preparing_runtime"
  | "installing"
  | "verifying"
  | "launching"
  | "completed"
  | "failed"
  | "cancelled"
  | "needs_user_action";

export interface StageEvent {
  op: string;
  package_id: string | null;
  stage: Stage;
  message: string;
  at_ms: number;
}

export interface OperationRecord {
  id: string;
  kind: string;
  status: string;
  started_at: number;
  finished_at: number | null;
  summary: string;
  package_id: string | null;
  detail: string | null;
}

export interface AppWindow {
  package_id: string;
  title: string;
  pid: number;
}

export interface InstallOutcome {
  package_id: string;
  version_name: string | null;
  version_code: number | null;
  updated: boolean;
  message: string;
}

export interface Paths {
  data_dir: string;
  log_dir: string;
  avd: string;
  system_image: string;
}

/** True when the page is hosted by the Tauri shell rather than a plain browser. */
export function isDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function toDwError(error: unknown): DwError {
  if (typeof error === "object" && error !== null && "code" in error && "summary" in error) {
    return error as DwError;
  }
  const summary = error instanceof Error ? error.message : String(error);
  return { code: "backend_failure", summary: summary || "The backend did not answer." };
}

function unavailable(): DwError {
  return {
    code: "backend_unavailable",
    summary: "The DroidWin backend is not running in this window.",
    detail:
      "This page was opened in a browser. Launch the desktop app (pnpm tauri dev) to inspect, install and run packages.",
  };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isDesktop()) throw unavailable();
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    throw toDwError(error);
  }
}

export const getEnv = () => call<EnvReport>("env_probe");
export const getRuntimeStatus = () => call<RuntimeStatus>("runtime_status");
export const probeRuntime = () => call<RuntimeControl>("runtime_probe");
export const startRuntime = () => call<RuntimeStatus>("runtime_start");
export const stopRuntime = () => call<void>("runtime_stop");
export const inspectApk = (path: string) => call<ApkInspection>("inspect_apk", { path });
export const installApk = (path: string, launch: boolean) =>
  call<InstallOutcome>("install_apk", { path, launch });
export const listApps = () => call<InstalledApp[]>("list_apps");
export const launchApp = (packageId: string) => call<AppWindow>("launch_app", { packageId });
export const closeAppWindow = (packageId: string) =>
  call<void>("close_app_window", { packageId });
export const stopApp = (packageId: string) => call<void>("stop_app", { packageId });
export const uninstallApp = (packageId: string, deleteData: boolean) =>
  call<void>("uninstall_app", { packageId, deleteData });
export const getActivity = (limit = 20) => call<OperationRecord[]>("activity", { limit });
export const getPaths = () => call<Paths>("paths");
export const openLogs = () => call<void>("open_logs");

/** Native file picker filtered to packages. Returns null when cancelled. */
export async function pickApk(): Promise<string | null> {
  if (!isDesktop()) throw unavailable();
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    directory: false,
    title: "Choose an APK",
    filters: [{ name: "Android package", extensions: ["apk"] }],
  });
  if (typeof selected === "string") return selected;
  return null;
}

export function onStage(handler: (event: StageEvent) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<StageEvent>("op://stage", (event) => handler(event.payload));
}

export function onRuntimeStatus(handler: (status: RuntimeStatus) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<RuntimeStatus>("runtime://status", (event) => handler(event.payload));
}

export function onAppsChanged(handler: () => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen("apps://changed", () => handler());
}

/** A second launch of DroidWin forwards its .apk path here. */
export function onApkActivation(handler: (path: string) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<string>("activation://apk", (event) => handler(event.payload));
}

/** Files dropped onto the window, resolved to absolute paths by the shell. */
export function onFileDrop(handler: (paths: string[]) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  let stop: UnlistenFn | undefined;
  void import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop") handler(event.payload.paths);
      })
      .then((unlisten) => {
        stop = unlisten;
      }),
  );
  return Promise.resolve(() => stop?.());
}
