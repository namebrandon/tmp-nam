// src/views/results.ts — the banner a finished send or install leaves on its page.
// Copy from design/prototype/Main.dc.html (finishOp, unplug) and design/HANDOFF.md.

import type { BannerAction, BannerTone } from "../ds";
import { plural } from "../lib/format";
import type { AppStore, OpResult, Source } from "../state/context";
import { nameList } from "../state/operation";

export interface ResultBanner {
  tone: BannerTone;
  title: string;
  body: string;
  actions: BannerAction[];
}

/** Sources an interrupted operation didn't get onto the unit. */
export function remaining(r: OpResult): Source[] {
  const unsent = [r.outcome.interrupted, ...r.outcome.not_sent];
  return r.sources.filter((s) => unsent.includes(s.name));
}

export function resultBanner(
  r: OpResult,
  app: AppStore,
  extra: { openCaptures?: () => void; remove?: (name: string) => void },
): ResultBanner {
  const { outcome } = r;
  const n = r.sources.length;
  const send = r.kind === "send";
  const noun = "capture";
  const sent = n - remaining(r).length;
  const blocked = !app.connected || app.busyReason !== null;
  const why = !app.connected
    ? "Connect the unit first"
    : (app.busyReason ?? undefined);
  const rest = remaining(r);
  // Discarding reloads the engine when the restart fallback sent files before the
  // unplug.
  const discardReloads = outcome.needs_restart && outcome.added.length > 0;
  const resume: BannerAction[] = [
    {
      label: `${send ? "Send" : "Install"} Remaining ${String(rest.length)}`,
      variant: "primary",
      disabled: blocked,
      title: why,
      onClick: () => void app.run(r.kind, rest),
    },
    {
      label: "Discard",
      disabled: discardReloads && blocked,
      title: discardReloads ? why : undefined,
      onClick: () => void app.discard(r.kind),
    },
  ];
  const open: BannerAction[] = extra.openCaptures
    ? [{ label: "Open Captures", onClick: extra.openCaptures }]
    : [];

  switch (outcome.stop?.kind) {
    case "disconnected":
      return {
        tone: "error",
        title: send
          ? "The unit disconnected while sending"
          : "The unit disconnected during the install",
        body: send
          ? `${String(sent)} of ${String(n)} captures reached the unit. The interrupted file was not kept. Reconnect to send the rest.`
          : `${sent === 0 ? "Nothing was sent yet." : `${String(sent)} of ${String(n)} captures reached the unit.`} Reconnect to install the rest.`,
        actions: resume,
      };
    case "disconnected_during_restart":
      return {
        tone: "warn",
        title: "The unit disconnected during the engine restart",
        body: `All ${plural(n, noun)} were sent. Reconnect to check they loaded.`,
        actions: [],
      };
    case "failed":
      return {
        tone: "error",
        title: send ? "Sending stopped" : "The install stopped",
        body: `${
          sent === 0
            ? "Nothing was sent to the unit."
            : `${String(sent)} of ${String(n)} ${noun}s reached the unit.`
        } ${outcome.stop.message}`,
        actions: rest.length > 0 ? resume : [],
      };
    default:
      break;
  }

  const dropped = outcome.failed_after_restart;
  if (dropped.length > 0) {
    const loaded = n - dropped.length;
    const them = dropped.length === 1 ? "it" : "them";
    const retry: BannerAction = {
      label: send ? "Send Again" : "Try Again",
      disabled: blocked,
      title: why,
      onClick: () =>
        void app.run(
          r.kind,
          dropped.flatMap((d) => {
            const s = app.failed[d];
            return s ? [s] : [];
          }),
        ),
    };
    return {
      tone: "warn",
      title: `${String(loaded)} of ${String(n)} ${noun}s ${send ? "added" : "installed"}`,
      body: `${nameList(dropped)} ${dropped.length === 1 ? "was" : "were"} sent, but the unit didn't load ${them}${outcome.needs_restart ? " after the engine restart" : ""}.${send ? ` ${dropped.length === 1 ? "It won't" : "They won't"} play until this is fixed.` : ""}`,
      actions: send
        ? [
            retry,
            {
              label: dropped.length === 1 ? "Remove It" : "Remove Them",
              disabled: app.busyReason !== null,
              onClick: () => {
                for (const d of dropped) extra.remove?.(d);
              },
            },
          ]
        : [retry, ...open],
    };
  }

  return send
    ? {
        tone: "ok",
        title: `${plural(n, "capture")} added`,
        body: "They appear under User IRs on the unit.",
        actions: [],
      }
    : {
        tone: "ok",
        title: `${plural(n, "capture")} installed`,
        body: "They're on the unit as User IRs, listed in Captures.",
        actions: open,
      };
}
