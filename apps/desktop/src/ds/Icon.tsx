// src/ds/Icon.tsx — the TMP NAM icon set: 16 px grid, 1.5 px stroke, colour from the text.

import type { CSSProperties, ReactNode } from "react";
import { cx } from "./util";

const ICONS = {
  captures: <path d="M1.5 8h2l2-5 3 10 2-7 1.2 2h2.8" />,
  tone3000: (
    <>
      <path d="M4.5 12.5a3 3 0 0 1-.3-6 4 4 0 0 1 7.7 1 2.5 2.5 0 0 1 .3 5" />
      <path d="M8 8v6m-2-2 2 2 2-2" />
    </>
  ),
  sdcard: (
    <>
      <path d="M5.5 1.5h6v13h-8V3.5z" />
      <path d="M7 4v2m2-2v2" />
    </>
  ),
  settings: (
    <>
      <path d="M2 4.5h7m3 0h2M2 11.5h2m3 0h7" />
      <circle cx="10.5" cy="4.5" r="1.5" />
      <circle cx="5.5" cy="11.5" r="1.5" />
    </>
  ),
  warning: (
    <>
      <path d="M8 2 14.5 13.5h-13z" />
      <path d="M8 6.5v3M8 11.6v.1" />
    </>
  ),
  error: (
    <>
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 4.5v4M8 11v.1" />
    </>
  ),
  info: (
    <>
      <circle cx="8" cy="8" r="6.5" />
      <path d="M8 7.5v4M8 5v.1" />
    </>
  ),
  usb: <path d="M6 1.5v3.5m4-3.5v3.5M4.5 5h7v3a3.5 3.5 0 0 1-7 0zM8 11.5v3" />,
  add: <path d="M8 3v10M3 8h10" />,
  minus: <path d="M3 8h10" />,
  check: <path d="M3 8.5 6.5 12 13 4.5" />,
  close: <path d="M4 4l8 8M12 4 4 12" />,
  external: <path d="M9 2.5h4.5V7M13.5 2.5 7.5 8.5M11.5 9.5v4h-9v-9h4" />,
  bookmark: <path d="M4.5 2h7v12L8 11l-3.5 3z" />,
  refresh: <path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5v3h-3" />,
  lock: (
    <>
      <rect x="3.5" y="7" width="9" height="7" rx="1.5" />
      <path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2" />
    </>
  ),
  chevron: <path d="M4.5 6.5 8 10l3.5-3.5" />,
} satisfies Record<string, ReactNode>;

export type IconName = keyof typeof ICONS;

export interface IconProps {
  name: IconName;
  size?: number;
  strokeWidth?: number;
  "aria-label"?: string;
  className?: string;
  style?: CSSProperties;
}

export function Icon({
  name,
  size = 16,
  strokeWidth = 1.5,
  className,
  style,
  "aria-label": label,
}: IconProps) {
  return (
    <svg
      className={cx("tn-icon", className)}
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={strokeWidth}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden={label ? undefined : true}
      role={label ? "img" : undefined}
      aria-label={label}
      style={style}
    >
      {ICONS[name]}
    </svg>
  );
}
