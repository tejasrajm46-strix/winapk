import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { AppShell } from "../components/AppShell";
import { Settings } from "../screens/Settings";
import { props } from "./fixtures";

/**
 * Settings talks to the backend directly, so the module is mocked the way the
 * other screen tests mock it. Only the name-related calls matter here; the rest
 * answer enough for the panels that read them on mount to settle.
 */
const backend = vi.hoisted(() => ({
  DEFAULT_APP_NAME: "WINAPK",
  isDesktop: () => true,
  getHardware: vi.fn(),
  getKeyboardMode: vi.fn(),
  getStopOnExit: vi.fn(),
  getSetupState: vi.fn(),
  getPhoneDisplay: vi.fn(),
  getAppVersion: vi.fn(),
  setAppName: vi.fn(),
  resetAppName: vi.fn(),
  applyHardware: vi.fn(),
  applyHardwarePreset: vi.fn(),
  setKeyboardMode: vi.fn(),
  setStopOnExit: vi.fn(),
  setSdkLocation: vi.fn(),
  pickSdkFolder: vi.fn(),
  setPhoneDisplay: vi.fn(),
  resetPhoneDisplay: vi.fn(),
}));

vi.mock("../backend", () => ({
  ...backend,
  toDwError: (error: unknown) =>
    typeof error === "object" && error !== null && "summary" in error
      ? error
      : { code: "backend_failure", summary: String(error) },
}));

const noHardware = { code: "backend_failure", summary: "Hardware report unavailable in this test." };

beforeEach(() => {
  // The hardware probe is irrelevant to naming; a failed read exercises the
  // error path without inventing a report the backend would have to produce.
  backend.getHardware.mockRejectedValue(noHardware);
  backend.getKeyboardMode.mockResolvedValue("auto");
  backend.getStopOnExit.mockResolvedValue(true);
  backend.getSetupState.mockResolvedValue({ sdk_root: "C:/Android", managed: true });
  backend.getPhoneDisplay.mockRejectedValue(noHardware);
  backend.getAppVersion.mockResolvedValue("0.1.0");
  backend.setAppName.mockReset();
  backend.resetAppName.mockReset();
});

function nameField(): HTMLInputElement {
  return screen.getByLabelText("Application display name") as HTMLInputElement;
}

function saveButton(): HTMLButtonElement {
  return screen.getByRole("button", { name: "Save name" }) as HTMLButtonElement;
}

describe("application name", () => {
  it("saves the trimmed name and reports the value the backend returned", async () => {
    const onAppNameChange = vi.fn();
    backend.setAppName.mockResolvedValue("Hill Climb");

    render(<Settings {...props({ appName: "WINAPK", onAppNameChange })} />);

    fireEvent.change(nameField(), { target: { value: "  Hill Climb  " } });
    fireEvent.click(saveButton());

    await waitFor(() => expect(backend.setAppName).toHaveBeenCalledWith("Hill Climb"));
    expect(onAppNameChange).toHaveBeenCalledWith("Hill Climb");
    // The panel says what happened, using the saved value rather than the draft.
    expect(await screen.findByText(/now Hill Climb/i)).toBeTruthy();
    // The backend's answer becomes the field, so trailing spaces do not linger.
    expect(nameField().value).toBe("Hill Climb");
  });

  it("refuses to save an empty name instead of storing one", async () => {
    render(<Settings {...props({ appName: "WINAPK" })} />);

    fireEvent.change(nameField(), { target: { value: "   " } });

    expect(saveButton().disabled).toBe(true);
    expect(backend.setAppName).not.toHaveBeenCalled();
  });

  it("does not offer to save the name that is already in use", () => {
    render(<Settings {...props({ appName: "WINAPK" })} />);
    expect(nameField().value).toBe("WINAPK");
    expect(saveButton().disabled).toBe(true);
  });

  it("shows the backend's refusal rather than a success when the name is rejected", async () => {
    const onAppNameChange = vi.fn();
    backend.setAppName.mockRejectedValue({
      code: "invalid_config",
      summary: "The application name is limited to 40 characters.",
    });

    render(<Settings {...props({ appName: "WINAPK", onAppNameChange })} />);

    fireEvent.change(nameField(), { target: { value: "x".repeat(40) } });
    fireEvent.click(saveButton());

    expect(await screen.findByText(/limited to 40 characters/i)).toBeTruthy();
    expect(onAppNameChange).not.toHaveBeenCalled();
    expect(screen.queryByText(/now x/i)).toBeNull();
  });

  it("returns to the shipped name on reset", async () => {
    const onAppNameChange = vi.fn();
    backend.resetAppName.mockResolvedValue("WINAPK");

    render(<Settings {...props({ appName: "Hill Climb", onAppNameChange })} />);

    fireEvent.click(screen.getByRole("button", { name: /reset to winapk/i }));

    await waitFor(() => expect(backend.resetAppName).toHaveBeenCalledTimes(1));
    expect(onAppNameChange).toHaveBeenCalledWith("WINAPK");
    expect(await screen.findByText(/back to WINAPK/i)).toBeTruthy();
  });
});

describe("shell branding", () => {
  it("renders the name it is given in the wordmark instead of a hardcoded one", () => {
    render(
      <AppShell
        screen="overview"
        onNavigate={() => {}}
        status={null}
        env={null}
        apps={null}
        agent={null}
        appName="Hill Climb"
        searchValue=""
        onSearch={() => {}}
      >
        <div />
      </AppShell>,
    );

    expect(screen.getByText("Hill Climb")).toBeTruthy();
    expect(screen.queryByText("WINAPK")).toBeNull();
  });

  it("keeps every navigation destination reachable by name in the narrow rail", () => {
    render(
      <AppShell
        screen="overview"
        onNavigate={() => {}}
        status={null}
        env={null}
        apps={null}
        agent={null}
        appName="WINAPK"
        searchValue=""
        onSearch={() => {}}
      >
        <div />
      </AppShell>,
    );

    // The label is hidden by CSS below 1040px, so the accessible name is what
    // has to survive.
    for (const label of ["Phone", "Overview", "Applications", "Installer", "Runtime", "Agent", "Activity", "Settings"]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
  });
});
