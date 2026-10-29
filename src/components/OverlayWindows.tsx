import {
  Fragment,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type ReactNode,
} from "react";
import * as backend from "../backend";
import type {
  InputAction,
  InputState,
  OverlayData,
  OverlayNavKey,
  UniversalBinding,
} from "../backend";
import "../styles/overlay.css";

/**
 * The two overlay windows, rendered from the same HTML entry point.
 *
 * Which one appears is decided by the window label the Rust side created:
 *
 * - `overlay-labels` is click-through and carries the saved mapping labels. It
 *   covers the whole app window, so nothing in it can take a click from the game.
 * - `overlay-nav` is the bottom bar. Its buttons send real Android key events
 *   through `overlay_nav`; the click never reaches the game window.
 * - `overlay-rail` is the action strip on the right edge: screenshot, rotate and
 *   the More actions, with key mapping first among them.
 * - `overlay-mapper` is the key-mapping panel.
 *
 * Both read their data from the backend. Nothing is mirrored here: hide, show,
 * mapping edits and window moves all arrive as fresh data, so the drawing cannot
 * drift from the stored profile.
 */

/** The video's rectangle inside the window, centred like scrcpy draws it. */
export function fitBox(
  viewportWidth: number,
  viewportHeight: number,
  displayWidth: number,
  displayHeight: number,
) {
  if (displayWidth < 1 || displayHeight < 1 || viewportWidth < 2 || viewportHeight < 2) {
    return { left: 0, top: 0, width: viewportWidth, height: viewportHeight };
  }
  const scale = Math.min(viewportWidth / displayWidth, viewportHeight / displayHeight);
  const width = Math.round(displayWidth * scale);
  const height = Math.round(displayHeight * scale);
  return {
    left: Math.round((viewportWidth - width) / 2),
    top: Math.round((viewportHeight - height) / 2),
    width,
    height,
  };
}

function useWindowSize() {
  const [size, setSize] = useState(() => [window.innerWidth, window.innerHeight]);
  useEffect(() => {
    const resize = () => setSize([window.innerWidth, window.innerHeight]);
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, []);
  return size;
}

/**
 * The overlay's own data: pushed by the backend, asked for until the first
 * answer arrives.
 *
 * Each overlay window is created hidden and shown later. A push emitted before
 * its page ran is lost, and the pushes after that only come when something
 * changes — so a window could sit blank until the next attach. Asking once a
 * second until an answer arrives closes that gap without polling for the rest
 * of its life: after the first answer the pushes keep it current.
 */
function useOverlayData(): OverlayData | null {
  const [data, setData] = useState<OverlayData | null>(null);
  useEffect(() => {
    let live = true;
    const pull = () =>
      backend
        .overlayCurrent()
        .then((next) => {
          if (!live || !next) return false;
          setData(next);
          return true;
        })
        .catch(() => false);
    void pull();
    const timer = window.setInterval(() => {
      void pull().then((answered) => {
        if (answered) window.clearInterval(timer);
      });
    }, 1_000);
    let stop: (() => void) | undefined;
    void backend.onOverlayData((next) => live && setData(next)).then((unlisten) => {
      stop = unlisten;
    });
    // A window that was hidden while the backend changed its state can miss a
    // push. Coming back in front is exactly when the panel must be current —
    // that is how the point clicked on the game reaches it.
    const refresh = () => void pull();
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      live = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
      stop?.();
    };
  }, []);
  return data;
}

export function OverlayWindow({ label }: { label: string }) {
  const data = useOverlayData();
  if (label === "overlay-nav") return <NavBar data={data} />;
  if (label === "overlay-rail") return <RailBar data={data} />;
  if (label === "overlay-mapper") return <MapperPanel data={data} />;
  return <Labels data={data} />;
}

/** A failed overlay action, shown in the window that asked for it. */
function useActionError() {
  const [error, setError] = useState<string | null>(null);
  const report = (err: unknown) => {
    const message = backend.toDwError(err).summary;
    setError(message);
    backend.emitOverlayError(message);
  };
  return { error, report, clear: () => setError(null) };
}

