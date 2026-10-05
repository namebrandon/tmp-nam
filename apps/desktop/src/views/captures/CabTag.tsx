// src/views/captures/CabTag.tsx — whether a capture includes the speaker cabinet.

import { Tag } from "../../ds";
import type { ModelInfo } from "../../lib/api";
import { hasCab } from "../../lib/format";

/** "With cab" when the capture includes the cabinet (no IR needed after it). */
export function CabTag({ info }: { info?: ModelInfo | null }) {
  if (hasCab(info) !== true) return null;
  return (
    <Tag title="The capture includes the speaker cabinet: no IR needed after it.">
      With cab
    </Tag>
  );
}
