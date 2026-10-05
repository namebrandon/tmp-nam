import { describe, expect, it } from "vitest";
import type { Capture } from "../lib/api";
import {
  a1Size,
  archTag,
  currentStep,
  family,
  formatBytes,
  gearLine,
  hasCab,
  sampleRate,
  sizeSteps,
} from "../lib/format";

const capture = (
  info: Capture["info"],
  extra: Partial<Capture> = {},
): Capture => ({
  name: "x.nam",
  file: "x.nam.wav",
  bytes: 1,
  registered: true,
  present: true,
  info,
  options: {},
  ...extra,
});

const a2 = {
  architecture: "SlimmableContainer",
  meta: {},
  submodels: [
    { architecture: "WaveNet", channels: 8, max_value: 1.0 },
    { architecture: "WaveNet", channels: 3, max_value: 0.5 },
  ],
};

describe("sizeSteps", () => {
  it("names A2 sizes by channel width, smallest first", () => {
    const steps = sizeSteps(a2.submodels);
    expect(steps.map((s) => s.label)).toEqual(["Lite", "Full"]);
    // Exclusive max_value thresholds: child i is reached at the previous threshold.
    expect(steps.map((s) => s.size)).toEqual([0, undefined]);
  });

  it("falls back to numbered sizes for unknown widths", () => {
    const steps = sizeSteps([
      { channels: 8, max_value: 1.0 },
      { channels: 3, max_value: 0.3 },
      { channels: 5, max_value: 0.6 },
    ]);
    expect(steps.map((s) => s.label)).toEqual(["Lite", "Size 2", "Full"]);
    expect(steps.map((s) => s.size)).toEqual([0, 0.3, undefined]);
  });

  it("treats no setting as the full network", () => {
    const steps = sizeSteps(a2.submodels);
    expect(currentStep(steps, undefined)).toBe(1);
    expect(currentStep(steps, 0)).toBe(0);
    expect(currentStep(steps, 0.42)).toBe(1);
  });
});

describe("family and tags", () => {
  it("classifies containers, A1 and unknown files", () => {
    expect(family(capture(a2))).toBe("a2");
    expect(
      family(
        capture({
          architecture: "WaveNet",
          version: "0.5.4",
          meta: {},
          submodels: [],
        }),
      ),
    ).toBe("a1");
    expect(family(capture(null))).toBe("unknown");
  });

  it("writes the type tag", () => {
    expect(archTag(capture(a2))).toBe("A2");
    const a1 = { architecture: "WaveNet", meta: {}, submodels: [] };
    expect(archTag(capture(a1))).toBe("A1");
    expect(
      archTag(
        capture(a1, {
          source: { tone_id: 1, model_id: 2, variant: "a1-feather" },
        }),
      ),
    ).toBe("A1 · Feather");
    expect(a1Size(capture(a1, { name: "JCM800-Lead-lite.nam" }))).toBe("Lite");
    // Added from a file: the network width names the size.
    expect(archTag(capture({ ...a1, channels: 8 }))).toBe("A1 · Feather");
    expect(archTag(capture({ ...a1, channels: 16 }))).toBe("A1 · Standard");
    expect(archTag(capture({ ...a1, channels: 5 }))).toBe("A1");
  });
});

describe("cab detection", () => {
  it("reads whether the chain includes the cabinet", () => {
    const g = (gear_type?: string) => ({ meta: { gear_type }, submodels: [] });
    expect(hasCab(g("amp_cab"))).toBe(true);
    expect(hasCab(g("amp_pedal_cab"))).toBe(true);
    expect(hasCab(g("full-rig"))).toBe(true);
    expect(hasCab(g("amp"))).toBe(false);
    expect(hasCab(g("pedal"))).toBe(false);
    expect(hasCab(g("studio"))).toBeNull();
    expect(hasCab(g())).toBeNull();
  });
});

describe("gear and file details", () => {
  it("joins make, model and type with em dashes for gaps", () => {
    expect(
      gearLine({
        meta: {
          gear_make: "Marshall",
          gear_model: "JCM800",
          gear_type: "amp_cab",
        },
        submodels: [],
      }),
    ).toBe("Marshall · JCM800 · Amp + cab");
    expect(gearLine({ meta: { gear_model: "AC30" }, submodels: [] })).toBe(
      "— · AC30 · —",
    );
    expect(gearLine({ meta: {}, submodels: [] })).toBeNull();
  });

  it("formats rates and sizes", () => {
    expect(sampleRate({ sample_rate: 44100, meta: {}, submodels: [] })).toBe(
      "44.1 kHz",
    );
    expect(sampleRate({ sample_rate: 48000, meta: {}, submodels: [] })).toBe(
      "48 kHz",
    );
    expect(formatBytes(312448)).toBe("312 KB");
    expect(formatBytes(2204118)).toBe("2.2 MB");
  });
});
