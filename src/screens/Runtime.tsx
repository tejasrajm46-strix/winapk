import { runtimeTone } from "../components/AppShell";
import { IconAlert, IconFolder, IconPlay, IconRefresh, IconStop } from "../components/icons";
import { Button, ErrorNote, Field, Panel, StatusPill } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";

const CAPABILITY_LABELS: { key: keyof NonNullable<ScreenProps["status"]>["capabilities"]; label: string }[] = [
  { key: "install", label: "Install packages" },
  { key: "uninstall", label: "Uninstall packages" },
  { key: "enumerate", label: "List installed apps" },
  { key: "launch", label: "Launch apps" },
  { key: "independent_windows", label: "Independent window per app" },
  { key: "concurrent_apps", label: "Several apps at once" },
  { key: "clipboard", label: "Clipboard between host and app" },
  { key: "file_drop", label: "Drop files onto an app window" },
  { key: "audio", label: "Audio forwarding" },
  { key: "hardware_accel", label: "Hardware acceleration" },
];

export function RuntimeScreen(props: ScreenProps) {
  const { status, env, busy } = props;
  const runtime = runtimeTone(status);

  return (
    <div className="page">
      <PageHead
        eyebrow="Management"
        title="Runtime"
        lede="What DroidWin measured on this machine. Capabilities are only marked supported after they were verified."
      >
        <Button variant="ghost" onClick={props.onRefresh} disabled={busy}>
          <IconRefresh size={14} />
          Re-probe
        </Button>
        {status?.state === "ready" ? (
          <Button variant="danger" onClick={props.onStopRuntime} disabled={busy}>
            <IconStop size={14} />
            Stop runtime
          </Button>
        ) : (
          <Button
            variant="primary"
            onClick={props.onStartRuntime}
            disabled={busy || status?.state === "not_configured"}
          >
            <IconPlay size={14} />
            Start runtime
          </Button>
        )}
      </PageHead>

      {props.error && <ErrorNote error={props.error} />}

      <div className="grid grid--aside">
        <Panel title="Backend" action={<StatusPill tone={runtime.tone} label={runtime.label} />}>
          <p className="secondary">{status?.detail ?? "Not probed yet."}</p>
          <dl className="fields">
            <Field label="Backend" value={status?.backend ?? "—"} />
            <Field label="Virtual device" value={status?.avd ?? "Not configured"} missing={!status?.avd} />
            <Field label="adb serial" value={status?.serial ?? "Not connected"} missing={!status?.serial} />
            <Field
              label="Android API"
              value={status?.android_api ? String(status.android_api) : "Unknown"}
              missing={!status?.android_api}
            />
            <Field
              label="Acceleration"
              value={
                env?.accel ? (
                  <span className="mono">{env.accel.replace(/\s+/g, " ").trim()}</span>
                ) : (
                  "Emulator did not report"
                )
              }
              missing={!env?.accel}
            />
          </dl>
          {status?.state === "not_configured" && (
            <div className="note note--warn">
              <div className="note__head">
                <IconAlert size={14} />
                <span>Setup needed</span>
              </div>
              <p className="note__body">
                DroidWin uses an Android SDK you install yourself — it does not redistribute Google's
                emulator. Install the missing packages with the SDK manager, then re-probe.
              </p>
            </div>
          )}
        </Panel>

        <Panel title="Capabilities" note="Only what was verified on this machine is marked supported.">
          <dl className="fields">
            {CAPABILITY_LABELS.map(({ key, label }) => {
              const supported = status?.capabilities?.[key] ?? false;
              return (
                <div className="field" key={key}>
                  <dt className="field__key">{label}</dt>
                  <dd className="field__value">
                    {supported ? (
                      <StatusPill tone="ok" label="Supported" />
                    ) : (
                      <StatusPill tone="idle" label="Not verified" />
                    )}
                  </dd>
                </div>
              );
            })}
          </dl>
        </Panel>
      </div>

      <div className="grid grid--2">
        <Panel title="Limits of this backend">
          <ul className="stack stack--sm secondary" style={{ paddingLeft: 18, margin: 0 }}>
            {(status?.limitations ?? ["Runtime not probed yet."]).map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
        </Panel>

        <Panel title="Detected tools" note="Found on this machine, read-only.">
          <dl className="fields">
            <Field label="SDK" value={env?.sdk_root ?? "Not found"} missing={!env?.sdk_root} />
            <Field label="System image" value={env?.system_image ?? "Not installed"} missing={!env?.system_image} />
            <Field label="adb" value={<span className="mono">{env?.adb ?? "missing"}</span>} missing={!env?.adb} />
            <Field
              label="Emulator"
              value={<span className="mono">{env?.emulator ?? "missing"}</span>}
              missing={!env?.emulator}
            />
            <Field label="aapt2" value={<span className="mono">{env?.aapt2 ?? "missing"}</span>} missing={!env?.aapt2} />
            <Field label="scrcpy" value={<span className="mono">{env?.scrcpy ?? "missing"}</span>} missing={!env?.scrcpy} />
            <Field
              label="AVDs"
              value={env?.avds.length ? env.avds.join(", ") : "None"}
              missing={!env?.avds.length}
            />
          </dl>
          <div className="inline">
            <Button variant="secondary" onClick={props.onOpenLogs}>
              <IconFolder size={14} />
              Open log folder
            </Button>
          </div>
          {env?.blocking && env.blocking.length > 0 && (
            <div className="note note--warn">
              <div className="note__head">
                <IconAlert size={14} />
                <span>Missing pieces</span>
              </div>
              <ul className="note__body" style={{ paddingLeft: 18, margin: 0 }}>
                {env.blocking.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </div>
          )}
        </Panel>
      </div>
    </div>
  );
}
