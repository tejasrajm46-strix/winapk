import { useEffect, useState } from "react";
import mark from "../assets/branding/droidwin-mark.png";
import * as backend from "../backend";
import { DEFAULT_APP_NAME } from "../backend";
import type {
  DisplayInfo,
  DwError,
  GpuMode,
  HardwareReport,
  KeyboardMode,
  PerfPreset,
  PlanMode,
} from "../backend";
import { IconAlert, IconFolder } from "../components/icons";
import { Button, Field, Panel, StatusPill } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";

const PRESETS: { id: PerfPreset; label: string; detail: string }[] = [
  {
    id: "balanced",
    label: "Balanced",
    detail: "The automatic plan: half the machine's threads at most, a quarter of RAM, host GPU.",
  },
  {
    id: "performance",
    label: "Performance",
    detail: "More vCPUs (still half the machine at most) and more RAM, with the host GPU.",
  },
  {
    id: "compatibility",
    label: "Compatibility",
    detail: "Software rendering: slower, but it depends least on the host graphics driver.",
  },
];

/** Display sizes a phone can plausibly use, relative to the AVD's own screen. */
const SIZES = [
  { label: "Android default", width: null as number | null, height: null as number | null },
  { label: "720 × 1600", width: 720, height: 1600 },
  { label: "1080 × 2400", width: 1080, height: 2400 },
  { label: "1440 × 3200", width: 1440, height: 3200 },
];

const DENSITIES = [
  { label: "Android default", value: null as number | null },
  { label: "280 dpi", value: 280 },
  { label: "420 dpi", value: 420 },
  { label: "480 dpi", value: 480 },
];

const KEYBOARD_LABEL: Record<KeyboardMode, string> = {
  auto: "Automatic — hardware keyboard on Android 9+",
  uhid: "Hardware keyboard (uhid)",
  sdk: "Simulated keys (sdk)",
};

function gb(mb: number): string {
  return `${(mb / 1024).toFixed(1)} GB`;
}

function effectiveSentence(report: HardwareReport): string {
  const facts = report.effective;
  if (!facts) return "The runtime is stopped, so Android has not reported anything yet.";
  const cores = facts.cores ? `${facts.cores} vCPUs` : "vCPU count unreadable";
  const ram = facts.ram_mb ? gb(facts.ram_mb) : "memory unreadable";
  const gpu =
    facts.hardware_gpu === true
      ? "hardware rendering"
      : facts.hardware_gpu === false
        ? "software rendering"
        : "renderer unknown";
  return `${cores} · ${ram} · ${gpu}`;
}

