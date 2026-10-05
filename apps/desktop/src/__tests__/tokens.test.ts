// Every token in design/design-system/tokens.json has its CSS variable in src/theme/tokens.css.
// Regenerate with `node scripts/gen-tokens.mjs` when the tokens change.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { join } from "node:path";

const root = join(__dirname, "..", "..");

interface Token {
  name: string;
  value: string;
}
interface Group {
  tokens: Token[];
}

describe("design tokens", () => {
  const json = JSON.parse(
    readFileSync(join(root, "design/design-system/tokens.json"), "utf8"),
  ) as Record<"color" | "spacing" | "radius" | "shadow" | "size", Group>;
  const css = readFileSync(join(root, "src/theme/tokens.css"), "utf8");

  it.each(["color", "spacing", "radius", "shadow", "size"] as const)(
    "%s tokens are in tokens.css",
    (group) => {
      for (const t of json[group].tokens)
        expect(css).toContain(`--${t.name}: ${t.value};`);
    },
  );
});
