// src/lib/mock.ts — in-browser stand-in for the Rust backend.
//
// Used only when the page runs outside Tauri (`bun run dev` in a browser, Vitest)
// so every page renders and can be clicked through without a unit. It mirrors the
// Rust `SimUnit` (src-tauri/src/unit.rs); real device behaviour lives in Rust.
//
// Query flags reach the states the handoff screens show:
//   ?unit=missing        no unit until ?unit is dropped (or after an unplug)
//   ?fail=disconnect     unplug halfway through the next send/install
//   ?restart=1           no HID channel: sends/removes go through an engine restart
//   ?fail=restart        unplug during the engine restart (implies ?restart=1)
//   ?fail=drop           the last file doesn't load
//   ?fail=verify         the card build fails reading back
//   ?fail=denied         the administrator prompt is cancelled
//   ?fail=blocked        macOS refuses Removable Volumes access
//   ?fail=key            Tone3000 rejects the API key at sign-in
//   ?flags=1             seed a "file missing" and a "not registered" capture
//   ?t3k=nokey|signedout|list   starting Tone3000 account state
//   ?sd=tools|assets     missing build tools / damaged app assets
//   ?tones=0             no tones on the Tone3000 account

import { ApiError } from "./api";
import type {
  AddOutcome,
  Capture,
  Disk,
  Inspected,
  ModelInfo,
  OpEvent,
  SdEnvironment,
  Settings,
  T3kPick,
  T3kTone,
  Variant,
} from "./api";

type Handler = (payload: unknown) => void;
const handlers = new Map<string, Set<Handler>>();

export function mockListen(event: string, cb: Handler): () => void {
  let set = handlers.get(event);
  if (!set) {
    set = new Set();
    handlers.set(event, set);
  }
  set.add(cb);
  return () => {
    set.delete(cb);
  };
}

function emit(event: string, payload: unknown) {
  handlers.get(event)?.forEach((h) => {
    h(payload);
  });
}

const op = (e: OpEvent) => {
  emit("op://event", e);
};

const params = (): URLSearchParams =>
  new URLSearchParams(
    typeof window === "undefined" ? "" : window.location.search,
  );
const flag = (k: string) => params().get(k);

/** Tests set this to make the mock instant. */
export const mockTiming = { scale: 1 };

const sleep = (ms: number) =>
  new Promise<void>((r) => {
    setTimeout(r, ms * mockTiming.scale);
  });

const VARIANTS: Variant[] = [
  { id: "a2", arch: 2, label: "Lite and Full, in one file", rank: 0 },
  { id: "a1-standard", arch: 1, label: "Standard", rank: 0 },
  { id: "a1-lite", arch: 1, label: "Lite", rank: 1 },
  { id: "a1-feather", arch: 1, label: "Feather", rank: 2 },
  { id: "a1-nano", arch: 1, label: "Nano", rank: 3 },
];

const a2 = (
  name: string,
  make: string,
  gear: string,
  rate = 48000,
): ModelInfo => ({
  architecture: "SlimmableContainer",
  version: "0.7.0",
  sample_rate: rate,
  meta: { name, gear_make: make, gear_model: gear, gear_type: "amp" },
  submodels: [
    { architecture: "WaveNet", channels: 3, max_value: 0.5 },
    { architecture: "WaveNet", channels: 8, max_value: 1.0 },
  ],
});

const a1 = (name: string, make: string, gear: string): ModelInfo => ({
  architecture: "WaveNet",
  version: "0.5.4",
  sample_rate: 48000,
  channels: 8,
  meta: { name, gear_make: make, gear_model: gear, gear_type: "amp" },
  submodels: [],
});

const capture = (
  name: string,
  info: ModelInfo | null,
  bytes: number,
  extra: Partial<Capture> = {},
): Capture => ({
  name,
  file: `${name}.wav`,
  bytes,
  registered: true,
  present: true,
  sha256: `sha-${name}`,
  info,
  options: {},
  ...extra,
});

