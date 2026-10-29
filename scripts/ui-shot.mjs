// Drives the running DroidWin window through the WebView2 DevTools endpoint.
//
// Used only as evidence tooling: it clicks real UI elements in the shipped
// renderer and captures what the window actually shows. Start the app with
// WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 .
//
//   node scripts/ui-shot.mjs --eval "<js expression>"
//   node scripts/ui-shot.mjs --shot out.png
//   node scripts/ui-shot.mjs --click "Runtime"
//   node scripts/ui-shot.mjs --wait "<js boolean expression>"

const PORT = process.env.DROIDWIN_CDP_PORT || "9222";

async function target() {
  const response = await fetch(`http://127.0.0.1:${PORT}/json`);
  const pages = await response.json();
  const page = pages.find((p) => p.type === "page" && p.webSocketDebuggerUrl);
  if (!page) throw new Error("no debuggable page on the DroidWin window");
  return page;
}

async function connect() {
  const page = await target();
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
    const image = await send("Page.captureScreenshot", { format: "png" });
    const { writeFileSync } = await import("node:fs");
    writeFileSync(shot, Buffer.from(image.data, "base64"));
    console.log(`saved ${shot}`);
    console.log(`title: ${page.title}`);
  }
} finally {
  close();
}
