// src/state/sd.ts — the SD-card build: inputs, the running build and its outcome.
//
// In the app store because the build keeps running (and shows in the sidebar)
// while the user is on other pages.

import { useCallback, useEffect, useRef, useState } from "react";
import {
  api,
  listen,
  pickFiles,
  type Disk,
  type FirmwareCheck,
  type SdEnvironment,
} from "../lib/api";
import { defer, errorText, timeLeft } from "../lib/format";

/** The eight stages, as `sdcard::STAGES` in Rust numbers them. */
export const STAGES = [
  "Checking the firmware file",
  "Extracting",
  "Verifying the extracted files",
  "Adding NAM support",
  "Building the filesystem",
  "Preparing partitions",
  "Writing to the card",
  "Reading back to verify",
];

/** What a failure in each stage means for the guitarist. */
export const FAILURES = [
  "The firmware file couldn't be checked. Choose it again, then try again.",
  "The firmware couldn't be unpacked on this computer. Try again; if it keeps failing, the log says why.",
  "The unpacked firmware didn't pass its check. Choose the firmware file again, then try again.",
  "The NAM files couldn't be added. Try again; if it keeps failing, the log says why.",
  "The card's filesystem couldn't be built on this computer. Try again; if it keeps failing, the log says why.",
  "The card's partitions couldn't be written. Check the card isn't locked, then try again.",
  "Writing to the card failed. The card may have been removed, or the card or reader may be faulty. Try again, or use another card or reader.",
  "Reading back to verify failed: what's on the card doesn't match what was written. The card or the reader may be faulty. Try another card or reader.",
];

export type SdPhase = "setup" | "running" | "success" | "failed";

export interface SdStore {
  env: SdEnvironment | null;
  firmware: FirmwareCheck | null;
  checking: boolean;
  disks: Disk[] | null;
  disk: string | null;
  phase: SdPhase;
  /** -1 while waiting for the administrator prompt. */
  stage: number;
  /** Progress within the current stage, 0–1. */
  fraction: number;
  /** Overall percent. */
  percent: number;
  /** "about 3 min left" for the current stage, once it can be estimated. */
  left: string | null;
  log: string[];
  showLog: boolean;
  denied: boolean;
  failure: { stage: number; message: string } | null;
  error: string | null;
  checkEnvironment: () => Promise<void>;
  chooseFirmware: () => Promise<void>;
  pickDisk: (device: string) => void;
  start: () => Promise<void>;
  toggleLog: () => void;
  dismissDenied: () => void;
  /** Back to the form (Done, Back); `keepDisk` false for Make Another. */
  reset: (keepDisk: boolean) => void;
}

const DISK_POLL_MS = 3000;

/** Platforms the card builder writes cards on (`card.rs`). */
export const CARD_PLATFORMS = ["macos", "linux"];

export function useSdBuild(): SdStore {
  const [env, setEnv] = useState<SdEnvironment | null>(null);
  const [firmware, setFirmware] = useState<FirmwareCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [disks, setDisks] = useState<Disk[] | null>(null);
  const [disk, setDisk] = useState<string | null>(null);
  const [phase, setPhase] = useState<SdPhase>("setup");
  const [stage, setStage] = useState(-1);
  const [fraction, setFraction] = useState(0);
  const [percent, setPercent] = useState(0);
  const [left, setLeft] = useState<string | null>(null);
  const stageStarted = useRef(0);
  const [log, setLog] = useState<string[]>([]);
  const [showLog, setShowLog] = useState(false);
  const [denied, setDenied] = useState(false);
  const [failure, setFailure] = useState<SdStore["failure"]>(null);
  const [error, setError] = useState<string | null>(null);
  const stageRef = useRef(-1);

  const checkEnvironment = useCallback(async () => {
    setEnv(await api.sdEnvironment());
  }, []);

  useEffect(() => {
    defer(checkEnvironment);
  }, [checkEnvironment]);

  // Watch for cards while the form shows (card writing is macOS and Linux only).
  const supported = env === null || CARD_PLATFORMS.includes(env.platform);
  useEffect(() => {
    if (phase !== "setup" || !supported) return;
    const poll = async () => {
      try {
        const list = await api.sdListDisks();
        setError(null);
        setDisks(list);
        setDisk((d) => {
          const ok = list.filter((x) => !x.rejected);
          if (d && ok.some((x) => x.device === d)) return d;
          return ok.length === 1 ? (ok[0]?.device ?? null) : null;
        });
      } catch (e) {
        setError(errorText(e));
      }
    };
    defer(poll);
    const id = setInterval(() => void poll(), DISK_POLL_MS);
    return () => {
      clearInterval(id);
    };
  }, [phase, supported]);

  useEffect(() => {
    const stamp = () => new Date().toLocaleTimeString([], { hour12: false });
    const offLog = listen("sd://log", (l) => {
      setLog((x) => [...x, `[${stamp()}] ${l.line}`]);
      if (l.stage != null) {
        if (l.stage !== stageRef.current) {
          stageRef.current = l.stage;
          setStage(l.stage);
          stageStarted.current = Date.now();
        }
        const f = l.fraction ?? 0;
        setFraction(f);
        setLeft(timeLeft(f, Date.now() - stageStarted.current));
      }
      if (l.percent != null) setPercent(l.percent);
    });
    const offDone = listen("sd://done", (d) => {
      if (d.outcome === "ok") {
        setPercent(100);
        setPhase("success");
      } else if (d.outcome === "denied") {
        setDenied(true);
        setPhase("setup");
      } else {
        setFailure({
          stage: d.stage ?? Math.max(0, stageRef.current),
          message: d.message,
        });
        setShowLog(true);
        setPhase("failed");
      }
    });
    return () => {
      offLog();
      offDone();
    };
  }, []);

  const chooseFirmware = useCallback(async () => {
    const paths = await pickFiles(
      [{ name: "Fender firmware", extensions: ["img"] }],
      false,
    );
    const path = paths?.[0];
    if (!path) return;
    setChecking(true);
    setFirmware(null);
    try {
      setFirmware(await api.sdCheckFirmware(path));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setChecking(false);
    }
  }, []);

  const start = useCallback(async () => {
    if (!firmware?.matches || !disk) return;
    stageRef.current = -1;
    setStage(-1);
    setFraction(0);
    setPercent(0);
    setLog([]);
    setDenied(false);
    setFailure(null);
    setShowLog(false);
    setPhase("running");
    try {
      await api.sdWriteCard(firmware.path, disk);
    } catch (e) {
      setFailure({ stage: 0, message: errorText(e) });
      setPhase("failed");
    }
  }, [firmware, disk]);

  return {
    env,
    firmware,
    checking,
    disks,
    disk,
    phase,
    stage,
    fraction,
    percent,
    left,
    log,
    showLog,
    denied,
    failure,
    error,
    checkEnvironment,
    chooseFirmware,
    pickDisk: setDisk,
    start,
    toggleLog: () => {
      setShowLog((s) => !s);
    },
    dismissDenied: () => {
      setDenied(false);
    },
    reset: (keepDisk) => {
      setPhase("setup");
      setShowLog(false);
      setLog([]);
      if (!keepDisk) setDisk(null);
    },
  };
}
