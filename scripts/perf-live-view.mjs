// Measure DroidWin's live phone view in the real WebView2 window, over CDP.
//
// What it reports: painted frames per second on the phone canvas while Android's
// screen is scrolling, main-thread long tasks, click-to-pixel latency for the
// phone's own Home button and for a tap on the screen, and what happens when
// Android's capture ends mid-session.
//
// It needs the app to expose WebView2's debugging port, which is not on in the
// shipped build. Add this to `app.windows[0]` in src-tauri/tauri.conf.json, build
// with `pnpm tauri build --no-bundle`, and remove it again afterwards:
//
//     "additionalBrowserArgs": "--remote-debugging-port=9222"
//
// Usage (with the app running):
//   node scripts/perf-live-view.mjs fps
//   node scripts/perf-live-view.mjs latency
//   node scripts/perf-live-view.mjs recover
//   node scripts/perf-live-view.mjs all
//
// Android screenrecord only emits pictures when the screen changes, so every
// rate here is measured with the phone actually scrolling. A run that cannot get
// the screen to move fails instead of reporting a frame rate for a still image.

import { execFile } from "node:child_process";
import path from "node:path";
import { promisify } from "node:util";

const run = promisify(execFile);
const ADB =
  process.env.DROIDWIN_ADB ||
  path.join(process.env.LOCALAPPDATA || "", "Android", "Sdk", "platform-tools", "adb.exe");
const SERIAL = process.env.DROIDWIN_SERIAL || "emulator-5554";
const PORT = Number(process.env.DROIDWIN_DEBUG_PORT || 9222);
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const adb = (...args) =>
  run(ADB, ["-s", SERIAL, ...args]).catch((error) => ({
    stdout: String(error.stdout ?? ""),
    stderr: String(error.stderr ?? ""),
  }));

async function connect() {
  const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
  const page = list.find((target) => target.type === "page");
  if (!page) throw new Error(`no page target on port ${PORT}: ${JSON.stringify(list)}`);
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  const pending = new Map();
  let next = 0;
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = () => reject(new Error(`could not attach to ${page.webSocketDebuggerUrl}`));
  });
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      pending.get(message.id)(message);
      pending.delete(message.id);
    }
  };
  const send = (method, params = {}) => {
    const id = ++next;
    ws.send(JSON.stringify({ id, method, params }));
    return new Promise((resolve) => pending.set(id, resolve));
  };
  const evaluate = async (expression) => {
    const reply = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
    if (reply.result?.exceptionDetails) throw new Error(JSON.stringify(reply.result.exceptionDetails));
    return reply.result?.result?.value;
  };
  return { send, evaluate, ws };
}

async function waitFor(evaluate, expression, timeoutMs, label) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await evaluate(expression)) return;
    await sleep(250);
  }
  throw new Error(`timed out waiting for ${label}`);
}

/** Put Android on a list that really scrolls. */
async function prepareScrollingScreen() {
  await adb("shell", "am", "force-stop", "com.google.android.deskclock");
  await adb("shell", "am", "start", "-a", "android.settings.SETTINGS");
  await sleep(2000);
  // Settings opens the page it last showed; one Back reaches the long list.
  await adb("shell", "input", "keyevent", "4");
  await sleep(1200);
}

/** Fresh page, Phone screen on, live canvas decoding. */
async function openPhone(cdp) {
  await cdp.send("Page.enable");
  await cdp.send("Runtime.enable");
  await cdp.send("Page.reload", { ignoreCache: false });
  await sleep(2500);
  await cdp.send("Page.bringToFront");
  const clicked = await cdp.evaluate(`(() => {
    const hit = [...document.querySelectorAll('button')].find((b) => (b.textContent || '').trim() === 'Phone');
    if (!hit) return false;
    hit.click();
    return true;
  })()`);
  if (!clicked) throw new Error("the Phone screen has no nav button");
  await waitFor(
    cdp.evaluate,
    `(() => { const c = document.querySelector('canvas.phone__screen');
      return !!c && getComputedStyle(c).display !== 'none' && c.width > 0; })()`,
    30_000,
    "the live phone canvas",
  );
  await sleep(1500);
  return cdp.evaluate(`(() => {
    const c = document.querySelector('canvas.phone__screen');
    const box = c.getBoundingClientRect();
    return { pixels: c.width + 'x' + c.height, css: Math.round(box.width) + 'x' + Math.round(box.height) };
  })()`);
}

