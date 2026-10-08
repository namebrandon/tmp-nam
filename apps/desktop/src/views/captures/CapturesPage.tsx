// src/views/captures/CapturesPage.tsx — NAM captures on the unit.
//
// States: looking for the unit · not found (connect steps) · interrupted send
// waiting for the unit · no captures · the table with the inspector (a sheet below
// 820 px). Sending runs in the app store, so it continues on other pages.

import { useEffect, useState } from "react";
import {
  Banner,
  Button,
  ConnectSteps,
  Icon,
  ProgressBar,
  Sheet,
  Spinner,
  StatusDot,
  Tag,
  Toolbar,
} from "../../ds";
import { api, NAM_FILTER, onFileDrop, pickFiles } from "../../lib/api";
import {
  displayName,
  formatBytes,
  gearLine,
  plural,
  archTag,
} from "../../lib/format";
import { useApp } from "../../state/context";
import { itemDetail } from "../../state/operation";
import { ReselectNote } from "../ReselectNote";
import { remaining, resultBanner } from "../results";
import { AddSheet } from "./AddSheet";
import { CabTag } from "./CabTag";
import { Inspector } from "./Inspector";
import { captureRows, FLAG_TEXT, type Row } from "./rows";

type SheetState =
  | { kind: "add"; paths: string[] }
  | { kind: "remove"; name: string }
  | { kind: "inspector" }
  | null;

