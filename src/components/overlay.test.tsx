import { describe, expect, it, vi, beforeEach } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { OverlayWindow, fitBox } from "./OverlayWindows";
import type { InputState, OverlayData } from "../backend";

const data: OverlayData = {
  package_id: "com.example.game",
  app_label: "Example Game",
  enabled: true,
  mappings: [
    { key: 87, label: "W", action: "tap 50% / 80%", x: 50, y: 80 },
    { key: 65, label: "A", action: "tap 20% / 80%", x: 20, y: 80 },
    { key: 32, label: "Space", action: "send KEYCODE_SPACE", x: null, y: null },
  ],
  display_width: 2400,
  display_height: 1080,
  labels_visible: true,
  placing: false,
  pending: null,
};

const inputState: InputState = {
  package_id: "com.example.game",
  enabled: true,
  mappings: [
    { key: 87, label: "W", action: { kind: "touch_tap", x: 50, y: 80 }, enabled: true },
    {
      key: 32,
      label: "Space",
      action: { kind: "key_event", keycode: "KEYCODE_SPACE" },
      enabled: true,
    },
  ],
  screen: { display_id: 1, width: 2400, height: 1080, orientation: "landscape" },
};

const backend = vi.hoisted(() => ({
  overlayCurrent: vi.fn(),
  onOverlayData: vi.fn(),
  onOverlayError: vi.fn(),
  overlayNav: vi.fn(),
  overlayToggleLabels: vi.fn(),
  overlayToggleMapper: vi.fn(),
  overlayOpenControls: vi.fn(),
  overlayBeginPlace: vi.fn(),
  overlayPlaceAt: vi.fn(),
  overlayCancelPlace: vi.fn(),
  overlayClearPending: vi.fn(),
  emitOverlayError: vi.fn(),
  getInputState: vi.fn(),
  universalBindings: vi.fn(),
  setUniversalBinding: vi.fn(),
  saveInputMapping: vi.fn(),
  removeInputMapping: vi.fn(),
  setInputProfileEnabled: vi.fn(),
  resetInputMappings: vi.fn(),
  toDwError: vi.fn((error: unknown) => error),
}));

vi.mock("../backend", () => ({ ...backend }));

/** The push channel the Rust side uses to hand new overlay data to a window. */
let pushes: ((next: OverlayData) => void) | null = null;

beforeEach(() => {
  // Call history must not leak between tests: two of them save a mapping.
  vi.clearAllMocks();
  pushes = null;
  backend.overlayCurrent.mockResolvedValue(data);
  backend.onOverlayData.mockImplementation(async (handler: (next: OverlayData) => void) => {
    pushes = handler;
    return () => {};
  });
  backend.onOverlayError.mockResolvedValue(() => {});
  backend.overlayNav.mockResolvedValue(undefined);
  backend.overlayToggleLabels.mockResolvedValue({
    package_id: data.package_id,
    labels_visible: false,
    mapper_open: false,
    visible: true,
    failure: null,
  });
  backend.overlayToggleMapper.mockResolvedValue({
    package_id: data.package_id,
    labels_visible: true,
    mapper_open: true,
    visible: true,
    failure: null,
  });
  backend.overlayOpenControls.mockResolvedValue(undefined);
  backend.overlayBeginPlace.mockResolvedValue({
    package_id: data.package_id,
    labels_visible: true,
    mapper_open: true,
    visible: true,
    failure: null,
  });
  backend.overlayPlaceAt.mockResolvedValue({
    package_id: data.package_id,
    labels_visible: true,
    mapper_open: true,
    visible: true,
    failure: null,
  });
  backend.overlayClearPending.mockResolvedValue({
    package_id: data.package_id,
    labels_visible: true,
    mapper_open: true,
    visible: true,
    failure: null,
  });
  backend.getInputState.mockResolvedValue(inputState);
  backend.universalBindings.mockResolvedValue([
    { action: "back", label: "Back", chord: "Shift+A", android_key: "KEYCODE_BACK" },
    { action: "home", label: "Home", chord: "Shift+H", android_key: "KEYCODE_HOME" },
  ]);
  backend.setUniversalBinding.mockImplementation((_action: string, chord: string) =>
    Promise.resolve([
      { action: "back", label: "Back", chord, android_key: "KEYCODE_BACK" },
      { action: "home", label: "Home", chord: "Shift+H", android_key: "KEYCODE_HOME" },
    ]),
  );
  backend.saveInputMapping.mockResolvedValue(inputState);
  backend.removeInputMapping.mockResolvedValue(inputState);
  backend.setInputProfileEnabled.mockResolvedValue(inputState);
  backend.resetInputMappings.mockResolvedValue(inputState);
});

