import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

import { Agent } from "./Agent";
import { agentRunning, agentStopped, props } from "../test/fixtures";

describe("Agent", () => {
  it("shows a stopped endpoint without inventing a URL", () => {
    render(<Agent {...props({ agent: agentStopped })} />);
    expect(screen.getByText(/The agent endpoint is stopped/)).toBeTruthy();
    expect(screen.queryByText(/http:\/\/127\.0\.0\.1/)).toBeNull();
    expect(screen.getAllByRole("button", { name: /start endpoint/i }).length).toBeGreaterThan(0);
  });

  it("shows the endpoint, the token file and the real tool count when running", () => {
    render(<Agent {...props({ agent: agentRunning })} />);
    expect(screen.getByText("http://127.0.0.1:51234/mcp")).toBeTruthy();
    expect(screen.getByText(/--mcp-stdio/)).toBeTruthy();
    expect(screen.getByText("27 MCP tools")).toBeTruthy();
    expect(screen.getByText(/app\.droidwin\.desktop\/agent\/mcp\.json/)).toBeTruthy();
  });

  it("says the agent is acting when the last action is recent", () => {
    render(<Agent {...props({ agent: agentRunning })} />);
    expect(screen.getByText(/Agent acting now/)).toBeTruthy();
  });

  it("does not claim an agent is acting from a stale action", () => {
    const stale = {
      ...agentRunning,
      actions: agentRunning.actions.map((action) => ({
        ...action,
        at_ms: Date.now() - 60_000,
      })),
    };
    render(<Agent {...props({ agent: stale })} />);
    expect(screen.queryByText(/Agent acting now/)).toBeNull();
    expect(screen.getAllByText(/Listening/).length).toBeGreaterThan(0);
  });

  it("reports the audit trail including the failure, and never a screenshot", () => {
    render(<Agent {...props({ agent: agentRunning })} />);
    expect(screen.getByText("tap")).toBeTruthy();
    expect(screen.getByText("Tapped 270,170 on display 2")).toBeTruthy();
    expect(screen.getByText("type_text")).toBeTruthy();
    expect(screen.getByText("Android rejected that input.")).toBeTruthy();
    expect(screen.getAllByText("Failed").length).toBe(1);
  });

  it("passes the chosen permission through instead of changing it locally", () => {
    const onAgentPermission = vi.fn();
    render(<Agent {...props({ agent: agentRunning, onAgentPermission })} />);
    fireEvent.change(screen.getByRole("combobox"), { target: { value: "read_only" } });
    expect(onAgentPermission).toHaveBeenCalledWith("read_only");
  });

  it("offers resume when paused and pause when running", () => {
    const onAgentPause = vi.fn();
    render(<Agent {...props({ agent: { ...agentRunning, paused: true }, onAgentPause })} />);
    expect(screen.getByText("Paused")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /resume automation/i }));
    expect(onAgentPause).toHaveBeenCalledWith(false);
  });

  it("toggles a skill with the opposite of its current state", () => {
    const onAgentToggleSkill = vi.fn();
    render(<Agent {...props({ agent: agentRunning, onAgentToggleSkill })} />);
    fireEvent.click(screen.getByRole("checkbox", { name: /enable ui flow/i }));
    expect(onAgentToggleSkill).toHaveBeenCalledWith("ui-flow", false);
  });

  it("shows an invalid skill with its reason rather than hiding it", () => {
    render(
      <Agent
        {...props({
          agent: {
            ...agentRunning,
            skills: [{ ...agentRunning.skills[0], invalid: "SKILL.md is missing or unreadable." }],
          },
        })}
      />,
    );
    expect(screen.getByText(/SKILL\.md is missing or unreadable/)).toBeTruthy();
  });
});