export function CapturesPage({ narrow }: { narrow: boolean }) {
  const app = useApp();
  const [sheet, setSheet] = useState<SheetState>(null);
  const [selected, setSelected] = useState<string | null>(null);
  const [removeError, setRemoveError] = useState<string | null>(null);

  const listed = app.captures !== null;
  const rows = app.captures
    ? captureRows(app.captures, app.failed, app.marks)
    : [];
  const sel = rows.find((r) => r.name === selected) ?? rows[0] ?? null;
  const canAdd =
    app.connected && app.busyReason === null && app.unit !== "busy";
  const addHint = !app.connected
    ? "Connect the unit first"
    : (app.busyReason ?? undefined);

  const openAdd = async () => {
    const paths = await pickFiles(NAM_FILTER, true);
    if (paths?.length) setSheet({ kind: "add", paths });
  };

  // Drop .nam files anywhere on the window while this page shows.
  useEffect(
    () =>
      onFileDrop((paths) => {
        const nam = paths.filter((p) => p.toLowerCase().endsWith(".nam"));
        if (nam.length > 0) setSheet({ kind: "add", paths: nam });
      }),
    [],
  );

  const restore = async (row: Row) => {
    const paths = await pickFiles(NAM_FILTER, false);
    const path = paths?.[0];
    if (!path) return;
    const [f] = await api.namInspect([path]);
    if (!f || f.error) {
      setRemoveError("That file isn't a valid .nam capture.");
      return;
    }
    void app.run("send", [
      {
        kind: "file",
        path,
        name: row.name,
        label: displayName(row.name),
        bytes: f.bytes,
      },
    ]);
  };

  const result = app.results.send;
  const banner = result
    ? resultBanner(result, app, {
        remove: (name) => {
          setSelected(name);
          setSheet({ kind: "remove", name });
        },
      })
    : null;
  const waitingToResume =
    !app.connected &&
    result?.outcome.stop?.kind === "disconnected" &&
    remaining(result).length > 0;

  const totalBytes = (app.captures ?? []).reduce(
    (a, c) => a + (c.present ? c.bytes : 0),
    0,
  );
  const subtitle = listed
    ? narrow
      ? String(rows.length)
      : `${String(app.captures?.length ?? 0)} on unit · ${formatBytes(totalBytes)}`
    : app.unit === "looking"
      ? "Looking for the unit…"
      : "Unit not connected";

  const flagged = rows.filter((r) => r.flag).length;
  const changed = rows.filter((r) => app.marks[r.name]?.changed).length;
  const sending = app.op?.kind === "send";
  const footer = sending
    ? "Size, gain and removal are paused until the transfer finishes."
    : flagged > 0
      ? `${flagged === 1 ? "1 capture needs" : `${String(flagged)} captures need`} attention · select one to fix it`
      : changed > 0
        ? "• Changed since last selected on the unit"
        : plural(rows.length, "capture");

  const remove = async (name: string) => {
    setSheet(null);
    const err = await app.unitCommand(() => api.unitRemove([name]));
    setRemoveError(err);
    if (!err) {
      app.forgetFailed(name);
      if (app.results.send?.outcome.failed_after_restart.includes(name))
        app.dismissResult("send");
      const i = rows.findIndex((r) => r.name === name);
      const next = rows.filter((r) => r.name !== name);
      setSelected(
        next[Math.min(Math.max(i, 0), next.length - 1)]?.name ?? null,
      );
    }
  };

  const inspector = sel && (
    <Inspector
      key={sel.name}
      row={sel}
      onRemove={() => {
        setSheet({ kind: "remove", name: sel.name });
      }}
      onRestore={() => void restore(sel)}
    />
  );

  return (
    <>
      <Toolbar title="Captures" subtitle={subtitle}>
        {narrow && listed && rows.length > 0 && (
          <Button
            disabled={!sel}
            onClick={() => {
              setSheet({ kind: "inspector" });
            }}
          >
            Size…
          </Button>
        )}
        <Button
          variant="primary"
          icon="add"
          iconOnly={narrow}
          disabled={!canAdd}
          title={addHint}
          onClick={() => void openAdd()}
        >
          {narrow ? "Add Captures" : "Add Captures…"}
        </Button>
      </Toolbar>

      {banner && !waitingToResume && (
        <div className="bwrap">
          <Banner
            tone={banner.tone}
            title={banner.title}
            actions={banner.actions}
            onDismiss={
              result && remaining(result).length > 0
                ? undefined
                : () => {
                    app.dismissResult("send");
                  }
            }
          >
            {banner.body}
          </Banner>
        </div>
      )}
      {app.settingsError && (
        <div className="bwrap">
          <Banner
            tone="warn"
            title="Player settings unavailable"
            actions={[
              {
                label: "Refresh settings",
                onClick: () => void app.refreshCaptures(),
                disabled: !canAdd,
              },
            ]}
          >
            {app.settingsError} Listed options may be unavailable or invalid.
            Refresh after checking the settings file. Size changes preserve any
            gain that can be read when saving.
          </Banner>
        </div>
      )}
      {removeError && (
        <div className="bwrap">
          <Banner
            tone="error"
            title="That didn't work"
            onDismiss={() => {
              setRemoveError(null);
            }}
          >
            {removeError}
          </Banner>
        </div>
      )}

      <ReselectNote />
      {sending && app.op && <SendPanel />}

      {waitingToResume ? (
        <Interrupted />
      ) : !listed ? (
        <div className="center">
          <div className="empty">
            {app.unit === "looking" ? (
              <>
                <div className="hrow" style={{ gap: 12 }}>
                  <Spinner size="lg" />
                  <h2 className="t-title">Looking for the Tone Master Pro…</h2>
                </div>
                <p className="muted">
                  This takes a few seconds after you plug it in. If the unit
                  isn&apos;t connected yet:
                </p>
              </>
            ) : (
              <>
                <Icon name="usb" size={28} style={{ color: "var(--text-3)" }} />
                <h2 className="t-title">Connect the Tone Master Pro</h2>
                <p className="muted">
                  Captures are read from the unit, so this page needs it.
                </p>
              </>
            )}
            <ConnectSteps />
            <p className="small muted3">
              It&apos;s detected automatically. Nothing to click here.
            </p>
            {app.unit !== "looking" && (
              <p className="small muted3">
                No NAM SD card yet?{" "}
                <Button
                  variant="plain"
                  size="sm"
                  onClick={() => {
                    app.navigate("sdcard");
                  }}
                >
                  Make an SD Card
                </Button>
              </p>
            )}
          </div>
        </div>
      ) : rows.length === 0 ? (
        <div className="center">
          <div className="empty">
            <h2 className="t-title">No captures on the unit</h2>
            <p className="muted">
              Captures you add show up on the Tone Master Pro as User IRs, ready
              to use in any preset. You can also drop .nam files onto this
              window.
            </p>
            <div className="hrow">
              <Button
                variant="primary"
                disabled={!canAdd}
                onClick={() => void openAdd()}
              >
                Add .nam Files…
              </Button>
              <Button
                onClick={() => {
                  app.navigate("tone3000");
                }}
              >
                Install from Tone3000
              </Button>
            </div>
          </div>
        </div>
      ) : (
        <div className="body">
          <div className="list">
            <div className="thead">
              <span>Name</span>
              <span>Type</span>
              <span className="r">Size</span>
            </div>
            {rows.map((r) => (
              <CaptureRow
                key={r.name}
                row={r}
                selected={r.name === sel?.name}
                onSelect={() => {
                  setSelected(r.name);
                }}
              />
            ))}
            <div className="footer">
              {flagged > 0 && !sending && <StatusDot tone="warn" />}
              {footer}
            </div>
          </div>
          {!narrow && (
            <aside className="inspector" aria-label="Capture details">
              {inspector}
            </aside>
          )}
        </div>
      )}

      {sheet?.kind === "add" && (
        <AddSheet
          paths={sheet.paths}
          blocked={canAdd ? null : (addHint ?? "Not available now")}
          onCancel={() => {
            setSheet(null);
          }}
          onSend={(sources) => {
            setSheet(null);
            void app.run("send", sources);
          }}
        />
      )}
      {sheet?.kind === "remove" && (
        <Sheet
          title={`Remove “${displayName(sheet.name)}” from the unit?`}
          width={440}
          alert
          onClose={() => {
            setSheet(null);
          }}
          actions={[
            {
              label: "Cancel",
              autoFocus: true,
              onClick: () => {
                setSheet(null);
              },
            },
            {
              label: "Remove",
              variant: "danger",
              onClick: () => void remove(sheet.name),
            },
          ]}
        >
          <p className="muted">
            Presets that use this capture will lose it. On the unit, you&apos;ll
            need to pick another IR in those presets.
          </p>
          <p className="small muted3">
            To get it back later, add the .nam file again. Its size setting is
            not kept.
          </p>
        </Sheet>
      )}
      {sheet?.kind === "inspector" && sel && (
        <Sheet
          title="Size"
          width={340}
          onClose={() => {
            setSheet(null);
          }}
          actions={[
            {
              label: "Done",
              variant: "primary",
              onClick: () => {
                setSheet(null);
              },
            },
          ]}
        >
          <div
            className="inspector"
            style={{ width: "auto", border: 0, margin: "-8px -24px" }}
          >
            {inspector}
          </div>
        </Sheet>
      )}
    </>
  );
}

