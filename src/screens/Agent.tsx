import { useEffect, useMemo, useState } from "react";
import * as backend from "../backend";
import type { AgentPermission, AgentSkill, AgentTool } from "../backend";
import { IconAlert, IconCheck, IconClose, IconFolder } from "../components/icons";
import { Button, EmptyState, ErrorNote, Field, Panel, StatusPill } from "../components/ui";
import { PageHead, type ScreenProps } from "./common";

function clock(at: number): string {
  const date = new Date(at);
  return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function size(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}

/** A value an agent needs verbatim, with a copy affordance that never lies. */
function CopyRow({
  label,
  value,
  copied,
  onCopy,
}: {
  label: string;
  value: string;
  copied: boolean;
  onCopy: () => void;
}) {
  return (
    <div className="copy-row">
      <span className="copy-row__label">{label}</span>
      <code className="copy-row__value mono" title={value}>
        {value}
      </code>
      <Button size="sm" variant="ghost" onClick={onCopy} aria-label={`Copy ${label}`}>
        {copied ? "Copied" : "Copy"}
      </Button>
    </div>
  );
}

export function Agent(props: ScreenProps) {
  const { agent, busy } = props;
  const [tools, setTools] = useState<AgentTool[] | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const catalog = await backend.getAgentTools();
        if (active) setTools(catalog);
      } catch {
        if (active) setTools(null);
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  const last = agent?.actions[0];
  // "Controlling" is a claim about recent evidence, not about the endpoint
  // being reachable: it requires an action in the last 20 seconds.
  const controlling = Boolean(
    last && !agent?.paused && Date.now() - last.at_ms < 20_000,
  );

  const groups = useMemo(() => {
    const read: AgentTool[] = [];
    const control: AgentTool[] = [];
    for (const tool of tools ?? []) {
      if (tool.annotations?.readOnlyHint === false) control.push(tool);
      else read.push(tool);
    }
    return { read, control };
  }, [tools]);

  const copy = async (value: string, key: string) => {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(key);
      window.setTimeout(() => setCopied(null), 1600);
    } catch {
      // Clipboard access can be denied by the webview; the value is still on
      // screen and selectable, so nothing is lost.
    }
  };

  const changePermission = (permission: AgentPermission) => {
    props.onAgentPermission(permission);
  };

  const toggleSkill = (skill: AgentSkill) => {
    props.onAgentToggleSkill(skill.id, !skill.enabled);
  };

  return (
    <div className="page">
      <PageHead
        eyebrow="Management"
        title="Agent"
        lede="A local MCP endpoint that lets an AI agent drive the same runtime this window uses. Actions are display-scoped, logged, and can be stopped at any moment."
      >
        <Button variant="ghost" onClick={props.onAgentRefresh} disabled={busy}>
          Refresh
        </Button>
        {agent?.running ? (
          <Button variant="danger" onClick={props.onAgentStop} disabled={busy}>
            <IconClose size={14} />
            Stop endpoint
          </Button>
        ) : (
          <Button variant="primary" onClick={props.onAgentStart} disabled={busy}>
            Start endpoint
          </Button>
        )}
      </PageHead>

      {props.error && <ErrorNote error={props.error} />}

      <div className="grid grid--2">
        <Panel
          title="Endpoint"
          action={
            <StatusPill
              tone={agent?.running ? (controlling ? "accent" : "ok") : "idle"}
              label={
                agent?.running
                  ? controlling
                    ? "Agent acting now"
                    : "Listening"
                  : "Stopped"
              }
            />
          }
        >
          {agent?.running ? (
            <>
              <p className="secondary">
                Only this machine can reach it. Every request must carry the session token, and
                browsers on other origins are refused.
              </p>
              <CopyRow
                label="URL"
                value={agent.endpoint ?? ""}
                copied={copied === "url"}
                onCopy={() => void copy(agent.endpoint ?? "", "url")}
              />
              <CopyRow
                label="stdio command"
                value={agent.stdio_command}
                copied={copied === "stdio"}
                onCopy={() => void copy(agent.stdio_command, "stdio")}
              />
              <dl className="fields">
                <Field label="Tools" value={`${agent.tool_count} MCP tools`} />
                <Field
                  label="Token file"
                  value={<span className="mono">{agent.token_file ?? "—"}</span>}
                />
                <Field label="Failed actions" value={`${agent.error_count} recorded`} />
              </dl>
              <p className="hw-note">
                For a client that launches a process, use the stdio command. For a client that
                connects to a server, use the URL with
                <code className="mono"> Authorization: Bearer &lt;token from the token file&gt;</code>.
              </p>
            </>
          ) : (
            <EmptyState
              title="The agent endpoint is stopped"
              body="Start it to give a local MCP client access. No port is open while it is stopped."
              action={
                <Button variant="primary" onClick={props.onAgentStart} disabled={busy}>
                  Start endpoint
                </Button>
              }
            />
          )}
        </Panel>

        <Panel
          title="Permission"
          note="Read-only lets an agent look without touching the screen."
          action={
            agent?.paused ? (
              <StatusPill tone="warn" label="Paused" />
            ) : (
              <StatusPill tone={agent?.permission === "control" ? "accent" : "ok"} label={
                agent?.permission === "control" ? "Full control" : "Read-only"
              } />
            )
          }
        >
          <label className="hw-field">
            <span className="hw-field__label">Agent access</span>
            <select
              className="hw-select"
              value={agent?.permission ?? "control"}
              onChange={(event) => changePermission(event.target.value as AgentPermission)}
            >
              <option value="control">Full control — open apps, tap, type, install</option>
              <option value="read_only">Read-only — inspect screens and status</option>
            </select>
          </label>

          <div className="row__actions" style={{ marginTop: "var(--space-md)" }}>
            <Button
              variant={agent?.paused ? "primary" : "secondary"}
              onClick={() => props.onAgentPause(!agent?.paused)}
              disabled={busy}
            >
              {agent?.paused ? "Resume automation" : "Pause automation"}
            </Button>
          </div>

          <p className="hw-note">
            Pausing stops waits that are already running and refuses further input. A running app
            window keeps working for you; only the agent is gated.
          </p>
          <div className="note note--warn">
            <div className="note__head">
              <IconAlert size={14} />
              <span>Agents act on what is on screen</span>
            </div>
            <p className="note__body">
              An agent can read visible app content, including personal data, and send taps or
              text to the focused display. DroidWin logs every action but cannot tell which taps
              are consequential; stop the endpoint when you are done.
            </p>
          </div>
        </Panel>
      </div>

      <Panel
        title="Action log"
        note="Newest first. Screenshots and typed text are never stored here."
        action={<span className="muted">{agent?.actions.length ?? 0} recent</span>}
        flush
      >
        {agent && agent.actions.length > 0 ? (
          <ul className="rows" aria-label="Recent agent actions">
            {agent.actions.map((action) => (
              <li className="row" key={action.id}>
                <div className="row__icon" aria-hidden="true">
                  {action.ok ? <IconCheck size={14} /> : <IconClose size={14} />}
                </div>
                <div className="row__main">
                  <div className="row__title mono">{action.tool}</div>
                  <div className="row__meta">
                    <span>{action.summary}</span>
                    {typeof action.display_id === "number" && (
                      <span>display {action.display_id}</span>
                    )}
                    <span>{action.duration_ms} ms</span>
                  </div>
                </div>
                <div className="row__actions">
                  <span className="muted mono">{clock(action.at_ms)}</span>
                  <StatusPill tone={action.ok ? "ok" : "error"} label={action.ok ? "OK" : "Failed"} />
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <div className="panel__body">
            <EmptyState
              title="Nothing recorded yet"
              body="Actions appear here the moment an agent calls a tool. This is the audit trail, not a preview."
            />
          </div>
        )}
      </Panel>

      <div className="grid grid--2">
        <Panel
          title="Skills"
          note="Folders of instructions an agent can read. Nothing in them is executed."
          action={
            <Button size="sm" variant="secondary" onClick={props.onAgentInstallSkill}>
              <IconFolder size={13} />
              Install folder
            </Button>
          }
          flush
        >
          {agent && agent.skills.length > 0 ? (
            <ul className="rows">
              {agent.skills.map((skill) => (
                <li className="row" key={skill.id}>
                  <div className="row__icon" aria-hidden="true">
                    <IconFolder size={14} />
                  </div>
                  <div className="row__main">
                    <div className="row__title">{skill.name}</div>
                    <div className="row__meta">
                      <span className="mono">{skill.id}</span>
                      <span>
                        {skill.files} file{skill.files === 1 ? "" : "s"} · {size(skill.bytes)}
                      </span>
                      {skill.invalid && <StatusPill tone="warn" label={skill.invalid} />}
                    </div>
                    {skill.description && <p className="row__note">{skill.description}</p>}
                  </div>
                  <div className="row__actions">
                    <label className="switch" title="Enable or disable this skill">
                      <input
                        type="checkbox"
                        checked={skill.enabled}
                        onChange={() => toggleSkill(skill)}
                        aria-label={`Enable ${skill.name}`}
                      />
                      <span>{skill.enabled ? "Enabled" : "Disabled"}</span>
                    </label>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => props.onAgentRemoveSkill(skill.id)}
                      aria-label={`Remove ${skill.name}`}
                    >
                      <IconClose size={13} />
                    </Button>
                  </div>
                </li>
              ))}
            </ul>
          ) : (
            <div className="panel__body">
              <EmptyState
                title="No skills installed"
                body="Install a folder that contains a SKILL.md with name and description frontmatter. Files stay in DroidWin's data directory; nothing is downloaded."
              />
            </div>
          )}
        </Panel>

        <Panel
          title="What an agent can call"
          note={
            tools
              ? `${groups.read.length} read-only · ${groups.control.length} that change the screen`
              : "Reading the tool catalog…"
          }
        >
          {tools ? (
            <details className="catalog">
              <summary className="catalog__summary">
                {tools.length} tools, schemas included
              </summary>
              <div className="catalog__group">
                <span className="hw-field__label">Read-only</span>
                <ul className="catalog__list">
                  {groups.read.map((tool) => (
                    <li key={tool.name}>
                      <span className="mono">{tool.name}</span>
                      <span className="muted"> — {tool.description}</span>
                    </li>
                  ))}
                </ul>
              </div>
              <div className="catalog__group">
                <span className="hw-field__label">Changes the Android screen</span>
                <ul className="catalog__list">
                  {groups.control.map((tool) => (
                    <li key={tool.name}>
                      <span className="mono">{tool.name}</span>
                      <span className="muted"> — {tool.description}</span>
                    </li>
                  ))}
                </ul>
              </div>
            </details>
          ) : (
            <p className="muted">The desktop backend is not available in this window.</p>
          )}
          <p className="hw-note">
            Every action names an Android display. An app window is a virtual display, so an
            unscoped tap would land on another app; the tools refuse to guess.
          </p>
        </Panel>
      </div>
    </div>
  );
}
