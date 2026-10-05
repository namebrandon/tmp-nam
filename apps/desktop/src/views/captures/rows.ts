// src/views/captures/rows.ts — the capture table's rows: the unit's list plus captures
// that were sent but didn't load after the engine restart (no longer on the unit).

import type { Capture } from "../../lib/api";
import type { Mark, Source } from "../../state/context";

export type Flag = "missing" | "unreg" | "failed";

export interface Row {
  name: string;
  capture: Capture | null;
  flag: Flag | null;
}

export function captureRows(
  captures: Capture[],
  failed: Readonly<Record<string, Source>>,
  marks: Readonly<Record<string, Mark>>,
): Row[] {
  const rows: Row[] = captures.map((c) => ({
    name: c.name,
    capture: c,
    flag: failed[c.name]
      ? "failed"
      : !c.present
        ? "missing"
        : !c.registered
          ? "unreg"
          : null,
  }));
  for (const name of Object.keys(failed))
    if (!rows.some((r) => r.name === name))
      rows.push({ name, capture: null, flag: "failed" });
  // New captures first, then the unit's order; flagged rows keep their place.
  return rows.sort((a, b) => {
    const na = marks[a.name]?.isNew ? 0 : 1;
    const nb = marks[b.name]?.isNew ? 0 : 1;
    return na - nb;
  });
}

export const FLAG_TEXT: Record<Flag, string> = {
  missing: "File missing",
  unreg: "Not registered",
  failed: "Didn't load",
};
