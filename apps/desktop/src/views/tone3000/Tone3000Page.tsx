// src/views/tone3000/Tone3000Page.tsx — install tones from the user's Tone3000 account.
//
// Account states: no key · signed out · signing in · error · loading · list. Browsing
// never needs the unit; installing does (download → send → engine restart).

import { useEffect, useRef, useState } from "react";
import {
  Banner,
  Button,
  Checkbox,
  ConnectSteps,
  Icon,
  Menu,
  PopupButton,
  ProgressBar,
  SegmentedControl,
  Spinner,
  StatusDot,
  Tag,
  TextField,
  Toolbar,
  type MenuItem,
} from "../../ds";
import { api, type T3kCapture, type T3kTone } from "../../lib/api";
import { defer, plural } from "../../lib/format";
import type { Source } from "../../state/context";
import { useApp } from "../../state/context";
import { installStatus, percent } from "../../state/operation";
import {
  captureKey,
  modelLabel,
  toneKey,
  type Filter,
  type T3kStore,
} from "../../state/t3k";
import { ReselectNote } from "../ReselectNote";
import { remaining, resultBanner } from "../results";
import { allowedLine } from "./allowed";
import { VariantsSheet } from "./Variants";

const FILTERS: { label: string; value: Filter }[] = [
  { label: "All", value: "all" },
  { label: "Bookmarked", value: "bookmark" },
  { label: "Mine", value: "mine" },
];

export function Tone3000Page() {
  const app = useApp();
  const { t3k } = app;
  const [variantsOpen, setVariantsOpen] = useState(false);
  const signed = t3k.account === "list" || t3k.account === "loading";

  const subtitle = signed
    ? `Signed in as ${t3k.username ?? "you"}`
    : t3k.account === "signingin"
      ? "Signing in…"
      : "Not signed in";

  return (
    <>
      <Toolbar title="Tone3000" subtitle={subtitle}>
        {t3k.account === "list" && (
          <>
            <SegmentedControl
              aria-label="Show"
              options={FILTERS}
              value={t3k.filter}
              onChange={t3k.setFilter}
            />
            <Button
              icon="refresh"
              iconOnly
              disabled={app.op?.kind === "install"}
              onClick={() => void t3k.loadTones()}
            >
              Refresh
            </Button>
          </>
        )}
      </Toolbar>

      {t3k.account === "checking" && (
        <div className="center">
          <Spinner size="lg" />
        </div>
      )}
      {t3k.account === "nokey" && <NoKey />}
      {t3k.account === "signedout" && (
        <div className="center">
          <div className="empty" style={{ maxWidth: 440, gap: 16 }}>
            <h2 className="t-title">Sign in to Tone3000</h2>
            <p className="muted">
              Your API key is saved. Sign in to see your bookmarks and your own
              tones.
            </p>
            <Button
              variant="primary"
              size="lg"
              icon="external"
              onClick={() => void t3k.signIn()}
            >
              Sign In with Browser
            </Button>
            <span className="small muted3">
              Opens tone3000.com in your browser. Approve access there, then
              come back.
            </span>
          </div>
        </div>
      )}
      {t3k.account === "signingin" && (
        <div className="center">
          <div
            className="empty"
            style={{ maxWidth: 440, gap: 16 }}
            aria-live="polite"
          >
            <div className="hrow" style={{ gap: 12 }}>
              <Spinner size="lg" />
              <h2 className="t-title">Waiting for you in the browser</h2>
            </div>
            <p className="muted">
              Approve TMP NAM on the Tone3000 page that just opened. This window
              continues on its own once you do.
            </p>
            <div className="hrow">
              <Button onClick={t3k.openAgain}>Open Page Again</Button>
              <Button onClick={t3k.cancelSignIn}>Cancel</Button>
            </div>
          </div>
        </div>
      )}
      {t3k.account === "error" && <AccountError />}
      {t3k.account === "loading" && (
        <div className="center" aria-live="polite">
          <div
            style={{
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 10,
            }}
          >
            <Spinner size="lg" />
            <span style={{ fontWeight: 600 }}>Loading your tones</span>
            <span className="small muted3">
              Bookmarks and your own tones, with the models each one offers
            </span>
          </div>
        </div>
      )}
      {t3k.account === "list" && (
        <ToneList
          onChangeVariants={() => {
            setVariantsOpen(true);
          }}
        />
      )}

      {variantsOpen && (
        <VariantsSheet
          allowed={t3k.settings?.variants ?? []}
          variants={t3k.variants}
          onCancel={() => {
            setVariantsOpen(false);
          }}
          onSave={(ids) => {
            setVariantsOpen(false);
            void t3k.saveAllowed(ids);
          }}
        />
      )}
    </>
  );
}

