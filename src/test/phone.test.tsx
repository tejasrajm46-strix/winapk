import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

import { Phone, devicePoint, paintedBox } from "../screens/Phone";
import type { PhoneScreen } from "../backend";
import { appLegacyTarget, appRunning, props, statusNotConfigured } from "./fixtures";

const shot: PhoneScreen = {
  display_id: 0,
  width: 1080,
  height: 2400,
  image: "data:image/png;base64,AAAA",
  captured_at: 1_700_000_000_000,
};

const backend = vi.hoisted(() => ({
  getPhoneScreen: vi.fn(),
  getPhoneDimensions: vi.fn(),
  startPhoneStream: vi.fn(),
  stopPhoneStream: vi.fn(),
  getHardware: vi.fn().mockResolvedValue(null),
  phoneTap: vi.fn(),
  phoneSwipe: vi.fn(),
  phoneText: vi.fn(),
  phoneKey: vi.fn(),
  phoneScreenshot: vi.fn(),
  startRecording: vi.fn(),
  stopRecording: vi.fn(),
  getRecordingStatus: vi.fn(),
  revealPath: vi.fn(),
  isDesktop: () => true,
}));

vi.mock("../backend", () => ({
  ...backend,
  toDwError: (error: unknown) => error,
}));

const IMAGE_ASPECT_RECT = { left: 0, top: 0, right: 270, bottom: 600, width: 270, height: 600 };

