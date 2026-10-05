// src/views/settings/SettingsPage.tsx — Tone3000 key and account, allowed variants, unit details.

import { useState } from "react";
import {
  Banner,
  Button,
  ConnectSteps,
  SegmentedControl,
  StatusDot,
  TextField,
  Toolbar,
} from "../../ds";
import { api } from "../../lib/api";
import { copyText } from "../../lib/format";
import { useApp } from "../../state/context";
import { toggled } from "../tone3000/allowed";
import { VariantColumns } from "../tone3000/Variants";

type Section = "t3k" | "variants" | "unit";

const SECTIONS: { label: string; value: Section }[] = [
  { label: "Tone3000", value: "t3k" },
  { label: "Allowed Variants", value: "variants" },
  { label: "Unit", value: "unit" },
];

export function SettingsPage() {
  const [section, setSection] = useState<Section>("t3k");
  return (
    <>
      <Toolbar title="Settings">
        <SegmentedControl
          aria-label="Settings section"
          options={SECTIONS}
          value={section}
          onChange={setSection}
        />
      </Toolbar>
      <div style={{ padding: "12px 40px", maxWidth: 680, overflowY: "auto" }}>
        {section === "t3k" && <Tone3000Settings />}
        {section === "variants" && <VariantSettings />}
        {section === "unit" && <UnitSettings />}
      </div>
    </>
  );
}

function Tone3000Settings() {
  const app = useApp();
  const { t3k } = app;
  const key = t3k.settings?.t3k_key ?? "";
  const [replacing, setReplacing] = useState(false);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const signed = t3k.account === "list" || t3k.account === "loading";

  return (
    <>
      <div className="formrow">
        <span className="lbl">API key</span>
        {key && !replacing ? (
          <TextField
            mono
            value={`••••••••••••••••${key.slice(-4)}`}
            disabled
            aria-label="API key"
            help="Your own key from tone3000.com › Settings › API. Stored on this computer. Removing it also signs you out."
          >
            <Button
              onClick={() => {
                setReplacing(true);
              }}
            >
              Replace…
            </Button>
            <Button destructiveText onClick={() => void t3k.removeKey()}>
              Remove
            </Button>
          </TextField>
        ) : replacing ? (
          <TextField
            mono
            aria-label="API key"
            placeholder="Paste your API key"
            value={draft}
            error={error ?? undefined}
            onChange={(e) => {
              setDraft(e.target.value);
              setError(null);
            }}
          >
            <Button
              onClick={() => {
                setReplacing(false);
                setDraft("");
              }}
            >
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={!draft.trim()}
              onClick={() =>
                void t3k.saveKey(draft).then((err) => {
                  setError(err);
                  if (!err) {
                    setReplacing(false);
                    setDraft("");
                  }
                })
              }
            >
              Save Key
            </Button>
          </TextField>
        ) : (
          <div className="hrow">
            <span className="muted">No key saved.</span>
            <Button
              onClick={() => {
                app.navigate("tone3000");
              }}
            >
              Add Key…
            </Button>
          </div>
        )}
      </div>
      <div className="formrow">
        <span className="lbl">Account</span>
        <div className="hrow" style={{ gap: 10 }}>
          <StatusDot
            tone={signed ? "ok" : "off"}
            label={
              signed
                ? `Signed in as ${t3k.username ?? "you"}`
                : t3k.account === "signingin"
                  ? "Signing in…"
                  : "Not signed in"
            }
          />
          <span style={{ marginLeft: "auto" }}>
            {signed && (
              <Button onClick={() => void t3k.signOut()}>Sign Out</Button>
            )}
            {(t3k.account === "signedout" || t3k.account === "error") && (
              <Button
                onClick={() => {
                  app.navigate("tone3000");
                  void t3k.signIn();
                }}
              >
                Sign In…
              </Button>
            )}
          </span>
        </div>
      </div>
    </>
  );
}

function VariantSettings() {
  const { t3k } = useApp();
  const allowed = t3k.settings?.variants ?? [];
  return (
    <>
      <div className="formrow">
        <span className="lbl">Sync may download</span>
        <VariantColumns
          allowed={allowed}
          variants={t3k.variants}
          onToggle={(id) => void t3k.saveAllowed(toggled(allowed, id))}
        />
      </div>
      <div className="formrow">
        <span className="lbl">Which one is picked</span>
        <span className="small muted" style={{ paddingTop: 5 }}>
          If a tone has several allowed models, A2 wins over A1, then the
          largest allowed size. You can override it per tone on the Tone3000
          page.
        </span>
      </div>
      <div className="formrow" style={{ borderBottom: 0 }}>
        <span className="lbl">Before playing live</span>
        <Banner tone="warn">
          Size labels don&apos;t guarantee a capture runs without glitches on
          the unit. Check each one by ear.
        </Banner>
      </div>
    </>
  );
}

function UnitSettings() {
  const app = useApp();
  const info = app.connected ? app.unitInfo : null;
  const [copyError, setCopyError] = useState<string | null>(null);
  const status =
    app.unit === "connected"
      ? `Connected over USB · NAM card${info?.simulated ? " (simulator)" : ""}`
      : app.unit === "busy"
        ? "Connected · engine restarting"
        : app.unit === "looking"
          ? "Looking for the unit…"
          : "Not connected";
  const [copied, setCopied] = useState(false);
  const copyDiagnostics = async () => {
    const err = await copyText(api.diagnostics());
    setCopyError(err);
    setCopied(err === null);
  };

  return (
    <>
      <div className="formrow">
        <span className="lbl">Connection</span>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: 10,
            paddingTop: 5,
          }}
        >
          <StatusDot
            tone={
              app.unit === "connected"
                ? "ok"
                : app.unit === "busy"
                  ? "warn"
                  : "off"
            }
            label={status}
          />
          {app.unit === "missing" && <ConnectSteps />}
        </div>
      </div>
      <div className="formrow" style={{ borderBottom: 0 }}>
        <span className="lbl">Diagnostics</span>
        <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
          <div className="hrow">
            <Button onClick={() => void copyDiagnostics()}>
              Copy Diagnostics
            </Button>
            {copied && <span className="small muted3">Copied</span>}
          </div>
          {copyError && (
            <span className="small" style={{ color: "var(--danger)" }}>
              Couldn&apos;t copy: {copyError}
            </span>
          )}
          <span className="small muted3">
            App and unit versions plus the recent log, to paste into a bug
            report. Your API key isn&apos;t included.
          </span>
        </div>
      </div>
    </>
  );
}