function NoKey() {
  const { t3k } = useApp();
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="center">
      <div className="empty" style={{ maxWidth: 480, gap: 16 }}>
        <h2 className="t-title">Install tones from your Tone3000 account</h2>
        <p className="muted">
          See your bookmarked tones and the tones you&apos;ve published, and
          send them to the unit in one go. This needs your own Tone3000 API key,
          entered once.
        </p>
        <ConnectSteps
          steps={[
            "On tone3000.com, open Settings › API and create a key.",
            "Paste the key below.",
          ]}
        />
        <TextField
          label="API key"
          mono
          placeholder="Paste your API key"
          value={draft}
          error={error ?? undefined}
          onChange={(e) => {
            setDraft(e.target.value);
            setError(null);
          }}
          help="Stored on this computer. You can change or remove it in Settings."
          style={{ alignSelf: "stretch" }}
        >
          <Button
            variant="primary"
            disabled={!draft.trim()}
            onClick={() => void t3k.saveKey(draft).then(setError)}
          >
            Save Key
          </Button>
        </TextField>
      </div>
    </div>
  );
}

const ERRORS: Record<string, { title: string; body: string }> = {
  key_rejected: {
    title: "Tone3000 didn't accept your API key",
    body: "The key may have been deleted or mistyped. Create a new one on tone3000.com › Settings › API.",
  },
  declined: {
    title: "Access wasn't approved",
    body: "Tone3000 says access to your account was declined. Sign in again and approve TMP NAM.",
  },
  network: {
    title: "Can't reach Tone3000",
    body: "Check this computer's internet connection, then try again.",
  },
  timeout: {
    title: "The sign-in timed out",
    body: "Nothing came back from the browser within 5 minutes. Sign in again.",
  },
};

function AccountError() {
  const { t3k } = useApp();
  const [draft, setDraft] = useState("");
  const [fieldError, setFieldError] = useState<string | null>(null);
  const code = t3k.error?.code ?? "other";
  const copy = ERRORS[code] ?? {
    title: "Tone3000 didn't respond as expected",
    body: t3k.error?.message ?? "Try again.",
  };
  // A network failure while signed in retries the list (signed out, it lands on Sign In).
  const retry =
    code === "network" ? () => void t3k.loadTones() : () => void t3k.signIn();
  return (
    <div className="center">
      <div className="empty" style={{ maxWidth: 460, gap: 16 }}>
        <Banner
          tone="error"
          title={copy.title}
          style={{ alignSelf: "stretch" }}
        >
          {copy.body}
        </Banner>
        {code === "key_rejected" ? (
          <TextField
            label="API key"
            mono
            placeholder="Paste your API key"
            value={draft}
            error={fieldError ?? true}
            onChange={(e) => {
              setDraft(e.target.value);
              setFieldError(null);
            }}
            style={{ alignSelf: "stretch" }}
          >
            <Button
              variant="primary"
              disabled={!draft.trim()}
              onClick={() =>
                void t3k.saveKey(draft).then((err) => {
                  if (err) setFieldError(err);
                  else void t3k.signIn();
                })
              }
            >
              Save and Sign In
            </Button>
          </TextField>
        ) : (
          <Button variant="primary" onClick={retry}>
            {code === "network" ? "Try Again" : "Sign In with Browser"}
          </Button>
        )}
      </div>
    </div>
  );
}