function CaptureRow({
  row,
  selected,
  onSelect,
}: {
  row: Row;
  selected: boolean;
  onSelect: () => void;
}) {
  const app = useApp();
  const { capture: c, flag } = row;
  const mark = app.marks[row.name] ?? {};
  return (
    <button
      type="button"
      className={["row", selected && "sel", flag === "missing" && "dim"]
        .filter(Boolean)
        .join(" ")}
      aria-pressed={selected}
      onClick={onSelect}
    >
      <span style={{ minWidth: 0 }}>
        <span className="nm">
          <span className="nm-text">{displayName(row.name)}</span>
          {mark.isNew && <Tag tone="accent">New</Tag>}
        </span>
        {flag ? (
          <span className="sub">
            <Tag tone={flag === "unreg" ? "warn" : "danger"}>
              {FLAG_TEXT[flag]}
            </Tag>
          </span>
        ) : (
          <span className="sub">
            {gearLine(c?.info) ?? "No gear details in file"}
          </span>
        )}
      </span>
      <span
        className="tags"
        style={{ flexWrap: "nowrap", alignItems: "center" }}
      >
        {c?.info ? (
          <Tag tone="arch">{archTag(c)}</Tag>
        ) : (
          <Tag tone="arch">—</Tag>
        )}
        <CabTag info={c?.info} />
        {mark.changed && <span className="chg"> •</span>}
      </span>
      <span className="r">
        {c?.present && c.bytes > 0 ? formatBytes(c.bytes) : "—"}
      </span>
    </button>
  );
}

