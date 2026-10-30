import { useState } from "react";
import { DropZone } from "../components/DropZone";
import { StageList } from "../components/StageList";
import { StageProgress } from "../components/StageProgress";
import { IconAlert, IconFolder, IconInstall, IconPlay } from "../components/icons";
import { Button, EmptyState, ErrorNote, Field, Panel, StatusPill } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";

function humanSize(bytes: number): string {
  const mb = bytes / (1024 * 1024);
  return mb >= 1 ? `${mb.toFixed(1)} MB` : `${Math.round(bytes / 1024)} KB`;
}

export function Installer(props: ScreenProps) {
  const { inspection, stages, opRunning, busy, error } = props;
  const [copied, setCopied] = useState(false);
  // Only offered when the package's own manifest says it targets a version
  // Android 14+ refuses; otherwise the flag stays invisible.
  const [allowOldTarget, setAllowOldTarget] = useState(false);

  const copyLog = async () => {
    const text = stages
      .map((s) => `${new Date(s.at_ms).toISOString()} ${s.stage} ${s.message}`)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="page">
      <PageHead
        eyebrow="Workspace"
        title="APK Installer"
        lede="Inspect first, install second. WINAPK never executes package code during inspection."
      >
        <Button variant="ghost" onClick={props.onBrowse} disabled={busy}>
          <IconFolder size={14} />
          Choose another APK
        </Button>
      </PageHead>

      {error && <ErrorNote error={error} />}
      {props.notice && (
        <div className="note note--info" role="status">
          {props.notice}
        </div>
      )}

      {props.env?.blocking && props.env.blocking.length > 0 && (
        <div className="note note--warn">
          <div className="note__head">
            <IconAlert size={14} />
            <span>The runtime is missing pieces, so installing will fail</span>
          </div>
          <ul className="note__body" style={{ paddingLeft: 18, margin: 0 }}>
            {props.env.blocking.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
          <div className="inline">
            <Button variant="secondary" onClick={() => props.onNavigate("runtime")}>
              Open runtime details
            </Button>
          </div>
        </div>
      )}

      <div className="grid grid--aside">
        <div className="stack">
          {!inspection ? (
            <Panel title="Package">
              <DropZone onBrowse={props.onBrowse} busy={busy} />
            </Panel>
          ) : (
            <>
              <Panel
                title="Package inspection"
                action={
                  <StatusPill
                    tone={inspection.signature.v1 || inspection.signature.v2_plus ? "ok" : "warn"}
                    label={
                      inspection.signature.v1 || inspection.signature.v2_plus
                        ? "Signed package"
                        : "No signature block"
                    }
                  />
                }
                note={inspection.file_name}
              >
                <div className="inline" style={{ alignItems: "flex-start", gap: "var(--space-md)" }}>
                  <div className="row__icon" aria-hidden="true" style={{ width: 56, height: 56 }}>
                    {inspection.icon ? <img src={inspection.icon} alt="" /> : <IconInstall size={20} />}
                  </div>
                  <div>
                    <div className="row__title" style={{ fontSize: 15 }}>
                      {inspection.label ?? inspection.package_id}
                    </div>
                    <div className="row__meta">
                      <span className="mono">{inspection.package_id}</span>
                      {!inspection.label && <span>name unavailable</span>}
                    </div>
                  </div>
                </div>

                <dl className="fields">
                  <Field
                    label="Version"
                    value={inspection.version_name ?? "Not declared"}
                    sub={inspection.version_code ? `version code ${inspection.version_code}` : undefined}
                    missing={!inspection.version_name}
                  />
                  <Field
                    label="Minimum SDK"
                    value={inspection.min_sdk ? `API ${inspection.min_sdk}` : "Not declared"}
                    missing={!inspection.min_sdk}
                  />
                  <Field
                    label="Target SDK"
                    value={inspection.target_sdk ? `API ${inspection.target_sdk}` : "Not declared"}
                    missing={!inspection.target_sdk}
                  />
                  <Field
                    label="CPU ABIs"
                    value={inspection.abis.length ? inspection.abis.join(", ") : "No native libraries"}
                    missing={inspection.abis.length === 0}
                  />
                  <Field label="Package size" value={humanSize(inspection.size_bytes)} />
                  <Field
                    label="Signature"
                    value={
                      inspection.signature.v2_plus
                        ? "APK signing block present (v2+)"
                        : inspection.signature.v1
                          ? "v1 signature present"
                          : "None found"
                    }
                    sub="Presence only — the certificate itself is verified by Android on install."
                  />
                  <Field label="File" value={<span className="mono">{inspection.path}</span>} />
                </dl>

                {inspection.unavailable.length > 0 && (
                  <div className="note note--info">
                    <div className="note__head">
                      <IconAlert size={14} />
                      <span>Not determined</span>
                    </div>
                    <ul className="note__body" style={{ paddingLeft: 18, margin: 0 }}>
                      {inspection.unavailable.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                  </div>
                )}

                {inspection.warnings.length > 0 && (
                  <div className="note note--warn">
                    <div className="note__head">
                      <IconAlert size={14} />
                      <span>Before installing</span>
                    </div>
                    <ul className="note__body" style={{ paddingLeft: 18, margin: 0 }}>
                      {inspection.warnings.map((item) => (
                        <li key={item}>{item}</li>
                      ))}
                    </ul>
                  </div>
                )}

                {inspection.target_sdk !== null && inspection.target_sdk < 23 && (
                  <label className="check">
                    <input
                      type="checkbox"
                      checked={allowOldTarget}
                      onChange={(event) => setAllowOldTarget(event.target.checked)}
                      disabled={busy || opRunning}
                    />
                    <span className="check__text">
                      <span className="check__title">Install anyway (Android testing flag)</span>
                      <span className="check__body">
                        This package targets Android API {inspection.target_sdk}. Current Android
                        refuses packages below API 23, so WINAPK has to pass adb&apos;s
                        low-target-SDK testing flag. Apps this old can also render a black window on
                        a separate display.
                      </span>
                    </span>
                  </label>
                )}

                <div className="inline">
                  <Button
                    variant="primary"
                    onClick={() => props.onInstall(true, allowOldTarget)}
                    disabled={busy || opRunning}
                  >
                    <IconPlay size={14} />
                    Install and open
                  </Button>
                  <Button
                    variant="secondary"
                    onClick={() => props.onInstall(false, allowOldTarget)}
                    disabled={busy || opRunning}
                  >
                    Install only
                  </Button>
                </div>
              </Panel>
            </>
          )}
        </div>

        <div className="stack">
          <Panel title="Operation" action={opRunning ? <StatusPill tone="warn" label="Running" /> : undefined}>
            <StageProgress events={stages} running={opRunning} />
            <StageList events={stages} running={opRunning} />
          </Panel>

          <Panel
            title="Diagnostic log"
            flush
            action={
              <Button size="sm" variant="ghost" onClick={copyLog} disabled={stages.length === 0}>
                {copied ? "Copied" : "Copy"}
              </Button>
            }
          >
            {stages.length === 0 ? (
              <p className="muted" style={{ padding: "var(--space-lg)" }}>
                Stages appear here as the backend reports them.
              </p>
            ) : (
              <pre className="log" style={{ margin: "var(--space-md)", maxHeight: 220 }}>
                {stages
                  .map((s) => `[${new Date(s.at_ms).toLocaleTimeString([], { hour12: false })}] ${s.stage.padEnd(16)} ${s.message}`)
                  .join("\n")}
              </pre>
            )}
          </Panel>

          <Panel title="What happens on install">
            <ul className="stack stack--sm muted" style={{ paddingLeft: 18, margin: 0 }}>
              <li>The package is read from disk and its manifest parsed.</li>
              <li>WINAPK starts the runtime if it is stopped.</li>
              <li>Android installs the package and enforces signature rules.</li>
              <li>WINAPK asks the runtime to confirm the package is present.</li>
              <li>Opening the app creates a virtual display and a window for it.</li>
            </ul>
            {!inspection && (
              <EmptyState
                title="Nothing selected"
                body="Drop an APK on this window, or browse for one. Multiple APKs can be dropped one at a time."
              />
            )}
          </Panel>
        </div>
      </div>
    </div>
  );
}