function seed(): Capture[] {
  const list = [
    capture(
      "Fender Deluxe Reverb '65 Vibrato.nam",
      a2("Deluxe Reverb Vibrato", "Fender", "Deluxe Reverb"),
      1_100_000,
      { options: { output_gain: 4 } },
    ),
    capture(
      "Marshall JCM800 2203 Crunch.nam",
      a2("JCM800 Crunch", "Marshall", "JCM800 2203"),
      1_100_000,
      { options: { size: 0, output_gain: 4 } },
    ),
    capture(
      "Vox AC30 Top Boost-feather.nam",
      a1("AC30 Top Boost", "Vox", "AC30"),
      296_000,
      { source: { tone_id: 110, model_id: 11002, variant: "a1-feather" } },
    ),
    capture(
      "Klon Centaur into Twin.nam",
      a1("Klon into Twin", "Klon", "Centaur"),
      301_000,
      { options: { output_gain: 2 } },
    ),
  ];
  if (flag("flags") === "1")
    list.push(
      capture("Dumble ODS Clean.nam", null, 0, {
        present: false,
        sha256: null,
      }),
      capture("bluesbreaker_v2.nam", a1("Bluesbreaker", "", ""), 301_000, {
        registered: false,
      }),
    );
  return list;
}

let models: Capture[] = seed();
let offlineUntil = flag("unit") === "missing" ? Infinity : 0;
let busy = false;
let unsent: string[] = [];

let settings: Settings = {
  t3k_key: flag("t3k") && flag("t3k") !== "nokey" ? "t3k_pub_mock" : "",
  variants: ["a2", "a1-feather", "a1-nano"],
};
let linked = flag("t3k") === "list";
const link = { cancelled: false };
const linkCancelled = () => link.cancelled;

const unplug = () => {
  offlineUntil = Date.now() + 6000;
};

const tModel = (
  id: number,
  arch: 1 | 2,
  size: string,
  name: string,
): T3kTone["models"][number] =>
  // Like the real API: A2 models carry no size (one file, all sizes).
  arch === 2
    ? {
        id,
        name,
        size: null,
        architecture_version: "2",
        variant: "a2",
        model_url: `https://www.tone3000.com/api/v1/models/${String(id)}/download`,
        ir_name: `${name}.nam`,
        size_hint: null,
      }
    : {
        id,
        name,
        size,
        architecture_version: "1",
        variant: `a1-${size}`,
        model_url: `https://www.tone3000.com/api/v1/models/${String(id)}/download`,
        ir_name: `${name}-${size}.nam`,
        size_hint: size,
      };

/** A tone from its captures: capture name → "A2" / "A1 <size>" models. */
function tone(
  id: number,
  title: string,
  author: string,
  source: "bookmark" | "created",
  captures: Record<string, string[]>,
): T3kTone {
  const models: T3kTone["models"] = [];
  const caps: T3kTone["captures"] = [];
  for (const [name, variants] of Object.entries(captures)) {
    const indices: number[] = [];
    for (const v of variants) {
      const [arch, size] = v.split(" ");
      indices.push(models.length);
      models.push(
        tModel(
          id * 100 + models.length,
          arch === "A2" ? 2 : 1,
          size ?? "",
          name,
        ),
      );
    }
    caps.push({ name, models: indices });
  }
  return { id, title, author, source, models, captures: caps };
}

const TONES: T3kTone[] = [
  tone(101, "Plexi Super Lead 1968", "tonehunter", "bookmark", {
    "Plexi Super Lead 1968": ["A2", "A1 standard", "A1 feather"],
  }),
  tone(102, "Fender Deluxe Reverb '65 Vibrato", "riffwright", "created", {
    "Fender Deluxe Reverb '65 Vibrato": ["A2"],
  }),
  tone(106, "Boss SD-1 Super Overdrive", "APP", "bookmark", {
    "APP-SD1-Drive-I": ["A2"],
    "APP-SD1-Drive-II": ["A2"],
    "APP-SD1-Boost-I": ["A2"],
  }),
  tone(103, "Dual Rectifier Modern", "mbtones", "bookmark", {
    "Red channel": ["A1 standard", "A1 feather", "A1 nano"],
    "Orange channel": ["A1 standard", "A1 nano"],
  }),
  tone(104, "TS808 into JC-120", "kpcaps", "bookmark", {
    "TS808 into JC-120": ["A1 standard"],
  }),
  tone(110, "Vox AC30 Top Boost", "amptraveler", "bookmark", {
    "Vox AC30 Top Boost": ["A1 standard", "A1 lite", "A1 feather", "A1 nano"],
  }),
];

