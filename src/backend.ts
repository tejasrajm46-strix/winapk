/**
 * The only module that talks to the Rust backend. Screens import these
 * functions and never build a command name or a backend invocation themselves.
 *
 * Types here mirror the Rust serde shapes one-for-one; if one side changes the
 * other stops compiling, which is the point.
 */

import { Channel, invoke } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ErrorCode =
  | "invalid_path"
  | "invalid_package"
  | "package_too_large"
  | "corrupt_archive"
  | "missing_manifest"
  | "unsupported_package"
  | "invalid_config"
  | "invalid_request"
  | "not_supported"
  | "permission_denied"
  | "automation_paused"
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

/** One Android runtime component as the setup screen lists it. */
export interface SetupComponent {
  id: string;
  label: string;
  present: boolean;
  detail: string;
  download_mb: number | null;
  required: boolean;
}

export type SetupState = "ready" | "starting" | "needs_install" | "installing" | "failed";

/** What the Android runtime currently has, and what it would take to finish it. */
export interface SetupReport {
  state: SetupState;
  sdk_root: string;
  managed: boolean;
  configured: boolean;
  components: SetupComponent[];
  installed_at: number | null;
  error: string | null;
  disk_free_mb: number;
  download_mb: number;
  licenses_accepted: boolean;
  system_image: string;
  avd: string;
  message: string;
}

