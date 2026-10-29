import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import type { AttentionProblem, AttentionReport } from "../backend";
import { RuntimeScreen } from "../screens/Runtime";
import { props } from "./fixtures";

const blank: AttentionProblem = {
  id: "app.blank-render:org.example.game",
  scope: "app",
  package_id: "org.example.game",
  severity: "blocking",
  confidence: "confirmed",
  title: "org.example.game is drawing a blank window",
  observed:
    "Two screenshots of display 4 taken 700 ms apart are both 1080x2400 and every pixel is #000000.",
  evidence: [
    {
      source: "screenshot",
      detail: "first capture 3763 bytes, hash 00000000000000aa; second capture 3763 bytes, hash 00000000000000aa — identical.",
    },
    { source: "probe", detail: "The package has a live process and the window is open on display 4." },
  ],
  recommendation:
    "Retry the launch first: DroidWin closes this window and opens org.example.game again on a fresh display.",
  recovery: { kind: "retry_launch", package_id: "org.example.game" },
  alternatives: [{ kind: "reinstall_package", package_id: "org.example.game" }],
};

const capture: AttentionProblem = {
  id: "app.capture-failed:org.fdroid.fdroid",
  scope: "app",
  package_id: "org.fdroid.fdroid",
  severity: "degraded",
  confidence: "suspected",
  title: "DroidWin could not capture org.fdroid.fdroid's display",
  observed:
    "The window is open on display 3, but its screenshot failed, so whether it renders correctly is unknown.",
  evidence: [{ source: "probe", detail: "Android did not return an image for display 3." }],
  recommendation: "Reopen this app's window so DroidWin recreates its display.",
  recovery: { kind: "reopen_window", package_id: "org.fdroid.fdroid" },
};

const runtimeFailed: AttentionProblem = {
  id: "runtime.unhealthy",
  scope: "runtime",
  severity: "blocking",
  confidence: "confirmed",
  title: "The runtime answered its probe with a failure",
  observed: "adb did not answer.",
  evidence: [{ source: "probe", detail: "adb devices: cannot connect to daemon" }],
  recommendation:
    "Restart the runtime. This is the last resort: it closes every app window.",
  recovery: { kind: "restart_runtime" },
};

function report(problems: AttentionProblem[]): AttentionReport {
  const confirmed = problems.filter((item) => item.confidence === "confirmed").length;
  return {
    checked_at_ms: Date.now(),
    deep: true,
    problems,
    confirmed,
    suspected: problems.length - confirmed,
    checked: ["environment: SDK C:/Android/Sdk, adb found, emulator found, scrcpy found, 1 AVD(s)"],
  };
}

describe("Attention Needed", () => {
  it("separates confirmed problems from suspicions and shows the evidence", () => {
    render(<RuntimeScreen {...props({ attention: report([blank, capture]) })} />);

    expect(screen.getByRole("region", { name: "Confirmed" })).toBeTruthy();
    expect(screen.getByRole("region", { name: "Suspected" })).toBeTruthy();
    expect(screen.getByText("org.example.game is drawing a blank window")).toBeTruthy();
    expect(screen.getByText(/every pixel is #000000/)).toBeTruthy();
    expect(screen.getByText(/could not confirm/i)).toBeTruthy();

    fireEvent.click(screen.getByText(/2 pieces of evidence/));
    expect(screen.getByText(/first capture 3763 bytes/)).toBeTruthy();
    expect(screen.getByText(/live process and the window is open/)).toBeTruthy();
  });

  it("offers targeted recovery without restarting anything else", () => {
    const onRecover = vi.fn();
    render(<RuntimeScreen {...props({ attention: report([blank]), onRecover })} />);

    fireEvent.click(screen.getByRole("button", { name: "Retry launch" }));
    expect(onRecover).toHaveBeenCalledWith({
      kind: "retry_launch",
      package_id: "org.example.game",
    });

    fireEvent.click(screen.getByRole("button", { name: "Reinstall package" }));
    expect(onRecover).toHaveBeenCalledWith({
      kind: "reinstall_package",
      package_id: "org.example.game",
    });
    expect(screen.queryByRole("button", { name: /Restart the runtime/ })).toBeNull();
  });

  it("labels a runtime restart as the last resort and nothing less", () => {
    const onRecover = vi.fn();
    render(<RuntimeScreen {...props({ attention: report([runtimeFailed]), onRecover })} />);

    const restart = screen.getByRole("button", {
      name: /Restart the runtime \(last resort — closes every app\)/,
    });
    expect(restart.className).toContain("btn--danger");
    fireEvent.click(restart);
    expect(onRecover).toHaveBeenCalledWith({ kind: "restart_runtime" });
  });

  it("says what the check looked at and reports an honest all-clear", () => {
    render(<RuntimeScreen {...props({ attention: report([]) })} />);

    expect(screen.getByText(/Nothing needs attention/)).toBeTruthy();
    fireEvent.click(screen.getByText(/What this check looked at/));
    expect(screen.getByText(/environment: SDK C:\/Android\/Sdk/)).toBeTruthy();
  });

  it("does not invent a diagnosis before the first check", () => {
    render(<RuntimeScreen {...props({ attention: null })} />);

    expect(screen.getByText(/has not run a check/)).toBeTruthy();
    expect(screen.queryByText(/Nothing needs attention/)).toBeNull();
  });
});
