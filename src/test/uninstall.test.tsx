import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { Applications } from "../screens/Applications";
import { AppRow } from "../screens/common";
import { appRunning, props } from "./fixtures";

function openFromRow() {
  fireEvent.click(screen.getByRole("button", { name: "Uninstall F-Droid" }));
}

describe("uninstall data choice", () => {
  it("asks before uninstalling and focuses the choice that keeps data", () => {
    render(<Applications {...props()} />);
    openFromRow();

    expect(screen.getByRole("dialog")).toBeTruthy();
    expect(screen.getByRole("heading", { name: "Uninstall F-Droid?" })).toBeTruthy();
    // The narrative states what will happen, not that anything already happened.
    expect(screen.getByText(/stays in the runtime until you delete it/i)).toBeTruthy();
    expect(screen.getByText(/cannot be undone/i)).toBeTruthy();

    const keep = screen.getByRole("button", { name: "Uninstall and keep app data" });
    expect(document.activeElement).toBe(keep);
  });

  it("keeps data when that is the chosen action, exactly once", () => {
    const onUninstall = vi.fn();
    render(<Applications {...props({ onUninstall })} />);
    openFromRow();

    fireEvent.click(screen.getByRole("button", { name: "Uninstall and keep app data" }));

    expect(onUninstall).toHaveBeenCalledTimes(1);
    expect(onUninstall).toHaveBeenCalledWith("org.fdroid.fdroid", false);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("deletes data only when the destructive action is chosen", () => {
    const onUninstall = vi.fn();
    render(<Applications {...props({ onUninstall })} />);
    openFromRow();

    fireEvent.click(screen.getByRole("button", { name: "Uninstall and delete app data" }));

    expect(onUninstall).toHaveBeenCalledTimes(1);
    expect(onUninstall).toHaveBeenCalledWith("org.fdroid.fdroid", true);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("neither uninstalls nor closes on cancel", () => {
    const onUninstall = vi.fn();
    render(<Applications {...props({ onUninstall })} />);
    openFromRow();

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(onUninstall).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes on Escape without uninstalling", () => {
    const onUninstall = vi.fn();
    render(<Applications {...props({ onUninstall })} />);
    openFromRow();

    fireEvent.keyDown(document, { key: "Escape" });

    expect(onUninstall).not.toHaveBeenCalled();
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes if the package leaves the runtime while the question is open", () => {
    const { rerender } = render(<Applications {...props()} />);
    openFromRow();
    expect(screen.getByRole("dialog")).toBeTruthy();

    rerender(<Applications {...props({ apps: [] })} />);

    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("offers no uninstall control in compact rows", () => {
    render(
      <AppRow
        app={appRunning}
        busy={false}
        compact
        onLaunch={() => {}}
        onStopApp={() => {}}
        onUninstall={() => {}}
      />,
    );

    expect(screen.queryByRole("button", { name: "Uninstall F-Droid" })).toBeNull();
  });
});
