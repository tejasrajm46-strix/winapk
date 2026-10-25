import { useEffect, useRef, type KeyboardEvent } from "react";
import type { InstalledApp } from "../backend";
import { Button } from "./ui";

/**
 * Uninstalling an Android package and deleting its data are two different
 * decisions. This dialog asks for the second one explicitly; nothing here
 * claims data was deleted, only what each choice will do.
 */
export function UninstallDialog({
  app,
  busy,
  onCancel,
  onConfirm,
}: {
  app: InstalledApp;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (deleteData: boolean) => void;
}) {
  const dialog = useRef<HTMLDivElement>(null);
  const name = app.label ?? app.package_id;

  // Escape closes, wherever focus currently is.
  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onCancel();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onCancel]);

  // Keep Tab inside the dialog while it is modal.
  const trapTab = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key !== "Tab") return;
    const nodes = dialog.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)");
    if (!nodes || nodes.length === 0) return;
    const first = nodes[0];
    const last = nodes[nodes.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  };

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div
        className="dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby="uninstall-title"
        aria-describedby="uninstall-body"
        ref={dialog}
        onKeyDown={trapTab}
      >
        <div>
          <span className="dialog__eyebrow">Applications</span>
          <h2 className="dialog__title" id="uninstall-title">
            Uninstall {name}?
          </h2>
        </div>

        <p className="dialog__body" id="uninstall-body">
          Uninstalling removes <strong>{name}</strong> from the Android runtime. Its app data —
          settings, sign-in state and files — stays in the runtime until you delete it, so
          reinstalling afterwards restores the app where you left it.
        </p>

        <dl className="fields">
          <div className="field">
            <dt className="field__key">Package</dt>
            <dd className="field__value">
              <span className="dialog__package">{app.package_id}</span>
            </dd>
          </div>
          <div className="field">
            <dt className="field__key">Version</dt>
            <dd className="field__value">
              {app.version_name ? `v${app.version_name}` : "Not reported by the runtime"}
            </dd>
          </div>
        </dl>

        <p className="dialog__danger">
          Deleting app data cannot be undone. DroidWin asks the runtime to remove the package and
          its data together.
        </p>

        <div className="dialog__actions">
          <Button variant="ghost" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button variant="secondary" autoFocus onClick={() => onConfirm(false)} disabled={busy}>
            Uninstall and keep app data
          </Button>
          <Button variant="danger" onClick={() => onConfirm(true)} disabled={busy}>
            Uninstall and delete app data
          </Button>
        </div>
      </div>
    </div>
  );
}