describe("mapping overlay geometry", () => {
  it("letterboxes a wide display inside a tall window and centres it", () => {
    const box = fitBox(800, 800, 2400, 1080);
    expect(box.width).toBe(800);
    expect(box.height).toBe(360);
    expect(box.left).toBe(0);
    expect(box.top).toBe(220);
  });

  it("fits a portrait display by height", () => {
    const box = fitBox(1200, 900, 1080, 2400);
    expect(box.height).toBe(900);
    expect(box.width).toBe(405);
    expect(box.left).toBe(398);
  });

  it("falls back to the window when the display size is unknown", () => {
    expect(fitBox(640, 480, 0, 0)).toEqual({ left: 0, top: 0, width: 640, height: 480 });
  });
});

describe("mapping labels", () => {
  it("draws a saved touch mapping at its percentage with its key label", async () => {
    render(<OverlayWindow label="overlay-labels" />);

    const chip = await screen.findByTitle("W: tap 50% / 80%");
    expect(chip.textContent).toBe("W");
    expect((chip as HTMLElement).style.left).toBe("50%");
    expect((chip as HTMLElement).style.top).toBe("80%");
    expect(screen.getByTitle("A: tap 20% / 80%")).toBeTruthy();
    // A key event has no position, so nothing is drawn for it on the game.
    expect(screen.queryByTitle("Space: send KEYCODE_SPACE")).toBeNull();
  });

  it("marks established mappings as off instead of hiding them", async () => {
    backend.overlayCurrent.mockResolvedValue({ ...data, enabled: false });
    render(<OverlayWindow label="overlay-labels" />);

    const chip = await screen.findByTitle("W: tap 50% / 80%");
    expect(chip.className).toContain("labels__chip--off");
  });
});

describe("click to place", () => {
  /** jsdom has no layout, so the video rectangle is stated outright. */
  const rect = {
    left: 0,
    top: 100,
    width: 1000,
    height: 500,
    right: 1000,
    bottom: 600,
    x: 0,
    y: 100,
    toJSON: () => ({}),
  } as DOMRect;

  it("turns a click on the game into a point on the Android display", async () => {
    backend.overlayCurrent.mockResolvedValue({ ...data, placing: true });
    render(<OverlayWindow label="overlay-labels" />);

    const layer = await screen.findByRole("button", { name: "Click the game control to map" });
    layer.getBoundingClientRect = () => rect;
    fireEvent.click(layer, { clientX: 500, clientY: 350 });

    // Halfway across and halfway down the video, not the window: the letterbox
    // bar above the picture must not shift the point.
    await waitFor(() =>
      expect(backend.overlayPlaceAt).toHaveBeenCalledWith(data.package_id, 50, 50),
    );
  });

  it("clamps a click that lands in the letterbox to the display edge", async () => {
    backend.overlayCurrent.mockResolvedValue({ ...data, placing: true });
    render(<OverlayWindow label="overlay-labels" />);

    const layer = await screen.findByRole("button", { name: "Click the game control to map" });
    layer.getBoundingClientRect = () => rect;
    fireEvent.click(layer, { clientX: 1200, clientY: 90 });

    await waitFor(() =>
      expect(backend.overlayPlaceAt).toHaveBeenCalledWith(data.package_id, 100, 0),
    );
  });

  it("captures no clicks at all when no point is being placed", async () => {
    render(<OverlayWindow label="overlay-labels" />);

    await screen.findByTitle("W: tap 50% / 80%");
    expect(screen.queryByRole("button", { name: "Click the game control to map" })).toBeNull();
  });

  it("shows the chosen point on the game while the key is being assigned", async () => {
    backend.overlayCurrent.mockResolvedValue({ ...data, pending: { x: 42, y: 73 } });
    render(<OverlayWindow label="overlay-labels" />);

    const dot = await screen.findByTitle("New point: 42% / 73%");
    expect((dot as HTMLElement).style.left).toBe("42%");
    expect((dot as HTMLElement).style.top).toBe("73%");
  });
});

describe("navigation bar", () => {
  it("sends each Android navigation key for the app it is over", async () => {
    render(<OverlayWindow label="overlay-nav" />);

    fireEvent.click(await screen.findByRole("button", { name: "Back" }));
    await waitFor(() => expect(backend.overlayNav).toHaveBeenCalledWith(data.package_id, "BACK"));

    fireEvent.click(screen.getByRole("button", { name: "Home" }));
    await waitFor(() => expect(backend.overlayNav).toHaveBeenCalledWith(data.package_id, "HOME"));

    fireEvent.click(screen.getByRole("button", { name: "Recent apps" }));
    await waitFor(() =>
      expect(backend.overlayNav).toHaveBeenCalledWith(data.package_id, "APP_SWITCH"),
    );
  });

  it("reports a failed key over the game", async () => {
    render(<OverlayWindow label="overlay-nav" />);

    await screen.findByRole("button", { name: "Back" });
    backend.overlayNav.mockRejectedValueOnce({ code: "launch_failed", summary: "No display yet" });
    fireEvent.click(screen.getByRole("button", { name: "Back" }));
    await waitFor(() =>
      expect(backend.emitOverlayError).toHaveBeenCalledWith("No display yet"),
    );
  });
});

