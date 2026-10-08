// src/state/AppProvider.tsx — the app store: navigation, unit, captures, the one long
// operation and its results, Tone3000 and the SD build. Above the page switch, so
// pages can unmount while work keeps going.

import {
  useCallback,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { Section, UnitState } from "../ds";
import { api, listen, type AddOutcome, type Capture } from "../lib/api";
import { defer, errorText } from "../lib/format";
import {
  AppContext,
  type AppStore,
  type Mark,
  type OpResult,
  type Source,
} from "./context";
import { applyEvent, startOp, type OpKind, type Operation } from "./operation";
import { useSdBuild } from "./sd";
import { useT3k } from "./t3k";
import { useUnitConnection } from "./unit";

export function AppProvider({ children }: { children: ReactNode }) {
  const [page, setPage] = useState<Section>("captures");
  const [focusTone, setFocusTone] = useState<string | null>(null);
  const conn = useUnitConnection();
  const { poll } = conn;
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const [captures, setCaptures] = useState<Capture[] | null>(null);
  const [marks, setMarks] = useState<Record<string, Mark>>({});
  const [failed, setFailed] = useState<Record<string, Source>>({});
  const [op, setOp] = useState<Operation | null>(null);
  const [working, setWorking] = useState(false);
  const [reselect, setReselect] = useState(false);
  const [results, setResults] = useState<Partial<Record<OpKind, OpResult>>>({});
  const t3k = useT3k();
  const sd = useSdBuild();
  const { deselect } = t3k;

  const connected = conn.status === "connected";
  // "Engine restarting" only while a restart really runs: the send/install restart
  // phase (the fallback when the HID channel is unavailable). Removes and registers
  // normally skip the restart, and the backend's `busy` just means another command
  // holds the unit (a size change or a list).
  const unit: UnitState = op?.phase === "restart" ? "busy" : conn.status;

  const refreshCaptures = useCallback(async () => {
    try {
      const listed = await api.unitList();
      setCaptures(listed.models);
      setSettingsError(listed.settings_error ?? null);
    } catch {
      // The unit went away; the next poll reports it.
    }
  }, []);

  useEffect(() => {
    defer(
      connected
        ? refreshCaptures
        : () => {
            setCaptures(null);
            setSettingsError(null);
          },
    );
  }, [connected, refreshCaptures]);

  const busyReason = op
    ? "Available when the transfer finishes"
    : sd.phase === "running"
      ? "Available when the SD card is done"
      : working
        ? "Available when the unit is ready"
        : null;

  const run = useCallback(
    async (kind: OpKind, sources: Source[]) => {
      if (sources.length === 0) return;
      setResults((r) => ({ ...r, [kind]: undefined }));
      let current = startOp(
        kind,
        sources.map((s) => ({
          name: s.name,
          label: s.label,
          total: s.kind === "file" ? s.bytes : 1,
        })),
      );
      setOp(current);
      const off = listen("op://event", (e) => {
        current = applyEvent(current, e);
        setOp(current);
      });
      let outcome: AddOutcome;
      try {
        outcome =
          kind === "send"
            ? await api.unitAddFiles(
                sources.flatMap((s) =>
                  s.kind === "file" ? [{ path: s.path, name: s.name }] : [],
                ),
              )
            : await api.t3kInstall(
                sources.flatMap((s) => (s.kind === "pick" ? [s.pick] : [])),
              );
      } catch (e) {
        outcome = {
          added: [],
          failed_after_restart: [],
          not_sent: sources.map((s) => s.name),
          stop: { kind: "failed", message: errorText(e) },
          needs_restart: false,
        };
      } finally {
        off();
      }
      // The list first, so the result banner never shows before the new rows.
      await refreshCaptures();
      setOp(null);
      poll();
      const { added, failed_after_restart: dropped } = outcome;
      if (outcome.needs_restart && (!outcome.stop || dropped.length > 0))
        setReselect(true);
      setMarks((m) => {
        const next: Record<string, Mark> = {};
        for (const [k, v] of Object.entries(m))
          next[k] = { ...v, isNew: false };
        for (const a of added) next[a] = { isNew: true };
        return next;
      });
      setFailed((f) => {
        const next = Object.fromEntries(
          Object.entries(f).filter(([k]) => !added.includes(k)),
        );
        for (const d of dropped) {
          const s = sources.find((x) => x.name === d);
          if (s) next[d] = s;
        }
        return next;
      });
      const items = current.items;
      setResults((r) => ({ ...r, [kind]: { kind, outcome, sources, items } }));
      if (kind === "install") {
        const unsent = [outcome.interrupted, ...outcome.not_sent];
        deselect(
          sources.flatMap((s) =>
            s.kind === "pick" && !unsent.includes(s.name) ? [s.captureKey] : [],
          ),
        );
      }
    },
    [deselect, refreshCaptures, poll],
  );

  const unitCommand = useCallback(
    async (f: () => Promise<boolean>) => {
      setWorking(true);
      try {
        if (await f()) setReselect(true);
        await refreshCaptures();
        return null;
      } catch (e) {
        return errorText(e);
      } finally {
        setWorking(false);
      }
    },
    [refreshCaptures],
  );

  const discard = useCallback(
    async (kind: OpKind) => {
      const r = results[kind];
      setResults((x) => ({ ...x, [kind]: undefined }));
      // Files the restart fallback sent before the unplug load only after an engine
      // restart; over the HID channel they're already in the unit's list.
      if (
        r?.outcome.needs_restart &&
        r.outcome.stop?.kind === "disconnected" &&
        r.outcome.added.length > 0
      )
        await unitCommand(() => api.unitReload().then(() => true));
    },
    [results, unitCommand],
  );

  const store = useMemo<AppStore>(
    () => ({
      page,
      navigate: setPage,
      focusTone,
      showInTone3000: (key) => {
        setFocusTone(key);
        setPage("tone3000");
      },
      clearFocusTone: () => {
        setFocusTone(null);
      },
      unit,
      unitInfo: conn.info,
      connected,
      captures,
      settingsError,
      refreshCaptures,
      marks,
      mark: (name, patch) => {
        setMarks((m) => ({ ...m, [name]: { ...m[name], ...patch } }));
      },
      failed,
      forgetFailed: (name) => {
        setFailed((f) =>
          Object.fromEntries(Object.entries(f).filter(([k]) => k !== name)),
        );
      },
      op,
      working,
      reselect,
      dismissReselect: () => {
        setReselect(false);
      },
      busyReason,
      results,
      dismissResult: (kind) => {
        setResults((r) => ({ ...r, [kind]: undefined }));
      },
      run,
      discard,
      unitCommand,
      t3k,
      sd,
    }),
    [
      page,
      focusTone,
      unit,
      conn.info,
      connected,
      captures,
      settingsError,
      refreshCaptures,
      marks,
      failed,
      op,
      working,
      reselect,
      busyReason,
      results,
      run,
      discard,
      unitCommand,
      t3k,
      sd,
    ],
  );

  return <AppContext.Provider value={store}>{children}</AppContext.Provider>;
}