function Labels({ data }: { data: OverlayData | null }) {
  const [width, height] = useWindowSize();
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => {
    let timer: number | undefined;
    let stop: (() => void) | undefined;
    void backend.onOverlayError((message) => {
      setToast(message);
      window.clearTimeout(timer);
      timer = window.setTimeout(() => setToast(null), 4000);
    }).then((unlisten) => {
      stop = unlisten;
    });
    return () => {
      window.clearTimeout(timer);
      stop?.();
    };
  }, []);

  const box = useMemo(
    () => fitBox(width, height, data?.display_width ?? 0, data?.display_height ?? 0),
    [width, height, data?.display_width, data?.display_height],
  );
  const targets = (data?.mappings ?? []).filter(
    (mapping) => mapping.x !== null && mapping.y !== null,
  );

  if (!data) return null;

  /**
   * While a point is being chosen this window stops being click-through, so
   * the click the user aims at a game control lands here. Its position inside
   * the video rectangle — not inside the window, which also holds the
   * letterbox bars — becomes the mapping point.
   */
  const place = (event: ReactMouseEvent<HTMLDivElement>) => {
    if (!data.placing) return;
    const rect = event.currentTarget.getBoundingClientRect();
    if (rect.width < 2 || rect.height < 2) return;
    const clamp = (value: number) => Math.min(100, Math.max(0, value));
    const x = clamp(((event.clientX - rect.left) / rect.width) * 100);
    const y = clamp(((event.clientY - rect.top) / rect.height) * 100);
    void backend
      .overlayPlaceAt(data.package_id, x, y)
      .catch((err) => setToast(backend.toDwError(err).summary));
  };

  // Only the mapped controls are drawn. A legend here sat on the game's own
  // header to say "no mappings yet", which is the panel's job, not the game's.
  return (
    <div className={`labels${data.placing ? " labels--placing" : ""}`}>
      <div
        className="labels__field"
        style={{ left: box.left, top: box.top, width: box.width, height: box.height }}
        aria-hidden="true"
      >
        {targets.map((mapping) => (
          <span
            key={mapping.key}
            className={`labels__chip${data.enabled ? "" : " labels__chip--off"}`}
            style={{ left: `${mapping.x}%`, top: `${mapping.y}%` }}
            title={`${mapping.label}: ${mapping.action}`}
          >
            {mapping.label}
          </span>
        ))}
        {data.pending && (
          <span
            className="labels__pending"
            style={{ left: `${data.pending.x}%`, top: `${data.pending.y}%` }}
            title={`New point: ${Math.round(data.pending.x)}% / ${Math.round(data.pending.y)}%`}
          />
        )}
      </div>

      {data.placing && (
        <div
          className="labels__place"
          style={{ left: box.left, top: box.top, width: box.width, height: box.height }}
          role="button"
          aria-label="Click the game control to map"
          onClick={place}
        >
          <span className="labels__place-hint">Click the game control</span>
        </div>
      )}

      {toast && (
        <div className="labels__toast" role="status">
          {toast}
        </div>
      )}
    </div>
  );
}

const NAV: { key: OverlayNavKey; label: string; icon: ReactNode }[] = [
  {
    key: "BACK",
    label: "Back",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M15 5 8 12l7 7" />
      </svg>
    ),
  },
  {
    key: "HOME",
    label: "Home",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="6.5" />
      </svg>
    ),
  },
  {
    key: "APP_SWITCH",
    label: "Recent apps",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="6" y="6" width="12" height="12" rx="2.5" />
      </svg>
    ),
  },
];