/** Mirrors `sim_restart`: the restart fallback instead of the HID channel. */
function restartMode(): boolean {
  return flag("restart") === "1" || flag("fail") === "restart";
}

/** Mirrors `SimUnit::add`, including the injected failures. */
async function send(
  items: { name: string; bytes: number; info: ModelInfo | null }[],
  source?: (i: number) => Capture["source"],
): Promise<AddOutcome> {
  const fail = flag("fail");
  const names = items.map((i) => i.name);
  const out: AddOutcome = {
    added: [],
    failed_after_restart: [],
    not_sent: [],
    needs_restart: restartMode(),
  };
  busy = true;
  try {
    const cut = Math.min(items.length, 2) - 1;
    for (const [i, it] of items.entries()) {
      for (let step = 1; step <= 10; step++) {
        await sleep(90);
        if (fail === "disconnect" && i === cut && step === 5) {
          unplug();
          unsent = names.slice(i);
          return {
            ...out,
            interrupted: it.name,
            not_sent: names.slice(i + 1),
            stop: { kind: "disconnected" },
          };
        }
        op({
          phase: "send",
          index: i,
          done: Math.round((it.bytes * step) / 10),
          total: it.bytes,
        });
      }
      models = [
        ...models.filter((m) => m.name !== it.name),
        capture(it.name, it.info, it.bytes, {
          sha256: `sha-${it.name}-${String(Date.now())}`,
          source: source?.(i) ?? null,
        }),
      ];
      op({ phase: "sent", index: i });
      out.added.push(it.name);
    }
    if (out.needs_restart) {
      op({ phase: "restart" });
      await sleep(2500);
    } else {
      await sleep(600);
    }
    if (fail === "restart") {
      unplug();
      return { ...out, stop: { kind: "disconnected_during_restart" } };
    }
    if (fail === "drop") {
      const last = out.added.pop();
      if (last) {
        out.failed_after_restart.push(last);
        models = models.filter((m) => m.name !== last);
      }
    }
    return out;
  } finally {
    busy = false;
  }
}

const SAMPLE_FILES: Record<string, { bytes: number; info: ModelInfo | null }> =
  {
    "/Users/me/Downloads/Plexi 1968 Super Lead.nam": {
      bytes: 1_100_000,
      info: a2("Plexi 1968", "Marshall", "1959 Super Lead"),
    },
    "/Users/me/Downloads/JC-120 Clean.nam": {
      bytes: 298_000,
      info: a1("JC-120 Clean", "Roland", "JC-120"),
    },
    "/Users/me/Downloads/Rectifier Modern Red.nam": {
      bytes: 1_600_000,
      info: { ...a2("Rectifier Modern Red", "", "", 44100), meta: {} },
    },
    "/Users/me/Downloads/orange_rv50.nam": { bytes: 12_000, info: null },
  };

/** Stand-in for the native file dialog. */
export function mockPick(extensions: string[]): string[] {
  if (extensions.includes("img"))
    return [
      flag("fail") === "wrongfile"
        ? "/Users/me/Downloads/ToneMasterPro_v1_9_02.img"
        : "/Users/me/Downloads/ToneMasterPro_v1_8_58.img",
    ];
  return Object.keys(SAMPLE_FILES);
}

const stem = (path: string) =>
  (path.split("/").pop() ?? path).replace(/\.nam$/i, "");

