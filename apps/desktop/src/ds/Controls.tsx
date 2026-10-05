// src/ds/Controls.tsx — banner, fields, checkbox/radio, pop-up button, menu, segmented control.
//
// Controlled when `value` is given, otherwise the component keeps its own state
// (`value ?? inner`, no syncing effect).

import {
  useId,
  useState,
  type ChangeEventHandler,
  type CSSProperties,
  type InputHTMLAttributes,
  type ReactNode,
} from "react";
import { Button, type ButtonProps } from "./Button";
import { Icon, type IconName } from "./Icon";
import { cx } from "./util";

export interface BannerAction {
  label: string;
  onClick?: () => void;
  variant?: ButtonProps["variant"];
  disabled?: boolean;
  /** Shown as the tooltip, e.g. why the action is disabled. */
  title?: string;
}

const BANNER_ICON: Record<BannerTone, IconName> = {
  info: "info",
  note: "info",
  warn: "warning",
  error: "error",
  ok: "check",
};

export type BannerTone = "info" | "note" | "warn" | "error" | "ok";

export function Banner({
  tone = "info",
  title,
  children,
  actions,
  onDismiss,
  dismissLabel = "Dismiss",
  className,
  style,
}: {
  /** note = the blue "applies next time" style. */
  tone?: BannerTone;
  title?: ReactNode;
  children?: ReactNode;
  actions?: BannerAction[];
  onDismiss?: () => void;
  dismissLabel?: string;
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <div
      className={cx("tn-banner", `tn-banner-${tone}`, className)}
      role={tone === "error" || tone === "warn" ? "alert" : "status"}
      style={style}
    >
      <Icon name={BANNER_ICON[tone]} />
      <div className="tn-banner-body">
        <div>
          {title && <div className="tn-banner-title">{title}</div>}
          {children}
        </div>
        {actions && actions.length > 0 && (
          <div className="tn-banner-actions">
            {actions.map((a) => (
              <Button
                key={a.label}
                size="sm"
                variant={a.variant ?? "secondary"}
                onClick={a.onClick}
                disabled={a.disabled}
                title={a.title}
              >
                {a.label}
              </Button>
            ))}
          </div>
        )}
      </div>
      {onDismiss && (
        <Button variant="plain" size="sm" onClick={onDismiss}>
          {dismissLabel}
        </Button>
      )}
    </div>
  );
}

export interface TextFieldProps extends Omit<
  InputHTMLAttributes<HTMLInputElement>,
  "children"
> {
  label?: ReactNode;
  mono?: boolean;
  /** true for the red outline only; a string also prints the message. */
  error?: boolean | string;
  help?: ReactNode;
  /** Buttons placed to the right of the input. */
  children?: ReactNode;
}

export function TextField({
  id,
  label,
  mono,
  error,
  help,
  children,
  className,
  style,
  ...rest
}: TextFieldProps) {
  const auto = useId();
  const fid = id ?? auto;
  const input = (
    <input
      {...rest}
      id={fid}
      className={cx("tn-input", mono && "is-mono", error && "is-error")}
      aria-invalid={error ? true : undefined}
    />
  );
  return (
    <div className={cx("tn-field", className)} style={style}>
      {label && (
        <label className="tn-field-label" htmlFor={fid}>
          {label}
        </label>
      )}
      {children ? (
        <div className="tn-field-row">
          {input}
          {children}
        </div>
      ) : (
        input
      )}
      {typeof error === "string" && <span className="tn-error">{error}</span>}
      {help && <span className="tn-help">{help}</span>}
    </div>
  );
}

export function Checkbox({
  label,
  checked,
  defaultChecked,
  disabled,
  onChange,
  title,
  indeterminate = false,
  "aria-label": ariaLabel,
}: {
  label?: ReactNode;
  checked?: boolean;
  defaultChecked?: boolean;
  disabled?: boolean;
  onChange?: ChangeEventHandler<HTMLInputElement>;
  title?: string;
  /** Some, not all, of what this box stands for is selected. */
  indeterminate?: boolean;
  "aria-label"?: string;
}) {
  return (
    <label className={cx("tn-check", disabled && "is-disabled")} title={title}>
      <input
        type="checkbox"
        ref={(el) => {
          if (el) el.indeterminate = indeterminate;
        }}
        aria-checked={indeterminate ? "mixed" : undefined}
        checked={checked}
        defaultChecked={defaultChecked}
        disabled={disabled}
        onChange={onChange}
        aria-label={label ? undefined : ariaLabel}
      />
      {label && <span>{label}</span>}
    </label>
  );
}

