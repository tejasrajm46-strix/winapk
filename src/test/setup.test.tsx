import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { SetupReport } from "../backend";
import { Setup } from "../screens/Setup";

function report(overrides: Partial<SetupReport> = {}): SetupReport {
  return {
    state: "needs_install",
    sdk_root: "C:\\Users\\player\\AppData\\Roaming\\app.droidwin.desktop\\android-sdk",
    managed: true,
    configured: false,
    components: [
      {
        id: "platform-tools",
        label: "Android platform-tools (adb)",
        present: false,
        detail: "adb talks to the emulator and installs packages.",
        download_mb: 15,
        required: true,
      },
      {
        id: "window-host",
        label: "Window host (scrcpy)",
        present: true,
        detail: "C:\\scrcpy\\scrcpy.exe",
        download_mb: null,
        required: true,
      },
    ],
    installed_at: null,
    error: null,
    disk_free_mb: 40 * 1024,
    download_mb: 15,
    licenses_accepted: false,
    system_image: "system-images;android-36;google_apis;x86_64",
    avd: "DroidWin_API36",
    message: "Missing: Android platform-tools (adb). About 0.0 GB to download.",
    ...overrides,
  };
}

const base = {
  progress: null,
  busy: false,
  error: null,
  onInstall: () => {},
  onCancel: () => {},
  onChangeLocation: () => {},
  onUseManaged: () => {},
  onRefresh: () => {},
  onSkip: () => {},
};

describe("Android runtime setup", () => {
  it("will not install until the licence is accepted, and then asks with consent", () => {
    const onInstall = vi.fn();
    render(<Setup {...base} report={report()} onInstall={onInstall} />);

    const install = screen.getByRole("button", { name: "Install Android runtime" }) as HTMLButtonElement;
    expect(install.disabled).toBe(true);
    expect(install.title).toContain("licence");

    fireEvent.click(screen.getByRole("checkbox"));
    expect(install.disabled).toBe(false);
    fireEvent.click(install);
    expect(onInstall).toHaveBeenCalledWith(true);
  });

  it("shows each component's measured state and price instead of a guess", () => {
    render(<Setup {...base} report={report()} />);
    // getByText throws when the node is absent, which is the assertion.
    screen.getByText("Android platform-tools (adb)");
    screen.getByText("15 MB");
    screen.getByText("C:\\scrcpy\\scrcpy.exe");
    screen.getByText("Installed");
    // The missing component carries the size it will cost, not a placeholder.
    expect(screen.queryByText("0 MB")).toBeNull();
  });

  it("reports a running installation with its real percentage and a cancel action", () => {
    render(
      <Setup
        {...base}
        report={report({ state: "installing" })}
        progress={{
          step: "tools",
          message: "Downloading command-line tools: 80 of 148 MB.",
          percent: 54,
          done: false,
          failed: null,
        }}
      />,
    );
    screen.getByText(/Downloading command-line tools/);
    screen.getByText(/54%/);
    const cancel = screen.getByRole("button", { name: "Cancel installation" }) as HTMLButtonElement;
    expect(cancel.disabled).toBe(false);
    expect(screen.queryByRole("button", { name: "Install Android runtime" })).toBeNull();
  });

  it("does not call the runtime ready before a boot was verified", () => {
    render(
      <Setup
        {...base}
        report={report({
          state: "starting",
          message: "Android is installed. Starting the emulator; WINAPK calls the runtime ready only after adb sees Android finish booting.",
        })}
      />,
    );
    screen.getByText("Starting");
    expect(screen.queryByText("Ready")).toBeNull();
    screen.getByRole("button", { name: "Start Android runtime" });
  });

  it("keeps an indeterminate bar when no total is known", () => {
    const { container } = render(
      <Setup
        {...base}
        report={report({ state: "installing" })}
        progress={{
          step: "packages",
          message: "Installing emulator.",
          percent: null,
          done: false,
          failed: null,
        }}
      />,
    );
    expect(container.querySelector(".setup__bar--indeterminate")).not.toBeNull();
    screen.getByText("Installing emulator.");
  });

  it("offers a way out for someone who only wants to look at the app", () => {
    const onSkip = vi.fn();
    render(<Setup {...base} report={report()} onSkip={onSkip} />);
    fireEvent.click(screen.getByRole("button", { name: "Open WINAPK anyway" }));
    expect(onSkip).toHaveBeenCalled();
  });
});