export function Settings(props: ScreenProps) {
  const { env, status, appName, onAppNameChange } = props;

  const [hardware, setHardware] = useState<HardwareReport | null>(null);
  const [hardwareError, setHardwareError] = useState<DwError | null>(null);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);

  const [mode, setMode] = useState<PlanMode>("auto");
  const [cores, setCores] = useState(4);
  const [ramMb, setRamMb] = useState(4096);
  const [gpu, setGpu] = useState<GpuMode>("host");

  const [keyboard, setKeyboard] = useState<KeyboardMode>("auto");
  const [keyboardBusy, setKeyboardBusy] = useState(false);

  // The emulator WINAPK started is stopped on close unless the user says not
  // to; the backend stores the choice, so it survives a restart.
  const [stopOnExit, setStopOnExitState] = useState(true);
  const [sdkBusy, setSdkBusy] = useState(false);
  const [sdkRoot, setSdkRoot] = useState<string | null>(null);
  const [sdkManaged, setSdkManaged] = useState(false);

  const [presetBusy, setPresetBusy] = useState(false);
  const [display, setDisplay] = useState<DisplayInfo | null>(null);
  const [displayError, setDisplayError] = useState<DwError | null>(null);
  const [sizeChoice, setSizeChoice] = useState(0);
  const [densityChoice, setDensityChoice] = useState(0);
  const [displayBusy, setDisplayBusy] = useState(false);

  // The draft starts from what the shell already shows, so the field never
  // disagrees with the name in the title bar until the user edits it.
  const [nameDraft, setNameDraft] = useState(appName);
  const [nameBusy, setNameBusy] = useState(false);
  const [nameError, setNameError] = useState<DwError | null>(null);
  useEffect(() => setNameDraft(appName), [appName]);
  // Read from the bundle, so the About panel cannot claim a version the
  // executable was not built with.
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const report = await backend.getHardware();
        if (!active) return;
        setHardware(report);
        setMode(report.plan.mode);
        setCores(report.plan.cores);
        setRamMb(report.plan.ram_mb);
        setGpu(report.plan.gpu);
        setKeyboard(await backend.getKeyboardMode());
        setStopOnExitState(await backend.getStopOnExit());
        const built = await backend.getAppVersion();
        if (active) setVersion(built);
        const setup = await backend.getSetupState();
        if (active) {
          setSdkRoot(setup.sdk_root);
          setSdkManaged(setup.managed);
        }
        if (active) setHardwareError(null);
      } catch (err) {
        if (active) setHardwareError(backend.toDwError(err));
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  const saveHardware = async () => {
    setSaving(true);
    setSaved(null);
    try {
      const report =
        mode === "auto"
          ? await backend.applyHardware("auto")
          : await backend.applyHardware("custom", cores, ramMb, gpu);
      setHardware(report);
      setMode(report.plan.mode);
      setHardwareError(null);
      setSaved(
        report.restart_required
          ? "Saved. Restart the runtime so the emulator boots with these values."
          : "Saved. The plan applies the next time the runtime starts.",
      );
    } catch (err) {
      setHardwareError(backend.toDwError(err));
    } finally {
      setSaving(false);
    }
  };

  // The display override is read from Android when the runtime is up; a stopped
  // runtime has no answer, and the panel says so instead of showing a default.
  useEffect(() => {
    if (!backend.isDesktop() || status?.state !== "ready") return;
    let live = true;
    backend
      .getPhoneDisplay()
      .then((next) => {
        if (!live) return;
        setDisplay(next);
        setDisplayError(null);
        const matchSize = SIZES.findIndex(
          (option) => option.width === next.width && option.height === next.height,
        );
        setSizeChoice(matchSize >= 0 ? matchSize : 0);
        const matchDensity = DENSITIES.findIndex((option) => option.value === next.density);
        setDensityChoice(matchDensity >= 0 ? matchDensity : 0);
      })
      .catch((err) => live && setDisplayError(backend.toDwError(err)));
    return () => {
      live = false;
    };
  }, [status?.state]);

  const applyPreset = async (preset: PerfPreset) => {
    setPresetBusy(true);
    setSaved(null);
    try {
      const report = await backend.applyHardwarePreset(preset);
      setHardware(report);
      setMode(report.plan.mode);
      setCores(report.plan.cores);
      setRamMb(report.plan.ram_mb);
      setGpu(report.plan.gpu);
      setHardwareError(null);
      setSaved(
        report.restart_required
          ? `${preset} preset saved. Restart the runtime so the emulator boots with it.`
          : `${preset} preset saved. It applies the next time the runtime starts.`,
      );
    } catch (err) {
      setHardwareError(backend.toDwError(err));
    } finally {
      setPresetBusy(false);
    }
  };

  const applyDisplay = async (reset: boolean) => {
    setDisplayBusy(true);
    setSaved(null);
    try {
      const next = reset
        ? await backend.resetPhoneDisplay()
        : await backend.setPhoneDisplay(
            SIZES[sizeChoice].width as number,
            SIZES[sizeChoice].height as number,
            DENSITIES[densityChoice].value ?? undefined,
          );
      setDisplay(next);
      setDisplayError(null);
      setSaved(
        reset
          ? "The phone display is back to the size and density its Android image defines."
          : `The phone display is now ${next.width} × ${next.height} at ${next.density ?? "its"} dpi. It applies immediately; app windows keep their own displays.`,
      );
    } catch (err) {
      setDisplayError(backend.toDwError(err));
    } finally {
      setDisplayBusy(false);
    }
  };

  const saveName = async () => {
    setNameBusy(true);
    setSaved(null);
    setNameError(null);
    try {
      const saved = await backend.setAppName(nameDraft.trim());
      onAppNameChange(saved);
      setNameDraft(saved);
      setSaved(
        `The application name is now ${saved}. Its window title changed too; windows opened before this keep the name they were created with.`,
      );
    } catch (err) {
      setNameError(backend.toDwError(err));
    } finally {
      setNameBusy(false);
    }
  };

  const resetName = async () => {
    setNameBusy(true);
    setSaved(null);
    setNameError(null);
    try {
      const saved = await backend.resetAppName();
      onAppNameChange(saved);
      setNameDraft(saved);
      setSaved(`The application name is back to ${saved}.`);
    } catch (err) {
      setNameError(backend.toDwError(err));
    } finally {
      setNameBusy(false);
    }
  };

  const changeKeyboard = async (next: KeyboardMode) => {
    setKeyboardBusy(true);
    setSaved(null);
    try {
      setKeyboard(await backend.setKeyboardMode(next));
      setSaved("Keyboard mode saved. It applies to windows opened from now on.");
    } catch (err) {
      setHardwareError(backend.toDwError(err));
    } finally {
      setKeyboardBusy(false);
    }
  };

  const toggleStopOnExit = async (next: boolean) => {
    setSaved(null);
    try {
      setStopOnExitState(await backend.setStopOnExit(next));
      setSaved(
        next
          ? "Saved. Closing WINAPK will stop the emulator it started."
          : "Saved. The emulator WINAPK started will keep running after WINAPK closes.",
      );
    } catch (err) {
      setHardwareError(backend.toDwError(err));
    }
  };

  const changeSdk = async (path: string | null) => {
    setSdkBusy(true);
    setSaved(null);
    try {
      const report = await backend.setSdkLocation(path);
      setSdkRoot(report.sdk_root);
      setSdkManaged(report.managed);
      setSaved(
        report.state === "ready"
          ? `Android SDK set to ${report.sdk_root}.`
          : `Android SDK set to ${report.sdk_root}. It is not installed there yet; the setup screen installs it.`,
      );
    } catch (err) {
      setHardwareError(backend.toDwError(err));
    } finally {
      setSdkBusy(false);
    }
  };

  const trimmedName = nameDraft.trim();
  const nameChanged = trimmedName !== appName;
  const canSaveName = trimmedName.length > 0 && nameChanged;

  return (
    <div className="page">
      <PageHead
        eyebrow="Management"
        title="Settings"
        lede="Every control here changes something real. Values that Android has confirmed are shown separately from the values WINAPK requested."
      >
        <Button variant="secondary" onClick={props.onOpenLogs}>
          <IconFolder size={14} />
          Open log folder
        </Button>
      </PageHead>

      {saved && (
        <div className="note note--info" role="status">
          {saved}
        </div>
      )}

      <div className="grid grid--2">
        <Panel title="Where things live" flush>
          <dl className="fields" style={{ padding: "0 var(--space-lg) var(--space-md)" }}>
            <Field label="Android SDK" value={<span className="mono">{env?.sdk_root ?? "not found"}</span>} missing={!env?.sdk_root} />
            <Field label="System image" value={<span className="mono">{env?.system_image ?? "not installed"}</span>} missing={!env?.system_image} />
            <Field label="Virtual device" value={<span className="mono">{status?.avd ?? "not created"}</span>} missing={!status?.avd} />
            <Field label="Window host" value={<span className="mono">{env?.scrcpy ?? "missing"}</span>} missing={!env?.scrcpy} />
            <Field label="Package inspector" value={<span className="mono">{env?.aapt2 ?? "missing (labels stay unresolved)"}</span>} missing={!env?.aapt2} />
          </dl>
        </Panel>

        <Panel title="About this build">
          <div className="about">
            <img className="about__mark" src={mark} alt={`${appName} logo`} />
            <div>
              <div className="row__title">{appName}</div>
              <div className="row__meta">Android apps in their own Windows windows</div>
            </div>
          </div>
          <dl className="fields">
            <Field
              label="Version"
              value={version ?? "unknown"}
              missing={!version}
              sub="Reported by this executable"
            />
            <Field label="Runtime model" value="One shared Android emulator, one virtual display per app window" />
            <Field label="Licensing" value="scrcpy is bundled under Apache-2.0. The Android SDK and emulator are provided by you and are not redistributed by WINAPK." />
            <Field label="Fonts" value="Instrument Serif and Onest, bundled under the SIL Open Font License 1.1" />
          </dl>
        </Panel>
      </div>

      <div className="grid grid--2">
        <Panel
          title="Android runtime"
          note="Where Android lives on this PC, and what closing WINAPK does to it."
        >
          <dl className="fields">
            <Field
              label="SDK directory"
              value={<span className="mono">{sdkRoot ?? "not found"}</span>}
              sub={sdkManaged ? "Installed by WINAPK" : "Existing SDK, used as it is"}
              missing={!sdkRoot}
            />
            <Field
              label="Emulator on close"
              value={stopOnExit ? "Stopped if WINAPK started it" : "Left running"}
            />
          </dl>
          <div className="inline">
            <Button
              variant="secondary"
              disabled={sdkBusy}
              onClick={() =>
                void (async () => {
                  try {
                    const path = await backend.pickSdkFolder();
                    if (path) await changeSdk(path);
                  } catch (err) {
                    setHardwareError(backend.toDwError(err));
                  }
                })()
              }
            >
              <IconFolder size={14} />
              Choose SDK folder
            </Button>
            {!sdkManaged && (
              <Button
                variant="ghost"
                disabled={sdkBusy}
                onClick={() => void changeSdk(null)}
              >
                Use WINAPK's folder
              </Button>
            )}
          </div>
          <label className="hw-field" style={{ marginTop: "var(--space-md)" }}>
            <span className="hw-field__label">Keep the emulator running after WINAPK closes</span>
            <input
              type="checkbox"
              checked={!stopOnExit}
              onChange={(event) => void toggleStopOnExit(!event.target.checked)}
            />
          </label>
        </Panel>

        <Panel
          title="Hardware allocation"
          note="Applied to the Android emulator only. Windows keeps the rest."
          action={
            hardware?.plan.gpu === "host" ? (
              <StatusPill tone="accent" label="Host GPU plan" />
            ) : undefined
          }
        >
          {hardwareError && (
            <div className="note note--warn" role="alert">
              <div className="note__head">
                <IconAlert size={14} />
                <span>{hardwareError.summary}</span>
              </div>
            </div>
          )}

          {hardware ? (
            <>
              <div className="hw-presets" role="group" aria-label="Performance preset">
                {PRESETS.map((preset) => (
                  <Button
                    key={preset.id}
                    size="sm"
                    variant={hardware.plan.preset === preset.id ? "primary" : "secondary"}
                    title={preset.detail}
                    aria-pressed={hardware.plan.preset === preset.id}
                    disabled={presetBusy || saving}
                    onClick={() => void applyPreset(preset.id)}
                  >
                    {preset.label}
                  </Button>
                ))}
              </div>

              <dl className="fields">
                <Field
                  label="Host CPU"
                  value={`${hardware.host.logical_cpus} logical processors`}
                />
                <Field
                  label="Host memory"
                  value={
                    hardware.host.memory_detected
                      ? `${gb(hardware.host.total_ram_mb)} total · ${gb(hardware.host.available_ram_mb)} free`
                      : "Windows did not report a memory size"
                  }
                  missing={!hardware.host.memory_detected}
                />
                <Field
                  label="Virtualization"
                  value={
                    hardware.acceleration_ok
                      ? "Hardware virtualization usable"
                      : "Not usable — the emulator would run without acceleration"
                  }
                  sub={hardware.acceleration ?? undefined}
                  missing={!hardware.acceleration_ok}
                />
                <Field label="Requested plan" value={hardware.plan.reason} />
                <Field
                  label="Android reports"
                  value={effectiveSentence(hardware)}
                  missing={!hardware.effective}
                />
              </dl>

              <div className="hw-form">
                <label className="hw-field">
                  <span className="hw-field__label">Mode</span>
                  <select
                    className="hw-select"
                    value={mode}
                    onChange={(event) => setMode(event.target.value as PlanMode)}
                  >
                    <option value="auto">Automatic</option>
                    <option value="custom">Custom</option>
                  </select>
                </label>

                {mode === "custom" && (
                  <>
                    <label className="hw-field">
                      <span className="hw-field__label">vCPUs</span>
                      <select
                        className="hw-select"
                        value={cores}
                        onChange={(event) => setCores(Number(event.target.value))}
                      >
                        {/* The emulator caps above 6 and says so in its log, so 8
                            is not offered: a plan it cannot apply is a promise. */}
                        {[2, 4, 6].map((value) => (
                          <option value={value} key={value}>
                            {value}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label className="hw-field">
                      <span className="hw-field__label">Memory</span>
                      <select
                        className="hw-select"
                        value={ramMb}
                        onChange={(event) => setRamMb(Number(event.target.value))}
                      >
                        {[2048, 3072, 4096, 6144, 8192].map((value) => (
                          <option value={value} key={value}>
                            {gb(value)}
                          </option>
                        ))}
                      </select>
                    </label>
                    <label className="hw-field">
                      <span className="hw-field__label">Graphics</span>
                      <select
                        className="hw-select"
                        value={gpu}
                        onChange={(event) => setGpu(event.target.value as GpuMode)}
                      >
                        <option value="host">Host GPU</option>
                        <option value="auto">Automatic</option>
                        <option value="software">Software (SwiftShader)</option>
                      </select>
                    </label>
                  </>
                )}

                <Button variant="primary" onClick={saveHardware} disabled={saving}>
                  {saving
                    ? "Saving…"
                    : mode === "auto"
                      ? "Use automatic plan"
                      : "Apply custom plan"}
                </Button>
              </div>

              {hardware.restart_required && (
                <div className="note note--warn">
                  <div className="note__head">
                    <IconAlert size={14} />
                    <span>Restart needed</span>
                  </div>
                  <p className="note__body">
                    The running emulator was started with different values. Stop and start the
                    runtime to boot it with this plan.
                  </p>
                </div>
              )}

              {hardware.notes.map((note) => (
                <p className="hw-note" key={note}>
                  {note}
                </p>
              ))}

              {hardware.effective?.gpu_renderer && (
                <p className="hw-renderer mono" title="The renderer Android reported">
                  {hardware.effective.gpu_renderer}
                </p>
              )}
            </>
          ) : (
            !hardwareError && <p className="muted">Reading host resources…</p>
          )}
        </Panel>

        <Panel
          title="Keyboard input"
          note="Applies to app windows opened after you change it."
        >
          <label className="hw-field">
            <span className="hw-field__label">Mode</span>
            <select
              className="hw-select"
              value={keyboard}
              disabled={keyboardBusy}
              onChange={(event) => void changeKeyboard(event.target.value as KeyboardMode)}
            >
              <option value="auto">Automatic — hardware keyboard on Android 9+</option>
              <option value="uhid">Hardware keyboard (uhid)</option>
              <option value="sdk">Simulated keys (sdk)</option>
            </select>
          </label>
          <p className="hw-note">
            A hardware keyboard sends Shift, Ctrl and Alt as their own key events, which is what
            games, remote desktops and apps that read raw keys expect. The simulated mode injects
            key events through Android&apos;s API instead; it is the fallback for runtimes that
            refuse a HID keyboard.
          </p>
          <p className="hw-note">Current mode: {KEYBOARD_LABEL[keyboard]}.</p>
        </Panel>

        <Panel
          title="Phone display"
          note="Android's own size and density, changed through wm. App windows keep the displays WINAPK created for them."
        >
          {displayError && (
            <div className="note note--warn" role="alert">
              <div className="note__head">
                <IconAlert size={14} />
                <span>{displayError.summary}</span>
              </div>
            </div>
          )}
          <dl className="fields">
            <Field
              label="Android reports"
              value={
                display
                  ? `${display.width ?? "?"} × ${display.height ?? "?"} at ${display.density ?? "?"} dpi`
                  : status?.state === "ready"
                    ? "Reading the display…"
                    : "The runtime is stopped, so Android has not reported it"
              }
              sub={
                display?.overridden
                  ? `Override in effect; the AVD's own display is ${display.default_width} × ${display.default_height}.`
                  : undefined
              }
              missing={!display}
            />
          </dl>
          <div className="hw-form">
            <label className="hw-field">
              <span className="hw-field__label">Size</span>
              <select
                className="hw-select"
                value={sizeChoice}
                disabled={displayBusy || !display}
                onChange={(event) => setSizeChoice(Number(event.target.value))}
              >
                {SIZES.map((option, index) => (
                  <option value={index} key={option.label}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <label className="hw-field">
              <span className="hw-field__label">Density</span>
              <select
                className="hw-select"
                value={densityChoice}
                disabled={displayBusy || !display}
                onChange={(event) => setDensityChoice(Number(event.target.value))}
              >
                {DENSITIES.map((option, index) => (
                  <option value={index} key={option.label}>
                    {option.label}
                  </option>
                ))}
              </select>
            </label>
            <Button
              variant="ghost"
              disabled={displayBusy || !display}
              onClick={() => void applyDisplay(true)}
            >
              Reset
            </Button>
            <Button
              variant="primary"
              disabled={displayBusy || !display || SIZES[sizeChoice].width === null}
              disabledReason={
                SIZES[sizeChoice].width === null
                  ? "Pick a size, or use Reset for Android's own display."
                  : undefined
              }
              onClick={() => void applyDisplay(false)}
            >
              {displayBusy ? "Applying…" : "Apply display"}
            </Button>
          </div>
          <p className="hw-note">
            This changes the phone screen and what Android reports for display 0. It does not
            restart the emulator, and it does not touch the size of the windows WINAPK opened
            for apps. A smaller display costs less to capture, which is the point of offering it
            on a slower machine.
          </p>
        </Panel>

        <Panel
          title="Application name"
          note="The name in the title bar, the top bar and the About panel."
        >
          {nameError && (
            <div className="note note--warn" role="alert">
              <div className="note__head">
                <IconAlert size={14} />
                <span>{nameError.summary}</span>
              </div>
              {nameError.detail && <p className="note__body">{nameError.detail}</p>}
            </div>
          )}
          <dl className="fields">
            <Field label="In use" value={appName} />
          </dl>
          <div className="hw-form">
            <label className="hw-field">
              <span className="hw-field__label">Display name</span>
              <input
                className="hw-select"
                type="text"
                value={nameDraft}
                // The backend enforces the same limit; this only stops the typing
                // that would be refused on save.
                maxLength={40}
                spellCheck={false}
                aria-label="Application display name"
                disabled={nameBusy}
                onChange={(event) => setNameDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && canSaveName) {
                    event.preventDefault();
                    void saveName();
                  }
                }}
              />
            </label>
            <Button
              variant="primary"
              disabled={nameBusy || !canSaveName}
              disabledReason={
                trimmedName.length === 0
                  ? "A name is required."
                  : `The name is already ${appName}.`
              }
              onClick={() => void saveName()}
            >
              {nameBusy ? "Saving…" : "Save name"}
            </Button>
            <Button
              variant="ghost"
              disabled={nameBusy || appName === DEFAULT_APP_NAME}
              disabledReason={`The shipped name ${DEFAULT_APP_NAME} is already in use.`}
              onClick={() => void resetName()}
            >
              Reset to {DEFAULT_APP_NAME}
            </Button>
          </div>
          <p className="hw-note">
            This is branding only. The executable, the installer&apos;s product name and the package
            identifier stay as they were built, so shortcuts, updates and installed copies keep
            working. The name is saved locally and is already in effect next time you start.
          </p>
        </Panel>
      </div>

      <Panel title="Not implemented yet" note="Listed so nothing here looks like a missing switch.">
        <ul className="stack stack--sm secondary" style={{ paddingLeft: 18, margin: 0 }}>
          <li>Deleting an app&apos;s Android data from the library (the backend supports it; the control is not built).</li>
          <li>Automatic SDK provisioning — WINAPK tells you what to install instead of downloading Google&apos;s SDK.</li>
          <li>Per-window audio is not routed: Android&apos;s sound is played once by the emulator, so every open window shares the device&apos;s audio rather than each contributing its own stream.</li>
          <li>Cloud sync, accounts, or any telemetry.</li>
        </ul>
        <div className="note note--warn">
          <div className="note__head">
            <IconAlert size={14} />
            <span>Compatibility is not universal</span>
          </div>
          <p className="note__body">
            Apps are only supported if their CPU ABI, Android API level, graphics use and services
            requirements match the runtime. ARM-only packages cannot run on this x86_64 image, and
            apps that insist on Google Play services will fail. WINAPK reports what Android reports
            rather than predicting compatibility.
          </p>
        </div>
      </Panel>
    </div>
  );
}
