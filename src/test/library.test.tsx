import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";

import { useAppLibrary } from "../state/apps";
import { statusNotConfigured, statusReady } from "./fixtures";

/**
 * The app library's timing rules are the part of the shell that used to live in
 * event handlers and was easy to break: a read that loses the race with a boot
 * is retried a bounded number of times, and a runtime that becomes ready reads
 * the list once more — but a poll repeating the same state does not.
 */
const backend = vi.hoisted(() => ({
  listApps: vi.fn(),
  onAppsChanged: vi.fn(),
  isDesktop: () => true,
  toDwError: (error: unknown) => ({
    code: "backend_failure",
    summary: error instanceof Error ? error.message : String(error),
    detail: null,
  }),
}));

vi.mock("../backend", () => backend);

beforeEach(() => {
  backend.listApps.mockReset();
  backend.onAppsChanged.mockReset();
  backend.onAppsChanged.mockResolvedValue(() => {});
});

afterEach(() => {
  vi.useRealTimers();
});

describe("app library", () => {
  it("retries a failed read a bounded number of times, then stops", async () => {
    vi.useFakeTimers();
    backend.listApps.mockRejectedValue(new Error("adb: device offline"));

    const { result } = renderHook(() => useAppLibrary({ status: null }));
    await act(async () => {
      await result.current.refresh();
    });
    expect(result.current.apps).toBeNull();
    expect(result.current.appsError?.summary).toBe("adb: device offline");

    // One retry every five seconds, five of them: a real failure stops instead
    // of polling forever, and nothing is left scheduled.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5_000);
    });
    expect(backend.listApps).toHaveBeenCalledTimes(6);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(backend.listApps).toHaveBeenCalledTimes(6);
  });

  it("reads again when the runtime becomes ready, not on every repeated poll", async () => {
    backend.listApps.mockResolvedValue([]);
    const { rerender } = renderHook(
      ({ status }: { status: typeof statusReady | typeof statusNotConfigured }) =>
        useAppLibrary({ status }),
      { initialProps: { status: statusNotConfigured } },
    );
    expect(backend.listApps).not.toHaveBeenCalled();

    rerender({ status: statusReady });
    await waitFor(() => expect(backend.listApps).toHaveBeenCalledTimes(1));

    // A poll returns the same state in a new object: no second read.
    rerender({ status: { ...statusReady } });
    await act(async () => {
      await Promise.resolve();
    });
    expect(backend.listApps).toHaveBeenCalledTimes(1);

    // A real transition away and back does read again.
    rerender({ status: statusNotConfigured });
    rerender({ status: { ...statusReady } });
    await waitFor(() => expect(backend.listApps).toHaveBeenCalledTimes(2));
  });
});
