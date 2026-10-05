// src/state/operation.ts — the one long operation (send / install), folded from op://event.
//
// Pure functions so the reducer is unit-tested; the store in ./app.tsx owns the state.

import type { OpEvent } from "../lib/api";
import { displayName, formatBytes, plural } from "../lib/format";
import type { Activity } from "../ds";

export type OpKind = "send" | "install";

export type ItemState =
  "waiting" | "downloading" | "downloaded" | "sending" | "sent";

export interface OpItem {
  /** Registry name the file gets on the unit. */
  name: string;
  /** What the row is called: capture name, or tone title for installs. */
  label: string;
  state: ItemState;
  done: number;
  total: number;
}

export interface Operation {
  kind: OpKind;
  phase: "download" | "send" | "restart";
  items: OpItem[];
}

export function startOp(
  kind: OpKind,
  items: { name: string; label: string; total: number }[],
): Operation {
  return {
    kind,
    phase: kind === "install" ? "download" : "send",
    items: items.map((i) => ({ ...i, state: "waiting", done: 0 })),
  };
}

export function applyEvent(op: Operation, e: OpEvent): Operation {
  if (e.phase === "restart") return { ...op, phase: "restart" };
  const items = op.items.map((it, i): OpItem => {
    if (i !== e.index) return it;
    switch (e.phase) {
      case "download":
        return {
          ...it,
          state: e.done >= e.total ? "downloaded" : "downloading",
        };
      case "send":
        return { ...it, state: "sending", done: e.done, total: e.total };
      case "sent":
        return { ...it, state: "sent", done: it.total };
    }
  });
  const phase = e.phase === "download" ? "download" : "send";
  return { ...op, phase, items };
}

/** Overall percent: downloads and sends weighted by item, sends by bytes. */
export function percent(op: Operation): number | undefined {
  if (op.phase === "restart") return undefined;
  if (op.phase === "download") {
    const done = op.items.filter((i) => i.state === "downloaded").length;
    return Math.round((done / Math.max(1, op.items.length)) * 100);
  }
  const total = op.items.reduce((a, i) => a + i.total, 0);
  const done = op.items.reduce((a, i) => a + i.done, 0);
  return total > 0 ? Math.round((done / total) * 100) : 0;
}

/** The sidebar activity card for a running operation. */
export function activity(op: Operation): Activity {
  const n = op.items.length;
  const what =
    op.kind === "send"
      ? "Sending captures"
      : `Installing ${plural(n, "capture")}`;
  if (op.phase === "restart")
    return { title: "Restarting audio engine", detail: "A few seconds" };
  if (op.phase === "download")
    return {
      title: what,
      detail: "Downloading from Tone3000",
      value: percent(op),
    };
  const at = Math.max(
    0,
    op.items.findIndex((i) => i.state !== "sent"),
  );
  const current = op.items[at];
  return {
    title: what,
    detail: `${String(at + 1)} of ${String(n)} · ${current?.label ?? ""}`,
    value: percent(op),
  };
}

/** Right-hand detail of a row in the sending panel. */
export function itemDetail(it: OpItem): string {
  if (it.state === "sent") return `Sent · ${formatBytes(it.total)}`;
  if (it.state === "sending")
    return `${formatBytes(it.done)} of ${formatBytes(it.total)}`;
  return "Waiting";
}

/** The Installed column of a capture being installed. */
export function installStatus(op: Operation, it: OpItem): string {
  if (op.phase === "download")
    return it.state === "downloaded" ? "Waiting to send" : "Downloading";
  if (it.state === "sending")
    return `Sending · ${String(Math.round((it.done / Math.max(1, it.total)) * 100))}%`;
  if (it.state === "sent") return op.phase === "restart" ? "Loading" : "Sent";
  return it.state === "downloaded" ? "Waiting to send" : "Waiting";
}

/** "A and B" / "A, B and C" from registry names. */
export function nameList(names: string[]): string {
  const shown = names.map(displayName);
  if (shown.length <= 1) return shown.join("");
  return `${shown.slice(0, -1).join(", ")} and ${shown[shown.length - 1] ?? ""}`;
}
