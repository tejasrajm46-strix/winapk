import { describe, expect, it } from "vitest";
import { nextPollDelay } from "./polling";

describe("runtime status polling policy", () => {
  it("checks a booting runtime quickly", () => {
    expect(nextPollDelay("starting", false)).toBeLessThanOrEqual(3_000);
  });

  it("keeps a steady runtime on a slow cadence", () => {
    expect(nextPollDelay("ready", false)).toBeGreaterThanOrEqual(15_000);
    expect(nextPollDelay("not_running", false)).toBeGreaterThanOrEqual(15_000);
  });

  it("does not poll a missing SDK often, because waiting cannot change it", () => {
    expect(nextPollDelay("not_configured", false)).toBeGreaterThan(nextPollDelay("ready", false));
  });

  it("almost stops while the window is hidden", () => {
    expect(nextPollDelay("starting", true)).toBeGreaterThan(nextPollDelay("ready", false));
  });
});
