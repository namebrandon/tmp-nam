// src/lib/format.ts — display rules shared by the pages (design/HANDOFF.md).

import type { Capture, ModelInfo, Submodel, UnitModel } from "./api";

/** Run `fn` after the current effect returns (no synchronous setState in effects). */
export function defer(fn: () => unknown): void {
  void Promise.resolve().then(fn);
}

/** "296 KB", "1.1 MB". */
export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  if (n >= 1e3) return `${String(Math.round(n / 1e3))} KB`;
  return `${String(n)} B`;
}

/**
 * Copy text, possibly still being fetched. WebKit only allows clipboard writes
 * inside the click, so a pending text goes through a `ClipboardItem` promise
 * instead of awaiting it first. Resolves to an error message, or null.
 */
export async function copyText(
  text: string | Promise<string>,
): Promise<string | null> {
  try {
    if (typeof text === "string") await navigator.clipboard.writeText(text);
    else if (typeof ClipboardItem === "function")
      await navigator.clipboard.write([
        new ClipboardItem({
          "text/plain": text.then((t) => new Blob([t], { type: "text/plain" })),
        }),
      ]);
    else await navigator.clipboard.writeText(await text);
    return null;
  } catch (e) {
    return errorText(e);
  }
}

export function errorText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

/** "1 capture" / "3 captures". */
export function plural(n: number, one: string, many = `${one}s`): string {
  return `${String(n)} ${n === 1 ? one : many}`;
}

/** Display name: the registry name without `.nam`. */
export function displayName(registryName: string): string {
  return registryName.replace(/\.nam$/i, "");
}

export type Family = "a1" | "a2" | "unknown";

/** A1 = single WaveNet/LSTM network; A2 = SlimmableContainer (or a ≥0.6 file). */
export function family(m: Pick<UnitModel, "info">): Family {
  const info = m.info;
  if (!info?.architecture) return "unknown";
  if (info.architecture === "SlimmableContainer") return "a2";
  const major = Number((info.version ?? "0").split(".")[1] ?? "0");
  return major >= 6 ? "a2" : "a1";
}

const GEAR_TYPES: Record<string, string> = {
  amp: "Amp",
  amp_cab: "Amp + cab",
  amp_pedal: "Pedal into amp",
  amp_pedal_cab: "Pedal, amp + cab",
  pedal: "Pedal",
  pedal_amp: "Pedal into amp",
  preamp: "Preamp",
  studio: "Studio",
  "full-rig": "Full rig",
  full_rig: "Full rig",
};

/** Chains that stop before the speaker cabinet: they need an IR after them. */
const NO_CAB = ["amp", "amp_pedal", "pedal", "pedal_amp", "preamp"];

/**
 * Whether the capture includes the speaker cabinet (so no IR is needed after it),
 * from its `gear_type` metadata; null when the file doesn't say.
 */
export function hasCab(info?: ModelInfo | null): boolean | null {
  const t = info?.meta.gear_type;
  if (!t) return null;
  // "full-rig" (Tone3000): the whole chain, cabinet and mic included.
  if (t.includes("cab") || t.replace("_", "-") === "full-rig") return true;
  return NO_CAB.includes(t) ? false : null;
}

/** "Make · Model · Type", an em dash for a missing value; null when the file has none. */
export function gearLine(info?: ModelInfo | null): string | null {
  const m = info?.meta;
  if (!m?.gear_make && !m?.gear_model && !m?.gear_type) return null;
  const type = m.gear_type ? (GEAR_TYPES[m.gear_type] ?? m.gear_type) : null;
  return [m.gear_make, m.gear_model, type]
    .map((v) => (v === undefined || v === null || v === "" ? "—" : v))
    .join(" · ");
}

/**
 * "48 kHz", "44.1 kHz". The unit's engine runs at 44.1 kHz: a 48 kHz capture is
 * resampled in and out (a little CPU and latency), a 44.1 kHz one runs as is.
 */
