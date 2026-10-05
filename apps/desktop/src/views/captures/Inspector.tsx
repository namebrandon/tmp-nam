// src/views/captures/Inspector.tsx — the selected capture: size, flags, removal.
//
// Level is set on the unit (the IR block's level, per preset), not here: the
// player's per-capture `output_gain` is left as it is on the unit.

import { useState } from "react";
import { Banner, Button, SizePicker, Tag } from "../../ds";
import { api } from "../../lib/api";
import {
  a1Size,
  archTag,
  currentStep,
  displayName,
  errorText,
  family,
  formatBytes,
  gearLine,
  sampleRate,
  sizeSteps,
} from "../../lib/format";
import { useApp } from "../../state/context";
import { CabTag } from "./CabTag";
import type { Row } from "./rows";

export function Inspector({
  row,
  onRemove,
  onRestore,
}: {
  row: Row;
  onRemove: () => void;
  /** Choose File… for a capture whose file is gone. */
  onRestore: () => void;
}) {
  const app = useApp();
  const { capture: c, flag, name } = row;
  const disabled =
    !app.connected || app.busyReason !== null || app.unit === "busy";
  const mark = app.marks[name] ?? {};
  const [error, setError] = useState<string | null>(null);

  const sha = c?.sha256 ?? null;
  const options = c?.options ?? {};

  const save = async (next: { size?: number; output_gain?: number }) => {
    if (!sha) return;
    try {
      await api.unitSetOptions(sha, next);
      await app.refreshCaptures();
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const steps = sizeSteps(c?.info?.submodels ?? []);
  const setSize = (label: string) => {
    const step = steps.find((s) => s.label === label);
    if (!step) return;
    app.mark(name, { changed: true });
    // Keep any output gain already on the unit; the app doesn't set it.
    void save({ size: step.size, output_gain: options.output_gain });
  };

  const fam = c ? family(c) : "unknown";
  const gear = gearLine(c?.info);
  const rate = sampleRate(c?.info);
  const size = c ? a1Size(c) : null;
  const toneId = c?.source?.tone_id;

  return (
    <>
      <div className="sec" style={{ gap: 6 }}>
        <h2 className="h15">{displayName(name)}</h2>
        <span className="small muted">
          {gear ??
            (flag ? "Gear details unavailable" : "No gear details in file")}
        </span>
        <div className="tags" style={{ marginTop: 4 }}>
          {c && fam !== "unknown" && <Tag tone="arch">{archTag(c)}</Tag>}
          <CabTag info={c?.info} />
          {flag === "missing" && <Tag tone="danger">File missing</Tag>}
          {rate && <Tag>{rate}</Tag>}
          {c && c.bytes > 0 && c.present && <Tag>{formatBytes(c.bytes)}</Tag>}
        </div>
      </div>

      {flag === "missing" && (
        <div className="sec">
          <Banner
            tone="error"
            title="The unit lists this capture, but its file is gone"
          >
            Presets that use it won&apos;t load it. Add the .nam file again, or
            remove the entry.
          </Banner>
          <span className="small">
            <span className="muted3">Expected file </span>
            <span className="mono" style={{ fontSize: 11 }}>
              {name}
            </span>
          </span>
          <div className="hrow">
            <Button variant="primary" disabled={disabled} onClick={onRestore}>
              Choose File…
            </Button>
            <Button destructiveText disabled={disabled} onClick={onRemove}>
              Remove Entry…
            </Button>
          </div>
        </div>
      )}

      {flag === "unreg" && (
        <div className="sec">
          <Banner tone="warn" title="On the unit, but not registered">
            The file is on the card, but the unit doesn&apos;t list it, so
            presets can&apos;t use it yet.
          </Banner>
          <span className="small muted3">
            Registering adds it to the unit&apos;s IR list right away. If Pro
            Control is open, the audio engine restarts instead and is silent for
            a few seconds.
          </span>
          <div className="hrow">
            <Button
              variant="primary"
              disabled={disabled}
              onClick={() =>
                void app
                  .unitCommand(() => api.unitRegister([name]))
                  .then(setError)
              }
            >
              Register
            </Button>
            <Button destructiveText disabled={disabled} onClick={onRemove}>
              Delete File…
            </Button>
          </div>
        </div>
      )}

      {flag === "failed" && (
        <div className="sec">
          <Banner tone="error" title="The unit didn't load it">
            It won&apos;t play until it&apos;s sent again or removed.
          </Banner>
          <div className="hrow">
            <Button
              variant="primary"
              disabled={disabled}
              onClick={() => {
                const s = app.failed[name];
                if (s)
                  void app.run(s.kind === "file" ? "send" : "install", [s]);
              }}
            >
              Send Again
            </Button>
            <Button
              destructiveText
              disabled={app.busyReason !== null}
              onClick={onRemove}
            >
              Remove…
            </Button>
          </div>
        </div>
      )}

      {!flag && c && (
        <>
          <div className="sec">
            <span className="lbl">
              {fam === "a2" ? "Size the player loads" : "Size"}
            </span>
            {fam === "a2" && steps.length > 0 && (
              <>
                <SizePicker
                  sizes={steps.map((s) => ({ label: s.label }))}
                  value={steps[currentStep(steps, options.size)]?.label}
                  disabled={disabled}
                  onChange={setSize}
                />
                <span className="small muted3">
                  Larger sizes sound closer to the amp but use more of the
                  unit&apos;s processing. If it crackles, go smaller.
                </span>
              </>
            )}
            {fam !== "a2" && size && (
              <>
                <span className="small muted">
                  This file is the {size} size. An A1 file holds only one size,
                  so to try another, install that variant from Tone3000.
                </span>
                {toneId != null && (
                  <div>
                    <Button
                      size="sm"
                      onClick={() => {
                        app.showInTone3000(String(toneId));
                      }}
                    >
                      Show in Tone3000
                    </Button>
                  </div>
                )}
              </>
            )}
            {fam !== "a2" && !size && (
              <span className="small muted">
                An A1 file holds only one size. To use another size, add that
                variant&apos;s .nam file.
              </span>
            )}
          </div>
          <div className="sec" style={{ flex: 1 }}>
            {fam === "a2" &&
              (mark.changed ? (
                <Banner tone="note" title="Saved to the unit">
                  The size applies the next time you select this capture on the
                  unit. If it&apos;s selected now, pick another capture and come
                  back.
                </Banner>
              ) : (
                <span className="small muted3">
                  The size applies the next time you select this capture on the
                  unit.
                </span>
              ))}
            {error && (
              <span className="small" style={{ color: "var(--danger)" }}>
                {error}
              </span>
            )}
            <div style={{ flex: 1 }} />
            <div>
              <Button
                destructiveText
                disabled={disabled}
                title={app.busyReason ?? undefined}
                onClick={onRemove}
              >
                Remove from Unit…
              </Button>
            </div>
          </div>
        </>
      )}
      {flag && error && (
        <div className="sec">
          <span className="small" style={{ color: "var(--danger)" }}>
            {error}
          </span>
        </div>
      )}
    </>
  );
}