function ToneList({ onChangeVariants }: { onChangeVariants: () => void }) {
  const app = useApp();
  const { t3k } = app;
  const [menuFor, setMenuFor] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set());
  const installing = app.op?.kind === "install" ? app.op : null;
  const busy = app.busyReason !== null;

  // "Show in Tone3000" from a capture opens its tone.
  const { focusTone, clearFocusTone } = app;
  useEffect(() => {
    if (!focusTone) return;
    defer(() => {
      t3k.setFilter("all");
      setExpanded((e) => new Set([...e, focusTone]));
      clearFocusTone();
    });
  }, [focusTone, clearFocusTone, t3k]);

  // Close the open menu on a click outside it.
  const menuRef = useRef<HTMLSpanElement | null>(null);
  useEffect(() => {
    if (!menuFor) return;
    const close = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node))
        setMenuFor(null);
    };
    document.addEventListener("mousedown", close);
    return () => {
      document.removeEventListener("mousedown", close);
    };
  }, [menuFor]);

  const shown = t3k.tones.filter(
    (t) =>
      t3k.filter === "all" ||
      (t3k.filter === "mine"
        ? t.source === "created"
        : t.source === "bookmark"),
  );
  const chosen = t3k.tones.flatMap((t) =>
    t.captures.flatMap((c) => {
      const i = t3k.pickFor(t, c);
      const m = i === null ? undefined : t.models[i];
      return t3k.selected.has(captureKey(t, c)) && m?.model_url
        ? [{ t, c, m, url: m.model_url }]
        : [];
    }),
  );
  const toneCount = new Set(chosen.map((x) => toneKey(x.t))).size;
  const failedCaptures = new Set(
    Object.values(app.failed).flatMap((s) =>
      s.kind === "pick" ? [s.captureKey] : [],
    ),
  );
  const onUnit = (t: T3kTone, c: T3kCapture) =>
    app.captures?.some(
      (u) =>
        u.present &&
        String(u.source?.tone_id) === toneKey(t) &&
        c.models.some(
          (i) => String(t.models[i]?.id) === String(u.source?.model_id),
        ),
    ) ?? false;

  const install = () => {
    const sources: Source[] = chosen.map(({ t, c, m, url }) => ({
      kind: "pick",
      name: m.ir_name,
      label: c.name,
      captureKey: captureKey(t, c),
      pick: {
        model_url: url,
        ir_name: m.ir_name,
        tone_id: t.id,
        model_id: m.id,
        variant: m.variant,
      },
    }));
    void app.run("install", sources);
  };

  const result = app.results.install;
  const banner = result
    ? resultBanner(result, app, {
        openCaptures: () => {
          app.navigate("captures");
        },
      })
    : null;

  if (t3k.tones.length === 0)
    return (
      <div className="center">
        <div className="empty">
          <h2 className="t-title">No tones yet</h2>
          <p className="muted">
            Bookmark tones on tone3000.com, or publish your own, and
            they&apos;ll show up here. Then press Refresh.
          </p>
          <div className="hrow">
            <Button icon="external" onClick={() => void api.t3kOpenSite()}>
              Open tone3000.com
            </Button>
            <Button onClick={() => void t3k.loadTones()}>Refresh</Button>
          </div>
        </div>
      </div>
    );

  /** Installed cell for one capture. */
  const status = (t: T3kTone, c: T3kCapture) => {
    const i = t3k.pickFor(t, c);
    const name = i === null ? undefined : t.models[i]?.ir_name;
    const item = installing?.items.find((x) => x.name === name);
    if (item && installing)
      return (
        <span className="small muted3">{installStatus(installing, item)}</span>
      );
    if (failedCaptures.has(captureKey(t, c)))
      return <Tag tone="danger">Didn&apos;t load</Tag>;
    if (onUnit(t, c)) return <Tag tone="ok">Installed</Tag>;
    if (app.connected) return <Tag>Not installed</Tag>;
    return <span className="small muted3">Unknown</span>;
  };

  /** Model pop-up and menu for one capture. */
  const modelCell = (t: T3kTone, c: T3kCapture) => {
    const key = captureKey(t, c);
    const pick = t3k.pickFor(t, c);
    const auto = t3k.autoFor(t, c);
    const model = pick === null ? undefined : t.models[pick];
    return (
      <span className="mcell" ref={menuFor === key ? menuRef : undefined}>
        <PopupButton
          tone={model ? "default" : "warn"}
          marked={t3k.overrides[key] !== undefined}
          open={menuFor === key}
          disabled={installing !== null}
          title={model ? modelLabel(model, t3k.variants) : undefined}
          onClick={() => {
            setMenuFor(menuFor === key ? null : key);
          }}
        >
          {model ? modelLabel(model, t3k.variants) : "No allowed model"}
        </PopupButton>
        {menuFor === key && (
          <span className="menu-pop">
            <Menu
              width={280}
              items={menuItems(
                t3k,
                t,
                c,
                pick,
                auto,
                (i) => {
                  t3k.setOverride(key, i === auto ? null : i);
                  setMenuFor(null);
                },
                () => {
                  setMenuFor(null);
                  onChangeVariants();
                },
              )}
            />
          </span>
        )}
      </span>
    );
  };

  return (
    <>
      <div className="strip">
        <span className="muted">Picking from your allowed variants:</span>
        <span>{allowedLine(t3k.settings?.variants ?? [], t3k.variants)}</span>
        <span style={{ marginLeft: "auto" }}>
          <Button
            variant="plain"
            size="sm"
            disabled={installing !== null}
            onClick={onChangeVariants}
          >
            Change…
          </Button>
        </span>
      </div>
      {banner && (
        <div className="bwrap">
          <Banner
            tone={banner.tone}
            title={banner.title}
            actions={banner.actions}
            onDismiss={
              result && remaining(result).length > 0
                ? undefined
                : () => {
                    app.dismissResult("install");
                  }
            }
          >
            {banner.body}
          </Banner>
        </div>
      )}
      <ReselectNote />
      {installing && (
        <section className="panel" aria-live="polite">
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <span className="panel-title">
              Installing {plural(installing.items.length, "capture")}
            </span>
            <span className="small muted">
              {installing.phase === "download"
                ? "Step 1 of 3 · downloading from Tone3000"
                : installing.phase === "send"
                  ? "Step 2 of 3 · sending to the unit"
                  : "Step 3 of 3 · restarting the audio engine, a few seconds"}
            </span>
          </div>
          <ProgressBar value={percent(installing)} />
        </section>
      )}
      <div className="tgrid thead2">
        <span />
        <span>Tone</span>
        <span>Model</span>
        <span>Installed</span>
      </div>
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto" }}>
        {shown.map((t) => {
          const tk = toneKey(t);
          const caps = t.captures;
          const keys = caps.map((c) => captureKey(t, c));
          const selectable = caps
            .filter((c) => t3k.pickFor(t, c) !== null)
            .map((c) => captureKey(t, c));
          const picked = keys.filter((k) => t3k.selected.has(k)).length;
          const all = selectable.length > 0 && picked === selectable.length;
          const single = caps.length === 1 ? caps[0] : undefined;
          const open = expanded.has(tk);
          const a1Only = !t.models.some((m) => m.architecture_version === "2");
          const loaded = caps.filter((c) => onUnit(t, c)).length;
          const sub = [
            `by ${t.author ?? "unknown"}`,
            single ? null : plural(caps.length, "capture"),
            !single && picked > 0 ? `${String(picked)} selected` : null,
            a1Only ? "A1 only" : null,
          ]
            .filter(Boolean)
            .join(" · ");
          return (
            <div key={tk}>
              <div
                className={[
                  "tgrid",
                  "trow",
                  picked > 0 && "sel",
                  selectable.length === 0 && "dim",
                ]
                  .filter(Boolean)
                  .join(" ")}
              >
                <Checkbox
                  aria-label={`Select ${t.title}`}
                  checked={all}
                  indeterminate={picked > 0 && !all}
                  disabled={selectable.length === 0 || installing !== null}
                  onChange={() => {
                    t3k.setSelected(single ? keys : selectable, !all);
                  }}
                />
                <span style={{ minWidth: 0 }}>
                  <span className="nm">
                    {!single && (
                      <button
                        type="button"
                        className="disclose"
                        aria-expanded={open}
                        aria-label={`${open ? "Hide" : "Show"} captures of ${t.title}`}
                        onClick={() => {
                          setExpanded((e) => {
                            const n = new Set(e);
                            if (open) n.delete(tk);
                            else n.add(tk);
                            return n;
                          });
                        }}
                      >
                        <Icon
                          name="chevron"
                          size={12}
                          style={{
                            transform: open ? undefined : "rotate(-90deg)",
                          }}
                        />
                      </button>
                    )}
                    <span className="nm-text">{t.title}</span>
                  </span>
                  <span className="sub">{sub}</span>
                </span>
                {single ? (
                  modelCell(t, single)
                ) : (
                  <span className="small muted3">
                    {plural(selectable.length, "capture")} allowed
                  </span>
                )}
                <span>
                  {single ? (
                    status(t, single)
                  ) : loaded > 0 || !app.connected ? (
                    <span className="small muted3">
                      {loaded > 0
                        ? `${String(loaded)} of ${String(caps.length)}`
                        : "Unknown"}
                    </span>
                  ) : (
                    <Tag>Not installed</Tag>
                  )}
                </span>
              </div>
              {!single &&
                open &&
                caps.map((c) => {
                  const key = captureKey(t, c);
                  const can = t3k.pickFor(t, c) !== null;
                  const on = t3k.selected.has(key) && can;
                  return (
                    <div
                      key={key}
                      className={[
                        "tgrid",
                        "trow",
                        "crow",
                        on && "sel",
                        !can && "dim",
                      ]
                        .filter(Boolean)
                        .join(" ")}
                    >
                      <Checkbox
                        aria-label={`Select ${c.name}`}
                        checked={on}
                        disabled={!can || installing !== null}
                        onChange={() => {
                          t3k.toggle(key);
                        }}
                      />
                      <span className="nm crow-name">
                        <span className="nm-text">{c.name}</span>
                      </span>
                      {modelCell(t, c)}
                      <span>{status(t, c)}</span>
                    </div>
                  );
                })}
            </div>
          );
        })}
      </div>
      <div
        className="footer"
        style={{ minHeight: 56, background: "var(--bg-content)" }}
      >
        {!app.connected && !installing ? (
          <>
            <StatusDot tone="off" />
            <div style={{ display: "flex", flexDirection: "column" }}>
              <span style={{ color: "var(--text)", fontWeight: 600 }}>
                Connect the unit to install
              </span>
              <span>
                Boot from the NAM SD card, wait for the preset screen, plug in
                USB-C. Your selection is kept.
              </span>
            </div>
          </>
        ) : installing ? (
          <span>
            Keep the USB cable connected. Selection and models are paused until
            this finishes.
          </span>
        ) : busy ? (
          <span>{app.busyReason}.</span>
        ) : (
          <>
            <span style={{ color: "var(--text)", fontWeight: 600 }}>
              {chosen.length
                ? `${plural(chosen.length, "capture")} from ${plural(toneCount, "tone")}`
                : "Nothing selected"}
            </span>
            {!chosen.length && <span> · select the captures to install</span>}
          </>
        )}
        <span style={{ flex: 1 }} />
        <Button
          disabled={!chosen.length || installing !== null}
          onClick={t3k.clearSelection}
        >
          Clear
        </Button>
        <Button
          variant="primary"
          disabled={
            !app.connected || busy || !chosen.length || app.unit === "busy"
          }
          onClick={install}
        >
          {chosen.length
            ? `Install ${String(chosen.length)} on Unit`
            : "Install on Unit"}
        </Button>
      </div>
    </>
  );
}