const swipe = (y1 = "1900", y2 = "800") => adb("shell", "input", "swipe", "540", y1, "540", y2, "160");

/** Fail loudly rather than report a frame rate measured on a still screen. */
async function assertMoving(cdp, label) {
  await cdp.evaluate(`(() => {
    window.__probe = 0;
    const target = document.querySelector('canvas.phone__screen');
    const original = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (...args) {
      if (this.canvas === target) window.__probe += 1;
      return original.apply(this, args);
    };
    return true;
  })()`);
  for (let i = 0; i < 4; i += 1) {
    await swipe();
    await sleep(220);
  }
  const frames = await cdp.evaluate("window.__probe");
  if (frames < 10) throw new Error(`${label}: the phone screen is not changing (${frames} frames while swiping)`);
}

async function measureFps(cdp, seconds = 10) {
  await prepareScrollingScreen();
  const canvas = await openPhone(cdp);
  await assertMoving(cdp, "fps");
  await cdp.evaluate(`(() => {
    const target = document.querySelector('canvas.phone__screen');
    window.__frames = [];
    window.__longtasks = [];
    const original = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (...args) {
      if (this.canvas === target) window.__frames.push(performance.now());
      return original.apply(this, args);
    };
    try {
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) window.__longtasks.push(entry.duration);
      }).observe({ entryTypes: ['longtask'] });
    } catch {}
    return true;
  })()`);

  let scrolling = true;
  const scroll = (async () => {
    while (scrolling) {
      await swipe();
      await sleep(120);
    }
  })();
  await sleep(seconds * 1000);
  scrolling = false;
  await scroll;

  const speed = await cdp.evaluate(`(() => {
    const { __frames: frames, __longtasks: longtasks } = window;
    const span = frames.length > 1 ? (frames[frames.length - 1] - frames[0]) / 1000 : 0;
    const gaps = frames.slice(1).map((t, i) => t - frames[i]).sort((a, b) => a - b);
    return {
      frames: frames.length,
      seconds: +span.toFixed(2),
      fps: +(span ? frames.length / span : 0).toFixed(1),
      gapMedianMs: +(gaps.length ? gaps[gaps.length >> 1] : 0).toFixed(1),
      gapP95Ms: +(gaps.length ? gaps[Math.floor(gaps.length * 0.95)] : 0).toFixed(1),
      gapMaxMs: +(gaps.length ? gaps[gaps.length - 1] : 0).toFixed(1),
      longtasks: longtasks.length,
      longtaskMs: Math.round(longtasks.reduce((sum, d) => sum + d, 0)),
    };
  })()`);
  return { canvas, ...speed };
}

async function measureLatency(cdp, kind) {
  const canvas = await openPhone(cdp);
  await cdp.evaluate(`(() => {
    const target = document.querySelector('canvas.phone__screen');
    const context = target.getContext('2d');
    const points = [[0.5, 0.3], [0.5, 0.45], [0.5, 0.6], [0.3, 0.5], [0.7, 0.5]];
    window.__samples = [];
    const original = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (...args) {
      if (this.canvas === target) {
        let pixels = '';
        try {
          pixels = points.map(([x, y]) => {
            const d = context.getImageData(Math.floor(target.width * x), Math.floor(target.height * y), 1, 1).data;
            return d[0] + '.' + d[1] + '.' + d[2];
          }).join('|');
        } catch {}
        window.__samples.push([performance.now(), pixels]);
      }
      return original.apply(this, args);
    };
    return true;
  })()`);

  const clickAt = async (x, y) => {
    for (const type of ["mousePressed", "mouseReleased"]) {
      await cdp.send("Input.dispatchMouseEvent", {
        type, x, y, button: "left", clickCount: 1, buttons: type === "mousePressed" ? 1 : 0,
      });
    }
  };

  await adb("shell", "am", "start", "-a", "android.settings.SETTINGS");
  await sleep(2500);
  let target;
  if (kind === "home") {
    // Home from inside Settings is a change DroidWin asks Android for itself.
    target = await cdp.evaluate(`(() => {
      const hit = [...document.querySelectorAll('button')].find((b) => b.getAttribute('aria-label') === 'Android Home');
      if (!hit) return null;
      const r = hit.getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
    })()`);
  } else {
    // A tap on Settings' list always lands on a row.
    target = await cdp.evaluate(`(() => {
      const r = document.querySelector('canvas.phone__screen').getBoundingClientRect();
      return { x: r.left + r.width / 2, y: r.top + r.height * 0.45 };
    })()`);
  }
  if (!target) return { kind, canvas, latencyMs: null, note: "target missing" };

  const before = await cdp.evaluate(`(() => { const s = window.__samples; return s.length ? s[s.length - 1][1] : ''; })()`);
  const started = await cdp.evaluate("performance.now()");
  await clickAt(target.x, target.y);
  await sleep(2500);
  const after = await cdp.evaluate(`window.__samples.filter(([t]) => t >= ${started}).map(([t, p]) => [t, p])`);
  const changed = after.find(([, pixels]) => pixels && pixels !== before);
  return { kind, canvas, latencyMs: changed ? Math.round(changed[0] - started) : null };
}