/** Progress of a running installation. `percent` is null when it is unknown. */
export interface SetupProgress {
  step: string;
  message: string;
  percent: number | null;
  done: boolean;
  failed: string | null;
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
  /** Manifest target SDK from install time; null when WINAPK did not install it. */
  target_sdk: number | null;
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

/** One Android action a physical key can be bound to. */
export type InputAction =
  | { kind: "key_event"; keycode: string }
  | { kind: "touch_tap"; x: number; y: number }
  | { kind: "touch_hold"; x: number; y: number };

export interface InputMapping {
  /** Windows virtual-key code, which is what the editor captures and the hook sees. */
  key: number;
  label: string;
  action: InputAction;
  enabled: boolean;
}

/** The display a touch target is measured in. */
export interface SurfaceInfo {
  display_id: number | null;
  width: number;
  height: number;
  orientation: "portrait" | "landscape";
}

export interface InputState {
  package_id: string;
  enabled: boolean;
  mappings: InputMapping[];
  screen: SurfaceInfo | null;
}

/**
 * The control overlay drawn over an app window.
 *
 * Positions are percentages of the Android display. The overlay fits them into
 * the video's letterboxed area, so the same label stays on the same game control
 * at any window size.
 */
export interface OverlayMapping {
  key: number;
  label: string;
  action: string;
  x: number | null;
  y: number | null;
}

export interface OverlayData {
  package_id: string;
  app_label: string;
  enabled: boolean;
  mappings: OverlayMapping[];
  display_width: number;
  display_height: number;
  labels_visible: boolean;
  /** The next click on the game picture chooses a mapping point. */
  placing: boolean;
  /** The point already chosen on the game, waiting for a key. */
  pending: { x: number; y: number } | null;
}

export interface OverlayStatus {
  package_id: string | null;
  labels_visible: boolean;
  /** The key-mapping panel is open over that window. */
  mapper_open: boolean;
  visible: boolean;
  failure: string | null;
}

/** One Android navigation key the overlay's bar can send. */
export type OverlayNavKey = "BACK" | "HOME" | "APP_SWITCH";

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

/** One still of the device's own screen, which is what the virtual phone shows. */
export interface PhoneScreen {
  display_id: number;
  /** Capture size in device pixels; the frame scales the image to fit. */
  width: number;
  height: number;
  /** `data:image/png;base64,…`, the same shape app icons travel in. */
  image: string;
  captured_at: number;
}

export interface SavedShot {
  path: string;
  width: number;
  height: number;
  bytes: number;
  captured_at: number;
}

export interface RecordingState {
  active: boolean;
  started_at: number | null;
  elapsed_ms: number;
  /** Where the finished file will be written. */
  directory: string | null;
}

export interface SavedRecording {
  path: string;
  bytes: number;
  started_at: number;
  finished_at: number;
  duration_ms: number;
}

/** Android's own display size and density, as the device reports them. */
export interface DisplayInfo {
  default_width: number | null;
  default_height: number | null;
  width: number | null;
  height: number | null;
  default_density: number | null;
  density: number | null;
  overridden: boolean;
}

export type PerfPreset = "balanced" | "performance" | "compatibility";

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

export interface HostFacts {
  logical_cpus: number;
  total_ram_mb: number;
  available_ram_mb: number;
  /** False when Windows would not report memory; the UI must not invent a value. */
  memory_detected: boolean;
}

export type PlanMode = "auto" | "custom";
export type GpuMode = "auto" | "host" | "software";

export interface HardwarePlan {
  mode: PlanMode;
  cores: number;
  ram_mb: number;
  gpu: GpuMode;
  reason: string;
  /** The named preset this plan came from, or null for a hand-set plan. */
  preset: PerfPreset | null;
}

export interface EffectiveHardware {
  cores: number | null;
  ram_mb: number | null;
  gpu_renderer: string | null;
  hardware_gpu: boolean | null;
}

export interface HardwareReport {
  host: HostFacts;
  plan: HardwarePlan;
  /** What Android reported; null while the runtime is stopped. */
  effective: EffectiveHardware | null;
  /** The emulator's own -accel-check answer, verbatim. */
  acceleration: string | null;
  acceleration_ok: boolean;
  restart_required: boolean;
  notes: string[];
}

/** "Attention Needed": confirmed observations and unconfirmed suspicions. */
export type AttentionConfidence = "confirmed" | "suspected";
export type AttentionSeverity = "notice" | "degraded" | "blocking";
export type AttentionScope = "environment" | "runtime" | "app";

/** A recovery that names exactly what it touches. */
export type AttentionRecovery =
  | { kind: "start_runtime" }
  | { kind: "reopen_window"; package_id: string }
  | { kind: "retry_launch"; package_id: string }
  | { kind: "reinstall_package"; package_id: string }
  | { kind: "restart_runtime" }
  | { kind: "recheck" }
  | { kind: "none" };

export interface AttentionEvidence {
  source: string;
  detail: string;
}

export interface AttentionProblem {
  id: string;
  scope: AttentionScope;
  package_id?: string;
  severity: AttentionSeverity;
  confidence: AttentionConfidence;
  title: string;
  observed: string;
  evidence: AttentionEvidence[];
  recommendation: string;
  recovery: AttentionRecovery;
  alternatives?: AttentionRecovery[];
}

export interface AttentionReport {
  checked_at_ms: number;
  deep: boolean;
  problems: AttentionProblem[];
  confirmed: number;
  suspected: number;
  checked: string[];
}

export interface RecoveryOutcome {
  ok: boolean;
  summary: string;
  detail?: string;
  report: AttentionReport;
}

export type KeyboardMode = "auto" | "uhid" | "sdk";

export type AgentPermission = "read_only" | "control";

/** One action an agent took, as recorded by the backend. */
export interface AgentAction {
  id: string;
  at_ms: number;
  tool: string;
  ok: boolean;
  summary: string;
  duration_ms: number;
  display_id?: number;
  package_id?: string;
}

export interface AgentSkill {
  id: string;
  name: string;
  description: string;
  path: string;
  enabled: boolean;
  files: number;
  bytes: number;
  invalid?: string;
}

export interface AgentStatus {
  running: boolean;
  endpoint: string | null;
  token_file: string | null;
  stdio_command: string;
  permission: AgentPermission;
  paused: boolean;
  tool_count: number;
  actions: AgentAction[];
  error_count: number;
  skills: AgentSkill[];
}

/** A tool as an MCP client sees it. The schema is opaque to the UI. */
export interface AgentTool {
  name: string;
  title: string;
  description: string;
  inputSchema: unknown;
  annotations?: { readOnlyHint?: boolean };
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
    summary: "The WINAPK backend is not running in this window.",
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
export const getSetupState = () => call<SetupReport>("runtime_setup_state");
export const startSetup = (acceptLicenses: boolean) =>
  call<SetupReport>("runtime_setup_start", { acceptLicenses });
export const cancelSetup = () => call<void>("runtime_setup_cancel");
/** `null` returns to WINAPK's own SDK directory. */
export const setSdkLocation = (path: string | null) =>
  call<SetupReport>("runtime_setup_location", { path });
export const getStopOnExit = () => call<boolean>("runtime_stop_on_exit");
export const setStopOnExit = (enabled: boolean) =>
  call<boolean>("set_runtime_stop_on_exit", { enabled });

/** The shipped name. Kept in step with `branding::DEFAULT_NAME` in the backend. */
export const DEFAULT_APP_NAME = "WINAPK";
/** The display name the title bar and the shell show. */
export const getAppName = () => call<string>("app_display_name");
/** Persist a name; the backend answers with the value now in effect. */
export const setAppName = (name: string) =>
  call<string>("set_app_display_name", { name });
/** Return to the shipped name. */
export const resetAppName = () => call<string>("reset_app_display_name");
/** The version the executable was built with. */
export const getAppVersion = () => call<string>("app_version");
export const inspectApk = (path: string) => call<ApkInspection>("inspect_apk", { path });
export const installApk = (path: string, launch: boolean, allowOldTarget = false) =>
  call<InstallOutcome>("install_apk", { path, launch, allowOldTarget });
export const listApps = () => call<InstalledApp[]>("list_apps");
export const launchApp = (packageId: string) => call<AppWindow>("launch_app", { packageId });
export const closeAppWindow = (packageId: string) =>
  call<void>("close_app_window", { packageId });
// Keyboard mapping. Off for every app until its profile is switched on.
export const getInputState = (packageId: string) =>
  call<InputState>("input_state", { packageId });
export const saveInputMapping = (packageId: string, key: number, action: InputAction) =>
  call<InputState>("save_input_mapping", { packageId, key, action });
export const removeInputMapping = (packageId: string, key: number) =>
  call<InputState>("remove_input_mapping", { packageId, key });
export const setInputMappingEnabled = (packageId: string, key: number, enabled: boolean) =>
  call<InputState>("set_input_mapping_enabled", { packageId, key, enabled });
export const setInputProfileEnabled = (packageId: string, enabled: boolean) =>
  call<InputState>("set_input_profile_enabled", { packageId, enabled });
export const resetInputMappings = (packageId: string) =>
  call<InputState>("reset_input_mappings", { packageId });
/** One Android control that keeps the same key in every app. */
export interface DetectedControls {
  package_id: string;
  controls: string[];
  note: string;
}
export interface UniversalBinding {
  action: string;
  label: string;
  chord: string;
  android_key: string | null;
}export const universalBindings = () => call<UniversalBinding[]>("universal_bindings");
export const setUniversalBinding = (action: string, chord: string) =>
  call<UniversalBinding[]>("set_universal_binding", { action, chord });
export const stopApp = (packageId: string) => call<void>("stop_app", { packageId });
// The overlay over the app window in front. It reads the same mapping row the
// Controls dialog writes and sends nav keys through the same runtime.
export const overlayCurrent = () => call<OverlayData | null>("overlay_current");
export const overlayStatus = () => call<OverlayStatus>("overlay_status");
export const overlayToggleLabels = () => call<OverlayStatus>("overlay_toggle_labels");
/** Open or close the key-mapping panel over the app window. */
export const overlayToggleMapper = (open: boolean, packageId?: string) =>
  call<OverlayStatus>("overlay_toggle_mapper", { open, packageId: packageId ?? null });
export const overlayHide = () => call<OverlayStatus>("overlay_hide");
/** Arm click-to-place: the next click on the game picture picks the point. */
export const overlayBeginPlace = (packageId?: string) =>
  call<OverlayStatus>("overlay_begin_place", { packageId: packageId ?? null });
/** The click that landed on the game, as percentages of the Android display. */
export const overlayPlaceAt = (packageId: string, x: number, y: number) =>
  call<OverlayStatus>("overlay_place_at", { packageId, x, y });
export const overlayCancelPlace = () => call<OverlayStatus>("overlay_cancel_place");
export const overlayClearPending = () => call<OverlayStatus>("overlay_clear_pending");
export const overlayNav = (packageId: string, key: OverlayNavKey) =>
  call<void>("overlay_nav", { packageId, key });
export const overlayOpenControls = (packageId: string) =>
  call<void>("overlay_open_controls", { packageId });
// Virtual phone: a still of the device's own screen, and the touches that go
// back to it. Coordinates are device pixels, resolved by the caller from the
// capture it is displaying.
export const getPhoneScreen = () => call<PhoneScreen>("phone_screen");
export const startPhoneStream = (id: string, onData: (data: ArrayBuffer) => void) => {
  const output = new Channel<ArrayBuffer>(onData);
  return call<void>("phone_stream_start", { id, output });
};
export const stopPhoneStream = (id: string) => call<void>("phone_stream_stop", { id });
export const getPhoneDimensions = () => call<[number, number]>("phone_dimensions");
/** Save a still of the Android screen into the user's Pictures folder. */
export const phoneScreenshot = () => call<SavedShot>("phone_screenshot");
/** One still of a single app window's own display — the rail's Screenshot. */
export const appScreenshot = (packageId: string) =>
  call<SavedShot>("app_screenshot", { packageId });
/** Reopen the window in the other orientation (the runtime has no live rotate). */
export const rotateAppWindow = (packageId: string) =>
  call<AppWindow>("rotate_app_window", { packageId });
export const startRecording = () => call<RecordingState>("recording_start");
export const stopRecording = () => call<SavedRecording>("recording_stop");
export const getRecordingStatus = () => call<RecordingState>("recording_status");
// Recording one app window mirrors that window's own Android display, not the
// phone's screen. One recording runs at a time.
export const detectControls = (packageId: string) =>
  call<DetectedControls>("detect_controls", { packageId });
export const windowRecordingStart = (packageId: string) =>
  call<RecordingState>("window_recording_start", { packageId });
export const windowRecordingStop = () => call<SavedRecording>("window_recording_stop");
export const windowRecordingStatus = () => call<RecordingState>("window_recording_status");
export const getPhoneDisplay = () => call<DisplayInfo>("phone_display");
export const setPhoneDisplay = (width: number, height: number, density?: number) =>
  call<DisplayInfo>("set_phone_display", { width, height, density });
export const resetPhoneDisplay = () => call<DisplayInfo>("reset_phone_display");
/** Show a capture WINAPK saved in Explorer. Other paths are refused. */
export const revealPath = (path: string) => call<void>("reveal_path", { path });
export const phoneTap = (x: number, y: number) => call<void>("phone_tap", { x, y });
export const phoneSwipe = (x1: number, y1: number, x2: number, y2: number, durationMs = 220) =>
  call<void>("phone_swipe", { x1, y1, x2, y2, durationMs });
/** Type into the phone. The backend spells spaces the way Android's `input` needs. */
export const phoneText = (text: string) => call<void>("phone_text", { text });
/** One Android key event on the phone, by name: ENTER, TAB, DEL, HOME, BACK, … */
export const phoneKey = (key: string) => call<void>("phone_key", { key });
export const uninstallApp = (packageId: string, deleteData: boolean) =>
  call<string>("uninstall_app", { packageId, deleteData });
export const getActivity = (limit = 20) => call<OperationRecord[]>("activity", { limit });
export const getHardware = () => call<HardwareReport>("hardware_report");
export const applyHardwarePreset = (preset: PerfPreset) =>
  call<HardwareReport>("hardware_preset", { preset });
export const applyHardware = (
  mode: PlanMode,
  cores?: number,
  ramMb?: number,
  gpu?: GpuMode,
) => call<HardwareReport>("hardware_apply", { mode, cores, ramMb, gpu });
export const getKeyboardMode = () => call<KeyboardMode>("keyboard_mode");
export const setKeyboardMode = (mode: KeyboardMode) =>
  call<KeyboardMode>("set_keyboard_mode", { mode });
export const getPaths = () => call<Paths>("paths");
export const openLogs = () => call<void>("open_logs");

/** `deep` captures each live window twice to confirm whether it is drawing. */
export const getAttention = (deep = false) =>
  call<AttentionReport>("attention_report", { deep });
export const recoverProblem = (recovery: AttentionRecovery) =>
  call<RecoveryOutcome>("attention_recover", { recovery });

export const getAgentStatus = () => call<AgentStatus>("agent_status");
export const startAgent = () => call<AgentStatus>("agent_start");
export const stopAgent = () => call<AgentStatus>("agent_stop");
export const setAgentPaused = (paused: boolean) =>
  call<AgentStatus>("agent_set_paused", { paused });
export const setAgentPermission = (permission: AgentPermission) =>
  call<AgentStatus>("agent_set_permission", { permission });
export const setAgentSkill = (id: string, enabled: boolean) =>
  call<AgentSkill[]>("agent_set_skill", { id, enabled });
export const removeAgentSkill = (id: string) => call<AgentSkill[]>("agent_remove_skill", { id });
export const installAgentSkill = (path: string) =>
  call<AgentSkill[]>("agent_install_skill", { path });
export const getAgentTools = () => call<AgentTool[]>("agent_tool_catalog");

/** Native folder picker for installing a skill. Returns null when cancelled. */
export async function pickSkillFolder(): Promise<string | null> {
  if (!isDesktop()) throw unavailable();
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    directory: true,
    title: "Choose a folder containing SKILL.md",
  });
  if (typeof selected === "string") return selected;
  return null;
}