/** jsdom reports a zero-sized rect for everything, so the frame is given one. */
function frameOf(rect = IMAGE_ASPECT_RECT): HTMLElement {
  const image = screen.getByAltText(/Android phone's screen/i);
  const frame = image.parentElement as HTMLElement;
  frame.getBoundingClientRect = () => ({ ...rect, x: rect.left, y: rect.top }) as DOMRect;
  return frame;
}

async function phoneShown() {
  await waitFor(() => expect(screen.getByAltText(/Android phone's screen/i)).toBeTruthy());
}

const canvasStubs: { mockRestore: () => void }[] = [];

/**
 * jsdom has no WebCodecs. This is just enough of a decoder for the live path to
 * run: it reports one frame per access unit, at the phone's size.
 */
function stubWebCodecs() {
  class FakeDecoder {
    state = "unconfigured";
    decodeQueueSize = 0;
    constructor(private readonly init: { output: (frame: unknown) => void }) {}
    configure() {
      this.state = "configured";
    }
    decode() {
      this.init.output({ displayWidth: 1080, displayHeight: 2400, close() {} });
    }
    close() {
      this.state = "closed";
    }
  }
  // jsdom's canvas has no 2d context and logs "not implemented" for every frame;
  // the decoder draws through `?.drawImage`, so `null` is the honest stub here.
  canvasStubs.push(vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null));
  vi.stubGlobal("VideoDecoder", FakeDecoder);
  vi.stubGlobal(
    "EncodedVideoChunk",
    class {
      constructor(readonly init: unknown) {}
    },
  );
}

/**
 * A real pointer press. jsdom has no `PointerEvent` constructor, so the native
 * event is dispatched as a mouse event carrying the pointer coordinates — which
 * is exactly what React's pointer listener reads, and what a mouse produces in
 * the app's own Chromium webview.
 */
function press(element: Element, type: "pointerdown" | "pointerup", x: number, y: number) {
  act(() => {
    element.dispatchEvent(new MouseEvent(type, { bubbles: true, clientX: x, clientY: y }));
  });
}

beforeEach(() => {
  // The spies are shared by every case, so a call from one must not look like a
  // call from the next.
  vi.clearAllMocks();
  backend.getPhoneScreen.mockResolvedValue(shot);
  backend.getPhoneDimensions.mockResolvedValue([1080, 2400]);
  backend.startPhoneStream.mockResolvedValue(undefined);
  backend.stopPhoneStream.mockResolvedValue(undefined);
  backend.phoneTap.mockResolvedValue(undefined);
  backend.phoneSwipe.mockResolvedValue(undefined);
  backend.phoneText.mockResolvedValue(undefined);
  backend.phoneKey.mockResolvedValue(undefined);
  backend.phoneScreenshot.mockResolvedValue({
    path: "C:\\Users\\tejas\\Pictures\\WINAPK\\droidwin-20261028-120000-000.png",
    width: 1080,
    height: 2400,
    bytes: 204_800,
    captured_at: 1_700_000_000_000,
  });
  backend.startRecording.mockResolvedValue({
    active: true,
    started_at: Date.now(),
    elapsed_ms: 0,
    directory: "C:\\Users\\tejas\\Videos\\WINAPK",
  });
  backend.stopRecording.mockResolvedValue({
    path: "C:\\Users\\tejas\\Videos\\WINAPK\\droidwin-20261028-120100-000-recording.mp4",
    bytes: 5_242_880,
    started_at: 1_700_000_000_000,
    finished_at: 1_700_000_012_000,
    duration_ms: 12_000,
  });
  backend.getRecordingStatus.mockResolvedValue({
    active: false,
    started_at: null,
    elapsed_ms: 0,
    directory: null,
  });
  backend.revealPath.mockResolvedValue(undefined);
});

afterEach(() => {
  for (const stub of canvasStubs.splice(0)) stub.mockRestore();
  vi.unstubAllGlobals();
});

describe("paintedBox", () => {
  it("leaves a frame that already has the capture's aspect alone", () => {
    const rect = { left: 10, top: 20, width: 270, height: 600 };
    expect(paintedBox(rect, shot)).toEqual(rect);
  });

  it("finds the letterboxed image when the frame has been clamped", () => {
    // 300x600 for a 1080x2400 capture: the image is drawn 270 wide, centred.
    const box = paintedBox({ left: 0, top: 0, width: 300, height: 600 }, shot);
    expect(box.width).toBeCloseTo(270);
    expect(box.height).toBeCloseTo(600);
    expect(box.left).toBeCloseTo(15);
    expect(box.top).toBeCloseTo(0);
  });

  it("does not divide by a frame that has not been laid out yet", () => {
    const rect = { left: 0, top: 0, width: 0, height: 0 };
    expect(paintedBox(rect, shot)).toEqual(rect);
  });
});

describe("devicePoint", () => {
  it("converts a click through the box's scale, not the frame's pixels", () => {
    // A 1080x2400 capture drawn in a 270x600 box is a quarter size.
    const box = { left: 0, top: 0, width: 270, height: 600 };
    expect(devicePoint(box, shot, { x: 135, y: 300 })).toEqual({ x: 540, y: 1200 });
    expect(devicePoint(box, shot, { x: 0, y: 0 })).toEqual({ x: 0, y: 0 });
  });

  it("accounts for where the image sits on the page", () => {
    const box = { left: 100, top: 40, width: 270, height: 600 };
    expect(devicePoint(box, shot, { x: 100, y: 40 })).toEqual({ x: 0, y: 0 });
  });

  it("clamps a click on the box's edge to the last device pixel", () => {
    const box = { left: 0, top: 0, width: 270, height: 600 };
    expect(devicePoint(box, shot, { x: 271, y: 601 })).toEqual({ x: 1079, y: 2399 });
    expect(devicePoint(box, shot, { x: -20, y: -20 })).toEqual({ x: 0, y: 0 });
  });

  it("does not divide by a box that has not been laid out yet", () => {
    expect(devicePoint({ left: 0, top: 0, width: 0, height: 0 }, shot, { x: 5, y: 5 })).toEqual({
      x: 5,
      y: 5,
    });
  });
});

describe("virtual phone", () => {
  it("shows no phone until the runtime is ready", () => {
    render(<Phone {...props({ status: statusNotConfigured, apps: [] })} />);

    expect(screen.getByText(/phone appears once the runtime is ready/i)).toBeTruthy();
    expect(screen.queryByAltText(/Android phone's screen/i)).toBeNull();
  });

  it("shows the capture Android produced and says when it was taken", async () => {
    render(<Phone {...props()} />);

    const image = await screen.findByAltText(/Android phone's screen/i);
    expect(image.getAttribute("src")).toBe(shot.image);
    expect(screen.getByText(/1080 × 2400/)).toBeTruthy();
  });

  it("taps the device at the point that was clicked", async () => {
    const onLaunch = vi.fn();
    render(<Phone {...props({ onLaunch })} />);

    await phoneShown();
    const frame = frameOf();

    press(frame, "pointerdown", 135, 300);
    press(frame, "pointerup", 135, 300);

    await waitFor(() => expect(backend.phoneTap).toHaveBeenCalledWith(540, 1200));
    expect(backend.phoneSwipe).not.toHaveBeenCalled();
    expect(onLaunch).not.toHaveBeenCalled();
  });

  it("maps a click through the letterboxed image, not the clamped frame", async () => {
    render(<Phone {...props()} />);

    await phoneShown();
    const frame = frameOf({ left: 0, top: 0, right: 300, bottom: 600, width: 300, height: 600 });

    // 15px of margin each side: the image starts at x=15, so its left edge is
    // device x=0 and its centre is device x=540.
    press(frame, "pointerdown", 15, 300);
    press(frame, "pointerup", 15, 300);
    await waitFor(() => expect(backend.phoneTap).toHaveBeenCalledWith(0, 1200));

    backend.phoneTap.mockClear();
    press(frame, "pointerdown", 150, 300);
    press(frame, "pointerup", 150, 300);
    await waitFor(() => expect(backend.phoneTap).toHaveBeenCalledWith(540, 1200));

    // The margin is not the phone.
    backend.phoneTap.mockClear();
    press(frame, "pointerdown", 4, 300);
    press(frame, "pointerup", 4, 300);
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(backend.phoneTap).not.toHaveBeenCalled();
  });

  it("sends a drag as a swipe rather than a tap", async () => {
    render(<Phone {...props()} />);

    await phoneShown();
    const frame = frameOf();

    press(frame, "pointerdown", 135, 300);
    press(frame, "pointerup", 135, 450);

    await waitFor(() =>
      expect(backend.phoneSwipe).toHaveBeenCalledWith(540, 1200, 540, 1800, expect.any(Number)),
    );
    expect(backend.phoneTap).not.toHaveBeenCalled();
  });

  it("types into the phone in one call per burst, and sends named keys", async () => {
    render(<Phone {...props()} />);
    await phoneShown();
    const frame = frameOf();

    fireEvent.keyDown(frame, { key: "f" });
    fireEvent.keyDown(frame, { key: "d" });
    fireEvent.keyDown(frame, { key: "r" });
    fireEvent.keyDown(frame, { key: "o" });
    fireEvent.keyDown(frame, { key: "i" });
    fireEvent.keyDown(frame, { key: "d" });

    // A word is one adb call, not six.
    await waitFor(() => expect(backend.phoneText).toHaveBeenCalledWith("fdroid"));
    expect(backend.phoneText).toHaveBeenCalledTimes(1);

    fireEvent.keyDown(frame, { key: "Enter" });
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("ENTER"));

    fireEvent.keyDown(frame, { key: "Backspace" });
    fireEvent.keyDown(frame, { key: "ArrowDown" });
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("DEL"));
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("DPAD_DOWN"));

    // On a phone the escape key is Back.
    fireEvent.keyDown(frame, { key: "Escape" });
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("BACK"));
    fireEvent.keyDown(frame, { key: "Home" });
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("HOME"));
  });

  it("sends typed text before the key that acts on it", async () => {
    render(<Phone {...props()} />);
    await phoneShown();
    const frame = frameOf();

    // The phone is slow on purpose: each command is its own adb process, so the
    // word has to finish before the Enter that submits it can start.
    let finishText: () => void = () => {};
    backend.phoneText.mockImplementation(
      () => new Promise<void>((done) => (finishText = done)),
    );

    fireEvent.keyDown(frame, { key: "f" });
    fireEvent.keyDown(frame, { key: "Enter" });

    await waitFor(() => expect(backend.phoneText).toHaveBeenCalledWith("f"));
    await new Promise((resolve) => setTimeout(resolve, 80));
    expect(backend.phoneKey).not.toHaveBeenCalled();

    act(() => finishText());
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("ENTER"));
  });

  it("keeps two quick taps in the order they were made", async () => {
    render(<Phone {...props()} />);
    await phoneShown();
    const frame = frameOf();

    const finished: number[] = [];
    backend.phoneTap.mockImplementation(async (x: number) => {
      // The first tap is the slow one: if commands raced, the second would land
      // first and the order would be the screen's, not the user's.
      if (x === 540) await new Promise((resolve) => setTimeout(resolve, 60));
      finished.push(x);
    });

    press(frame, "pointerdown", 135, 300);
    press(frame, "pointerup", 135, 300);
    press(frame, "pointerdown", 200, 300);
    press(frame, "pointerup", 200, 300);

    await waitFor(() => expect(finished).toEqual([540, 800]));
  });

  it("does not repeat Back when Escape is held down", async () => {
    render(<Phone {...props()} />);
    await phoneShown();
    const frame = frameOf();

    fireEvent.keyDown(frame, { key: "Escape" });
    fireEvent.keyDown(frame, { key: "Escape", repeat: true });
    fireEvent.keyDown(frame, { key: "Escape", repeat: true });

    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("BACK"));
    expect(backend.phoneKey).toHaveBeenCalledTimes(1);
  });

  it("leaves the application's own shortcuts alone", async () => {
    render(<Phone {...props()} />);
    await phoneShown();

    fireEvent.keyDown(frameOf(), { key: "k", ctrlKey: true });

    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(backend.phoneText).not.toHaveBeenCalled();
    expect(backend.phoneKey).not.toHaveBeenCalled();
  });

  it("sends Android's own Back, Home and recent-apps actions", async () => {
    render(<Phone {...props()} />);
    await phoneShown();

    fireEvent.click(screen.getByRole("button", { name: "Android Back" }));
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("BACK"));

    fireEvent.click(screen.getByRole("button", { name: "Android Home" }));
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("HOME"));

    fireEvent.click(screen.getByRole("button", { name: "Android recent apps" }));
    await waitFor(() => expect(backend.phoneKey).toHaveBeenCalledWith("APP_SWITCH"));

    // One click is one key event: the toolbar does not also tap the screen.
    expect(backend.phoneKey).toHaveBeenCalledTimes(3);
    expect(backend.phoneTap).not.toHaveBeenCalled();
  });

  it("does not offer controls before the runtime is ready", () => {
    render(<Phone {...props({ status: statusNotConfigured, apps: [] })} />);

    const disabled = (name: string) =>
      (screen.getByRole("button", { name }) as HTMLButtonElement).disabled;
    expect(disabled("Android Back")).toBe(true);
    expect(disabled("Android Home")).toBe(true);
    expect(disabled("Android recent apps")).toBe(true);
    expect(disabled("Save screenshot")).toBe(true);
  });

  it("saves a screenshot and offers to show the saved file", async () => {
    render(<Phone {...props()} />);
    await phoneShown();

    fireEvent.click(screen.getByRole("button", { name: "Save screenshot" }));

    await waitFor(() => expect(backend.phoneScreenshot).toHaveBeenCalledTimes(1));
    expect(await screen.findByText(/droidwin-20261028-120000-000\.png/)).toBeTruthy();
    expect(screen.getByText(/200 KB/)).toBeTruthy();

    fireEvent.click(screen.getByRole("button", { name: /Open/ }));
    await waitFor(() =>
      expect(backend.revealPath).toHaveBeenCalledWith(
        "C:\\Users\\tejas\\Pictures\\WINAPK\\droidwin-20261028-120000-000.png",
      ),
    );
  });

  it("starts and stops a recording, and reports the saved video", async () => {
    render(<Phone {...props()} />);
    await phoneShown();

    fireEvent.click(screen.getByRole("button", { name: "Start recording" }));
    await waitFor(() => expect(backend.startRecording).toHaveBeenCalledTimes(1));
    // The control now offers to stop, and the page says a recording is running.
    const stop = await screen.findByRole("button", { name: "Stop recording" });
    expect(screen.getByText(/Recording 00:00/)).toBeTruthy();

    fireEvent.click(stop);
    await waitFor(() => expect(backend.stopRecording).toHaveBeenCalledTimes(1));
    expect(
      await screen.findByText(/droidwin-20261028-120100-000-recording\.mp4/),
    ).toBeTruthy();
    expect(screen.getByText(/00:12 · 5\.0 MB/)).toBeTruthy();
  });

  it("keeps the live view when Android's own capture ends", async () => {
    // screenrecord has a three-minute lifetime, so the stream ending is routine.
    // A live view that treated it as fatal spent the rest of the session on still
    // captures, one every 750 ms — the slow rendering this pass was sent to fix.
    stubWebCodecs();
    const streams: ((data: ArrayBuffer) => void)[] = [];
    backend.startPhoneStream.mockImplementation((_id, onData) => {
      streams.push(onData);
      return Promise.resolve();
    });

    render(<Phone {...props()} />);
    await waitFor(() => expect(streams.length).toBe(1));

    // An SPS and one slice: enough for the decoder to produce a frame.
    const bytes = new Uint8Array([
      0, 0, 0, 1, 0x67, 0x42, 0xc0, 0x1f, 0x8c, 0x68, 0, 0, 0, 1, 0x65, 0x88, 0x84, 0, 0,
    ]);
    act(() => streams[0](bytes.buffer as ArrayBuffer));
    await waitFor(() =>
      expect(getComputedStyle(screen.getByLabelText("Live Android phone screen")).display).toBe(
        "block",
      ),
    );

    act(() => streams[0](new ArrayBuffer(0)));

    await waitFor(() => expect(streams.length).toBe(2));
    expect(backend.stopPhoneStream).toHaveBeenCalled();
    expect(screen.queryByText(/still captures instead/i)).toBeNull();
  });

  it("falls back to still captures only when Android sent no video at all", async () => {
    stubWebCodecs();
    let data: ((chunk: ArrayBuffer) => void) | undefined;
    backend.startPhoneStream.mockImplementation((_id, onData) => {
      data = onData;
      return Promise.resolve();
    });

    render(<Phone {...props()} />);
    await waitFor(() => expect(data).toBeTruthy());

    act(() => data!(new ArrayBuffer(0)));

    expect(await screen.findByText(/still captures instead/i)).toBeTruthy();
    expect(backend.startPhoneStream).toHaveBeenCalledTimes(1);
  });

  it("launches an installed app from the launcher, and filters by search", async () => {
    const onLaunch = vi.fn();
    render(<Phone {...props({ apps: [appRunning, appLegacyTarget], onLaunch })} />);

    fireEvent.click(screen.getByRole("button", { name: /F-Droid/ }));
    expect(onLaunch).toHaveBeenCalledWith("org.fdroid.fdroid");

    fireEvent.change(screen.getByLabelText(/Search installed apps/i), {
      target: { value: "evaluator" },
    });
    expect(screen.queryByRole("button", { name: /F-Droid/i })).toBeNull();
    expect(screen.getByRole("button", { name: /Expression Evaluator/i })).toBeTruthy();
  });

  it("reports installation stages while an install is running", () => {
    render(
      <Phone
        {...props({
          opRunning: true,
          stages: [
            {
              op: "op-1",
              package_id: null,
              stage: "installing",
              message: "Installing the package.",
              at_ms: 1_700_000_000_000,
            },
          ],
        })}
      />,
    );

    expect(screen.getByText("Installing package")).toBeTruthy();
    expect(screen.getByText(/stages WINAPK has reported/i)).toBeTruthy();
  });

  it("reports one failure once, however many panels saw it", () => {
    const failure = { code: "backend_unavailable" as const, summary: "The backend is not running." };
    render(<Phone {...props({ error: failure, appsError: failure, apps: null })} />);

    expect(screen.getAllByRole("alert")).toHaveLength(1);
    expect(screen.getByText(/library is unavailable until the backend answers/i)).toBeTruthy();
  });

  it("never claims a ready runtime when the SDK is missing", () => {
    render(<Phone {...props({ status: statusNotConfigured, env: null, apps: [] })} />);

    expect(screen.queryByText(/Runtime ready/)).toBeNull();
    expect(screen.getByText(/Start runtime/)).toBeTruthy();
  });
});