export function Radio({
  label,
  detail,
  note,
  name,
  checked,
  defaultChecked,
  disabled,
  row,
  onChange,
}: {
  label: ReactNode;
  detail?: ReactNode;
  /** Right-aligned reason, e.g. "Not a USB reader". */
  note?: ReactNode;
  name?: string;
  checked?: boolean;
  defaultChecked?: boolean;
  disabled?: boolean;
  /** List-row layout with divider and selected fill (SD card list). */
  row?: boolean;
  onChange?: ChangeEventHandler<HTMLInputElement>;
}) {
  return (
    <label
      className={cx(
        "tn-check",
        row && "tn-choice",
        row && checked && "is-selected",
        disabled && "is-disabled",
      )}
    >
      <input
        type="radio"
        name={name}
        checked={checked}
        defaultChecked={defaultChecked}
        disabled={disabled}
        onChange={onChange}
      />
      <span className="tn-check-body">
        <span style={{ fontWeight: detail ? 500 : 400 }}>{label}</span>
        {detail && <span className="tn-check-detail">{detail}</span>}
      </span>
      {note && <span className="tn-check-note">{note}</span>}
    </label>
  );
}

export function PopupButton({
  children,
  tone = "default",
  open,
  marked,
  disabled,
  onClick,
  style,
  title,
}: {
  children?: ReactNode;
  tone?: "default" | "warn";
  open?: boolean;
  /** Blue dot: a manual override. */
  marked?: boolean;
  disabled?: boolean;
  onClick?: () => void;
  style?: CSSProperties;
  title?: string;
}) {
  return (
    <button
      type="button"
      className={cx(
        "tn-popup",
        tone === "warn" && "tn-popup-warn",
        open && "is-open",
      )}
      onClick={onClick}
      disabled={disabled}
      aria-haspopup="listbox"
      aria-expanded={open === true}
      style={style}
      title={title}
    >
      <span>
        {children}
        {marked && <span className="tn-popup-mark"> •</span>}
      </span>
      <Icon name="chevron" />
    </button>
  );
}

export type MenuItem =
  | { type: "header"; label: string }
  | { type: "separator" }
  | {
      type?: "item";
      label: string;
      value?: string;
      checked?: boolean;
      disabled?: boolean;
      note?: string;
      link?: boolean;
      active?: boolean;
      onClick?: () => void;
    };

export function Menu({
  items,
  onSelect,
  width,
  style,
  "aria-label": ariaLabel,
}: {
  items: MenuItem[];
  onSelect?: (value: string) => void;
  width?: number;
  style?: CSSProperties;
  "aria-label"?: string;
}) {
  return (
    <div
      className="tn-menu"
      role="listbox"
      aria-label={ariaLabel}
      style={{ width, ...style }}
    >
      {items.map((it, i) => {
        const key = String(i);
        if (it.type === "header")
          return (
            <div key={key} className="tn-menu-head">
              {it.label}
            </div>
          );
        if (it.type === "separator")
          return <div key={key} className="tn-menu-sep" role="separator" />;
        return (
          <button
            key={key}
            type="button"
            role="option"
            aria-selected={it.checked === true}
            disabled={it.disabled}
            className={cx(
              "tn-menu-item",
              it.active && "is-active",
              it.link && "is-link",
            )}
            onClick={() => {
              it.onClick?.();
              onSelect?.(it.value ?? it.label);
            }}
          >
            <span className="tn-menu-check">
              {it.checked && <Icon name="check" size={12} />}
            </span>
            {it.label}
            {it.note && <span className="tn-menu-note">{it.note}</span>}
          </button>
        );
      })}
    </div>
  );
}

export interface SegOption<V extends string> {
  label: string;
  value: V;
}

export function SegmentedControl<V extends string>({
  options,
  value,
  onChange,
  "aria-label": ariaLabel,
}: {
  options: SegOption<V>[];
  value?: V;
  onChange?: (value: V) => void;
  "aria-label"?: string;
}) {
  const [inner, setInner] = useState<V | undefined>(options[0]?.value);
  const current = value ?? inner;
  return (
    <div className="tn-seg" role="tablist" aria-label={ariaLabel}>
      {options.map((o) => {
        const on = o.value === current;
        return (
          <button
            key={o.value}
            type="button"
            role="tab"
            aria-selected={on}
            className={on ? "is-on" : ""}
            onClick={() => {
              setInner(o.value);
              onChange?.(o.value);
            }}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}
