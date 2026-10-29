import { describe, expect, it, vi, beforeEach } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";

import { ControlsDialog } from "../components/ControlsDialog";
import type { InputState } from "../backend";
import { appRunning } from "./fixtures";

const emptyState: InputState = {
  package_id: appRunning.package_id,
  enabled: false,
  mappings: [],
  screen: { display_id: 5, width: 2400, height: 1080, orientation: "landscape" },
};

const backend = vi.hoisted(() => ({
  getInputState: vi.fn(),
  saveInputMapping: vi.fn(),
  removeInputMapping: vi.fn(),
  setInputMappingEnabled: vi.fn(),
  setInputProfileEnabled: vi.fn(),
  resetInputMappings: vi.fn(),
}));

vi.mock("../backend", () => ({
  ...backend,
  toDwError: (error: unknown) => error,
}));

/** jsdom has no `keyCode` in its KeyboardEvent init, so it is attached directly. */
function pressWithKeyCode(element: Element, key: string, keyCode: number, modifiers: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...modifiers });
  Object.defineProperty(event, "keyCode", { get: () => keyCode });
  fireEvent(element, event);
}

beforeEach(() => {
  backend.getInputState.mockResolvedValue(emptyState);
  backend.saveInputMapping.mockResolvedValue(emptyState);
  backend.setInputProfileEnabled.mockResolvedValue({ ...emptyState, enabled: true });
  backend.removeInputMapping.mockResolvedValue(emptyState);
  backend.setInputMappingEnabled.mockResolvedValue(emptyState);
  backend.resetInputMappings.mockResolvedValue(emptyState);
});

describe("configure controls", () => {
  it("opens switched off and names the display touch points are measured in", async () => {
    render(<ControlsDialog app={appRunning} onClose={() => {}} />);

    expect(await screen.findByRole("button", { name: "Off — turn on" })).toBeTruthy();
    expect(screen.getByText(/2400×1080 \(landscape\)/)).toBeTruthy();
    expect(screen.getByText(/No mappings yet/i)).toBeTruthy();
  });

  it("captures a physical key and saves it as a touch target for this app only", async () => {
    render(<ControlsDialog app={appRunning} onClose={() => {}} />);

    const capture = await screen.findByRole("button", { name: "Press the key to map" });
    capture.focus();
    pressWithKeyCode(capture, "j", 74);
    expect(screen.getByRole("button", { name: "Press the key to map" }).textContent).toBe(
      "j (VK 74)",
    );

    fireEvent.click(screen.getByRole("button", { name: "Add mapping" }));

    await waitFor(() => expect(backend.saveInputMapping).toHaveBeenCalledTimes(1));
    expect(backend.saveInputMapping).toHaveBeenCalledWith(appRunning.package_id, 74, {
      kind: "touch_tap",
      x: 50,
      y: 80,
    });
  });

  it("saves a modifier combination without changing plain-key bindings", async () => {
    backend.saveInputMapping.mockClear();
    render(<ControlsDialog app={appRunning} onClose={() => {}} />);
    const capture = await screen.findByRole("button", { name: "Press the key to map" });
    pressWithKeyCode(capture, "j", 74, { ctrlKey: true, shiftKey: true });
    expect(capture.textContent).toContain("Ctrl+Shift+J");
    fireEvent.click(screen.getByRole("button", { name: "Add mapping" }));
    await waitFor(() => expect(backend.saveInputMapping).toHaveBeenCalledWith(appRunning.package_id, 74 | (5 << 8), {
      kind: "touch_tap", x: 50, y: 80,
    }));
  });

  it("lets Escape be captured as a mapping instead of closing the dialog", async () => {
    const onClose = vi.fn();
    render(<ControlsDialog app={appRunning} onClose={onClose} />);

    const capture = await screen.findByRole("button", { name: "Press the key to map" });
    capture.focus();
    pressWithKeyCode(capture, "Escape", 27);

    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Press the key to map" }).textContent).toBe(
      "Escape (VK 27)",
    );
  });

  it("switches mapping on for that app, and the button reflects the stored state", async () => {
    backend.setInputProfileEnabled.mockResolvedValue({ ...emptyState, enabled: true });
    render(<ControlsDialog app={appRunning} onClose={() => {}} />);

    fireEvent.click(await screen.findByRole("button", { name: "Off — turn on" }));

    await waitFor(() =>
      expect(backend.setInputProfileEnabled).toHaveBeenCalledWith(appRunning.package_id, true),
    );
    expect(await screen.findByRole("button", { name: "On — turn off" })).toBeTruthy();
  });

  it("lists an existing mapping and removes it by key", async () => {
    backend.getInputState.mockResolvedValue({
      ...emptyState,
      enabled: true,
      mappings: [
        {
          key: 74,
          label: "J",
          action: { kind: "touch_tap", x: 50, y: 80 },
          enabled: true,
        },
      ],
    });
    render(<ControlsDialog app={appRunning} onClose={() => {}} />);

    expect(await screen.findByText("J")).toBeTruthy();
    expect(screen.getByText("Tap 50% / 80%")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    await waitFor(() =>
      expect(backend.removeInputMapping).toHaveBeenCalledWith(appRunning.package_id, 74),
    );
  });

  it("resets every mapping for the app when asked", async () => {
    backend.getInputState.mockResolvedValue({
      ...emptyState,
      enabled: true,
      mappings: [
        { key: 32, label: "Space", action: { kind: "key_event", keycode: "KEYCODE_A" }, enabled: true },
      ],
    });
    render(<ControlsDialog app={appRunning} onClose={() => {}} />);

    fireEvent.click(await screen.findByRole("button", { name: "Reset to defaults" }));
    await waitFor(() =>
      expect(backend.resetInputMappings).toHaveBeenCalledWith(appRunning.package_id),
    );
  });
});
