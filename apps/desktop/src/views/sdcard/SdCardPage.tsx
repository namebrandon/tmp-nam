// src/views/sdcard/SdCardPage.tsx — build the bootable NAM SD card.
//
// Form (prerequisites, firmware file, card) → confirm sheet → administrator prompt
// (the system's) → progress → success or failure. The build runs in the app store,
// so the user can use other pages meanwhile. There is no Stop: the builder has no
// abort path (design/HANDOFF.md, open question 8).

import { useState } from "react";
import {
  Banner,
  Button,
  ConnectSteps,
  Icon,
  ProgressBar,
  Radio,
  Sheet,
  Spinner,
  StageList,
  Tag,
  Toolbar,
  type Stage,
} from "../../ds";
import { api, type Disk } from "../../lib/api";
import { copyText, errorText, formatBytes, plural } from "../../lib/format";
import { useApp } from "../../state/context";
import { CARD_PLATFORMS, FAILURES, STAGES } from "../../state/sd";

const FIRMWARE = "ToneMasterPro_v1_8_58.img";
const IS_MAC = navigator.userAgent.includes("Mac");

/** Card-builder refusals (`tmp_sdcard::card`) in the guitarist's words. */
function refusal(reason: string): string {
  const r = reason.toLowerCase();
  if (r.includes("too small")) return "Too small for the card";
  if (r.includes("write-protected")) return "Locked: slide the lock switch up";
  if (r.includes("card reader") || r.includes("usb/mmc"))
    return "Not a card reader";
  if (r.includes("startup disk")) return "This computer's startup disk";
  if (r.includes("fender")) return "The Tone Master Pro's own storage";
  if (r.includes("removable") || r.includes("physical"))
    return "Not a removable card";
  return "Can't be used";
}

function diskDetail(d: Disk): string {
  const bus =
    d.protocol === "Secure Digital" ? "SD slot" : d.protocol || "Unknown bus";
  return `${formatBytes(d.bytes)} · ${bus}`;
}

/** Packages that provide each build tool (docs/sd-card.md): Homebrew, then Debian/Ubuntu. */
const PACKAGES: Record<string, [string, string]> = {
  unsquashfs: ["squashfs", "squashfs-tools"],
  mke2fs: ["e2fsprogs", "e2fsprogs"],
  debugfs: ["e2fsprogs", "e2fsprogs"],
  e2fsck: ["e2fsprogs", "e2fsprogs"],
  mformat: ["mtools", "mtools"],
  mcopy: ["mtools", "mtools"],
  sfdisk: ["util-linux", "fdisk"],
};

/** Install hint for the missing build tools on this platform. */
function toolsHint(
  platform: string,
  missing: string[],
): { text: string; command: string | null } {
  const pkgs = (col: 0 | 1) => [
    ...new Set(
      missing
        .map((t) => PACKAGES[t]?.[col])
        .filter((p): p is string => Boolean(p)),
    ),
  ];
  if (platform === "macos")
    return {
      text: "Install them with Homebrew in Terminal:",
      command: `brew install ${pkgs(0).join(" ")}`,
    };
  if (platform === "linux")
    return {
      text: "Install them with your package manager, for example:",
      command: `sudo apt install ${pkgs(1).join(" ")}`,
    };
  return { text: "Building cards needs macOS or Linux.", command: null };
}

