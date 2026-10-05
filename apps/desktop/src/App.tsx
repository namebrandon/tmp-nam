// src/App.tsx — TMP NAM window: sidebar (pages, activity, unit status) + the page.
//
//   • Captures → NAM files on the unit (needs the unit)
//   • Tone3000 → bookmarks and own tones (browsing never needs the unit)
//   • SD Card  → build the bootable card (never needs the unit)
//   • Settings → Tone3000 key, allowed variants, unit details
//
// Connection is automatic; the unit's state is always at the bottom of the sidebar.
// Below 820 px wide the sidebar collapses to icons (design/HANDOFF.md, Global rules).

import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { cx, Sheet, Sidebar, type Activity } from "./ds";
import { api, listen } from "./lib/api";
import { defer } from "./lib/format";
import { AppProvider } from "./state/AppProvider";
import { useApp } from "./state/context";
import { activity } from "./state/operation";
import { STAGES } from "./state/sd";
import { CapturesPage } from "./views/captures/CapturesPage";
import { SdCardPage } from "./views/sdcard/SdCardPage";
import { SettingsPage } from "./views/settings/SettingsPage";
import { Tone3000Page } from "./views/tone3000/Tone3000Page";

const NARROW = "(max-width: 819px)";

/** True below 820 px window width. */
function useNarrow(): boolean {
  return useSyncExternalStore(
    (cb) => {
      const mq = window.matchMedia(NARROW);
      mq.addEventListener("change", cb);
      return () => {
        mq.removeEventListener("change", cb);
      };
    },
    () => window.matchMedia(NARROW).matches,
  );
}

const IS_MAC = navigator.userAgent.includes("Mac");

export default function App() {
  return (
    <AppProvider>
      <Shell />
    </AppProvider>
  );
}

/** What quitting now would interrupt, or null. */
type Running = "send" | "install" | "card" | null;

/**
 * Ask before the window closes or the app quits while a send, install or card
 * build runs (quitting mid-write can leave the card unusable). The backend holds
 * the close and the quit (⌘Q) while the guard is on and emits `app://quit-requested`.
 */
function useCloseGuard(running: Running): [Running, () => void, () => void] {
  const [asking, setAsking] = useState<Running>(null);
  const current = useRef(running);
  useEffect(() => {
    current.current = running;
    void api.setQuitGuard(running !== null);
    // The work ended while the question was up: nothing left to warn about.
    if (running === null)
      defer(() => {
        setAsking(null);
      });
  }, [running]);
  useEffect(
    () =>
      listen("app://quit-requested", () => {
        setAsking(current.current);
      }),
    [],
  );
  return [
    asking,
    () => void api.quitNow(),
    () => {
      setAsking(null);
    },
  ];
}

const QUIT_COPY: Record<
  Exclude<Running, null>,
  { title: string; body: string }
> = {
  send: {
    title: "Quit while captures are being sent?",
    body: "The file being sent won't be kept and the rest won't be sent. Captures already sent stay on the unit.",
  },
  install: {
    title: "Quit while tones are being installed?",
    body: "The tone being sent won't be kept and the rest won't be installed. Tones already sent stay on the unit.",
  },
  card: {
    title: "Quit while the SD card is being created?",
    body: "Quitting now can leave the card unusable; you'd need to create it again. Your unit and its firmware are unaffected.",
  },
};

function Shell() {
  const app = useApp();
  const compact = useNarrow();
  const { sd } = app;
  const running: Running = app.op
    ? app.op.kind
    : sd.phase === "running"
      ? "card"
      : null;
  const [asking, quit, keep] = useCloseGuard(running);

  let current: Activity | null = null;
  if (app.op) current = activity(app.op);
  else if (sd.phase === "running")
    current = {
      title: "Creating SD card",
      detail:
        sd.stage < 0
          ? "Waiting for administrator approval"
          : `${STAGES[sd.stage] ?? ""}${sd.stage === 6 ? ` · ${String(Math.round(sd.fraction * 100))}%` : ""}`,
      value: sd.stage < 0 ? undefined : Math.round(sd.percent),
    };

  return (
    <div className={cx("tn-root", "win", IS_MAC && "is-mac")}>
      <Sidebar
        active={app.page}
        onNavigate={app.navigate}
        captureCount={app.captures?.length ?? null}
        unit={app.unit}
        activity={current}
        compact={compact}
      />
      <main className="main">
        {app.page === "captures" && <CapturesPage narrow={compact} />}
        {app.page === "tone3000" && <Tone3000Page />}
        {app.page === "sdcard" && <SdCardPage />}
        {app.page === "settings" && <SettingsPage />}
      </main>
      {asking && (
        <Sheet
          title={QUIT_COPY[asking].title}
          width={440}
          alert
          onClose={keep}
          actions={[
            { label: "Keep Running", autoFocus: true, onClick: keep },
            { label: "Quit Anyway", variant: "danger", onClick: quit },
          ]}
        >
          <p className="muted">{QUIT_COPY[asking].body}</p>
        </Sheet>
      )}
    </div>
  );
}
