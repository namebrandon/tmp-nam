// src/ds/Button.tsx — buttons, tags, status dots, spinners and progress bars.

import type { ButtonHTMLAttributes, CSSProperties, ReactNode } from "react";
import { Icon, type IconName } from "./Icon";
import { cx } from "./util";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** secondary is the default. One primary per view. danger only on the button that erases or removes. */
  variant?: "secondary" | "primary" | "danger" | "plain";
  size?: "sm" | "md" | "lg";
  icon?: IconName;
  /** Hide the label; a string child becomes the accessible name and tooltip. */
  iconOnly?: boolean;
  /** Red text on a secondary button ("Remove from Unit…"). */
  destructiveText?: boolean;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  iconOnly,
  destructiveText,
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  const name = iconOnly && typeof children === "string" ? children : undefined;
  return (
    <button
      {...rest}
      type={type === "submit" ? "submit" : "button"}
      aria-label={rest["aria-label"] ?? name}
      title={rest.title ?? name}
      className={cx(
        "tn-btn",
        `tn-btn-${variant}`,
        size !== "md" && `tn-btn-${size}`,
        iconOnly && "tn-btn-icon",
        destructiveText && "tn-btn-danger-text",
        className,
      )}
    >
      {icon && <Icon name={icon} />}
      {!iconOnly && children}
    </button>
  );
}

export type Tone = "neutral" | "arch" | "ok" | "warn" | "danger" | "accent";

export function Tag({
  tone = "neutral",
  children,
  className,
  style,
  title,
}: {
  tone?: Tone;
  children?: ReactNode;
  className?: string;
  style?: CSSProperties;
  /** Tooltip. */
  title?: string;
}) {
  return (
    <span
      title={title}
      className={cx(
        "tn-tag",
        tone !== "neutral" && `tn-tag-${tone}`,
        className,
      )}
      style={style}
    >
      {children}
    </span>
  );
}

export function StatusDot({
  tone = "off",
  label,
}: {
  tone?: "ok" | "warn" | "danger" | "off";
  label?: ReactNode;
}) {
  const dot = <span className={`tn-dot tn-dot-${tone}`} aria-hidden />;
  if (!label) return dot;
  return (
    <span className="tn-status">
      {dot}
      {label}
    </span>
  );
}

export function Spinner({
  size = "sm",
  label = "Working",
}: {
  size?: "sm" | "lg";
  label?: string;
}) {
  return (
    <span
      className={cx("tn-spinner", size === "lg" && "tn-spinner-lg")}
      role="progressbar"
      aria-label={label}
    />
  );
}

export interface ProgressBarProps {
  /** 0–100. Omit for indeterminate. */
  value?: number;
  indeterminate?: boolean;
  tone?: "accent" | "ok" | "danger";
  size?: "md" | "lg";
  label?: ReactNode;
  /** Right-aligned detail, e.g. "143 of 298 KB". */
  detail?: ReactNode;
}

export function ProgressBar({
  value,
  indeterminate,
  tone,
  size,
  label,
  detail,
}: ProgressBarProps) {
  const indet = indeterminate === true || value === undefined;
  const bar = (
    <div
      className={cx(
        "tn-bar",
        size === "lg" && "tn-bar-lg",
        tone && tone !== "accent" && `tn-bar-${tone}`,
        indet && "tn-bar-indet",
      )}
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={indet ? undefined : value}
      aria-label={typeof label === "string" ? label : "Progress"}
    >
      <i
        style={
          indet
            ? undefined
            : { width: `${String(Math.max(0, Math.min(100, value)))}%` }
        }
      />
    </div>
  );
  if (!label && !detail) return bar;
  return (
    <div className="tn-progress">
      <div className="tn-progress-head">
        <span>{label}</span>
        <span className="tn-progress-detail">{detail}</span>
      </div>
      {bar}
    </div>
  );
}
