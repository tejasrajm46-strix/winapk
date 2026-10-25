/**
 * Presentation primitives. These hold no state and know nothing about the
 * runtime — they render what they are given, including the absence of a value.
 */

import type { ButtonHTMLAttributes, ReactNode } from "react";
import type { DwError } from "../backend";
import { IconAlert, IconCheck, IconClose } from "./icons";

export type Tone = "ok" | "warn" | "error" | "idle" | "accent";

export function Panel({
  title,
  note,
  action,
  flush = false,
  quiet = false,
  children,
  className,
}: {
  title?: string;
  note?: ReactNode;
  action?: ReactNode;
  flush?: boolean;
  quiet?: boolean;
  children: ReactNode;
  className?: string;
}) {
  const classes = ["panel", flush ? "panel--flush" : "", quiet ? "panel--quiet" : "", className ?? ""]
    .filter(Boolean)
    .join(" ");
  return (
    <section className={classes}>
      {(title || action) && (
        <div className="panel__head">
          <div>
            {title && <h2 className="panel__title">{title}</h2>}
            {note && <p className="panel__note">{note}</p>}
          </div>
          {action}
        </div>
      )}
      {flush ? children : <div className="panel__body">{children}</div>}
    </section>
  );
}

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "md" | "sm";
  /** Why the control cannot be used. Rendered as the tooltip and announced. */
  disabledReason?: string;
};

export function Button({
  variant = "secondary",
  size = "md",
  disabledReason,
  className,
  children,
  ...rest
}: ButtonProps) {
  const classes = ["btn", `btn--${variant}`, size === "sm" ? "btn--sm" : "", className ?? ""]
    .filter(Boolean)
    .join(" ");
  return (
    <button
      type="button"
      {...rest}
      className={classes}
      disabled={rest.disabled}
      title={disabledReason ?? rest.title}
    >
      {children}
    </button>
  );
}

export function StatusPill({
  tone,
  label,
  icon,
}: {
  tone: Tone;
  label: string;
  icon?: ReactNode;
}) {
  return (
    <span className={`pill pill--${tone}`}>
      {icon}
      {label}
    </span>
  );
}

export function Field({
  label,
  value,
  sub,
  missing,
}: {
  label: string;
  value: ReactNode;
  sub?: ReactNode;
  missing?: boolean;
}) {
  return (
    <div className="field">
      <dt className="field__key">{label}</dt>
      <dd className={`field__value${missing ? " field__value--missing" : ""}`}>
        {value}
        {sub && <span className="field__sub">{sub}</span>}
      </dd>
    </div>
  );
}

export function EmptyState({
  title,
  body,
  action,
}: {
  title: string;
  body: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <p className="empty__title">{title}</p>
      <p className="empty__body">{body}</p>
      {action && <div className="dropzone__actions">{action}</div>}
    </div>
  );
}

export function ErrorNote({ error }: { error: DwError }) {
  const isUnavailable = error.code === "backend_unavailable";
  return (
    <div className={`note note--${isUnavailable ? "info" : "error"}`} role="alert">
      <div className="note__head">
        {isUnavailable ? <IconAlert size={14} /> : <IconClose size={14} />}
        <span>{error.summary}</span>
        <span className="note__code">{error.code}</span>
      </div>
      {error.detail && (
        <details>
          <summary className="note__body" style={{ cursor: "pointer" }}>
            Technical detail
          </summary>
          <pre className="note__detail">{error.detail}</pre>
        </details>
      )}
    </div>
  );
}

export function InfoNote({ title, children }: { title: string; children?: ReactNode }) {
  return (
    <div className="note note--info">
      <div className="note__head">
        <IconCheck size={14} />
        <span>{title}</span>
      </div>
      {children && <div className="note__body">{children}</div>}
    </div>
  );
}
