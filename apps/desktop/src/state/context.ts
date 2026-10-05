// src/state/context.ts — the app store's shape and its hook (provider in ./AppProvider.tsx).

import { createContext, useContext } from "react";
import type { Section, UnitState } from "../ds";
import type { AddOutcome, Capture, T3kPick, UnitInfo } from "../lib/api";
import type { OpItem, OpKind, Operation } from "./operation";
import type { SdStore } from "./sd";
import type { T3kStore } from "./t3k";

/** Something the app can send to the unit, kept to retry it. */
export type Source =
  | { kind: "file"; path: string; name: string; label: string; bytes: number }
  | {
      kind: "pick";
      pick: T3kPick;
      name: string;
      label: string;
      /** `captureKey` of the capture this installs. */
      captureKey: string;
    };

/** Session-only state of a capture, keyed by registry name. */
export interface Mark {
  isNew?: boolean;
  /** Size changed: blue dot and the "Saved to the unit" note. */
  changed?: boolean;
}

/** The last finished operation of a kind, shown as a banner until dismissed. */
export interface OpResult {
  kind: OpKind;
  outcome: AddOutcome;
  sources: Source[];
  /** Rows as the operation ended, for "Interrupted at 48%". */
  items: OpItem[];
}

export interface AppStore {
  page: Section;
  navigate: (page: Section) => void;
  /** Tone whose model menu "Show in Tone3000" opens. */
  focusTone: string | null;
  showInTone3000: (toneKey: string) => void;
  clearFocusTone: () => void;

  unit: UnitState;
  unitInfo: UnitInfo | null;
  connected: boolean;

  captures: Capture[] | null;
  refreshCaptures: () => Promise<void>;
  marks: Readonly<Record<string, Mark>>;
  mark: (name: string, patch: Mark) => void;
  /** Sent but not loaded after the engine restart, with what to resend. */
  failed: Readonly<Record<string, Source>>;
  forgetFailed: (name: string) => void;

  op: Operation | null;
  /** A short command (remove, register, size, gain) is running. */
  working: boolean;
  /**
   * The engine restarted (send, install, remove, register): NAM stays off until a
   * capture is selected again on the unit, so the pages remind the user.
   */
  reselect: boolean;
  dismissReselect: () => void;
  /** Why actions that would conflict are disabled, or null. */
  busyReason: string | null;
  results: Partial<Record<OpKind, OpResult>>;
  dismissResult: (kind: OpKind) => void;
  run: (kind: OpKind, sources: Source[]) => Promise<void>;
  /** Discard what an interrupted operation didn't send; reloads files that did land. */
  discard: (kind: OpKind) => Promise<void>;
  /** A short unit command; refreshes the list afterwards. */
  /** `f` resolves `true` when the engine restarted (then NAM needs a reselect). */
  unitCommand: (f: () => Promise<boolean>) => Promise<string | null>;

  t3k: T3kStore;
  sd: SdStore;
}

export const AppContext = createContext<AppStore | null>(null);

export function useApp(): AppStore {
  const app = useContext(AppContext);
  if (!app) throw new Error("useApp outside AppProvider");
  return app;
}