export function SdCardPage() {
  const app = useApp();
  const { sd } = app;
  const [confirm, setConfirm] = useState(false);
  const disk = sd.disks?.find((d) => d.device === sd.disk) ?? null;

  const subtitle =
    sd.phase === "running" || sd.phase === "failed"
      ? disk
        ? `${disk.name} · ${formatBytes(disk.bytes)}`
        : "Creating the NAM card"
      : "Make a card that boots the unit with NAM support";

  return (
    <>
      <Toolbar title="SD Card" subtitle={subtitle}>
        {sd.phase !== "setup" && (
          <Button variant="plain" onClick={sd.toggleLog}>
            {sd.showLog ? "Hide Log" : "Show Log"}
          </Button>
        )}
      </Toolbar>
      {sd.phase === "setup" && (
        <Form
          onCreate={() => {
            setConfirm(true);
          }}
        />
      )}
      {sd.phase === "running" && <Running />}
      {sd.phase === "success" && <Success />}
      {sd.phase === "failed" && (
        <Failed
          onRetry={() => {
            sd.reset(true);
            setConfirm(true);
          }}
        />
      )}

      {confirm && disk && (
        <Sheet
          title="Erase this card and create the NAM card?"
          width={460}
          alert
          onClose={() => {
            setConfirm(false);
          }}
          actions={[
            {
              label: "Cancel",
              autoFocus: true,
              onClick: () => {
                setConfirm(false);
              },
            },
            {
              label: "Erase and Create",
              variant: "danger",
              onClick: () => {
                setConfirm(false);
                void sd.start();
              },
            },
          ]}
        >
          <div
            className="cardx card-b"
            style={{ gap: 4, background: "var(--bg-subtle)" }}
          >
            <span style={{ fontWeight: 600 }}>
              {disk.name || "Unnamed card"}
            </span>
            <span className="small muted">
              {diskDetail(disk)} ·{" "}
              <span className="mono" style={{ fontSize: 11 }}>
                {disk.device}
              </span>
            </span>
          </div>
          <p className="muted">
            Everything on this card will be deleted. Check it&apos;s the card
            you meant: other disks are never touched.
          </p>
          <p className="small muted3">
            Next, {IS_MAC ? "macOS" : "the system"} asks for your administrator
            password. Writing then takes several minutes. Keep the card in and
            the computer awake.
          </p>
        </Sheet>
      )}
    </>
  );
}

