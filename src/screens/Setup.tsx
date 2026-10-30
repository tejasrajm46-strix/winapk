import { useState } from "react";
import mark from "../assets/branding/droidwin-mark.png";
import type { DwError, SetupProgress, SetupReport } from "../backend";
import { IconAlert, IconCheck, IconFolder, IconRefresh } from "../components/icons";
import { Button, ErrorNote, StatusPill } from "../components/ui";

/**
 * First-run Android runtime setup. It is the only screen that can install
 * anything, and every sentence on it comes from what the backend measured: the
 * component list is a filesystem answer, the progress bar is only filled when a
 * real total is known, and "ready" is never assumed before the runtime booted.
 */
export function Setup({
  report,
  progress,
  busy,
  error,
  onInstall,
  onCancel,
  onChangeLocation,
  onUseManaged,
  onRefresh,
  onSkip,
}: {
  report: SetupReport | null;
  progress: SetupProgress | null;
  busy: boolean;
  error: DwError | null;
  onInstall: (acceptLicenses: boolean) => void;
  onCancel: () => void;
  onChangeLocation: () => void;
  onUseManaged: () => void;
  onRefresh: () => void;
  onSkip: () => void;
}) {
  const [consent, setConsent] = useState(false);
  const installing = report?.state === "installing";
  const accepted = report?.licenses_accepted ?? false;
  const canInstall = Boolean(report) && !busy && !installing && (consent || accepted);
  const percent = progress?.percent ?? null;

  return (
    <div className="setup">
      <div className="setup__card">
        <div className="setup__brand">
          <img src={mark} alt="" className="setup__mark" />
          <div>
            <span className="page__eyebrow">Android runtime</span>
            <h1 className="setup__title">Set up Android</h1>
          </div>
          {report && (
            <StatusPill
              tone={report.state === "ready" ? "ok" : report.state === "failed" ? "error" : "warn"}
              label={
                report.state === "ready"
                  ? "Ready"
                  : report.state === "installing"
                    ? "Installing"
                    : report.state === "starting"
                      ? "Starting"
                      : report.state === "failed"
                        ? "Failed"
                        : "Not installed"
              }
            />
          )}
        </div>

        <p className="setup__lede">
          WINAPK runs Android apps in their own windows. It needs the Android
          emulator and one system image — about 2.2 GB, downloaded once from
          Google. Android Studio is not required.
        </p>

        {error && <ErrorNote error={error} />}

        {!report ? (
          <p className="secondary">Checking what is already installed…</p>
        ) : (
          <>
            <p className={`setup__message${report.state === "failed" ? " setup__message--bad" : ""}`}>
              {report.state === "failed" && <IconAlert size={14} />}
              {report.message}
            </p>

            <ul className="setup__parts">
              {report.components.map((component) => (
                <li
                  key={component.id}
                  className={`setup__part${component.present ? " setup__part--on" : ""}`}
                >
                  <span className="setup__part-icon" aria-hidden="true">
                    {component.present ? <IconCheck size={14} /> : null}
                  </span>
                  <span className="setup__part-body">
                    <span className="setup__part-label">
                      {component.label}
                      {!component.required && component.present === false && (
                        <span className="setup__part-optional">optional</span>
                      )}
                    </span>
                    <span className="setup__part-detail" title={component.detail}>
                      {component.detail}
                    </span>
                  </span>
                  <span className="setup__part-size">
                    {component.present
                      ? "Installed"
                      : component.download_mb
                        ? `${component.download_mb} MB`
                        : "—"}
                  </span>
                </li>
              ))}
            </ul>

            <div className="setup__meta">
              <span>
                Location <span className="mono">{report.sdk_root}</span>
                {report.managed ? " (WINAPK's own)" : " (existing SDK)"}
              </span>
              {report.disk_free_mb > 0 && (
                <span>{(report.disk_free_mb / 1024).toFixed(1)} GB free</span>
              )}
            </div>

            {installing && (
              <div className="setup__progress" role="status" aria-live="polite">
                <div
                  className={`setup__bar${percent === null ? " setup__bar--indeterminate" : ""}`}
                  style={percent === null ? undefined : { width: `${percent}%` }}
                />
                <span className="setup__progress-text">
                  {progress?.message ?? "Starting the installation."}
                  {percent !== null ? ` ${Math.round(percent)}%` : ""}
                </span>
              </div>
            )}

            {!accepted && !installing && (
              <label className="setup__consent">
                <input
                  type="checkbox"
                  checked={consent}
                  onChange={(event) => setConsent(event.target.checked)}
                />
                <span>
                  I accept Google's Android SDK licence terms. The SDK manager
                  records the acceptance in {report.sdk_root}.
                </span>
              </label>
            )}

            <div className="setup__actions">
              {installing ? (
                <Button variant="danger" onClick={onCancel}>
                  Cancel installation
                </Button>
              ) : (
                <Button
                  variant="primary"
                  onClick={() => onInstall(accepted || consent)}
                  disabled={!canInstall}
                  disabledReason={
                    accepted || consent
                      ? undefined
                      : "Accept the Android SDK licence terms first."
                  }
                >
                  {report.state === "ready"
                    ? "Finish setup"
                    : report.state === "starting"
                      ? "Start Android runtime"
                      : "Install Android runtime"}
                </Button>
              )}
              <Button variant="ghost" onClick={onChangeLocation} disabled={installing || busy}>
                <IconFolder size={14} />
                Choose folder
              </Button>
              {report.configured && (
                <Button variant="ghost" onClick={onUseManaged} disabled={installing || busy}>
                  Use WINAPK's folder
                </Button>
              )}
              <Button variant="ghost" onClick={onRefresh} disabled={busy || installing}>
                <IconRefresh size={14} />
                Check again
              </Button>
              <Button variant="ghost" onClick={onSkip}>
                Open WINAPK anyway
              </Button>
            </div>

            <p className="setup__footnote">
              Nothing is installed without this button. A complete SDK already on
              this PC is used where it is; only missing pieces are added to it.
            </p>
          </>
        )}
      </div>
    </div>
  );
}