function NavBar({ data }: { data: OverlayData | null }) {
  const [busy, setBusy] = useState<OverlayNavKey | null>(null);
  const { error, report, clear } = useActionError();

  if (!data) return null;

  const send = async (key: OverlayNavKey) => {
    setBusy(key);
    clear();
    try {
      await backend.overlayNav(data.package_id, key);
    } catch (err) {
      report(err);
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="ctrlbar" role="toolbar" aria-label="Android navigation">
      {NAV.map((item, index) => (
        <Fragment key={item.key}>
          {index > 0 && <span className="ctrlbar__divider" aria-hidden="true" />}
          <button
            type="button"
            className={`ctrlbar__key${busy === item.key ? " ctrlbar__key--busy" : ""}`}
            title={`${item.label} (Android ${item.key})`}
            aria-label={item.label}
            disabled={busy !== null}
            onClick={() => void send(item.key)}
          >
            {item.icon}
            <span className="ctrlbar__label">{item.label}</span>
          </button>
        </Fragment>
      ))}
      {error && (
        <span className="ctrlbar__error" title={error} aria-label={error}>
          !
        </span>
      )}
    </div>
  );
}

/** Everything the rail can do, besides More. */
const RAIL: { id: string; label: string; title: string; icon: ReactNode }[] = [
  {
    id: "screenshot",
    label: "Screenshot",
    title: "Save a picture of this window's display",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <rect x="3" y="6.5" width="18" height="12" rx="2.5" />
        <circle cx="12" cy="12.5" r="3.2" />
        <path d="M8.5 6.5 10 4.5h4l1.5 2" />
      </svg>
    ),
  },
  {
    id: "rotate",
    label: "Rotate",
    title: "Reopen this window in the other orientation",
    icon: (
      <svg viewBox="0 0 24 24" aria-hidden="true">
        <path d="M20 11a8 8 0 1 0-2.3 6.3" />
        <path d="M20 5v6h-6" />
      </svg>
    ),
  },
];

/**
 * The action rail on the right edge of the app window.
 *
 * Each button is one real operation on this window: Screenshot captures the
 * window's own Android display, Rotate reopens the window on a surface of the
 * other shape (a virtual display cannot be rotated in place), and More expands
 * into the mapping actions inside the rail itself — this window is tall enough
 * for that, which a popup from the short bottom bar was not.
 */
function RailBar({ data }: { data: OverlayData | null }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [menu, setMenu] = useState(false);
  const { error, report, clear } = useActionError();

  useEffect(() => {
    if (!note) return;
    const timer = window.setTimeout(() => setNote(null), 4000);
    return () => window.clearTimeout(timer);
  }, [note]);

  if (!data) {
    return (
      <div className="rail">
        <p className="rail__note">Waiting for the game window…</p>
      </div>
    );
  }

  const run = async (id: string, work: () => Promise<string>) => {
    setBusy(id);
    clear();
    setNote(null);
    try {
      setNote(await work());
    } catch (err) {
      report(err);
    } finally {
      setBusy(null);
    }
  };

  const screenshot = () =>
    run("screenshot", async () => {
      const shot = await backend.appScreenshot(data.package_id);
      return `Saved ${shot.width}×${shot.height} to DroidWin`;
    });

  const rotate = () =>
    run("rotate", async () => {
      const height = data.display_height || 0;
      const width = data.display_width || 0;
      const next = width > height ? "portrait" : "landscape";
      await backend.rotateAppWindow(data.package_id);
      return `Reopening in ${next}`;
    });

  const toggleLabels = () =>
    run("labels", async () => {
      const status = await backend.overlayToggleLabels();
      return status.labels_visible ? "Labels shown" : "Labels hidden";
    });

  if (menu) {
    return (
      <div className="rail" role="menu" aria-label="More">
        <button
          type="button"
          role="menuitem"
          className="rail__item"
          onClick={() => {
            setMenu(false);
            void run("mapper", async () => {
              await backend.overlayToggleMapper(true, data.package_id);
              return "Mapping panel open";
            });
          }}
        >
          Key mapping
        </button>
        <button
          type="button"
          role="menuitem"
          className="rail__item"
          aria-label={data.labels_visible ? "Hide mapping labels" : "Show mapping labels"}
          onClick={() => {
            setMenu(false);
            void toggleLabels();
          }}
        >
          {data.labels_visible ? "Hide labels" : "Show labels"}
        </button>
        <button
          type="button"
          role="menuitem"
          className="rail__item"
          aria-label="Controls"
          onClick={() => {
            setMenu(false);
            void backend.overlayOpenControls(data.package_id);
          }}
        >
          Controls
        </button>
        <button
          type="button"
          className="rail__item rail__item--back"
          aria-label="Back to actions"
          onClick={() => setMenu(false)}
        >
          Back
        </button>
        {(note || error) && (
          <p className="rail__note" role={error ? "alert" : "status"}>
            {error ?? note}
          </p>
        )}
      </div>
    );
  }

  return (
    <div className="rail" role="toolbar" aria-label="Window actions">
      {RAIL.map((item) => (
        <button
          key={item.id}
          type="button"
          className={`rail__btn${busy === item.id ? " rail__btn--busy" : ""}`}
          title={item.title}
          aria-label={item.label}
          disabled={busy !== null}
          onClick={() => void (item.id === "screenshot" ? screenshot() : rotate())}
        >
          {item.icon}
          <span className="rail__label">{item.label}</span>
        </button>
      ))}
      <button
        type="button"
        className="rail__btn"
        title="More — key mapping, labels, controls"
        aria-label="More"
        aria-expanded={menu}
        onClick={() => setMenu(true)}
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="5" cy="12" r="1.5" />
          <circle cx="12" cy="12" r="1.5" />
          <circle cx="19" cy="12" r="1.5" />
        </svg>
        <span className="rail__label">More</span>
      </button>
      {(note || error) && (
        <p className="rail__note" role={error ? "alert" : "status"}>
          {error ?? note}
        </p>
      )}
    </div>
  );
}

