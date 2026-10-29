import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import type { DwError, InputAction, InputState, InstalledApp } from "../backend";
import * as backend from "../backend";
import { Button } from "./ui";

/**
 * Configure Controls for one application.
 *
 * It is opened from that app's row, never shown by default, and nothing here is
 * armed until "Enable for this app" is switched on. Touch targets are percentages
 * of the app's own display, so moving or resizing its window does not move them.
 */

const KEYCODES = [
  "KEYCODE_SPACE",
  "KEYCODE_ENTER",
  "KEYCODE_ESCAPE",
  "KEYCODE_TAB",
  "KEYCODE_BACK",
  "KEYCODE_HOME",
  "KEYCODE_SHIFT_LEFT",
  "KEYCODE_DPAD_UP",
  "KEYCODE_DPAD_DOWN",
  "KEYCODE_DPAD_LEFT",
  "KEYCODE_DPAD_RIGHT",
  ...Array.from({ length: 26 }, (_, i) => `KEYCODE_${String.fromCharCode(65 + i)}`),
  ...Array.from({ length: 10 }, (_, i) => `KEYCODE_${i}`),
];

type Draft = "key_event" | "touch_tap" | "touch_hold";

function actionText(action: InputAction): string {
  if (action.kind === "key_event") return `Send ${action.keycode}`;
  const verb = action.kind === "touch_tap" ? "Tap" : "Hold";
  return `${verb} ${Math.round(action.x)}% / ${Math.round(action.y)}%`;
}

