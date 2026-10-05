// src/state/t3k.ts — Tone3000 account, tone list, and the per-capture selection.
//
// A tone holds captures (models grouped by name, see `t3k::group_captures`); each
// capture is published as A2 and/or A1 sizes. The user selects captures; each
// installs one model: the allowed one picked automatically (A2 first, then the
// largest A1 size) unless overridden. Lives in the app store so the selection
// survives page switches ("Your selection is kept").

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  api,
  ApiError,
  type Settings,
  type T3kCapture,
  type T3kModel,
  type T3kTone,
  type Variant,
} from "../lib/api";
import { defer, errorText } from "../lib/format";

export type Account =
  | "checking"
  | "nokey"
  | "signedout"
  | "signingin"
  | "error"
  | "loading"
  | "list";

export type Filter = "all" | "bookmark" | "mine";

export interface T3kError {
  code: string;
  message: string;
}

export const toneKey = (t: Pick<T3kTone, "id">): string => String(t.id);

/** Stable across refreshes: the tone id and the capture's name. */
export const captureKey = (t: Pick<T3kTone, "id">, c: T3kCapture): string =>
  `${toneKey(t)}/${c.name.trim().toLowerCase()}`;

/** Index (into `tone.models`) of the capture's best allowed model, or null. */
export function autoPick(
  tone: T3kTone,
  capture: T3kCapture,
  allowed: string[],
  variants: Variant[],
): number | null {
  let best: number | null = null;
  let bestKey = Infinity;
  for (const i of capture.models) {
    const m = tone.models[i];
    if (!m?.model_url || !m.variant || !allowed.includes(m.variant)) continue;
    const v = variants.find((x) => x.id === m.variant);
    if (!v) continue;
    // A2 before A1; within A2 (one variant) the largest size word wins.
    const key = v.arch === 2 ? sizeRank(m.size_hint) : 100 + v.rank;
    if (key < bestKey) {
      best = i;
      bestKey = key;
    }
  }
  return best;
}

const SIZE_RANK = ["full", "standard", "lite", "feather", "nano"];

/** Lower = larger; no size word counts as the full network. */
function sizeRank(hint?: string | null): number {
  const i = hint ? SIZE_RANK.indexOf(hint) : 0;
  return i < 0 ? 0 : i;
}

const titled = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/**
 * "A1 · Feather", "A2", or `A2 · “Lite”` when an uploader published several
 * A2 files of one capture and named them with a size word. An A2 file always holds
 * its own sizes (picked on the unit), so that word is only a name, not a size.
 */
export function modelLabel(m: T3kModel, variants: Variant[]): string {
  const v = variants.find((x) => x.id === m.variant);
  if (v?.arch === 2)
    return m.size_hint ? `A2 · “${titled(m.size_hint)}”` : "A2";
  if (v) return `A1 · ${v.label}`;
  return `A${m.architecture_version} · ${m.size ?? "unlabelled"}`;
}

export interface T3kStore {
  account: Account;
  username: string | null;
  error: T3kError | null;
  settings: Settings | null;
  variants: Variant[];
  tones: T3kTone[];
  filter: Filter;
  /** Selected capture keys (`captureKey`). */
  selected: ReadonlySet<string>;
  /** Model index chosen by hand, by capture key. */
  overrides: Readonly<Record<string, number>>;
  /** Model index this capture installs, or null when nothing is allowed. */
  pickFor: (t: T3kTone, c: T3kCapture) => number | null;
  autoFor: (t: T3kTone, c: T3kCapture) => number | null;
  saveKey: (key: string) => Promise<string | null>;
  removeKey: () => Promise<void>;
  signIn: () => Promise<void>;
  openAgain: () => void;
  cancelSignIn: () => void;
  signOut: () => Promise<void>;
  loadTones: () => Promise<void>;
  setFilter: (f: Filter) => void;
  toggle: (key: string) => void;
  /** Select or clear several captures at once (a tone's checkbox). */
  setSelected: (keys: string[], on: boolean) => void;
  clearSelection: () => void;
  deselect: (keys: string[]) => void;
  setOverride: (key: string, index: number | null) => void;
  saveAllowed: (ids: string[]) => Promise<void>;
}

function asError(e: unknown): T3kError {
  return e instanceof ApiError
    ? { code: e.code, message: e.message }
    : { code: "other", message: errorText(e) };
}