const KEYCODE_OPTIONS = [
  "KEYCODE_SPACE",
  "KEYCODE_ENTER",
  "KEYCODE_ESCAPE",
  "KEYCODE_SHIFT_LEFT",
  "KEYCODE_TAB",
  "KEYCODE_BACK",
  ...Array.from({ length: 26 }, (_, i) => `KEYCODE_${String.fromCharCode(65 + i)}`),
  ...Array.from({ length: 10 }, (_, i) => `KEYCODE_${i}`),
];

/**
 * The chord a key event describes, in the form the backend saves: modifiers plus
 * one key (`Shift+A`, `Ctrl+Alt+Delete`). A modifier on its own returns null, so
 * pressing Shift alone never binds anything.
 */
export function chordFromEvent(event: KeyboardEvent): string | null {
  if (["Shift", "Control", "Alt", "Meta"].includes(event.key)) return null;
  const parts: string[] = [];
  if (event.ctrlKey) parts.push("Ctrl");
  if (event.altKey) parts.push("Alt");
  if (event.shiftKey) parts.push("Shift");
  if (event.metaKey) parts.push("Win");
  if (parts.length === 0) return null;
  const named: Record<string, string> = {
    " ": "Space",
    Escape: "Escape",
    Enter: "Enter",
    Tab: "Tab",
    ArrowUp: "Up",
    ArrowDown: "Down",
    ArrowLeft: "Left",
    ArrowRight: "Right",
  };
  const key = named[event.key] ?? (event.key.length === 1 ? event.key.toUpperCase() : event.key);
  parts.push(key);
  return parts.join("+");
}

/** Shown until the backend answers, so the panel is never blank. */
const DEFAULT_UNIVERSAL: UniversalBinding[] = [
  { action: "back", label: "Back", chord: "Shift+A", android_key: "KEYCODE_BACK" },
  { action: "home", label: "Home", chord: "Shift+H", android_key: "KEYCODE_HOME" },
  { action: "recents", label: "Recent Apps", chord: "Shift+R", android_key: "KEYCODE_APP_SWITCH" },
  { action: "menu", label: "Menu", chord: "Shift+M", android_key: "KEYCODE_MENU" },
  { action: "screenshot", label: "Screenshot", chord: "Shift+P", android_key: null },
  { action: "rotate", label: "Rotate", chord: "Shift+O", android_key: null },
];

function actionText(action: InputAction): string {
  switch (action.kind) {
    case "key_event":
      return `Android ${action.keycode.replace("KEYCODE_", "")}`;
    case "touch_tap":
      return `Tap ${Math.round(action.x)}% / ${Math.round(action.y)}%`;
    case "touch_hold":
      return `Hold ${Math.round(action.x)}% / ${Math.round(action.y)}%`;
  }
}

/**
 * The key-mapping panel over the app window.
 *
 * It edits the same per-package profile the Controls dialog edits, from where the
 * game is: press a key, say what it does (tap or hold a point, or send an Android
 * key), and the label appears on the game immediately. Every change is saved as
 * it is made, so closing the panel cannot lose one; "Reset" is the only action
 * that throws work away and it asks by being a second, separate button.
 */
