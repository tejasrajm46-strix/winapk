import type { RuntimeState } from "./backend";

/**
 * How long the shell waits before asking the backend for runtime status again.
 *
 * A poll is one `adb` process on the Rust side, so the interval is chosen from
 * what can actually change: a booting runtime is checked quickly, a steady one
 * slowly, and a window nobody is looking at almost never. Nothing here changes
 * what is reported — only how often the same answer is re-read.
 */
export function nextPollDelay(state: RuntimeState | null, hidden: boolean): number {
  if (hidden) return 20_000;
  switch (state) {
    case "starting":
      return 3_000;
    case "unhealthy":
      return 5_000;
    case "not_configured":
      // The SDK does not appear by itself; re-checking often cannot help.
      return 30_000;
    case "ready":
    case "not_running":
    default:
      return 15_000;
  }
}