/** Scroll, kill the capture in the middle, and see whether the view comes back. */
async function measureRecovery(cdp, seconds = 18) {
  await prepareScrollingScreen();
  const canvas = await openPhone(cdp);
  await assertMoving(cdp, "recovery");
  await cdp.evaluate(`(() => {
    const target = document.querySelector('canvas.phone__screen');
    window.__frames = [];
    const original = CanvasRenderingContext2D.prototype.drawImage;
    CanvasRenderingContext2D.prototype.drawImage = function (...args) {
      if (this.canvas === target) window.__frames.push(performance.now());
      return original.apply(this, args);
    };
    return true;
  })()`);

  let scrolling = true;
  const scroll = (async () => {
    while (scrolling) {
      await swipe();
      await sleep(120);
    }
  })();

  const started = Date.now();
  await sleep(6000);
  const ps = await adb("shell", "ps -A | grep screenrecord");
  const pid = (String(ps.stdout).match(/shell\s+(\d+)/) || [])[1];
  if (!pid) throw new Error(`no screenrecord to end: ${ps.stdout}`);
  await adb("shell", "kill", pid);
  const killedAt = Date.now();
  await sleep(seconds * 1000 - 6000);
  scrolling = false;
  await scroll;

  const before = (killedAt - started) / 1000;
  const result = await cdp.evaluate(`(() => {
    const frames = window.__frames;
    const t0 = frames.length ? frames[0] : 0;
    // Timestamps are milliseconds; the windows here are seconds.
    const rate = (from, to) => {
      const inside = frames.filter((t) => {
        const at = (t - t0) / 1000;
        return at >= from && at < to;
      });
      return +(inside.length / (to - from)).toFixed(1);
    };
    const gaps = frames.slice(1).map((t, i) => t - frames[i]);
    const canvas = document.querySelector('canvas.phone__screen');
    const image = document.querySelector('img.phone__screen');
    return {
      fpsBeforeDrop: rate(0, ${before.toFixed(2)}),
      fpsAfterDrop: rate(${(before + 0.5).toFixed(2)}, ${seconds}),
      maxGapMs: gaps.length ? Math.round(Math.max(...gaps)) : null,
      canvasVisible: !!canvas && getComputedStyle(canvas).display !== 'none',
      stillCaptures: !!image && getComputedStyle(image).display !== 'none',
    };
  })()`);
  const live = await adb("shell", "ps -A | grep screenrecord");
  return { canvas, ...result, captureRunning: /screenrecord/.test(String(live.stdout)) };
}

const main = async () => {
  const cdp = await connect();
  const mode = process.argv[2] || "all";
  try {
    if (mode === "fps" || mode === "all") console.log("fps:", JSON.stringify(await measureFps(cdp)));
    if (mode === "latency" || mode === "all") {
      console.log("home click:", JSON.stringify(await measureLatency(cdp, "home")));
      console.log("canvas tap:", JSON.stringify(await measureLatency(cdp, "tap")));
    }
    if (mode === "recover" || mode === "all") {
      console.log("capture drop:", JSON.stringify(await measureRecovery(cdp)));
    }
  } finally {
    cdp.ws.close();
  }
};

main().catch((error) => {
  console.error("FAILED:", error.message);
  process.exitCode = 1;
});