export function ControlsDialog({ app, onClose }: { app: InstalledApp; onClose: () => void }) {
  const dialog = useRef<HTMLDivElement>(null);
  const captureButton = useRef<HTMLButtonElement>(null);
  const [state, setState] = useState<InputState | null>(null);
  const [error, setError] = useState<DwError | null>(null);
  const [busy, setBusy] = useState(false);
  const [captured, setCaptured] = useState<{ vk: number; name: string } | null>(null);
  const [draft, setDraft] = useState<Draft>("touch_tap");
  const [keycode, setKeycode] = useState("KEYCODE_SPACE");
  const [xPct, setXPct] = useState(50);
  const [yPct, setYPct] = useState(80);
  const name = app.label ?? app.package_id;

  useEffect(() => {
    let live = true;
    backend
      .getInputState(app.package_id)
      .then((next) => live && setState(next))
      .catch((err) => live && setError(backend.toDwError(err)));
    return () => {
      live = false;
    };
  }, [app.package_id]);

  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent) => {
      // While the capture button has focus every key — Escape included — belongs
      // to the mapping being recorded, not to the dialog.
      if (event.target === captureButton.current) return;
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  const run = useCallback(async (work: () => Promise<InputState>) => {
    setBusy(true);
    setError(null);
    try {
      setState(await work());
    } catch (err) {
      setError(backend.toDwError(err));
    } finally {
      setBusy(false);
    }
  }, []);

  const capture = (event: KeyboardEvent<HTMLButtonElement>) => {
    event.preventDefault();
    // Physical key identity is what the hook reports; keyCode is the Windows
    // virtual-key code for that key on the active layout.
    if (event.key === "Tab") return;
    const isModifier = ["Shift", "Control", "Alt", "Meta"].includes(event.key);
    const modifiers = isModifier ? 0 : (Number(event.ctrlKey) | (Number(event.altKey) << 1) | (Number(event.shiftKey) << 2) | (Number(event.metaKey) << 3));
    const prefix = [event.ctrlKey && "Ctrl", event.altKey && "Alt", event.shiftKey && "Shift", event.metaKey && "Win"].filter(Boolean).join("+");
    const name = event.key === " " ? "Space" : event.key;
    setCaptured({ vk: event.keyCode | (modifiers << 8), name: modifiers ? `${prefix}+${name.toUpperCase()}` : name });
  };

  const draftAction = (): InputAction | null => {
    if (draft === "key_event") return { kind: "key_event", keycode };
    if (draft === "touch_tap") return { kind: "touch_tap", x: xPct, y: yPct };
    return { kind: "touch_hold", x: xPct, y: yPct };
  };

  const add = () => {
    const action = draftAction();
    if (!captured || !action) return;
    void run(() => backend.saveInputMapping(app.package_id, captured.vk, action)).then(() =>
      setCaptured(null),
    );
  };

  const screen = state?.screen
    ? `${state.screen.width}×${state.screen.height} (${state.screen.orientation})`
    : "not open — open the app to see its display size";

  return (
    <div
      className="dialog-backdrop"
      role="presentation"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        className="dialog dialog--wide"
        role="dialog"
        aria-modal="true"
        aria-labelledby="controls-title"
        ref={dialog}
      >
        <div>
          <span className="dialog__eyebrow">Controls</span>
          <h2 className="dialog__title" id="controls-title">
            {name}
          </h2>
        </div>

        <p className="dialog__body">
          Mapping sends a key to this app only: a Windows hook takes mapped keys before the
          window does, so a key is never delivered twice. Unmapped keys and shortcuts pass through;
          modifier combinations are intercepted only when explicitly mapped.
        </p>

        {error && (
          <p className="dialog__danger" role="alert">
            {error.summary}
          </p>
        )}

        <div className="field field--row">
          <span className="field__key">Enabled for this app</span>
          <Button
            size="sm"
            variant={state?.enabled ? "secondary" : "primary"}
            disabled={busy || !state}
            aria-pressed={state?.enabled ?? false}
            onClick={() =>
              state &&
              void run(() => backend.setInputProfileEnabled(app.package_id, !state.enabled))
            }
          >
            {state?.enabled ? "On — turn off" : "Off — turn on"}
          </Button>
        </div>

        <p className="dialog__meta">
          Touch targets are percentages of this app&apos;s display: {screen}.
        </p>

        <fieldset className="controls-form">
          <legend>New mapping</legend>
          <div className="field field--row">
            <span className="field__key">Key</span>
            <button
              type="button"
              className="controls-capture"
              ref={captureButton}
              onKeyDown={capture}
              aria-label="Press the key to map"
            >
              {captured ? `${captured.name} (VK ${captured.vk})` : "Click, then press a key"}
            </button>
          </div>

          <div className="field field--row">
            <label className="field__key" htmlFor="controls-kind">
              Action
            </label>
            <select
              id="controls-kind"
              value={draft}
              onChange={(event) => setDraft(event.target.value as Draft)}
            >
              <option value="touch_tap">Tap a point</option>
              <option value="touch_hold">Hold a point</option>
              <option value="key_event">Send an Android key</option>
            </select>
          </div>

          {draft === "key_event" ? (
            <div className="field field--row">
              <label className="field__key" htmlFor="controls-keycode">
                Android key
              </label>
              <select
                id="controls-keycode"
                value={keycode}
                onChange={(event) => setKeycode(event.target.value)}
              >
                {KEYCODES.map((code) => (
                  <option key={code} value={code}>
                    {code}
                  </option>
                ))}
              </select>
            </div>
          ) : (
            <div className="field field--row">
              <span className="field__key">Point</span>
              <span className="controls-point">
                <label>
                  X %
                  <input
                    type="number"
                    min={0}
                    max={100}
                    value={xPct}
                    onChange={(event) => setXPct(Number(event.target.value))}
                  />
                </label>
                <label>
                  Y %
                  <input
                    type="number"
                    min={0}
                    max={100}
                    value={yPct}
                    onChange={(event) => setYPct(Number(event.target.value))}
                  />
                </label>
              </span>
            </div>
          )}

          <div className="dialog__actions">
            <Button
              size="sm"
              variant="primary"
              disabled={busy || !captured}
              onClick={add}
            >
              Add mapping
            </Button>
          </div>
        </fieldset>

        <div className="controls-list">
          {state && state.mappings.length === 0 && (
            <p className="muted">No mappings yet. Nothing is intercepted for this app.</p>
          )}
          {state?.mappings.map((mapping) => (
            <div className="controls-row" key={mapping.key}>
              <span className="controls-row__key">{mapping.label}</span>
              <span className="controls-row__action">{actionText(mapping.action)}</span>
              <label className="controls-row__toggle">
                <input
                  type="checkbox"
                  checked={mapping.enabled}
                  disabled={busy}
                  onChange={(event) =>
                    void run(() =>
                      backend.setInputMappingEnabled(
                        app.package_id,
                        mapping.key,
                        event.target.checked,
                      ),
                    )
                  }
                />
                on
              </label>
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() =>
                  void run(() => backend.removeInputMapping(app.package_id, mapping.key))
                }
              >
                Remove
              </Button>
            </div>
          ))}
        </div>

        <div className="dialog__actions">
          <Button
            variant="ghost"
            disabled={busy || !state || state.mappings.length === 0}
            onClick={() => void run(() => backend.resetInputMappings(app.package_id))}
          >
            Reset to defaults
          </Button>
          <Button variant="secondary" onClick={onClose}>
            Close
          </Button>
        </div>
      </div>
    </div>
  );
}