function SendPanel() {
  const { op } = useApp();
  if (!op) return null;
  const restart = op.phase === "restart";
  const total = op.items.reduce((a, i) => a + i.total, 0);
  const done = op.items.reduce((a, i) => a + i.done, 0);
  const title = restart
    ? "Restarting the unit's audio engine"
    : `Sending ${plural(op.items.length, "capture")} to the unit`;
  return (
    <section className="panel" aria-live="polite">
      <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
        <span className="panel-title">{title}</span>
        <span className="small muted">
          {restart
            ? "All sent. The unit is silent for a few seconds while it loads them."
            : "Keep the USB cable connected. You can switch pages; the transfer keeps going."}
        </span>
      </div>
      <ProgressBar
        value={
          restart ? undefined : Math.round((done / Math.max(1, total)) * 100)
        }
        label={undefined}
      />
      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        {op.items.map((it) => (
          <div className="items" key={it.name}>
            <span style={{ display: "flex", alignItems: "center" }}>
              {it.state === "sent" && (
                <Icon name="check" style={{ color: "var(--ok-dot)" }} />
              )}
              {it.state === "sending" && <Spinner />}
              {it.state === "waiting" && <StatusDot tone="off" />}
            </span>
            <span
              style={{
                fontWeight: it.state === "sending" ? 600 : undefined,
                color: it.state === "waiting" ? "var(--text-3)" : undefined,
              }}
            >
              {it.label}
            </span>
            <span className="tr muted3">{itemDetail(it)}</span>
          </div>
        ))}
      </div>
    </section>
  );
}

/** The unit left mid-send: what reached it, and how to finish (design/screens/Cap-Disconnected). */
function Interrupted() {
  const app = useApp();
  const result = app.results.send;
  if (!result) return null;
  const banner = resultBanner(result, app, {});
  const rest = remaining(result);
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        gap: 20,
        padding: "24px 32px",
        maxWidth: 640,
        overflowY: "auto",
      }}
    >
      <Banner tone="error" title={banner.title}>
        {banner.body}
      </Banner>
      <div className="cardx">
        <div className="card-h">
          <span className="t-head">This transfer</span>
        </div>
        <div
          style={{
            display: "grid",
            gridTemplateColumns: "18px 1fr auto",
            gap: 10,
            alignItems: "center",
            padding: "12px 16px",
          }}
        >
          {result.items.map((it) => {
            const interrupted = it.name === result.outcome.interrupted;
            const sent = result.outcome.added.includes(it.name);
            return [
              <span key={`${it.name}-i`} style={{ display: "flex" }}>
                {sent ? (
                  <Icon name="check" style={{ color: "var(--ok-dot)" }} />
                ) : interrupted ? (
                  <Icon name="close" style={{ color: "var(--danger)" }} />
                ) : (
                  <StatusDot tone="off" />
                )}
              </span>,
              <span
                key={`${it.name}-n`}
                className={sent || interrupted ? undefined : "muted"}
              >
                {it.label}
              </span>,
              <span
                key={`${it.name}-s`}
                className="small"
                style={{
                  color: interrupted ? "var(--danger)" : "var(--text-3)",
                }}
              >
                {sent
                  ? "Sent"
                  : interrupted
                    ? `Interrupted at ${String(Math.round((it.done / Math.max(1, it.total)) * 100))}%`
                    : "Not sent"}
              </span>,
            ];
          })}
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
        <span className="t-head">
          Reconnect to send the other {rest.length}
        </span>
        <ConnectSteps
          steps={[
            "Check the unit still shows the preset screen. If it restarted, wait for it.",
            "Plug the USB-C cable back in. Try a different port if it keeps dropping.",
          ]}
        />
        <div className="hrow" style={{ marginTop: 4 }}>
          {banner.actions.map((a) => (
            <Button
              key={a.label}
              variant={a.variant}
              disabled={a.disabled}
              title={a.title}
              onClick={a.onClick}
            >
              {a.label}
            </Button>
          ))}
          <span className="small muted3 hrow" style={{ gap: 6 }}>
            <Spinner />
            Waiting for the unit
          </span>
        </div>
      </div>
    </div>
  );
}