function MapperPanel({ data }: { data: OverlayData | null }) {
  const packageId = data?.package_id;
  const [state, setState] = useState<InputState | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [captured, setCaptured] = useState<{ vk: number; name: string } | null>(null);
  const [kind, setKind] = useState<InputAction["kind"]>("touch_tap");
  const [keycode, setKeycode] = useState("KEYCODE_SPACE");
  const captureRef = useRef<HTMLButtonElement>(null);
  const pending = data?.pending ?? null;
  const [universal, setUniversal] = useState<UniversalBinding[]>(DEFAULT_UNIVERSAL);
  // Which universal shortcut is waiting for its new keys.
  const [editing, setEditing] = useState<string | null>(null);
  const [chordError, setChordError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    backend
      .universalBindings()
      .then((next) => live && setUniversal(next))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  // The point came from a click on the game. The panel is back in front, so the
  // next key the user presses is the one being assigned — no second click.
  useEffect(() => {
    if (pending) captureRef.current?.focus();
  }, [pending]);

  useEffect(() => {
    if (!packageId) return;
    let live = true;
    backend
      .getInputState(packageId)
      .then((next) => live && setState(next))
      .catch((err) => live && setError(backend.toDwError(err).summary));
    return () => {
      live = false;
    };
  }, [packageId]);

  // The window is opaque and this page is its only content: an early return here
  // would paint a white rectangle over the game while the data is in flight.
  if (!data || !packageId) return <section className="mapper" aria-label="Key mapping" />;

  const run = async (work: () => Promise<InputState>) => {
    setBusy(true);
    setError(null);
    try {
      setState(await work());
    } catch (err) {
      setError(backend.toDwError(err).summary);
    } finally {
      setBusy(false);
    }
  };

  const capture = (event: KeyboardEvent<HTMLButtonElement>) => {
    event.preventDefault();
    if (event.key === "Tab") return;
    const isModifier = ["Shift", "Control", "Alt", "Meta"].includes(event.key);
    const modifiers = isModifier
      ? 0
      : Number(event.ctrlKey) |
        (Number(event.altKey) << 1) |
        (Number(event.shiftKey) << 2) |
        (Number(event.metaKey) << 3);
    const prefix = [
      event.ctrlKey && "Ctrl",
      event.altKey && "Alt",
      event.shiftKey && "Shift",
      event.metaKey && "Win",
    ]
      .filter(Boolean)
      .join("+");
    const name = event.key === " " ? "Space" : event.key;
    setCaptured({
      vk: event.keyCode | (modifiers << 8),
      name: modifiers ? `${prefix}+${name.toUpperCase()}` : name,
    });
  };

  const mappings = state?.mappings ?? [];
  const enabled = state?.enabled ?? data.enabled;
  const needsPoint = kind !== "key_event";
  const ready = captured !== null && (!needsPoint || pending !== null);

  const add = () => {
    if (!captured || !ready) return;
    const action: InputAction =
      kind === "key_event"
        ? { kind, keycode }
        : { kind, x: pending?.x ?? 50, y: pending?.y ?? 80 };
    void run(() => backend.saveInputMapping(packageId, captured.vk, action)).then(() =>
      setCaptured(null),
    );
  };

  const place = () => {
    setError(null);
    void backend
      .overlayBeginPlace(packageId)
      .catch((err) => setError(backend.toDwError(err).summary));
  };

  return (
    <section className="mapper" aria-label="Key mapping">
      <header className="mapper__head">
        <div>
          <span className="mapper__eyebrow">Key mapping</span>
          <strong className="mapper__title">{data.app_label}</strong>
        </div>
        <button
          type="button"
          className="mapper__close"
          aria-label="Close key mapping"
          onClick={() => void backend.overlayToggleMapper(false)}
        >
          ✕
        </button>
      </header>

      <label className="mapper__switch">
        <input
          type="checkbox"
          checked={enabled}
          disabled={busy}
          onChange={(event) =>
            void run(() => backend.setInputProfileEnabled(packageId, event.target.checked))
          }
        />
        <span>{enabled ? "Mappings armed" : "Mappings off"}</span>
      </label>

      {/* Android's own controls, on the same keys in every app. They are decided
          before the profile, so a game can never swallow them. */}
      <div className="mapper__universal" aria-label="Universal controls">
        <span className="mapper__section">Universal controls</span>
        <ul className="mapper__keys">
          {universal.map((binding) => (
            <li key={binding.action}>
              <span className="mapper__universal-label">{binding.label}</span>
              <button
                type="button"
                className={`mapper__chord${editing === binding.action ? " mapper__chord--editing" : ""}`}
                aria-label={`Change ${binding.label}`}
                disabled={busy}
                onClick={() => {
                  setChordError(null);
                  setEditing(editing === binding.action ? null : binding.action);
                }}
                onKeyDown={(event) => {
                  if (editing !== binding.action) return;
                  event.preventDefault();
                  const chord = chordFromEvent(event);
                  if (!chord) return;
                  setEditing(null);
                  void backend
                    .setUniversalBinding(binding.action, chord)
                    .then(setUniversal)
                    .catch((err) => setChordError(backend.toDwError(err).summary));
                }}
              >
                {editing === binding.action ? "Press keys…" : binding.chord}
              </button>
            </li>
          ))}
          <li>
            <span className="mapper__universal-label">Show / hide mapping</span>
            <kbd className="mapper__chord mapper__chord--fixed">Ctrl+Shift+R</kbd>
          </li>
        </ul>
        {chordError && (
          <p className="mapper__error" role="alert">
            {chordError}
          </p>
        )}
      </div>

      <div className="mapper__rows">
        {mappings.map((mapping) => (
          <div className="mapper__row" key={mapping.key}>
            <span className="mapper__key">{mapping.label}</span>
            <span className="mapper__action">{actionText(mapping.action)}</span>
            <button
              type="button"
              className="mapper__remove"
              aria-label={`Remove ${mapping.label}`}
              disabled={busy}
              onClick={() => void run(() => backend.removeInputMapping(packageId, mapping.key))}
            >
              ✕
            </button>
          </div>
        ))}
        {mappings.length === 0 && (
          <p className="mapper__empty">
            Nothing mapped yet. Press a key below, choose what it does, then Add.
          </p>
        )}
      </div>

      <fieldset className="mapper__add">
        <legend>Add a key</legend>
        <button
          ref={captureRef}
          type="button"
          className="mapper__capture"
          aria-label="Press a key to map"
          onKeyDown={capture}
        >
          {captured ? captured.name : "Press a key"}
        </button>
        <select
          className="mapper__select"
          aria-label="What the key does"
          value={kind}
          onChange={(event) => setKind(event.target.value as InputAction["kind"])}
        >
          <option value="touch_tap">Tap a point</option>
          <option value="touch_hold">Hold a point</option>
          <option value="key_event">Android key</option>
        </select>
        {kind === "key_event" ? (
          <select
            className="mapper__select"
            aria-label="Android key"
            value={keycode}
            onChange={(event) => setKeycode(event.target.value)}
          >
            {KEYCODE_OPTIONS.map((option) => (
              <option key={option} value={option}>
                {option.replace("KEYCODE_", "")}
              </option>
            ))}
          </select>
        ) : (
          // The game itself is the selection surface: the panel steps aside
          // and the next click on the running picture becomes the point. The
          // point is stored as a percentage of the display, so a resized or
          // rotated window still lands on the same control.
          <div className="mapper__place">
            <button
              type="button"
              className="mapper__place-btn"
              onClick={place}
            >
              {pending ? "Place again" : "Place on the game"}
            </button>
            {pending ? (
              <span className="mapper__point">
                Point {Math.round(pending.x)}% / {Math.round(pending.y)}%
                <button
                  type="button"
                  className="mapper__point-clear"
                  aria-label="Discard the chosen point"
                  onClick={() => void backend.overlayClearPending()}
                >
                  ✕
                </button>
              </span>
            ) : (
              <span className="mapper__place-label">
                Click the control on the game — the panel gets out of the way.
              </span>
            )}
          </div>
        )}
        <button
          type="button"
          className="mapper__add-key"
          disabled={!ready || busy}
          title={needsPoint && !pending ? "Place the point on the game first" : undefined}
          onClick={add}
        >
          Add
        </button>
      </fieldset>

      <footer className="mapper__foot">
        <button
          type="button"
          className="mapper__reset"
          disabled={busy}
          onClick={() => void run(() => backend.resetInputMappings(packageId))}
        >
          Reset mappings
        </button>
        <button
          type="button"
          className="mapper__done"
          onClick={() => void backend.overlayToggleMapper(false)}
        >
          Done
        </button>
      </footer>

      {error && (
        <p className="mapper__error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