function Form({ onCreate }: { onCreate: () => void }) {
  const app = useApp();
  const { sd } = app;
  const env = sd.env;
  const unsupported = env !== null && !CARD_PLATFORMS.includes(env.platform);
  const missing = unsupported ? [] : (env?.tools.filter((t) => !t.path) ?? []);
  const blocked = unsupported || missing.length > 0 || Boolean(env?.kit_error);
  const fw = sd.firmware;
  const fwOk = fw?.matches === true;
  const busy = app.busyReason;
  const disk = sd.disks?.find((d) => d.device === sd.disk) ?? null;
  const [settingsError, setSettingsError] = useState<string | null>(null);
  const hint = toolsHint(
    env?.platform ?? "",
    missing.map((t) => t.name),
  );
  const fileName = fw ? (fw.path.split(/[\\/]/).pop() ?? fw.path) : null;
  const folder = fw ? fw.path.split(/[\\/]/).slice(-2, -1)[0] : null;

  const createHint = blocked
    ? null
    : !fw && !disk
      ? "Choose a firmware file and a card first."
      : !fw
        ? "Choose the firmware file first."
        : !disk
          ? "Choose a card first."
          : !fwOk
            ? "Needs the right firmware file."
            : busy
              ? `${busy}.`
              : `Erases ${disk.name || disk.device}. Takes several minutes.`;

  return (
    <div className="sdwrap">
      <div
        className={["sdmain", blocked && "is-blocked"]
          .filter(Boolean)
          .join(" ")}
      >
        {unsupported && (
          <Banner tone="info" title="Building the card needs macOS or Linux">
            This computer can manage captures and Tone3000, but writing the SD
            card works only on a Mac or a Linux PC for now.
          </Banner>
        )}
        {sd.error && !unsupported && (
          <Banner tone="error" title="Couldn't read this computer's disks">
            {sd.error}
          </Banner>
        )}
        {sd.refused === "blocked" && (
          <Banner
            tone="error"
            title="macOS blocked access to the SD card"
            onDismiss={sd.dismissRefused}
            actions={[
              {
                label: "Open Privacy & Security",
                variant: "primary",
                onClick: () => {
                  setSettingsError(null);
                  api.sdOpenPrivacySettings().catch((e: unknown) => {
                    setSettingsError(errorText(e));
                  });
                },
              },
            ]}
          >
            Nothing was written to the card. In System Settings, open Privacy
            &amp; Security › Files and Folders, turn on Removable Volumes under
            TMP NAM, then create the card again.
            {settingsError && (
              <p className="small">
                Couldn&apos;t open System Settings ({settingsError}). Open it
                from the Apple menu instead.
              </p>
            )}
          </Banner>
        )}
        {sd.refused === "denied" && (
          <Banner
            tone="error"
            title="Administrator access wasn't given"
            onDismiss={sd.dismissRefused}
          >
            Nothing was written to the card. Writing to an SD card needs the
            password of an administrator account on this computer.
          </Banner>
        )}
        {missing.length > 0 && (
          <Banner
            tone="warn"
            title={`${plural(missing.length, "command-line tool")} ${missing.length === 1 ? "is" : "are"} missing`}
          >
            <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
              <span>
                This computer needs{" "}
                {missing.map((t, i) => (
                  <span key={t.name}>
                    {i > 0 && (i === missing.length - 1 ? " and " : ", ")}
                    <span className="mono">{t.name}</span>
                  </span>
                ))}{" "}
                to build the card. {hint.text}
              </span>
              {hint.command && (
                <div className="hrow" style={{ flexWrap: "nowrap" }}>
                  <span
                    className="code"
                    style={{ background: "var(--bg-content)" }}
                  >
                    {hint.command}
                  </span>
                  <Button
                    size="sm"
                    onClick={() => void copyText(hint.command ?? "")}
                  >
                    Copy
                  </Button>
                </div>
              )}
              <div>
                <Button size="sm" onClick={() => void sd.checkEnvironment()}>
                  Check Again
                </Button>
              </div>
            </div>
          </Banner>
        )}
        {env?.kit_error && !unsupported && (
          <Banner tone="error" title="Some of this app's files are damaged">
            The NAM files the app adds to the card failed their check. Download
            TMP NAM again and replace this copy.
          </Banner>
        )}

        <div className="cardx">
          <div className="card-h">
            <span
              className={["stepnum", fwOk && "ok", fw && !fwOk && "bad"]
                .filter(Boolean)
                .join(" ")}
            >
              {fwOk ? "✓" : "1"}
            </span>
            <span style={{ fontWeight: 600 }}>Fender firmware file</span>
          </div>
          <div className="card-b">
            {sd.checking ? (
              <span className="small muted hrow" style={{ gap: 8 }}>
                <Spinner /> Checking the file…
              </span>
            ) : !fw ? (
              <>
                <p className="small muted">
                  The official <span className="mono">{FIRMWARE}</span> from
                  Fender. Other versions won&apos;t work.
                </p>
                <div>
                  <Button
                    disabled={blocked || busy !== null}
                    onClick={() => void sd.chooseFirmware()}
                  >
                    Choose File…
                  </Button>
                </div>
              </>
            ) : (
              <>
                <div className="hrow" style={{ gap: 10, flexWrap: "nowrap" }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div
                      className="mono"
                      style={{ fontWeight: 600, overflowWrap: "anywhere" }}
                    >
                      {fileName}
                    </div>
                    {folder && <div className="small muted3">In {folder}</div>}
                  </div>
                  <Tag tone={fwOk ? "ok" : "danger"}>
                    {fwOk ? "Verified" : "Wrong file"}
                  </Tag>
                  <Button
                    size="sm"
                    disabled={busy !== null}
                    onClick={() => void sd.chooseFirmware()}
                  >
                    Change…
                  </Button>
                </div>
                {!fwOk && (
                  <p className="small" style={{ color: "var(--danger)" }}>
                    This isn&apos;t the expected firmware. The card needs
                    exactly {FIRMWARE}, unmodified. A renamed or partly
                    downloaded file fails this check too.
                  </p>
                )}
              </>
            )}
          </div>
        </div>

        <div className="cardx">
          <div className="card-h">
            <span
              className={["stepnum", disk && "ok"].filter(Boolean).join(" ")}
            >
              {disk ? "✓" : "2"}
            </span>
            <span style={{ fontWeight: 600 }}>SD card</span>
            {!blocked && (
              <span
                className="small muted3 hrow"
                style={{ marginLeft: "auto", gap: 6 }}
              >
                {sd.disks?.length ? (
                  "Detected automatically"
                ) : (
                  <>
                    <Spinner /> Watching for cards
                  </>
                )}
              </span>
            )}
          </div>
          {blocked ? (
            <div className="card-b">
              <p className="small muted">
                Available once the problems above are fixed.
              </p>
            </div>
          ) : sd.disks?.length ? (
            <div role="radiogroup" aria-label="SD card">
              {sd.disks.map((d) => (
                <Radio
                  key={d.device}
                  row
                  name="disk"
                  label={d.name || d.device}
                  detail={diskDetail(d)}
                  note={d.rejected ? refusal(d.rejected) : undefined}
                  checked={sd.disk === d.device}
                  disabled={Boolean(d.rejected) || busy !== null}
                  onChange={() => {
                    sd.pickDisk(d.device);
                  }}
                />
              ))}
            </div>
          ) : (
            <div className="card-b">
              <p className="small muted">
                Insert the SD card into this computer&rsquo;s SD slot or a USB
                card reader. It appears here on its own.
              </p>
            </div>
          )}
        </div>

        <div className="hrow" style={{ gap: 14, marginTop: 4 }}>
          <Button
            variant="primary"
            size="lg"
            disabled={blocked || !fwOk || !disk || busy !== null}
            onClick={onCreate}
          >
            Create SD Card…
          </Button>
          {createHint && <span className="small muted3">{createHint}</span>}
        </div>
      </div>
      <About />
    </div>
  );
}

function About() {
  return (
    <aside className="aside">
      <span className="label">About the card</span>
      <p>
        <b>The unit itself isn&apos;t changed.</b>{" "}
        <span className="muted">Its internal firmware is never modified.</span>
      </p>
      <p>
        <b>Remove the card to go back.</b>{" "}
        <span className="muted">
          Without it, the unit boots its stock firmware.
        </span>
      </p>
      <p>
        <b>Keep your current card.</b>{" "}
        <span className="muted">
          If you already have a working NAM card, keep it as a spare and use a
          new card here.
        </span>
      </p>
    </aside>
  );
}

function stages(
  stage: number,
  phase: string,
  detail: (i: number) => string,
): Stage[] {
  return STAGES.map((label, i): Stage => {
    if (phase === "success" || i < stage)
      return { label, state: "done", detail: "done" };
    if (i === stage)
      return {
        label,
        state: phase === "failed" ? "fail" : "now",
        detail: phase === "failed" ? "failed" : detail(i),
      };
    return { label, state: "todo", detail: i >= 6 ? "minutes" : "" };
  }).filter((_, i) => !(phase === "failed" && i > stage));
}

function Running() {
  const { sd } = useApp();
  const writing = sd.stage === 6;
  const left = writing ? sd.left : null;
  const step =
    sd.stage < 0
      ? "Waiting for administrator approval…"
      : `Step ${String(sd.stage + 1)} of 8 · ${STAGES[sd.stage] ?? ""}${sd.stage >= 6 ? ". This one takes minutes." : ""}`;
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        justifyContent: "center",
        padding: 24,
        overflowY: "auto",
      }}
    >
      <div
        style={{
          width: 540,
          display: "flex",
          flexDirection: "column",
          gap: 16,
        }}
        aria-live="polite"
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 className="t-title">Creating the NAM card</h2>
          <span className="small muted">{step}</span>
        </div>
        <ProgressBar
          value={sd.stage < 0 ? undefined : Math.round(sd.percent)}
          size="lg"
        />
        <div className="cardx" style={{ padding: "2px 16px" }}>
          <StageList
            stages={stages(sd.stage, "running", (i) =>
              i === 6
                ? `${String(Math.round(sd.fraction * 100))}%${left ? ` · ${left}` : ""}`
                : i === 7
                  ? "a few min"
                  : "",
            )}
          />
        </div>
        <Banner tone="info">
          Keep the card in and the computer awake. You can use other pages
          meanwhile.
        </Banner>
        {sd.showLog && <pre className="log">{sd.log.join("\n")}</pre>}
      </div>
    </div>
  );
}