export async function mockInvoke(
  cmd: string,
  args: Record<string, unknown> = {},
): Promise<unknown> {
  switch (cmd) {
    case "unit_connect": {
      await sleep(300);
      if (Date.now() < offlineUntil)
        throw new Error(
          "No unit found. Boot the Tone Master Pro from the NAM SD card and connect USB-C.",
        );
      if (unsent.length) {
        const drop = unsent;
        models = models.filter((m) => !(drop.includes(m.name) && !m.present));
        unsent = [];
      }
      return {
        port: "simulator",
        build_id: "nam-card-2026.10-r1",
        dispatch_sha256: "efcc31677743a42f",
        python: "3.5.6",
        simulated: true,
        busy,
      };
    }
    case "unit_list":
      await sleep(300);
      return models;
    case "nam_inspect": {
      await sleep(300);
      return (args.paths as string[]).map((path): Inspected => {
        const f = SAMPLE_FILES[path];
        return {
          path,
          name: `${stem(path)}.nam`,
          bytes: f?.bytes ?? 0,
          info: f?.info ?? null,
          error: f?.info ? null : 'not a .nam file: missing "weights"',
        };
      });
    }
    case "unit_add_files": {
      const files = args.files as { path: string; name?: string }[];
      return send(
        files.map((f) => ({
          name: `${stem(f.name ?? f.path)}.nam`,
          bytes: SAMPLE_FILES[f.path]?.bytes ?? 300_000,
          info: SAMPLE_FILES[f.path]?.info ?? a1("Restored", "", ""),
        })),
      );
    }
    case "unit_remove": {
      const names = args.names as string[];
      const restart = restartMode();
      busy = true;
      await sleep(restart ? 1500 : 400);
      busy = false;
      models = models.filter((m) => !names.includes(m.name));
      return restart;
    }
    case "unit_register": {
      const names = args.names as string[];
      const restart = restartMode();
      busy = true;
      await sleep(restart ? 1500 : 400);
      busy = false;
      models = models.map((m) =>
        names.includes(m.name) ? { ...m, registered: true } : m,
      );
      return restart;
    }
    case "unit_reload":
      busy = true;
      await sleep(1500);
      busy = false;
      return null;
    case "unit_set_options": {
      const sha = args.sha256 as string;
      await sleep(150);
      models = models.map((m) =>
        m.sha256 === sha
          ? { ...m, options: args.options as Capture["options"] }
          : m,
      );
      return null;
    }
    case "settings_get":
      return settings;
    case "settings_set": {
      const next = args.settings as Settings;
      if (next.t3k_key && !next.t3k_key.startsWith("t3k_pub_"))
        throw new Error("The Tone3000 key should start with t3k_pub_");
      settings = next;
      return null;
    }
    case "variants_list":
      return VARIANTS;
    case "t3k_status":
      await sleep(200);
      return {
        has_key: settings.t3k_key !== "",
        linked,
        username: linked ? "riffwright" : null,
      };
    case "t3k_link": {
      link.cancelled = false;
      await sleep(3000);
      if (linkCancelled()) throw new ApiError("cancelled", "Sign-in cancelled");
      if (flag("fail") === "key")
        throw new ApiError(
          "key_rejected",
          "Tone3000 refused the sign-in: invalid_client",
        );
      linked = true;
      return "riffwright";
    }
    case "t3k_open_link_again":
    case "t3k_open_site":
    case "sd_open_privacy_settings":
      return null;
    case "t3k_cancel_link":
      link.cancelled = true;
      return null;
    case "t3k_unlink":
      linked = false;
      return null;
    case "t3k_tones":
      await sleep(900);
      if (!linked)
        throw new ApiError("signed_out", "Not signed in to Tone3000");
      return flag("tones") === "0" ? [] : TONES;
    case "t3k_install": {
      const picks = args.picks as T3kPick[];
      for (const [i] of picks.entries()) {
        op({ phase: "download", index: i, done: 0, total: 1 });
        await sleep(400);
        op({ phase: "download", index: i, done: 1, total: 1 });
      }
      return send(
        picks.map((p) => ({
          name: p.ir_name,
          bytes: p.variant?.startsWith("a2") ? 1_100_000 : 300_000,
          info: p.variant?.startsWith("a2")
            ? a2(stem(p.ir_name), "", "")
            : a1(stem(p.ir_name), "", ""),
        })),
        (i) => {
          const p = picks[i];
          return p
            ? { tone_id: p.tone_id, model_id: p.model_id, variant: p.variant }
            : null;
        },
      );
    }
    case "sd_environment": {
      const tools = [
        "unsquashfs",
        "mke2fs",
        "debugfs",
        "e2fsck",
        "mformat",
        "mcopy",
        "sfdisk",
      ];
      const env: SdEnvironment = {
        kit_root: "/Applications/TMP NAM.app/Contents/Resources/device",
        kit_error:
          flag("sd") === "assets"
            ? "Card assets failed verification: helpers/register_nam_ir.py: SHA-256 mismatch"
            : null,
        tools: tools.map((name) => ({
          name,
          path:
            flag("sd") === "tools" && (name === "mke2fs" || name === "debugfs")
              ? null
              : `/opt/homebrew/bin/${name}`,
        })),
        default_firmware: null,
        firmware_sha256: "392dd4a2",
        platform: "macos",
      };
      return env;
    }
    case "sd_check_firmware": {
      await sleep(800);
      const path = args.path as string;
      return {
        path,
        bytes: 564635158,
        sha256: "392dd4a2",
        matches: path.endsWith("v1_8_58.img"),
      };
    }
    case "sd_list_disks": {
      await sleep(200);
      const disks: Disk[] = [
        {
          device: "/dev/disk4",
          name: "Generic STORAGE DEVICE",
          bytes: 31_914_983_424,
          protocol: "USB",
          rejected: null,
        },
        {
          device: "/dev/disk5",
          name: "SanDisk SDDR-B531",
          bytes: 63_864_569_856,
          protocol: "USB",
          rejected: null,
        },
        {
          device: "/dev/disk6",
          name: "Built-in SD Card Reader",
          bytes: 63_864_569_856,
          protocol: "Secure Digital",
          rejected: null,
        },
        {
          device: "/dev/disk7",
          name: "Samsung PSSD T7",
          bytes: 1_000_204_886_016,
          protocol: "PCI-Express",
          rejected: "target is not on a USB card reader or SD slot",
        },
      ];
      return disks;
    }
    case "set_quit_guard":
    case "quit_now":
      return null;
    case "diagnostics":
      return [
        "TMP NAM 0.1.0 (browser mock)",
        "Platform: browser",
        "Unit: simulator · card build nam-card-2026.10-r1",
      ].join("\n");
    case "sd_write_card":
      void buildCard();
      return null;
    default:
      throw new Error(`mock: unknown command ${cmd}`);
  }
}

