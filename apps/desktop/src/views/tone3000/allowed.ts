// src/views/tone3000/allowed.ts — allowed-variants helpers.

import type { Variant } from "../../lib/api";

export const DEFAULT_ALLOWED = ["a2", "a1-feather", "a1-nano"];

/** "A2 Feather, Nano · A1 Feather, Nano". */
export function allowedLine(allowed: string[], variants: Variant[]): string {
  const part = (arch: number) => {
    // A2 is one variant: allowed or not.
    if (arch === 2)
      return allowed.some((id) =>
        variants.some((v) => v.id === id && v.arch === 2),
      )
        ? "A2"
        : null;
    const labels = variants
      .filter((v) => v.arch === arch && allowed.includes(v.id))
      .sort((a, b) => a.rank - b.rank)
      .map((v) => v.label);
    return labels.length ? `A${String(arch)} ${labels.join(", ")}` : null;
  };
  const parts = [part(2), part(1)].filter((p): p is string => p !== null);
  return parts.length ? parts.join(" · ") : "none (nothing can be installed)";
}

export const toggled = (list: string[], id: string): string[] =>
  list.includes(id) ? list.filter((x) => x !== id) : [...list, id];
