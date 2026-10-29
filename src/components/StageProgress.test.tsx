import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { Stage, StageEvent } from "../backend";
import { PHASES, phaseFor, phaseStates, StageProgress } from "./StageProgress";

function event(stage: Stage, at_ms = 0, message = "stage"): StageEvent {
  return { op: "op-1", package_id: null, stage, message, at_ms };
}

describe("stage phases", () => {
  it("assigns every backend stage to a phase or to none", () => {
    expect(phaseFor("received")).toBe("prepare");
    expect(phaseFor("validating")).toBe("prepare");
    expect(phaseFor("inspecting")).toBe("prepare");
    expect(phaseFor("checking_runtime")).toBe("prepare");
    expect(phaseFor("preparing_runtime")).toBe("start");
    expect(phaseFor("installing")).toBe("install");
    expect(phaseFor("verifying")).toBe("install");
    expect(phaseFor("launching")).toBe("launch");
    expect(phaseFor("completed")).toBe("ready");
    // Terminal and waiting states are not progress.
    expect(phaseFor("failed")).toBe(null);
    expect(phaseFor("cancelled")).toBe(null);
    expect(phaseFor("needs_user_action")).toBe(null);
  });

  it("has one segment per phase, in operation order", () => {
    expect(PHASES.map((phase) => phase.id)).toEqual([
      "prepare",
      "start",
      "install",
      "launch",
      "ready",
    ]);
  });

  it("marks the reported phases done and the next one active", () => {
    const states = phaseStates(
      [event("received", 0), event("checking_runtime", 400), event("preparing_runtime", 900)],
      true,
    );
    expect(states.map((state) => state.status)).toEqual([
      "done",
      "active",
      "pending",
      "pending",
      "pending",
    ]);
  });

  it("marks the phase that was running when the operation failed", () => {
    const states = phaseStates(
      [event("received", 0), event("installing", 400), event("failed", 900)],
      false,
    );
    // Reaching the install phase means the runtime was already up, so the boot
    // segment counts as behind us even though it reported nothing this run.
    expect(states.map((state) => state.status)).toEqual([
      "done",
      "done",
      "failed",
      "pending",
      "pending",
    ]);
  });

  it("fills every segment once the operation completed", () => {
    const states = phaseStates([event("launching", 0), event("completed", 100)], false);
    expect(states.every((state) => state.status === "done")).toBe(true);
  });
});

describe("progress rendering", () => {
  it("renders nothing before the first stage", () => {
    const { container } = render(<StageProgress events={[]} running={false} />);
    expect(container.firstChild).toBeNull();
  });

  it("shows the elapsed time between the first and last stage", () => {
    const { container } = render(
      <StageProgress
        events={[event("received", 1_000), event("completed", 6_000)]}
        running={false}
      />,
    );
    expect(container.querySelector(".progress__time")?.textContent).toBe("0:05");
  });

  it("exposes the active phase to assistive technology", () => {
    const { container } = render(
      <StageProgress
        events={[event("received", 0), event("installing", 2_000)]}
        running
      />,
    );
    const bar = container.querySelector('[role="progressbar"]');
    expect(bar?.getAttribute("aria-valuetext")).toBe("Installing and verifying");
    expect(bar?.getAttribute("aria-valuenow")).toBe("2");
    expect(container.querySelectorAll('[data-status="active"]')).toHaveLength(1);
  });
});
