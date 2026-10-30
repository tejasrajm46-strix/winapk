// Drives the running DroidWin window through the WebView2 DevTools endpoint.
//
// Used only as evidence tooling: it clicks real UI elements in the shipped
// renderer and captures what the window actually shows. Start the app with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 .
//
//   node scripts/ui-shot.mjs --eval "<js expression>"
//   node scripts/ui-shot.mjs --shot out.png
//   node scripts/ui-shot.mjs --shot panel.png --element ".phone__frame"
//   node scripts/ui-shot.mjs --click "Runtime"
//   node scripts/ui-shot.mjs --wait "<js boolean expression>"

const PORT = process.env.DROIDWIN_CDP_PORT || "9222";

async function targets() {
  const response = await fetch(`http://127.0.0.1:${PORT}/json`);
  const pages = await response.json();
  const found = pages.filter((p) => p.type === "page" && p.webSocketDebuggerUrl);
  if (!found.length) throw new Error("no debuggable page on the DroidWin window");
  return found;
}

async function open(page) {
  const socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = () => reject(new Error("devtools socket failed"));
  });
  let id = 0;
  const pending = new Map();
  socket.onmessage = (event) => {
    const message = JSON.parse(event.data);
    const waiter = pending.get(message.id);
    if (!waiter) return;
    pending.delete(message.id);
    if (message.error) waiter.reject(new Error(message.error.message));
    else waiter.resolve(message.result);
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const next = ++id;
      pending.set(next, { resolve, reject });
      socket.send(JSON.stringify({ id: next, method, params }));
    });
  return { page, send, close: () => socket.close() };
}

// The app owns five webviews with the same title: the manager window and four
// overlay windows. The manager is the largest one, so pick by real width rather
// than by list order (which is not stable between runs).
async function connect() {
  let best = null;
  for (const page of await targets()) {
    const candidate = await open(page);
    try {
      const textResult = await candidate.send("Runtime.evaluate", {
        expression: "document.body.innerText",
        returnByValue: true,
      });
      const text = String(textResult.result?.value ?? "");
      if (selectText && !text.includes(selectText)) {
        candidate.close();
        continue;
      }
      const result = await candidate.send("Runtime.evaluate", {
        expression: "window.innerWidth",
        returnByValue: true,
      });
      const width = Number(result.result?.value ?? 0);
      // With a selector, the smallest matching page wins: overlay windows are
      // short, while the manager's text can mention the same words.
      const better = selectText
        ? !best || text.length < best.textLength
        : !best || width > best.width;
      if (better) {
        best?.connection.close();
        best = { connection: candidate, width, textLength: text.length };
      } else {
        candidate.close();
      }
    } catch {
      candidate.close();
    }
  }
  if (!best) throw new Error("no responsive DroidWin webview");
  return best.connection;
}

async function evaluate(send, expression) {
  const result = await send("Runtime.evaluate", {
    expression,
    awaitPromise: true,
    returnByValue: true,
  });
  if (result.exceptionDetails) {
    throw new Error(result.exceptionDetails.exception?.description || "evaluation failed");
  }
  return result.result.value;
}

const args = process.argv.slice(2);
// Optional page selector: overlay windows share the manager's title, so a
// caller that needs the rail or the mapping panel picks its page by text.
const selectAt = args.indexOf("--select");
const selectText = selectAt >= 0 ? args[selectAt + 1] : null;
if (selectAt >= 0) args.splice(selectAt, 2);
const flag = (name) => {
  const at = args.indexOf(name);
  return at === -1 ? null : args[at + 1];
};

const { page, send, close } = await connect();
try {
  const waitFor = flag("--wait");
  if (waitFor) {
    const deadline = Date.now() + 30_000;
    for (;;) {
      if (await evaluate(send, `Boolean(${waitFor})`)) break;
      if (Date.now() > deadline) throw new Error(`timed out waiting for ${waitFor}`);
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    console.log("waited");
  }

  const script = flag("--eval");
  if (script) {
    const value = await evaluate(send, script);
    console.log(typeof value === "string" ? value : JSON.stringify(value));
  }

  const label = flag("--click");
  if (label) {
    const clicked = await evaluate(
      send,
      `(() => {
        const wanted = ${JSON.stringify(label)};
        const nodes = [...document.querySelectorAll("button, a, [role=button]")];
        const hit = nodes.find((node) => (node.textContent || "").trim() === wanted)
          || nodes.find((node) => (node.textContent || "").trim().includes(wanted));
        if (!hit) return "missing: " + wanted;
        hit.click();
        return "clicked: " + (hit.textContent || "").trim();
      })()`,
    );
    console.log(clicked);
  }

  const shot = flag("--shot");
  if (shot) {
    // `--element` captures the whole box even when it is taller than the
    // viewport (the phone panel is), so the asset is the panel rather than a
    // crop of whatever happened to be scrolled into view.
    const selector = flag("--element");
    let params = { format: "png" };
    if (selector) {
      const box = await evaluate(
        send,
        `(() => {
          const element = document.querySelector(${JSON.stringify(selector)});
          if (!element) return null;
          const rect = element.getBoundingClientRect();
          return { x: rect.left + window.scrollX, y: rect.top + window.scrollY, width: rect.width, height: rect.height };
        })()`,
      );
      if (!box) throw new Error(`missing element: ${selector}`);
      params = {
        format: "png",
        captureBeyondViewport: true,
        clip: { ...box, scale: 1 },
      };
    }
    const image = await send("Page.captureScreenshot", params);
    const { writeFileSync } = await import("node:fs");
    writeFileSync(shot, Buffer.from(image.data, "base64"));
    console.log(`saved ${shot}`);
    console.log(`title: ${page.title}`);
  }
} finally {
  close();
}