export function sampleRate(info?: ModelInfo | null): string | null {
  const r = info?.sample_rate;
  return r ? `${String(Math.round(r / 100) / 10)} kHz` : null;
}

export interface SizeStep {
  label: string;
  /** Value written to player.json; `undefined` = default (the full network). */
  size: number | undefined;
}

/**
 * Names of an A2 container's sizes, by channel width. Tone3000 ships A2 as an
 * 8-channel "Full" and a 3-channel "Lite" network in one file
 * (tone3000.com/guides/nam-a2-the-complete-guide); other widths get "Size N".
 */
function a2Label(channels: unknown, i: number, n: number): string {
  if (channels === 8) return "Full";
  if (channels === 3) return "Lite";
  return i === n - 1 ? "Full" : `Size ${String(i + 1)}`;
}

/**
 * The selectable children of an A2 container, smallest → largest. Upstream selects
 * the first child whose (exclusive) `max_value` is above the requested size, so
 * child i is reached with size = previous child's max_value (0 for the first). The
 * last child is the player default (size 1.0) and is written as "no setting".
 */
export function sizeSteps(subs: Submodel[]): SizeStep[] {
  const sorted = [...subs].sort((a, b) => a.max_value - b.max_value);
  const seen = new Set<string>();
  return sorted.map((s, i) => {
    const last = i === sorted.length - 1;
    const prev = i === 0 ? 0 : (sorted[i - 1]?.max_value ?? 0);
    let label = a2Label(s.channels, i, sorted.length);
    if (seen.has(label)) label = `Size ${String(i + 1)}`;
    seen.add(label);
    return { label, size: last ? undefined : prev };
  });
}

/** Index of the step the current options select (default → last). */
export function currentStep(
  steps: SizeStep[],
  size: number | undefined,
): number {
  if (size === undefined) return steps.length - 1;
  const i = steps.findIndex((s) => s.size === size);
  return i < 0 ? steps.length - 1 : i;
}

const A1_SIZES = ["Standard", "Lite", "Feather", "Nano"];

/** NAM's A1 WaveNet sizes by first-layer width (16/8, 12/6, 8/4, 4/2). */
const A1_WIDTHS: Record<number, string> = {
  16: "Standard",
  12: "Lite",
  8: "Feather",
  4: "Nano",
};

/**
 * An A1 file's size: from the Tone3000 install record, else the network width in
 * the file, else the `-feather.nam` suffix Tone3000 installs carry.
 */
export function a1Size(c: Capture): string | null {
  const fromVariant = c.source?.variant?.replace(/^a1-/, "");
  const raw = fromVariant?.toLowerCase();
  const recorded = A1_SIZES.find((s) => s.toLowerCase() === raw);
  if (recorded) return recorded;
  const width = c.info?.channels;
  if (width != null && A1_WIDTHS[width]) return A1_WIDTHS[width];
  const fromName = /-(standard|lite|feather|nano)\.nam$/i.exec(c.name)?.[1];
  return (
    A1_SIZES.find((s) => s.toLowerCase() === fromName?.toLowerCase()) ?? null
  );
}

/**
 * Type tag: `A2`, `A1 · Feather` or `A1`. An A2 file's sizes are picked in the
 * inspector, so the tag doesn't count them; an A1 file is one size, worth naming.
 */
export function archTag(c: Capture): string {
  const fam = family(c);
  if (fam === "a2") return "A2";
  if (fam === "a1") {
    const size = a1Size(c);
    return size ? `A1 · ${size}` : "A1";
  }
  return "—";
}

/** "about 3 min left" from progress and elapsed time. */
export function timeLeft(fraction: number, elapsedMs: number): string | null {
  if (fraction <= 0.02 || elapsedMs < 3000) return null;
  const left = (elapsedMs / fraction) * (1 - fraction);
  const min = Math.max(1, Math.round(left / 60000));
  return `about ${String(min)} min left`;
}