function Success() {
  const { sd } = useApp();
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        justifyContent: "center",
        padding: "40px 24px",
        overflowY: "auto",
      }}
    >
      <div
        style={{
          width: 480,
          display: "flex",
          flexDirection: "column",
          gap: 20,
        }}
      >
        <div className="hrow" style={{ gap: 14, flexWrap: "nowrap" }}>
          <span
            className="outcome-mark"
            style={{ background: "var(--ok-dot)" }}
          >
            <Icon name="check" strokeWidth={2} />
          </span>
          <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
            <h2 className="t-display">Card ready</h2>
            <span className="muted">
              Written and verified. The card has been ejected; you can remove
              it.
            </span>
          </div>
        </div>
        <div className="cardx">
          <div className="card-h">
            <span style={{ fontWeight: 600 }}>
              Next, on the Tone Master Pro
            </span>
          </div>
          <div className="card-b">
            <ConnectSteps
              steps={[
                "Power the unit off.",
                "Insert the card.",
                "Power it on. It boots with NAM support.",
              ]}
            />
          </div>
        </div>
        <p className="small muted">
          When the preset screen shows, plug in USB-C to manage captures here.
          To go back to stock, power off and remove the card. Keep your previous
          working card as a spare.
        </p>
        {sd.showLog && <pre className="log">{sd.log.join("\n")}</pre>}
        <div className="hrow">
          <Button
            variant="primary"
            onClick={() => {
              sd.reset(true);
            }}
          >
            Done
          </Button>
          <Button
            onClick={() => {
              sd.reset(false);
            }}
          >
            Make Another
          </Button>
        </div>
      </div>
    </div>
  );
}