/** Native folder picker for the Android SDK directory. Null when cancelled. */
export async function pickSdkFolder(): Promise<string | null> {
  if (!isDesktop()) throw unavailable();
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    multiple: false,
    directory: true,
    title: "Choose an Android SDK folder",
  });
  if (typeof selected === "string") return selected;
  return null;
}

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

export function onSetupProgress(handler: (progress: SetupProgress) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<SetupProgress>("runtime://setup", (event) => handler(event.payload));
}

export function onAppsChanged(handler: () => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen("apps://changed", () => handler());
}

/** A second launch of WINAPK forwards its .apk path here. */
export function onApkActivation(handler: (path: string) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<string>("activation://apk", (event) => handler(event.payload));
}

/**
 * Packages queued before the webview was ready (cold start from Explorer, or a
 * forwarding while the window was loading). Ask once on mount.
 */
export const takePendingActivation = () => call<string[]>("take_pending_activation");

/** Fresh overlay data, sent whenever the attached window moves or its mapping changes. */
export function onOverlayData(handler: (data: OverlayData) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<OverlayData>("overlay://data", (event) => handler(event.payload));
}

/** The nav bar's Controls button asks the manager window to open the editor. */
export function onOverlayControls(handler: (packageId: string) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<string>("overlay://controls", (event) => handler(event.payload));
}

/** An error the nav bar hit, shown over the game by the click-through label layer. */
export function onOverlayError(handler: (message: string) => void): Promise<UnlistenFn> {
  if (!isDesktop()) return Promise.resolve(() => {});
  return listen<string>("overlay://error", (event) => handler(event.payload));
}

/** Report a nav-bar failure where the user is looking, over the game. */
export function emitOverlayError(message: string): void {
  if (!isDesktop()) return;
  void emit("overlay://error", message);
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