export function useT3k(): T3kStore {
  const [account, setAccount] = useState<Account>("checking");
  const [username, setUsername] = useState<string | null>(null);
  const [error, setError] = useState<T3kError | null>(null);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [variants, setVariants] = useState<Variant[]>([]);
  const [tones, setTones] = useState<T3kTone[]>([]);
  const [filter, setFilter] = useState<Filter>("all");
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  const [overrides, setOverrides] = useState<Record<string, number>>({});
  const signing = useRef(false);

  const allowed = useMemo(() => settings?.variants ?? [], [settings]);
  const autoFor = useCallback(
    (t: T3kTone, c: T3kCapture) => autoPick(t, c, allowed, variants),
    [allowed, variants],
  );
  const pickFor = useCallback(
    (t: T3kTone, c: T3kCapture) => overrides[captureKey(t, c)] ?? autoFor(t, c),
    [overrides, autoFor],
  );

  const loadTones = useCallback(async () => {
    setAccount("loading");
    try {
      setTones(await api.t3kTones());
      setAccount("list");
    } catch (e) {
      const err = asError(e);
      setError(err);
      setAccount(err.code === "signed_out" ? "signedout" : "error");
    }
  }, []);

  const start = useCallback(async () => {
    const [s, v, status] = await Promise.all([
      api.settingsGet(),
      api.variantsList(),
      api.t3kStatus(),
    ]);
    setSettings(s);
    setVariants(v);
    setUsername(status.username ?? null);
    if (!status.has_key) setAccount("nokey");
    else if (!status.linked) setAccount("signedout");
    else await loadTones();
  }, [loadTones]);

  useEffect(() => {
    defer(start);
  }, [start]);

  const signIn = useCallback(async () => {
    if (signing.current) return;
    signing.current = true;
    setError(null);
    setAccount("signingin");
    try {
      setUsername(await api.t3kLink());
      await loadTones();
    } catch (e) {
      const err = asError(e);
      if (err.code === "cancelled") setAccount("signedout");
      else {
        setError(err);
        setAccount("error");
      }
    } finally {
      signing.current = false;
    }
  }, [loadTones]);

  const saveKey = useCallback(
    async (key: string) => {
      const next = { t3k_key: key.trim(), variants: allowed };
      try {
        await api.settingsSet(next);
      } catch (e) {
        return errorText(e);
      }
      setSettings(next);
      setError(null);
      setAccount("signedout");
      return null;
    },
    [allowed],
  );

  const removeKey = useCallback(async () => {
    const next = { t3k_key: "", variants: allowed };
    await api.t3kUnlink();
    await api.settingsSet(next);
    setSettings(next);
    setUsername(null);
    setTones([]);
    setAccount("nokey");
  }, [allowed]);

  const signOut = useCallback(async () => {
    await api.t3kUnlink();
    setUsername(null);
    setTones([]);
    setAccount("signedout");
  }, []);

  const saveAllowed = useCallback(
    async (ids: string[]) => {
      const next = { t3k_key: settings?.t3k_key ?? "", variants: ids };
      await api.settingsSet(next);
      setSettings(next);
      // Overrides that are no longer allowed go; so do selections with no model left.
      const all = tones.flatMap((t) =>
        t.captures.map((c) => ({ t, c, key: captureKey(t, c) })),
      );
      const find = (k: string) => all.find((x) => x.key === k);
      setOverrides((o) =>
        Object.fromEntries(
          Object.entries(o).filter(([k, i]) => {
            const v = find(k)?.t.models[i]?.variant;
            return v != null && ids.includes(v);
          }),
        ),
      );
      setSelected(
        (s) =>
          new Set(
            [...s].filter((k) => {
              const x = find(k);
              return (
                x !== undefined && autoPick(x.t, x.c, ids, variants) !== null
              );
            }),
          ),
      );
    },
    [settings, tones, variants],
  );

  return {
    account,
    username,
    error,
    settings,
    variants,
    tones,
    filter,
    selected,
    overrides,
    pickFor,
    autoFor,
    saveKey,
    removeKey,
    signIn,
    openAgain: () => void api.t3kOpenLinkAgain(),
    cancelSignIn: () => void api.t3kCancelLink(),
    signOut,
    loadTones,
    setFilter,
    toggle: (key) => {
      setSelected((s) => {
        const n = new Set(s);
        if (n.has(key)) n.delete(key);
        else n.add(key);
        return n;
      });
    },
    setSelected: (keys, on) => {
      setSelected((s) => {
        const n = new Set(s);
        for (const k of keys) {
          if (on) n.add(k);
          else n.delete(k);
        }
        return n;
      });
    },
    clearSelection: () => {
      setSelected(new Set());
    },
    deselect: (keys) => {
      setSelected((s) => new Set([...s].filter((k) => !keys.includes(k))));
    },
    setOverride: (key, index) => {
      setOverrides((o) => {
        const rest = Object.fromEntries(
          Object.entries(o).filter(([k]) => k !== key),
        );
        return index === null ? rest : { ...rest, [key]: index };
      });
    },
    saveAllowed,
  };
}
