// src/views/tone3000/Variants.tsx — allowed variants: the sheet (Tone3000 page) and the
// checklist it shares with Settings › Allowed Variants. Same setting in both places.

import { useState } from "react";
import { Banner, Checkbox, Sheet } from "../../ds";
import type { Variant } from "../../lib/api";
import { DEFAULT_ALLOWED, toggled } from "./allowed";

export function VariantColumns({
  allowed,
  variants,
  onToggle,
  cards,
}: {
  allowed: string[];
  variants: Variant[];
  onToggle: (id: string) => void;
  /** Bordered cards with a caption (the sheet) instead of plain columns. */
  cards?: boolean;
}) {
  const column = (arch: number, caption: string) => (
    <div className={cards ? "cardx card-b" : "vcol"}>
      <span style={{ fontWeight: 600 }}>
        A{arch}{" "}
        {cards && (
          <span className="small muted3" style={{ fontWeight: 400 }}>
            · {caption}
          </span>
        )}
      </span>
      {variants
        .filter((v) => v.arch === arch)
        .sort((a, b) => a.rank - b.rank)
        .map((v) => (
          <Checkbox
            key={v.id}
            label={v.label}
            checked={allowed.includes(v.id)}
            onChange={() => {
              onToggle(v.id);
            }}
          />
        ))}
    </div>
  );
  return (
    <div className="vgrid">
      {column(2, "sizes are picked on the unit")}
      {column(1, "one network")}
    </div>
  );
}

export function VariantsSheet({
  allowed,
  variants,
  onCancel,
  onSave,
}: {
  allowed: string[];
  variants: Variant[];
  onCancel: () => void;
  onSave: (ids: string[]) => void;
}) {
  const [draft, setDraft] = useState(allowed);
  return (
    <Sheet
      title="Allowed variants"
      width={520}
      onClose={onCancel}
      extra={{
        label: "Restore Default",
        onClick: () => {
          setDraft(DEFAULT_ALLOWED);
        },
      }}
      actions={[
        { label: "Cancel", onClick: onCancel },
        {
          label: "Save",
          variant: "primary",
          onClick: () => {
            onSave(draft);
          },
        },
      ]}
    >
      <p className="muted">
        An A1 capture comes in up to four sizes, one per file. An A2 capture
        holds Lite and Full in one file; you pick the size on the unit. Choose
        what the sync may download.
      </p>
      <VariantColumns
        cards
        allowed={draft}
        variants={variants}
        onToggle={(id) => {
          setDraft((d) => toggled(d, id));
        }}
      />
      <p className="small muted">
        When a tone has more than one allowed model, the sync picks A2 first,
        then the largest allowed size. You can still choose another model per
        tone.
      </p>
      <Banner tone="warn" title="Size labels are not a guarantee">
        A model labelled Feather or Nano can still crackle or drop out on the
        unit. Check every capture by ear before using it live.
      </Banner>
    </Sheet>
  );
}
