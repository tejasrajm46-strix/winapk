#!/usr/bin/env node
/**
 * One command for every live check.
 *
 * The live checks each boot or reuse the Android emulator, so they cannot run
 * together: this runs them one after another, keeps each run's full output in
 * `.droidwin/live/<area>.log`, and prints one line per area with the result, the
 * time it took and the test's own summary line.
 *
 *   node scripts/live.mjs            the checks this repo keeps green (including the CLI)
 *   node scripts/live.mjs --all      also agent and hcr (see below)
 *   node scripts/live.mjs hook       only the areas named
 *
 * `agent` installs a real package and `hcr` needs a game already installed in
 * the AVD, so both are opt-in rather than default: a summary that is red on every
 * machine for reasons unrelated to the code teaches nobody anything.
 *
 * The exit status is 1 when any area failed. A run that hangs is killed after its
 * timeout, reported as a failure, and says so — a silent hang is worse than a red
 * line. The screenshot/recording/audio/perf sweeps that need the desktop app
 * running are not here: they drive the GUI, not a test target.
 */

import { spawn } from "node:child_process";
import { createWriteStream, existsSync, mkdirSync } from "node:fs";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const LOG_DIR = join(ROOT, ".droidwin", "live");
const CWD = join(ROOT, "src-tauri");

const AREAS = [
  {
    name: "capture",
    target: "capture_live",
    does: "Home, Back, recents, screenshot, recording, display override",
    timeoutMs: 10 * 60_000,
  },
  {
    name: "phone",
    target: "phone_live",
    does: "the phone display captures pixels and receives a touch",
    timeoutMs: 8 * 60_000,
  },
  {
    name: "mapping",
    target: "mapping_live",
    does: "mapped keys fire once and every release lands",
    timeoutMs: 8 * 60_000,
  },
  {
    name: "hook",
    target: "hook_live",
    does: "real desktop keystrokes through the Windows keyboard hook",
    timeoutMs: 8 * 60_000,
  },
  {
    name: "isolation",
    target: "isolation_live",
    does: "the guest cannot reach the host's loopback, and networking still works",
    timeoutMs: 8 * 60_000,
  },
  {
    name: "cli",
    target: "cli_live",
    does: "the real command line drives the runtime, mapping, captures and navigation",
    timeoutMs: 10 * 60_000,
  },
  {
    name: "agent",
    target: "agent_live",
    does: "an agent installs, opens and drives an app",
    timeoutMs: 15 * 60_000,
    optIn: true,
  },
  {
    name: "hcr",
    target: "hcr_recovery",
    does: "a game Android keeps off its own display is reported, not left black",
    timeoutMs: 10 * 60_000,
    optIn: true,
  },
];

/** Cargo, wherever it is. A shell that has not sourced the Rust environment has
 * no `cargo` on PATH, so the conventional install directory is tried as well. */
function cargo() {
  if (process.env.CARGO) return process.env.CARGO;
  const names = process.platform === "win32" ? ["cargo.exe", "cargo"] : ["cargo"];
  const found = (process.env.PATH ?? "")
    .split(delimiter)
    .flatMap((dir) => names.map((name) => join(dir, name)))
    .find((path) => existsSync(path));
  if (found) return found;
  const home = process.env.USERPROFILE ?? process.env.HOME ?? "";
  const installed = names.map((name) => join(home, ".cargo", "bin", name));
  return installed.find((path) => existsSync(path)) ?? "cargo";
}

const CARGO = cargo();

function run(area) {
  return new Promise((resolve) => {
    const log = join(LOG_DIR, `${area.name}.log`);
    const file = createWriteStream(log);
    const started = Date.now();
    const child = spawn(
      CARGO,
      ["test", "--test", area.target, "--", "--ignored", "--nocapture"],
      { cwd: CWD, stdio: ["ignore", "pipe", "pipe"], windowsHide: true },
    );
    // The log holds everything; only its tail is kept in memory, for the summary.
    let text = "";
    const collect = (chunk) => {
      file.write(chunk);
      text = (text + chunk).slice(-200_000);
    };
    child.stdout.on("data", collect);
    child.stderr.on("data", collect);

    let timedOut = false;
    const timer = setTimeout(() => {
      timedOut = true;
      // The whole tree: a stuck adb or window host would otherwise outlive the
      // test and hold the emulator for the next area.
      spawn("taskkill", ["/PID", String(child.pid), "/T", "/F"], {
        stdio: "ignore",
        windowsHide: true,
      });
    }, area.timeoutMs);

    let settled = false;
    const finish = (result) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      file.end();
      resolve(result);
    };
    child.on("error", (err) =>
      finish({ area, ok: false, seconds: 0, detail: err.message, log }),
    );
    child.on("close", (code) => {
      const summary = text
        .split(/\r?\n/)
        .filter((line) => /^test result:/.test(line.trim()))
        .pop();
      finish({
        area,
        ok: !timedOut && code === 0,
        seconds: (Date.now() - started) / 1000,
        detail: timedOut
          ? `killed after ${Math.round(area.timeoutMs / 1000)} s`
          : (summary?.trim() ?? `exit ${code}`),
        log,
      });
    });
  });
}

async function main() {
  const args = process.argv.slice(2);
  const all = args.includes("--all");
  const asked = args.filter((arg) => !arg.startsWith("--"));
  const known = new Set(AREAS.map((area) => area.name));
  const unknown = asked.filter((name) => !known.has(name));
  if (unknown.length) {
    console.error(`no such area: ${unknown.join(", ")}`);
    console.error(`areas: ${AREAS.map((area) => area.name).join(", ")}`);
    process.exit(2);
  }
  const chosen = AREAS.filter((area) =>
    asked.length ? asked.includes(area.name) : all || !area.optIn,
  );

  mkdirSync(LOG_DIR, { recursive: true });
  console.log(`DroidWin live checks — ${CARGO}`);
  console.log(`${chosen.length} area(s), one after another; logs in .droidwin/live\n`);

  const results = [];
  for (const area of chosen) {
    process.stdout.write(`${area.name.padEnd(9)} running…`);
    const result = await run(area);
    results.push(result);
    process.stdout.write(
      `\r${area.name.padEnd(9)} ${result.ok ? "PASS" : "FAIL"}  ${result.seconds.toFixed(1).padStart(5)} s  ${result.detail}\n`,
    );
  }

  const failed = results.filter((result) => !result.ok);
  console.log(
    `\n${results.length - failed.length}/${results.length} passed` +
      (failed.length ? ` — failed: ${failed.map((r) => r.area.name).join(", ")}` : ""),
  );
  const skipped = AREAS.filter((area) => !chosen.includes(area));
  if (skipped.length) {
    console.log(`not run: ${skipped.map((area) => area.name).join(", ")} (add --all or name them)`);
  }
  if (failed.length) {
    for (const result of failed) console.log(`  ${result.area.name}: ${result.log}`);
    process.exit(1);
  }
}

main();
