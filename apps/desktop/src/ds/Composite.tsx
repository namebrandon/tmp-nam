// src/ds/Composite.tsx — size picker, stage list, sheet, sidebar, toolbar,
// unit status, activity card and connect steps.

import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import {
  Button,
  ProgressBar,
  Spinner,
  StatusDot,
  type ButtonProps,
} from "./Button";
import { Icon, type IconName } from "./Icon";
import { CONNECT_STEPS, cx } from "./util";

export interface SizeOption {
  label: string;
  note?: string;
}

/** A2 container sizes, smallest → largest, labelled as the container names them. */
export function SizePicker({
  sizes,
  value,
  onChange,
  disabled,
}: {
  sizes: SizeOption[];
  value?: string;
  onChange?: (label: string) => void;
  disabled?: boolean;
}) {
  const [inner, setInner] = useState(sizes[sizes.length - 1]?.label);
  const current = value ?? inner;
  const n = sizes.length;
  return (
    <div
      className={cx("tn-sizepick", disabled && "is-disabled")}
      role="radiogroup"
      aria-label="Size the player loads"
    >
      {sizes.map((s, i) => {
        const on = s.label === current;
        const note =
          s.note ?? (i === 0 ? "smallest" : i === n - 1 ? "largest" : "");
        return (
          <button
            key={s.label}
            type="button"
            role="radio"
            aria-checked={on}
            className={on ? "is-on" : ""}
            disabled={disabled}
            onClick={() => {
              setInner(s.label);
              onChange?.(s.label);
            }}
          >
            {s.label}
            {note && <small>{note}</small>}
          </button>
        );
      })}
    </div>
  );
}

export interface Stage {
  label: string;
  state?: "done" | "now" | "todo" | "fail";
  detail?: string;
}

export function StageList({ stages }: { stages: Stage[] }) {
  return (
    <ul className="tn-stages">
      {stages.map((s) => {
        const state = s.state ?? "todo";
        return (
          <li key={s.label} className={`tn-stage-${state}`}>
            <span className="tn-stage-ic">
              {state === "done" && <Icon name="check" strokeWidth={2.2} />}
              {state === "fail" && <Icon name="close" strokeWidth={2.2} />}
              {state === "now" && <Spinner />}
            </span>
            <span>{s.label}</span>
            <span className="tn-stage-detail">{s.detail ?? ""}</span>
          </li>
        );
      })}
    </ul>
  );
}

export interface SheetAction {
  label: string;
  variant?: ButtonProps["variant"];
  onClick?: () => void;
  disabled?: boolean;
  autoFocus?: boolean;
}

