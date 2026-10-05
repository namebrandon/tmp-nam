// src/views/ReselectNote.tsx — after an engine restart, NAM captures stay silent
// until one is selected again on the unit; say so.

import { Banner } from "../ds";
import { useApp } from "../state/context";

export function ReselectNote() {
  const app = useApp();
  if (!app.reselect || !app.connected || app.op) return null;
  return (
    <div className="bwrap">
      <Banner
        tone="note"
        title="Reselect your preset on the unit"
        onDismiss={app.dismissReselect}
      >
        The audio engine restarted, so NAM captures stay silent until you select
        the preset, or a capture, again on the Tone Master Pro.
      </Banner>
    </div>
  );
}