describe("action rail", () => {
  it("toggles only the labels from More", async () => {
    render(<OverlayWindow label="overlay-rail" />);

    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Hide mapping labels" }));
    await waitFor(() => expect(backend.overlayToggleLabels).toHaveBeenCalled());
  });

  it("opens the existing Controls dialog for the same package", async () => {
    render(<OverlayWindow label="overlay-rail" />);

    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Controls" }));
    await waitFor(() =>
      expect(backend.overlayOpenControls).toHaveBeenCalledWith(data.package_id),
    );
  });

  it("opens the key-mapping panel from the same menu", async () => {
    render(<OverlayWindow label="overlay-rail" />);

    fireEvent.click(await screen.findByRole("button", { name: "More" }));
    fireEvent.click(screen.getByRole("menuitem", { name: "Key mapping" }));
    await waitFor(() =>
      expect(backend.overlayToggleMapper).toHaveBeenCalledWith(true, data.package_id),
    );
  });
});

describe("key-mapping panel", () => {
  it("captures a key for a point clicked on the game, then saves it", async () => {
    render(<OverlayWindow label="overlay-mapper" />);

    // Rows come from the same profile the Controls dialog edits.
    expect(await screen.findByText("W")).toBeTruthy();
    expect(screen.getByText("Tap 50% / 80%")).toBeTruthy();
    expect(screen.getByText("Android SPACE")).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: "Place on the game" }));
    await waitFor(() => expect(backend.overlayBeginPlace).toHaveBeenCalledWith(data.package_id));

    // The click the user aimed at the game comes back as this package's point.
    act(() => pushes?.({ ...data, placing: false, pending: { x: 42, y: 73 } }));
    expect(await screen.findByText(/Point 42% \/ 73%/)).toBeTruthy();

    fireEvent.keyDown(screen.getByRole("button", { name: "Press a key to map" }), {
      key: "k",
      keyCode: 75,
    });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(backend.saveInputMapping).toHaveBeenCalledWith(data.package_id, 75, {
        kind: "touch_tap",
        x: 42,
        y: 73,
      }),
    );
  });

  it("will not save a tap before a point has been placed", async () => {
    render(<OverlayWindow label="overlay-mapper" />);

    await screen.findByText("W");
    fireEvent.keyDown(screen.getByRole("button", { name: "Press a key to map" }), {
      key: "k",
      keyCode: 75,
    });
    expect((screen.getByRole("button", { name: "Add" }) as HTMLButtonElement).disabled).toBe(true);
    expect(backend.saveInputMapping).not.toHaveBeenCalled();
  });

  it("rebinds a universal control and shows the chosen chord", async () => {
    render(<OverlayWindow label="overlay-mapper" />);

    const change = await screen.findByRole("button", { name: "Change Back" });
    expect(change.textContent).toBe("Shift+A");
    fireEvent.click(change);
    expect(change.textContent).toBe("Press keys…");

    fireEvent.keyDown(change, { key: "B", shiftKey: true });
    await waitFor(() =>
      expect(backend.setUniversalBinding).toHaveBeenCalledWith("back", "Shift+B"),
    );
    await waitFor(() => expect(change.textContent).toBe("Shift+B"));
  });

  it("ignores a modifier pressed on its own while rebinding", async () => {
    render(<OverlayWindow label="overlay-mapper" />);

    const change = await screen.findByRole("button", { name: "Change Back" });
    fireEvent.click(change);
    fireEvent.keyDown(change, { key: "Shift", shiftKey: true });
    expect(backend.setUniversalBinding).not.toHaveBeenCalled();
  });

  it("saves an Android key without needing a point, and removes one by its label", async () => {
    render(<OverlayWindow label="overlay-mapper" />);

    await screen.findByText("W");
    fireEvent.change(screen.getByLabelText("What the key does"), {
      target: { value: "key_event" },
    });
    fireEvent.keyDown(screen.getByRole("button", { name: "Press a key to map" }), {
      key: "Escape",
      keyCode: 27,
    });
    fireEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(backend.saveInputMapping).toHaveBeenCalledWith(data.package_id, 27, {
        kind: "key_event",
        keycode: "KEYCODE_SPACE",
      }),
    );

    fireEvent.click(screen.getByRole("button", { name: "Remove W" }));
    await waitFor(() => expect(backend.removeInputMapping).toHaveBeenCalledWith(data.package_id, 87));
  });
});
