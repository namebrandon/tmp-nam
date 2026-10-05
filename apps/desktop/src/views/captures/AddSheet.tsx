// src/views/captures/AddSheet.tsx — "Add captures to the unit": check each file, then send.

import { useEffect, useState } from "react";
import { Banner, Icon, Sheet, Tag } from "../../ds";
import { api, NAM_FILTER, pickFiles, type Inspected } from "../../lib/api";
import {
  defer,
  displayName,
  errorText,
  formatBytes,
  gearLine,
  plural,
  sampleRate,
} from "../../lib/format";
import type { Source } from "../../state/context";

function tag(f: Inspected): string {
  if (f.info?.architecture === "SlimmableContainer")
    return `A2 · ${String(f.info.submodels.length)}`;
  return "A1";
}

export function AddSheet({
  paths,
  onCancel,
  onSend,
  blocked,
}: {
  paths: string[];
  onCancel: () => void;
  onSend: (sources: Source[]) => void;
  /** Why sending is unavailable right now, or null. */
  blocked: string | null;
}) {
  const [files, setFiles] = useState<Inspected[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const inspect = async (more: string[]) => {
    try {
      const checked = await api.namInspect(more);
      setFiles((f) => {
        const known = new Set((f ?? []).map((x) => x.path));
        return [...(f ?? []), ...checked.filter((x) => !known.has(x.path))];
      });
    } catch (e) {
      setError(errorText(e));
    }
  };

  useEffect(() => {
    defer(() => inspect(paths));
    // Only the paths the sheet opened with; "Add More Files…" appends.
  }, [paths]);

  const valid = (files ?? []).filter((f) => !f.error);
  const n = valid.length;
  const send = () => {
    onSend(
      valid.map((f) => ({
        kind: "file",
        path: f.path,
        name: f.name,
        label: displayName(f.name),
        bytes: f.bytes,
      })),
    );
  };

  return (
    <Sheet
      title="Add captures to the unit"
      width={540}
      onClose={onCancel}
      extra={{
        label: "Add More Files…",
        onClick: () =>
          void pickFiles(NAM_FILTER, true).then((more) => {
            if (more) void inspect(more);
          }),
      }}
      actions={[
        { label: "Cancel", onClick: onCancel },
        {
          label: n === 1 ? "Send 1 Capture" : `Send ${String(n)} Captures`,
          variant: "primary",
          disabled: n === 0 || blocked !== null,
          onClick: send,
        },
      ]}
    >
      <p className="muted">
        {files === null
          ? "Checking the files…"
          : files.length === n
            ? `${plural(files.length, "file")} checked. ${n === 1 ? "It's a valid capture" : "All are valid captures"} and will be sent.`
            : `${plural(files.length, "file")} checked. ${String(n)} ${n === 1 ? "is a valid capture" : "are valid captures"} and will be sent.`}
      </p>
      {files && files.length > 0 && (
        <div className="cardx">
          {files.map((f, i) => {
            const last = i === files.length - 1;
            const rowStyle = {
              display: "grid",
              gridTemplateColumns: "18px minmax(0, 1fr) auto",
              gap: 10,
              alignItems: "center",
              padding: "10px 12px",
              borderBottom: last ? undefined : "1px solid var(--sep)",
              background: f.error ? "var(--danger-weak)" : undefined,
            } as const;
            const file = f.path.split(/[\\/]/).pop() ?? f.path;
            return (
              <div key={f.path} style={rowStyle}>
                <Icon
                  name={f.error ? "close" : "check"}
                  style={{ color: f.error ? "var(--danger)" : "var(--ok-dot)" }}
                />
                <div style={{ minWidth: 0 }}>
                  <div style={{ fontWeight: 500, overflowWrap: "anywhere" }}>
                    {file}
                  </div>
                  <div
                    className="small"
                    style={{
                      color: f.error ? "var(--danger)" : "var(--text-3)",
                    }}
                  >
                    {f.error
                      ? "Not a valid .nam capture. The file is damaged or incomplete."
                      : (gearLine(f.info) ?? "No gear details in file")}
                  </div>
                </div>
                <span className="tags" style={{ justifyContent: "flex-end" }}>
                  {f.error ? (
                    <span className="small muted">Skipped</span>
                  ) : (
                    <>
                      <Tag tone="arch">{tag(f)}</Tag>
                      {sampleRate(f.info) && <Tag>{sampleRate(f.info)}</Tag>}
                      <span
                        className="small muted"
                        style={{ width: 52, textAlign: "right" }}
                      >
                        {formatBytes(f.bytes)}
                      </span>
                    </>
                  )}
                </span>
              </div>
            );
          })}
        </div>
      )}
      {error && <Banner tone="error">{error}</Banner>}
      <Banner tone="info">
        Captures show up in the unit&apos;s IR list as soon as they&apos;re
        sent. If Pro Control is open, the audio engine restarts instead and is
        silent for a few seconds, so close it first.
      </Banner>
      {blocked && <span className="small muted3">{blocked}.</span>}
    </Sheet>
  );
}