/** Drops from the toolbar edge and blocks only the window; Escape and the scrim close it. */
export function Sheet({
  open = true,
  title,
  width = 480,
  alert,
  actions = [],
  extra,
  onClose,
  children,
}: {
  open?: boolean;
  title?: ReactNode;
  width?: number;
  /** alertdialog role for destructive confirmations. */
  alert?: boolean;
  /** Right-aligned buttons, primary last. */
  actions?: SheetAction[];
  /** Left-aligned plain button, e.g. "Add More Files…". */
  extra?: SheetAction;
  onClose?: () => void;
  children?: ReactNode;
}) {
  useEffect(() => {
    if (!open || !onClose) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("keydown", onKey);
    };
  }, [open, onClose]);
  if (!open) return null;
  return (
    <div
      className="tn-scrim"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose?.();
      }}
    >
      <div
        className="tn-sheet"
        role={alert ? "alertdialog" : "dialog"}
        aria-modal
        aria-label={typeof title === "string" ? title : undefined}
        style={{ width }}
      >
        <div className="tn-sheet-body">
          {title && <h2 className="tn-sheet-title">{title}</h2>}
          {children}
        </div>
        {(actions.length > 0 || extra) && (
          <div className="tn-sheet-foot">
            {extra && (
              <Button
                variant="plain"
                className="tn-sheet-extra"
                onClick={extra.onClick}
                disabled={extra.disabled}
              >
                {extra.label}
              </Button>
            )}
            {actions.map((a) => (
              <Button
                key={a.label}
                variant={a.variant ?? "secondary"}
                onClick={a.onClick}
                disabled={a.disabled}
                autoFocus={a.autoFocus}
              >
                {a.label}
              </Button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

export type UnitState = "connected" | "looking" | "missing" | "busy";

const UNIT: Record<UnitState, ["ok" | "warn" | "off" | "spin", string]> = {
  connected: ["ok", "Connected · NAM card"],
  looking: ["spin", "Looking for the unit…"],
  missing: ["off", "Not connected"],
  busy: ["warn", "Engine restarting"],
};

export function UnitStatus({
  state = "missing",
  compact,
}: {
  state?: UnitState;
  compact?: boolean;
}) {
  const [tone, text] = UNIT[state];
  const mark =
    tone === "spin" ? (
      <Spinner label="Looking for the unit" />
    ) : (
      <StatusDot tone={tone} />
    );
  if (compact)
    return (
      <div className="tn-unit" title={`Tone Master Pro · ${text}`}>
        {mark}
      </div>
    );
  return (
    <div className="tn-unit">
      <span className="tn-unit-name">Tone Master Pro</span>
      <span className="tn-status">
        {mark}
        {text}
      </span>
    </div>
  );
}

export interface Activity {
  title: string;
  detail?: string;
  /** 0–100; omit for indeterminate. */
  value?: number;
}

export function ActivityCard({ title, detail, value }: Activity) {
  return (
    <div className="tn-activity" aria-live="polite">
      <span className="tn-activity-title">{title}</span>
      {detail && <span className="tn-activity-detail">{detail}</span>}
      <ProgressBar value={value} label={undefined} />
    </div>
  );
}

export type Section = "captures" | "tone3000" | "sdcard" | "settings";

const NAV: [Section, string, IconName][] = [
  ["captures", "Captures", "captures"],
  ["tone3000", "Tone3000", "tone3000"],
  ["sdcard", "SD Card", "sdcard"],
];

export function Sidebar({
  active,
  onNavigate,
  captureCount,
  unit = "missing",
  activity,
  compact,
  style,
}: {
  active: Section;
  onNavigate?: (key: Section) => void;
  captureCount?: number | null;
  unit?: UnitState;
  /** A running long operation; shown on every page. */
  activity?: Activity | null;
  /** Icon-only, below 820 px window width. */
  compact?: boolean;
  style?: CSSProperties;
}) {
  const item = (
    key: Section,
    label: string,
    icon: IconName,
    count?: number | null,
  ) => (
    <button
      key={key}
      type="button"
      className={cx("tn-nav-item", active === key && "is-on")}
      aria-current={active === key ? "page" : undefined}
      aria-label={compact ? label : undefined}
      title={compact ? label : undefined}
      onClick={() => onNavigate?.(key)}
    >
      <Icon name={icon} />
      {!compact && label}
      {!compact && count != null && (
        <span className="tn-nav-count">{count}</span>
      )}
    </button>
  );
  return (
    <nav
      className={cx("tn-sidebar", compact && "is-compact")}
      aria-label="Sections"
      style={style}
    >
      {/* Room for the native window buttons (overlay title bar on macOS); also the drag area. */}
      <div className="tn-lights" data-tauri-drag-region aria-hidden />
      <div className="tn-nav">
        {NAV.map(([k, l, i]) =>
          item(k, l, i, k === "captures" ? captureCount : undefined),
        )}
      </div>
      <div className="tn-sidebar-fill" />
      {activity && !compact && <ActivityCard {...activity} />}
      <div className="tn-nav" style={{ paddingBottom: 8 }}>
        {item("settings", "Settings", "settings")}
      </div>
      <UnitStatus state={unit} compact={compact} />
    </nav>
  );
}

export function Toolbar({
  title,
  subtitle,
  children,
  style,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  children?: ReactNode;
  style?: CSSProperties;
}) {
  return (
    <header className="tn-toolbar" data-tauri-drag-region style={style}>
      <h1 className="tn-toolbar-title">{title}</h1>
      {subtitle && <span className="tn-toolbar-sub">{subtitle}</span>}
      {children && <div className="tn-toolbar-actions">{children}</div>}
    </header>
  );
}

export function ConnectSteps({ steps = CONNECT_STEPS }: { steps?: string[] }) {
  return (
    <ol className="tn-steps">
      {steps.map((s) => (
        <li key={s}>{s}</li>
      ))}
    </ol>
  );
}