function menuItems(
  t3k: T3kStore,
  t: T3kTone,
  c: T3kCapture,
  pick: number | null,
  auto: number | null,
  choose: (i: number) => void,
  changeVariants: () => void,
): MenuItem[] {
  const rank = (i: number) => {
    const m = t.models[i];
    const v = t3k.variants.find((x) => x.id === m?.variant);
    return v ? (v.arch === 2 ? 0 : 100) + v.rank : 1000;
  };
  const indices = [...c.models].sort((a, b) => rank(a) - rank(b));
  const allowedIds = t3k.settings?.variants ?? [];
  const isAllowed = (i: number) => {
    const m = t.models[i];
    return (
      m?.model_url != null &&
      m.variant != null &&
      allowedIds.includes(m.variant)
    );
  };
  const allowed = indices.filter(isAllowed);
  const other = indices.filter((i) => !isAllowed(i));
  const label = (i: number) => {
    const m = t.models[i];
    return m ? modelLabel(m, t3k.variants) : "";
  };
  const items: MenuItem[] = [];
  if (allowed.length === 0)
    items.push({
      label: "Nothing it offers is in your allowed variants",
      disabled: true,
    });
  for (const i of allowed)
    items.push({
      label: label(i),
      value: String(i),
      checked: i === pick,
      note: i === auto ? "recommended" : undefined,
      onClick: () => {
        choose(i);
      },
    });
  if (other.length > 0) {
    items.push(
      { type: "separator" },
      { type: "header", label: "Not in your allowed variants" },
    );
    for (const i of other)
      items.push({ label: label(i), value: `x${String(i)}`, disabled: true });
  }
  items.push(
    { type: "separator" },
    { label: "Change Allowed Variants…", link: true, onClick: changeVariants },
  );
  return items;
}