function Failed({ onRetry }: { onRetry: () => void }) {
  const { sd } = useApp();
  const stage = sd.failure?.stage ?? 0;
  return (
    <div
      style={{
        flex: 1,
        display: "flex",
        flexDirection: "column",
        gap: 16,
        padding: "22px 28px",
        overflowY: "auto",
      }}
    >
      <div
        className="hrow"
        style={{ gap: 14, alignItems: "flex-start", flexWrap: "nowrap" }}
      >
        <span
          className="outcome-mark"
          style={{ width: 32, height: 32, background: "var(--danger-fill)" }}
        >
          <Icon name="close" strokeWidth={2} />
        </span>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          <h2 className="t-title">The card couldn&apos;t be created</h2>
          <p className="muted">{FAILURES[stage] ?? sd.failure?.message}</p>
          <p className="small muted3">
            {stage >= 5
              ? "The card isn't usable as it is. "
              : "Nothing was written to the card. "}
            Your unit and its firmware are unaffected.
          </p>
        </div>
      </div>
      <div className="hrow" style={{ paddingLeft: 46 }}>
        <Button variant="primary" onClick={onRetry}>
          Try Again
        </Button>
        <Button
          onClick={() => {
            sd.reset(true);
          }}
        >
          Back
        </Button>
      </div>
      <div
        className="hrow"
        style={{ alignItems: "flex-start", flexWrap: "nowrap", gap: 16 }}
      >
        <div
          className="cardx"
          style={{ width: 270, flex: "none", padding: "2px 14px" }}
        >
          <StageList stages={stages(stage, "failed", () => "")} />
        </div>
        {sd.showLog && (
          <pre className="log" style={{ flex: 1, minWidth: 0, maxHeight: 330 }}>
            {sd.log.join("\n")}
          </pre>
        )}
      </div>
    </div>
  );
}