const BUILD: [string, number][] = [
  ["Validating official firmware", 700],
  ["Extracting firmware", 1200],
  ["Verifying firmware payload", 700],
  ["Adding root console and NAM support", 500],
  ["Building root filesystem", 1000],
  ["Preparing partitions", 500],
  ["Writing rootfs partition", 6000],
  ["Verifying rootfs", 3000],
];

async function buildCard() {
  if (flag("fail") === "denied") {
    await sleep(800);
    emit("sd://done", {
      ok: false,
      code: 130,
      message: "Administrator access was not granted; nothing was written",
      outcome: "denied",
      stage: null,
    });
    return;
  }
  if (flag("fail") === "blocked") {
    await sleep(800);
    emit("sd://done", {
      ok: false,
      code: 77,
      message: "macOS refused access to the SD card; nothing was written",
      outcome: "blocked",
      stage: null,
    });
    return;
  }
  await sleep(1200);
  let percent = 0;
  for (const [stage, [label, ms]] of BUILD.entries()) {
    const ticks = stage >= 6 ? 20 : 1;
    for (let t = 1; t <= ticks; t++) {
      await sleep(ms / ticks);
      percent = Math.min(100, percent + 100 / BUILD.length / ticks);
      if (stage === 7 && t === 10 && flag("fail") === "verify") {
        emit("sd://log", {
          line: "ERROR: rootfs partition readback hash mismatch",
          stage,
        });
        emit("sd://done", {
          ok: false,
          code: 1,
          message: "rootfs partition readback hash mismatch",
          outcome: "failed",
          stage,
        });
        return;
      }
      emit("sd://log", {
        line: label,
        label,
        percent,
        stage,
        fraction: t / ticks,
      });
    }
  }
  emit("sd://log", { line: "Card verified and ejected — ready to boot" });
  emit("sd://done", {
    ok: true,
    code: 0,
    message: "Done",
    outcome: "ok",
    stage: null,
  });
}
