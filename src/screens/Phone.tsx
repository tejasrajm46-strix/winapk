import { useEffect, useRef, useState } from "react";
import * as backend from "../backend";
import type { DwError, InstalledApp, PhoneScreen } from "../backend";
import { StageList } from "../components/StageList";
import { runtimeTone } from "../components/AppShell";
import {
  IconBack,
  IconCamera,
  IconFolder,
  IconHome,
  IconOpen,
  IconPackage,
  IconPhone,
  IconRecents,
  IconRecord,
  IconRefresh,
  IconSearch,
  IconStop,
} from "../components/icons";
import { Button, EmptyState, ErrorNote, Panel, StatusPill } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";
import { PhoneVideo } from "../phoneVideo";

/**
 * The virtual phone: Android's own display, inside WINAPK.
 *
 * It is the emulator WINAPK already runs, mirrored — device display 0, the
 * launcher, the notification shade and Android's settings, which is the one
 * display WINAPK did not create for an app. No second emulator and no
 * placeholder wallpaper are involved: every pixel here came from a capture
 * Android produced, and every touch and keystroke goes back to it.
 *
 * The integrated screen uses a demand-driven Android H.264 capture. A still
 * capture remains the fallback when this WebView cannot decode the stream.
 */

type Box = { left: number; top: number; width: number; height: number };

/**
 * The box a capture is actually painted in.
 *
 * `object-fit: contain` letterboxes the image inside its element, and a CSS
 * max-height can clamp that element away from the capture's aspect ratio. A click
 * has to be measured against the pixels the user sees, not against the element
 * the browser laid out, or every tap is short by the width of the bars.
 */
export function paintedBox(rect: Box, size: { width: number; height: number }): Box {
  if (rect.width <= 0 || rect.height <= 0 || size.width <= 0 || size.height <= 0) return rect;
  const scale = Math.min(rect.width / size.width, rect.height / size.height);
  const width = size.width * scale;
  const height = size.height * scale;
  return {
    left: rect.left + (rect.width - width) / 2,
    top: rect.top + (rect.height - height) / 2,
    width,
    height,
  };
}

/** A click inside `box`, in the device pixels the capture itself uses. */
export function devicePoint(
  box: Box,
  size: { width: number; height: number },
  client: { x: number; y: number },
): { x: number; y: number } {
  const scaleX = box.width > 0 ? size.width / box.width : 1;
  const scaleY = box.height > 0 ? size.height / box.height : 1;
  const axis = (value: number, max: number) =>
    Math.max(0, Math.min(Math.round(value), Math.max(0, max - 1)));
  return {
    x: axis((client.x - box.left) * scaleX, size.width),
    y: axis((client.y - box.top) * scaleY, size.height),
  };
}

function inside(box: Box, client: { x: number; y: number }): boolean {
  return (
    client.x >= box.left &&
    client.x <= box.left + box.width &&
    client.y >= box.top &&
    client.y <= box.top + box.height
  );
}

/** Physical keys the phone takes, and the Android key each one is. */
const PHONE_KEYS: Record<string, string> = {
  Enter: "ENTER",
  // On a phone the escape key means Back; a literal ESCAPE can still be sent
  // through the command by name.
  Escape: "BACK",
  Backspace: "DEL",
  Tab: "TAB",
  Delete: "FORWARD_DEL",
  ArrowUp: "DPAD_UP",
  ArrowDown: "DPAD_DOWN",
  ArrowLeft: "DPAD_LEFT",
  ArrowRight: "DPAD_RIGHT",
  Home: "HOME",
  PageUp: "PAGE_UP",
  PageDown: "PAGE_DOWN",
};

/** A drag shorter than this many device pixels is a tap, not a swipe. */
const DRAG_THRESHOLD = 8;
/**
 * How often the phone is captured.
 *
 * Every capture is one adb process and one 1080x2400 PNG, so the cadence is
 * bounded by what the viewer is doing rather than left running flat out:
 *
 * - right after a tap, swipe or key, follow the phone closely, because something
 *   is about to change and the user is watching for it;
 * - once that window passes, look rarely — a phone held still does not need a
 *   screenshot every second, and pixel equality is the wrong test (a clock, a
 *   cursor or an animation keeps a live screen from ever being byte-identical);
 * - and not at all while the frame is scrolled out of view or the window is
 *   hidden. Refresh brings it back immediately.
 */
