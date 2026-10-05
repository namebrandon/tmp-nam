// src/state/unit.ts — automatic unit connection (no Connect button).
//
// Polls `unit_connect` every second while the unit is missing and every 2 s once
// connected (the same poll notices an unplug).
// While a transfer holds the unit the backend answers at once with `busy`.

import { useCallback, useEffect, useRef, useState } from "react";
import { api, type UnitInfo } from "../lib/api";
import { defer, errorText } from "../lib/format";

/** While connected: notice an unplug. While not: notice a plug-in quickly. */
const CONNECTED_POLL_MS = 2000;
const MISSING_POLL_MS = 1000;

export interface UnitConnection {
  info: UnitInfo | null;
  status: "looking" | "connected" | "missing";
  error: string | null;
  /** Ask again now, e.g. right after a transfer that may have lost the unit. */
  poll: () => void;
}

export function useUnitConnection(): UnitConnection {
  const [conn, setConn] = useState<Omit<UnitConnection, "poll">>({
    info: null,
    status: "looking",
    error: null,
  });
  const inFlight = useRef(false);

  const attempt = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const info = await api.unitConnect();
      setConn({ info, status: "connected", error: null });
    } catch (e) {
      setConn({ info: null, status: "missing", error: errorText(e) });
    } finally {
      inFlight.current = false;
    }
  }, []);

  const connected = conn.status === "connected";
  useEffect(() => {
    defer(attempt);
    const id = setInterval(
      () => void attempt(),
      connected ? CONNECTED_POLL_MS : MISSING_POLL_MS,
    );
    return () => {
      clearInterval(id);
    };
  }, [attempt, connected]);

  const poll = useCallback(() => void attempt(), [attempt]);
  return { ...conn, poll };
}
