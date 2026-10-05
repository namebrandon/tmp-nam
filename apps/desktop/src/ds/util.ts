// src/ds/util.ts — non-component helpers shared by the design-system components.

/** Join truthy class names. */
export function cx(...names: (string | false | null | undefined)[]): string {
  return names.filter(Boolean).join(" ");
}

export const CONNECT_STEPS = [
  "Insert the NAM SD card and power the unit on.",
  "Wait for the preset screen.",
  "Plug the unit's USB-C cable into this computer.",
];
