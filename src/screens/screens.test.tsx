import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { Applications } from "./Applications";
import { Installer } from "./Installer";
import { Overview } from "./Overview";
import { RuntimeScreen } from "./Runtime";
import { Activity } from "./Activity";
import {
  appWithoutLabel,
  envMissing,
  inspectionOldTarget,
  inspectionPartial,
  props,
  statusNotConfigured,
} from "../test/fixtures";

describe("Overview", () => {
  it("never claims a ready runtime when the SDK is missing", () => {
    render(<Overview {...props({ status: statusNotConfigured, env: envMissing, apps: [] })} />);

    expect(screen.queryByText(/Runtime ready/)).toBeNull();
    expect(screen.getAllByText(/Runtime not configured|Not configured/i).length).toBeGreaterThan(0);
    expect(screen.getByText(/Not verified on this machine/)).toBeTruthy();
  });

  it("disables the start button when the environment is incomplete", () => {
    render(<Overview {...props({ status: statusNotConfigured, env: envMissing, apps: [] })} />);
    const start = screen.getByRole("button", { name: /start runtime/i });
    expect((start as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows the empty state instead of inventing applications", () => {
    render(<Overview {...props({ apps: [] })} />);
    expect(screen.getByText(/No apps yet/)).toBeTruthy();
  });
});

describe("Installer", () => {
  it("marks undetermined manifest values instead of guessing them", () => {
    render(<Installer {...props({ inspection: inspectionPartial })} />);

    expect(screen.getAllByText(/Not declared/).length).toBe(3); // version, min SDK and target SDK
    expect(screen.getByText(/App name — declared as a resource reference/)).toBeTruthy();
    expect(screen.getByText(/Minimum SDK — not declared in the manifest/)).toBeTruthy();
    // The package id is the honest fallback for a missing label.
    expect(screen.getAllByText("com.example.demo").length).toBeGreaterThan(0);
  });

  it("reports signature presence, not signature validity", () => {
    render(<Installer {...props({ inspection: inspectionPartial })} />);
    expect(screen.getByText(/APK signing block present \(v2\+\)/)).toBeTruthy();
    expect(screen.queryByText(/signature verified/i)).toBeNull();
  });

  it("shows only the stages the backend reported", () => {
    render(
      <Installer
        {...props({
          inspection: inspectionPartial,
          opRunning: true,
          stages: [
            {
              op: "op-1",
              package_id: null,
              stage: "received",
              message: "Received the package.",
              at_ms: 1_700_000_000_000,
            },
            {
              op: "op-1",
              package_id: null,
              stage: "validating",
              message: "Validated a 1.0 MB package.",
              at_ms: 1_700_000_000_500,
            },
          ],
        })}
      />,
    );
    expect(screen.getByText("Received the package.")).toBeTruthy();
    expect(screen.getByText("Validated a 1.0 MB package.")).toBeTruthy();
    expect(screen.queryByText(/Installing package/)).toBeNull();
  });
});

describe("Applications", () => {
  it("shows the package id and says the name is unresolved", () => {
    render(<Applications {...props({ apps: [appWithoutLabel] })} />);
    expect(screen.getAllByText("com.example.unlabelled").length).toBeGreaterThan(0);
    expect(screen.getByText(/name unresolved/)).toBeTruthy();
  });

  it("derives running state from the window registry", () => {
    render(<Applications {...props()} />);
    expect(screen.getByText(/Window open/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /^Stop$/i })).toBeTruthy();
  });

  it("explains a stopped runtime with an action instead of an empty list", () => {
    render(
      <Applications
        {...props({
          status: { ...statusNotConfigured, state: "not_running" },
          apps: null,
          appsError: {
            code: "runtime_unavailable",
            summary: "The Android runtime is not running.",
          },
        })}
      />,
    );
    expect(screen.getByText(/The Android runtime is not running/)).toBeTruthy();
    expect(screen.getByRole("button", { name: /start runtime/i })).toBeTruthy();
  });
});

describe("Runtime", () => {
  it("marks only verified capabilities as supported", () => {
    render(<RuntimeScreen {...props()} />);
    // clipboard, file drop and audio are not verified in this build.
    expect(screen.getAllByText(/Not verified/).length).toBe(3);
    expect(screen.getAllByText(/Supported/).length).toBe(7);
  });

  it("surfaces the environment's blocking items", () => {
    render(<RuntimeScreen {...props({ status: statusNotConfigured, env: envMissing })} />);
    expect(screen.getByText(/Missing pieces/)).toBeTruthy();
    expect(screen.getAllByText(/sdkmanager "emulator"/).length).toBeGreaterThan(0);
  });
});

describe("Activity", () => {
  it("shows an empty state rather than a placeholder history", () => {
    render(<Activity {...props({ activity: [] })} />);
    expect(screen.getByText(/Nothing recorded yet/)).toBeTruthy();
  });
});

describe("Installer and old target SDKs", () => {
  it("offers the Android testing flag only when the manifest asks for a refused SDK", () => {
    render(<Installer {...props({ inspection: inspectionOldTarget })} />);
    expect(screen.getByText(/Install anyway/)).toBeTruthy();
    expect(screen.getAllByText(/API 17/).length).toBeGreaterThan(1);
  });

  it("passes the explicit choice through instead of applying it silently", () => {
    const onInstall = vi.fn();
    render(
      <Installer
        {...props({ inspection: inspectionOldTarget, onInstall })}
      />,
    );
    fireEvent.click(screen.getByRole("checkbox"));
    fireEvent.click(screen.getByRole("button", { name: /Install only/i }));
    expect(onInstall).toHaveBeenCalledWith(false, true);
  });

  it("keeps the flag off by default and hides it for modern packages", () => {
    const onInstall = vi.fn();
    render(
      <Installer
        {...props({ inspection: { ...inspectionOldTarget, target_sdk: 36 }, onInstall })}
      />,
    );
    expect(screen.queryByText(/Install anyway/)).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: /Install only/i }));
    expect(onInstall).toHaveBeenCalledWith(false, false);
  });
});