const REFRESH_ACTIVE_MS = 750;
const REFRESH_IDLE_MS = 2_500;
const ACTIVE_WINDOW_MS = 4_000;
/** Typing is batched, so one adb call carries a word instead of one per letter. */
const TEXT_FLUSH_MS = 140;
/**
 * Keys whose auto-repeat is not a new action. Holding Escape (Back) or Home on a
 * real phone navigates once; repeated key-downs here would each be a separate
 * Android Back, unwinding the app screen by screen.
 */
const NO_REPEAT = new Set(["Escape", "Home"]);

/** `01:12` — how long a recording has been running. */
function clock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
}

function fileSize(bytes: number): string {
  return bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MB`
    : `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** The last path segment: what the user has to recognise in a folder. */
function fileName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

function PhoneFrame({ props }: { props: ScreenProps }) {
  const { status } = props;
  const ready = status?.state === "ready";
  const [shot, setShot] = useState<PhoneScreen | null>(null);
  const [videoSize, setVideoSize] = useState<{ width: number; height: number } | null>(null);
  const [videoMode, setVideoMode] = useState<"starting" | "live" | "fallback">("starting");
  const [visible, setVisible] = useState(true);
  const [error, setError] = useState<DwError | null>(null);
  const [nonce, setNonce] = useState(0);
  // Saved captures are facts from the backend: a path it wrote, or a reason
  // nothing was saved. The toolbar never shows a file that does not exist.
  const [recording, setRecording] = useState<backend.RecordingState | null>(null);
  const [captureBusy, setCaptureBusy] = useState(false);
  const [saved, setSaved] = useState<
    { kind: "screenshot" | "recording"; path: string; detail: string } | null
  >(null);
  const [, setTick] = useState(0);
  const frame = useRef<HTMLDivElement>(null);
  const videoCanvas = useRef<HTMLCanvasElement>(null);
  const inFlight = useRef(false);
  const onScreen = useRef(true);
  const lastInput = useRef(0);
  const dragStart = useRef<{ x: number; y: number; at: number } | null>(null);
  const pendingText = useRef("");
  const textTimer = useRef<number | undefined>(undefined);
  /** The tail of the Android input queue; see `send`. */
  const order = useRef<Promise<void>>(Promise.resolve());
  /** Whether this screen has ever shown a live frame; a restart keeps the last one. */
  const liveOnce = useRef(false);

  // A recording started before this screen was opened is still running: ask once
  // rather than showing "Record" over a live capture.
  useEffect(() => {
    if (!backend.isDesktop() || !ready) return;
    let live = true;
    backend
      .getRecordingStatus()
      .then((state) => live && setRecording(state.active ? state : null))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [ready]);

  // The elapsed clock only re-renders once a second while something is being
  // recorded; there is no timer otherwise.
  useEffect(() => {
    if (!recording?.active) return;
    const timer = window.setInterval(() => setTick((value) => value + 1), 1_000);
    return () => window.clearInterval(timer);
  }, [recording?.active]);

  /** True while the phone is worth watching: on screen, and recently used. */
  const cadence = () =>
    Date.now() - lastInput.current < ACTIVE_WINDOW_MS ? REFRESH_ACTIVE_MS : REFRESH_IDLE_MS;

  useEffect(() => {
    // A phone scrolled out of view is not being looked at.
    const element = frame.current;
    let observer: IntersectionObserver | undefined;
    if (element && typeof IntersectionObserver !== "undefined") {
      observer = new IntersectionObserver((entries) => {
        onScreen.current = entries.some((entry) => entry.isIntersecting);
        setVisible(onScreen.current && document.visibilityState !== "hidden");
      });
      observer.observe(element);
    }
    const visibility = () => setVisible(onScreen.current && document.visibilityState !== "hidden");
    document.addEventListener("visibilitychange", visibility);
    return () => { observer?.disconnect(); document.removeEventListener("visibilitychange", visibility); };
  }, []);

  useEffect(() => {
    if (!backend.isDesktop() || !ready || !visible) return;
    if (typeof VideoDecoder === "undefined" || typeof EncodedVideoChunk === "undefined") {
      setVideoMode("fallback");
      return;
    }
    const canvas = videoCanvas.current;
    if (!canvas) return;
    let cancelled = false;
    let started = false;
    let timeout: number | undefined;
    let dimensionsTimer: number | undefined;
    let dimensions: { width: number; height: number } | null = null;
    let decodedSize: { width: number; height: number } | null = null;
    // The live state only changes once per stream. Repeating the setters from
    // every decoded frame makes React compare two identical values 60 times a
    // second for no visible result.
    let reported = false;
    const id = `phone-${Date.now()}-${Math.floor(Math.random() * 1_000_000)}`;
    // A restart keeps the picture that is already there instead of blanking the
    // panel for the second the new stream takes to reach its first frame.
    if (!liveOnce.current) {
      setVideoMode("starting");
      setVideoSize(null);
    }
    const fail = (message: string) => {
      if (cancelled) return;
      cancelled = true;
      console.warn("Phone video fallback:", message);
      window.clearTimeout(timeout);
      window.clearInterval(dimensionsTimer);
      player.close();
      if (started) void backend.stopPhoneStream(id).catch(() => {});
      setVideoMode("fallback");
      setError({ code: "runtime_unavailable", summary: "Live phone video is unavailable; showing still captures instead.", detail: message });
    };
    const player = new PhoneVideo(canvas, (width, height) => {
      if (cancelled || reported) return;
      liveOnce.current = true;
      decodedSize = { width, height };
      if (dimensions && ((width > height) === (dimensions.width > dimensions.height))) {
        setVideoSize(dimensions);
        setVideoMode("live");
        window.clearTimeout(timeout);
        reported = true;
      }
    }, (err) => fail(err.message));
    const refreshDimensions = async () => {
      try {
        const [width, height] = await backend.getPhoneDimensions();
        if (cancelled) return;
        dimensions = { width, height };
        if (decodedSize && ((decodedSize.width > decodedSize.height) === (width > height))) {
          setVideoSize(dimensions);
          setVideoMode("live");
        } else if (decodedSize) {
          // screenrecord does not dynamically rotate: restart instead of mapping
          // touches against a stale orientation.
          setVideoSize(null);
          setNonce((value) => value + 1);
        }
      } catch (err) { fail(backend.toDwError(err).summary); }
    };
    void refreshDimensions();
    // Rotation is rare and costs a heavy `dumpsys display`; checking every two
    // seconds spent adb work on a picture that had not changed. Five seconds
    // still catches a rotation long before a user would call it stuck.
    dimensionsTimer = window.setInterval(() => void refreshDimensions(), 5_000);
    timeout = window.setTimeout(() => fail("The video decoder did not produce a frame within 10 seconds."), 10_000);
    void backend.startPhoneStream(id, (data) => {
      if (cancelled) return;
      if (data.byteLength === 0) {
        // Android's capture ended: own time limit, a runtime restart, or the
        // encoder dying. That is routine, so pick the stream up again instead of
        // spending the rest of the session on 750 ms still captures. A stream
        // that never produced a picture is a real failure, not a hiccup.
        if (decodedSize) setNonce((value) => value + 1);
        else fail("Android did not send any video.");
        return;
      }
      try { player.push(data); } catch (err) { fail(String(err)); }
    }).then(() => {
      started = true;
      if (cancelled) void backend.stopPhoneStream(id).catch(() => {});
    }).catch((err) => fail(backend.toDwError(err).summary));
    return () => {
      cancelled = true;
      window.clearTimeout(timeout);
      window.clearInterval(dimensionsTimer);
      player.close();
      if (started) void backend.stopPhoneStream(id).catch(() => {});
    };
  }, [ready, visible, nonce]);

  useEffect(() => {
    if (!backend.isDesktop() || !ready || videoMode !== "fallback") return;
    let timer: number | undefined;
    let cancelled = false;

    const schedule = () => {
      timer = window.setTimeout(() => void capture(), cadence());
    };

    const capture = async () => {
      if (cancelled) return;
      // A hidden window or a frame nobody is looking at needs no fresh picture,
      // and no capture is worth a second adb process while one is still running.
      if (
        document.visibilityState === "hidden" ||
        !onScreen.current ||
        inFlight.current
      ) {
        schedule();
        return;
      }
      inFlight.current = true;
      try {
        const next = await backend.getPhoneScreen();
        if (cancelled) return;
        setShot(next);
      } catch (err) {
        if (!cancelled) setError(backend.toDwError(err));
      } finally {
        inFlight.current = false;
        if (!cancelled) schedule();
      }
    };

    void capture();
    const onVisibility = () => {
      if (document.visibilityState !== "visible") return;
      window.clearTimeout(timer);
      void capture();
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [ready, nonce, videoMode]);

  /**
   * Ask Android for something.
   *
   * Commands are queued, not fired in parallel: each one is its own adb process,
   * so a word being typed and the Enter that submits it would otherwise race, and
   * two quick taps could arrive in the wrong order. The work is created only when
   * its turn comes, which is what makes the order real rather than hopeful.
   */
  const send = (work: () => Promise<void>) => {
    // Input means the phone is about to move: watch it closely for a moment.
    lastInput.current = Date.now();
    order.current = order.current.then(async () => {
      try {
        await work();
        setError(null);
      } catch (err) {
        setError(backend.toDwError(err));
      }
    });
  };

  const size = videoMode === "live" ? videoSize : videoMode === "fallback" ? shot : null;
  const box = () => {
    const element = frame.current;
    if (!element || !size) return null;
    const rect = element.getBoundingClientRect();
    return paintedBox(
      { left: rect.left, top: rect.top, width: rect.width, height: rect.height },
      size,
    );
  };

  const pointAt = (clientX: number, clientY: number) => {
    const painted = box();
    if (!painted || !size) return null;
    // The letterbox margin is not the phone: a click there is not a tap.
    if (!inside(painted, { x: clientX, y: clientY })) return null;
    return devicePoint(painted, size, { x: clientX, y: clientY });
  };

  const takeScreenshot = async () => {
    setCaptureBusy(true);
    try {
      const shot = await backend.phoneScreenshot();
      setSaved({
        kind: "screenshot",
        path: shot.path,
        detail: `${shot.width} × ${shot.height} · ${fileSize(shot.bytes)}`,
      });
      setError(null);
    } catch (err) {
      setError(backend.toDwError(err));
    } finally {
      setCaptureBusy(false);
    }
  };

  const toggleRecording = async () => {
    setCaptureBusy(true);
    try {
      if (recording?.active) {
        const file = await backend.stopRecording();
        setRecording(null);
        setSaved({
          kind: "recording",
          path: file.path,
          detail: `${clock(file.duration_ms)} · ${fileSize(file.bytes)}`,
        });
      } else {
        setRecording(await backend.startRecording());
        setSaved(null);
      }
      setError(null);
    } catch (err) {
      setError(backend.toDwError(err));
    } finally {
      setCaptureBusy(false);
    }
  };

  const flushText = () => {
    window.clearTimeout(textTimer.current);
    const text = pendingText.current;
    pendingText.current = "";
    if (text) send(() => backend.phoneText(text));
  };

  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    const point = pointAt(event.clientX, event.clientY);
    if (!point) return;
    dragStart.current = { ...point, at: Date.now() };
    // Capture keeps the drag even if the pointer leaves the frame mid-swipe.
    frame.current?.setPointerCapture?.(event.pointerId);
  };

  const onPointerUp = (event: React.PointerEvent<HTMLDivElement>) => {
    const origin = dragStart.current;
    const point = pointAt(event.clientX, event.clientY);
    dragStart.current = null;
    if (!origin || !point) return;
    const distance = Math.hypot(point.x - origin.x, point.y - origin.y);
    if (distance <= DRAG_THRESHOLD) {
      send(() => backend.phoneTap(point.x, point.y));
      return;
    }
    const elapsed = Date.now() - origin.at;
    send(() =>
      backend.phoneSwipe(
        origin.x,
        origin.y,
        point.x,
        point.y,
        Math.min(Math.max(elapsed, 60), 800),
      ),
    );
  };

  const onKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (!ready || !size) return;
    // Ctrl/Alt/Meta belong to the app (Ctrl+K searches the library), not the phone.
    if (event.ctrlKey || event.altKey || event.metaKey) return;
    if (event.repeat && NO_REPEAT.has(event.key)) {
      // Swallowed, not forwarded: one hold is one Back.
      event.preventDefault();
      return;
    }
    const named = PHONE_KEYS[event.key];
    if (named) {
      event.preventDefault();
      // A word being typed has to land before the key that acts on it; the queue
      // in `send` is what makes that true.
      flushText();
      send(() => backend.phoneKey(named));
      return;
    }
    if (event.key.length === 1) {
      event.preventDefault();
      pendingText.current += event.key;
      window.clearTimeout(textTimer.current);
      textTimer.current = window.setTimeout(flushText, TEXT_FLUSH_MS);
    }
  };

  const runtime = runtimeTone(status);

  return (
    <Panel
      title="Phone screen"
      note={videoMode === "live" ? "Live view of Android's own display." : "Android's own display."}
    >
      <div className="phone">
        {/* The phone's own navigation, sent to Android rather than to Windows.
            Compact on purpose: the screen below keeps the room. */}
        <div className="phone__bar" role="toolbar" aria-label="Android controls">
          <Button
            size="sm"
            variant="ghost"
            className="phone__key"
            title="Android Back — previous screen or close a dialog"
            aria-label="Android Back"
            disabled={!ready}
            disabledReason={ready ? undefined : "The runtime is not ready."}
            onClick={() => send(() => backend.phoneKey("BACK"))}
          >
            <IconBack size={15} />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="phone__key"
            title="Android Home — return to the launcher"
            aria-label="Android Home"
            disabled={!ready}
            disabledReason={ready ? undefined : "The runtime is not ready."}
            onClick={() => send(() => backend.phoneKey("HOME"))}
          >
            <IconHome size={15} />
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="phone__key"
            title="Android recent apps — switch applications"
            aria-label="Android recent apps"
            disabled={!ready}
            disabledReason={ready ? undefined : "The runtime is not ready."}
            onClick={() => send(() => backend.phoneKey("APP_SWITCH"))}
          >
            <IconRecents size={15} />
          </Button>
          <span className="phone__split" role="separator" aria-orientation="vertical" />
          <Button
            size="sm"
            variant="ghost"
            className="phone__key phone__key--label"
            title="Save a screenshot of the Android screen to your Pictures folder"
            aria-label="Save screenshot"
            disabled={!ready || captureBusy}
            disabledReason={ready ? undefined : "The runtime is not ready."}
            onClick={() => void takeScreenshot()}
          >
            <IconCamera size={15} />
            <span>Screenshot</span>
          </Button>
          <Button
            size="sm"
            variant={recording?.active ? "danger" : "ghost"}
            className="phone__key phone__key--label"
            title={
              recording?.active
                ? "Stop recording and save the video to your Videos folder"
                : "Record the Android screen to your Videos folder"
            }
            aria-label={recording?.active ? "Stop recording" : "Start recording"}
            aria-pressed={recording?.active ?? false}
            disabled={!ready || captureBusy}
            disabledReason={ready ? undefined : "The runtime is not ready."}
            onClick={() => void toggleRecording()}
          >
            {recording?.active ? <IconStop size={13} /> : <IconRecord size={11} />}
            <span>
              {recording?.active
                ? `Stop · ${clock(Date.now() - (recording.started_at ?? Date.now()))}`
                : "Record"}
            </span>
          </Button>
        </div>
        <div
          className="phone__frame"
          ref={frame}
          tabIndex={0}
          aria-label="The phone's screen: click to tap, drag to swipe, type to enter text"
          // The capture's own shape: with equal aspect ratios there is no
          // letterbox, and the box is the pixels the user sees.
          style={size ? { aspectRatio: `${size.width} / ${size.height}` } : undefined}
          onPointerDown={onPointerDown}
          onPointerUp={onPointerUp}
          onPointerCancel={() => {
            dragStart.current = null;
          }}
          // Capture can be lost without a pointerup (released over another
          // window, or the app switched away mid-drag). An origin kept from
          // that drag would turn the next release into a swipe to nowhere.
          onLostPointerCapture={() => {
            dragStart.current = null;
          }}
          onKeyDown={onKeyDown}
          onBlur={flushText}
        >
          <canvas ref={videoCanvas} className="phone__screen" aria-label="Live Android phone screen" style={{ display: ready && videoMode === "live" ? "block" : "none" }} />
          {ready && videoMode === "fallback" && shot ? (
            <img
              className="phone__screen"
              src={shot.image}
              alt={`The Android phone's screen, ${shot.width} by ${shot.height}`}
              draggable={false}
            />
          ) : videoMode !== "live" || !ready ? (
            <div className="phone__idle">
              <IconPhone size={22} />
              <p>
                {ready
                  ? "Capturing the phone's screen…"
                  : "The phone appears once the runtime is ready."}
              </p>
              {!ready && (
                <Button
                  size="sm"
                  variant="primary"
                  onClick={props.onStartRuntime}
                  disabled={props.busy || status?.state === "not_configured"}
                  disabledReason={
                    status?.state === "not_configured"
                      ? "The Android SDK is incomplete; see the Runtime screen."
                      : undefined
                  }
                >
                  Start runtime
                </Button>
              )}
            </div>
          ) : null}
        </div>
        <div className="phone__meta">
          <StatusPill tone={runtime.tone} label={runtime.label} />
          {recording?.active && (
            <StatusPill
              tone="error"
              label={`Recording ${clock(Date.now() - (recording.started_at ?? Date.now()))}`}
              icon={<span className="phone__rec-dot" aria-hidden="true" />}
            />
          )}
          {size && (
            <span className="muted">
              {size.width} × {size.height} · {videoMode === "live" ? "live video" : `captured ${new Date(shot!.captured_at).toLocaleTimeString([], { hour12: false })}`}
            </span>
          )}
        </div>
        <p className="phone__hint">
          Click to tap, drag to swipe. Type to search or fill a field; Enter opens, Escape is Back,
          Home returns to the launcher, Tab and the arrows move the selection, Backspace deletes.
          Ctrl+letter shortcuts still belong to WINAPK.
        </p>
        {saved && (
          <div className="phone__saved" role="status">
            <span>
              {saved.kind === "screenshot" ? "Screenshot saved" : "Recording saved"} —{" "}
              <span className="mono" title={saved.path}>
                {fileName(saved.path)}
              </span>{" "}
              <span className="muted">{saved.detail}</span>
            </span>
            <Button
              size="sm"
              variant="ghost"
              title={`Show ${saved.path} in Explorer`}
              onClick={() =>
                void backend
                  .revealPath(saved.path)
                  .catch((err) => setError(backend.toDwError(err)))
              }
            >
              <IconOpen size={12} />
              Open
            </Button>
          </div>
        )}
        {error && <ErrorNote error={error} />}
      </div>
    </Panel>
  );
}

function Launcher({ props }: { props: ScreenProps }) {
  // Its own search box, deliberately: the top bar's query navigates to the
  // library, and searching here must not throw the user out of the phone.
  const [query, setQuery] = useState("");
  const installed = props.apps ?? [];
  const wanted = query.trim().toLowerCase();
  const shown = wanted
    ? installed.filter(
        (app) =>
          (app.label ?? "").toLowerCase().includes(wanted) ||
          app.package_id.toLowerCase().includes(wanted),
      )
    : installed;
  const running = installed.filter((app) => app.running).length;

  return (
    <Panel
      title="Applications"
      note={
        props.apps === null
          ? "Reading the runtime's package list…"
          : `${installed.length} installed${running > 0 ? ` · ${running} window${running === 1 ? "" : "s"} open` : ""}`
      }
      action={
        <Button size="sm" variant="primary" onClick={props.onBrowse} disabled={props.busy}>
          <IconFolder size={13} />
          Install APK
        </Button>
      }
    >
      {props.appsError ? (
        // The page already reported this failure when it is the same one; a second
        // identical alert is noise, not information.
        props.appsError.code === props.error?.code ? (
          <p className="muted">The library is unavailable until the backend answers.</p>
        ) : (
          <ErrorNote error={props.appsError} />
        )
      ) : (
        <>
          <label className="search search--inline">
            <IconSearch size={14} />
            <input
              type="search"
              value={query}
              placeholder="Search installed apps"
              aria-label="Search installed apps"
              onChange={(event) => setQuery(event.target.value)}
            />
          </label>
          {shown.length === 0 ? (
            <EmptyState
              title={installed.length === 0 ? "No apps installed" : "Nothing matches that search"}
              body={
                installed.length === 0
                  ? "Install a package and it appears on the phone's home screen."
                  : "Clear the search to see every installed package."
              }
            />
          ) : (
            <ul className="launcher">
              {shown.map((app) => (
                <LauncherTile
                  key={app.package_id}
                  app={app}
                  busy={props.busy}
                  onLaunch={props.onLaunch}
                />
              ))}
            </ul>
          )}
        </>
      )}
    </Panel>
  );
}

function LauncherTile({
  app,
  busy,
  onLaunch,
}: {
  app: InstalledApp;
  busy: boolean;
  onLaunch: (packageId: string) => void;
}) {
  const name = app.label ?? app.package_id;
  return (
    <li className="launcher__cell">
      <button
        type="button"
        className="launcher__tile"
        onClick={() => onLaunch(app.package_id)}
        disabled={busy}
        // Opening a window that is already open returns it rather than making a
        // second one, so "open" is honest for a running app too.
        title={app.running ? `${name} — show its window` : `${name} — open in its own window`}
      >
        <span className="launcher__icon" aria-hidden="true">
          {app.icon ? <img src={app.icon} alt="" /> : <IconPackage size={20} />}
        </span>
        <span className="launcher__name">{name}</span>
        {app.running && <span className="launcher__dot" aria-label="window open" />}
      </button>
    </li>
  );
}

function Installations({ props }: { props: ScreenProps }) {
  const installs = props.activity.filter((record) => record.kind === "install").slice(0, 5);
  return (
    <Panel
      title="Installations"
      note={
        props.opRunning
          ? "An installation is running; these are the stages WINAPK has reported."
          : "The last packages WINAPK installed."
      }
    >
      {props.stages.length > 0 ? (
        <StageList events={props.stages} running={props.opRunning} />
      ) : installs.length === 0 ? (
        <p className="muted">
          Nothing installed yet. Choose an APK with Install APK, or open one from Explorer.
        </p>
      ) : (
        <ul className="rows">
          {installs.map((record) => (
            <li className="row" key={record.id}>
              <div className="row__main">
                <div className="row__title">{record.summary}</div>
                <div className="row__meta">
                  <span className="mono">{record.package_id ?? "—"}</span>
                  <span>{new Date(record.started_at).toLocaleString()}</span>
                </div>
              </div>
              <div className="row__actions">
                <StatusPill
                  tone={
                    record.status === "completed"
                      ? "ok"
                      : record.status === "failed"
                        ? "error"
                        : "idle"
                  }
                  label={record.status}
                />
              </div>
            </li>
          ))}
        </ul>
      )}
    </Panel>
  );
}

function Connection({ props }: { props: ScreenProps }) {
  const { status } = props;
  const runtime = runtimeTone(status);
  return (
    <Panel
      title="Runtime and connection"
      action={<StatusPill tone={runtime.tone} label={runtime.label} />}
    >
      <p className="secondary">{status?.detail ?? "The runtime has not been probed yet."}</p>
      <dl className="fields">
        <div className="field">
          <dt className="field__key">Device</dt>
          <dd className="field__value">{status?.serial ?? status?.avd ?? "—"}</dd>
        </div>
        <div className="field">
          <dt className="field__key">Android</dt>
          <dd className="field__value">
            {status?.android_api ? `API ${status.android_api}` : "Not running"}
          </dd>
        </div>
        <div className="field">
          <dt className="field__key">Package list</dt>
          <dd className="field__value">
            {status?.capabilities.enumerate ? "Available" : "Unavailable"}
          </dd>
        </div>
      </dl>
      {status && status.limitations.length > 0 && (
        <ul className="stack stack--sm secondary" style={{ paddingLeft: 18, margin: 0 }}>
          {status.limitations.map((line) => (
            <li key={line}>{line}</li>
          ))}
        </ul>
      )}
      <div className="inline">
        <Button variant="ghost" size="sm" onClick={props.onRefresh} disabled={props.busy}>
          <IconRefresh size={13} />
          Re-check
        </Button>
        <Button variant="ghost" size="sm" onClick={() => props.onNavigate("runtime")}>
          Runtime details
        </Button>
      </div>
    </Panel>
  );
}

export function Phone(props: ScreenProps) {
  return (
    <div className="page">
      <PageHead
        eyebrow="Virtual phone"
        title="Android, inside WINAPK"
        lede="The runtime's own phone screen, the apps it has installed, and what WINAPK is installing."
      >
        <Button variant="secondary" onClick={() => props.onNavigate("applications")}>
          Library
        </Button>
      </PageHead>

      {props.error && <ErrorNote error={props.error} />}
      {props.notice && (
        <div className="note note--info" role="status">
          {props.notice}
        </div>
      )}

      <div className="grid grid--phone">
        <div className="stack">
          <PhoneFrame props={props} />
        </div>
        <div className="stack">
          <Launcher props={props} />
          <Installations props={props} />
        </div>
      </div>

      <Connection props={props} />
      <Performance props={props} />
    </div>
  );
}

/**
 * What WINAPK can measure, and nothing else. Every value comes from the
 * hardware report or from recorded operation times; a value the backend did not
 * report is shown as not measured, never as a guess.
 */
function Performance({ props }: { props: ScreenProps }) {
  const [report, setReport] = useState<backend.HardwareReport | null>(null);
  useEffect(() => {
    let live = true;
    backend
      .getHardware()
      .then((next) => live && setReport(next))
      .catch(() => live && setReport(null));
    return () => {
      live = false;
    };
  }, []);

  const installs = props.activity.filter(
    (record) => record.kind === "install" && record.finished_at != null,
  );
  const durations = installs.map((r) => (r.finished_at as number) - r.started_at);
  const last = durations[0];
  const rows: [string, string][] = [
    ["Host CPUs", report ? `${report.host.logical_cpus} logical` : "not measured"],
    [
      "Host RAM",
      report?.host.memory_detected
        ? `${Math.round(report.host.available_ram_mb / 1024)} GB free of ${Math.round(report.host.total_ram_mb / 1024)} GB`
        : "not measured",
    ],
    ["Runtime cores", report?.effective?.cores != null ? `${report.effective.cores} (Android reports)` : "not measured"],
    ["Runtime RAM", report?.effective?.ram_mb != null ? `${report.effective.ram_mb} MB (Android reports)` : "not measured"],
    ["Renderer", report?.effective?.gpu_renderer ?? "not measured"],
    [
      "Acceleration",
      report?.effective?.hardware_gpu == null
        ? "not measured"
        : report.effective.hardware_gpu
          ? "hardware (host GPU)"
          : "software rendering",
    ],
    [
      "Last install",
      last != null ? `${(last / 1000).toFixed(1)} s (from WINAPK's record)` : "no install recorded",
    ],
  ];

  return (
    <Panel title="Performance" note="Measured values only. Anything Android did not report is marked not measured.">
      <dl className="perf">
        {rows.map(([label, value]) => (
          <div className="perf__row" key={label}>
            <dt>{label}</dt>
            <dd className="mono">{value}</dd>
          </div>
        ))}
      </dl>
    </Panel>
  );
}
