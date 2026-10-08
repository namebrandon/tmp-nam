// src/lib/api.ts — typed wrappers over the Rust commands (src-tauri/src/lib.rs).
//
// Outside Tauri (plain `vite` in a browser, Vitest) every call resolves from the
// in-memory mock in ./mock so the pages render and can be clicked through.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { mockInvoke, mockListen, mockPick } from "./mock";

export interface ModelMeta {
  name?: string | null;
  modeled_by?: string | null;
  gear_make?: string | null;
  gear_model?: string | null;
  gear_type?: string | null;
  tone_type?: string | null;
}

export interface Submodel {
  architecture?: string | null;
  channels?: number | null;
  max_value: number;
}

export interface ModelInfo {
  architecture?: string | null;
  version?: string | null;
  sample_rate?: number | null;
  /** First-layer width of a single network (A1): names its size. */
  channels?: number | null;
  meta: ModelMeta;
  submodels: Submodel[];
}

export interface PlayerOptions {
  size?: number;
  output_gain?: number;
}

/** Omitted fields keep the stored value; null removes an override. */
export interface PlayerOptionsPatch {
  size?: number | null;
  output_gain?: number | null;
}

export interface CaptureList {
  models: Capture[];
  settings_error?: string | null;
}

export interface UnitModel {
  name: string;
  file: string;
  bytes: number;
  registered: boolean;
  present: boolean;
  sha256?: string | null;
  info?: ModelInfo | null;
  error?: string | null;
  options: PlayerOptions;
}

export interface UnitInfo {
  port: string;
  build_id?: string | null;
  dispatch_sha256?: string | null;
  python?: string | null;
  simulated: boolean;
  /** A transfer or engine restart holds the connection. */
  busy: boolean;
}

/** Where a capture came from when Tone3000 installed it (installs.json). */
export interface InstallSource {
  tone_id: number | string;
  model_id: number | string;
  variant?: string | null;
}

export interface Capture extends UnitModel {
  source?: InstallSource | null;
}

export interface Inspected {
  path: string;
  name: string;
  bytes: number;
  info?: ModelInfo | null;
  error?: string | null;
}

export type Stop =
  | { kind: "disconnected" }
  | { kind: "disconnected_during_restart" }
  | { kind: "failed"; message: string };

export interface AddOutcome {
  added: string[];
  failed_after_restart: string[];
  interrupted?: string | null;
  not_sent: string[];
  stop?: Stop | null;
  /** Went through the restart fallback (no HID channel, e.g. Pro Control open). */
  needs_restart: boolean;
}

export type OpEvent =
  | { phase: "download"; index: number; done: number; total: number }
  | { phase: "send"; index: number; done: number; total: number }
  | { phase: "sent"; index: number }
  | { phase: "restart" };

export interface Variant {
  id: string;
  arch: number;
  label: string;
  rank: number;
}

export interface Settings {
  t3k_key: string;
  variants: string[];
}

export interface T3kStatus {
  has_key: boolean;
  linked: boolean;
  username?: string | null;
}

export interface T3kModel {
  id: number | string;
  name?: string | null;
  size?: string | null;
  architecture_version: string;
  variant?: string | null;
  model_url?: string | null;
  ir_name: string;
  /** Size from `size` or a size word in the name; tells several A2 models apart. */
  size_hint?: string | null;
}

/** One capture of a tone: its models (A2 and/or A1 sizes) by index into `models`. */
export interface T3kCapture {
  name: string;
  models: number[];
}

export interface T3kTone {
  id: number | string;
  title: string;
  author?: string | null;
  source: "bookmark" | "created";
  models: T3kModel[];
  captures: T3kCapture[];
}

export interface SdTool {
  name: string;
  path?: string | null;
}

export interface SdEnvironment {
  kit_root?: string | null;
  kit_error?: string | null;
  tools: SdTool[];
  default_firmware?: string | null;
  firmware_sha256?: string | null;
  platform: string;
}

export interface Disk {
  device: string;
  name: string;
  bytes: number;
  protocol: string;
  rejected?: string | null;
}

export interface FirmwareCheck {
  path: string;
  bytes: number;
  sha256: string;
  matches: boolean;
}

export interface SdLog {
  line: string;
  percent?: number | null;
  label?: string | null;
  /** 0–7, the build stage the line belongs to. */
  stage?: number | null;
  /** Progress within the stage, 0–1. */
  fraction?: number | null;
}

export interface SdDone {
  ok: boolean;
  code: number;
  message: string;
  /** `denied`: the administrator prompt was cancelled; `blocked`: macOS refused
   *  this app Removable Volumes access. Nothing was written in either case. */
  outcome: "ok" | "failed" | "denied" | "blocked";
  stage?: number | null;
}

export interface T3kPick {
  model_url: string;
  ir_name: string;
  tone_id: number | string;
  model_id: number | string;
  variant?: string | null;
}

/** An error from a command that classifies its failures (Tone3000). */
export class ApiError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

export function isTauri(): boolean {
  return "__TAURI_INTERNALS__" in window;
}

async function call<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    if (isTauri()) return await tauriInvoke<T>(cmd, args);
    return (await mockInvoke(cmd, args)) as T;
  } catch (e) {
    if (e instanceof ApiError) throw e;
    if (typeof e === "object" && e !== null && "code" in e && "message" in e)
      throw new ApiError(String(e.code), String(e.message));
    throw new Error(
      typeof e === "string" ? e : e instanceof Error ? e.message : String(e),
    );
  }
}

/** Backend events and their payloads. */
export interface Events {
  "op://event": OpEvent;
  "sd://log": SdLog;
  "sd://done": SdDone;
  "app://quit-requested": null;
}

/** Subscribe to a backend event; returns the unsubscribe function. */
export function listen<K extends keyof Events>(
  event: K,
  cb: (payload: Events[K]) => void,
): () => void {
  if (!isTauri())
    return mockListen(event, (p) => {
      cb(p as Events[K]);
    });
  const un = tauriListen<Events[K]>(event, (e) => {
    cb(e.payload);
  });
  return () => {
    void un.then((f) => {
      f();
    });
  };
}

export const api = {
  unitConnect: () => call<UnitInfo>("unit_connect"),
  unitList: () => call<CaptureList>("unit_list"),
  namInspect: (paths: string[]) => call<Inspected[]>("nam_inspect", { paths }),
  unitAddFiles: (files: { path: string; name?: string }[]) =>
    call<AddOutcome>("unit_add_files", { files }),
  /** Remove and register resolve `true` when the audio engine had to restart. */
  unitRemove: (names: string[]) => call<boolean>("unit_remove", { names }),
  unitRegister: (names: string[]) => call<boolean>("unit_register", { names }),
  unitReload: () => call<null>("unit_reload"),
  unitSetOptions: (sha256: string, options: PlayerOptionsPatch) =>
    call<string | null>("unit_set_options", { sha256, options }),

  settingsGet: () => call<Settings>("settings_get"),
  settingsSet: (settings: Settings) => call<null>("settings_set", { settings }),
  variantsList: () => call<Variant[]>("variants_list"),

  t3kStatus: () => call<T3kStatus>("t3k_status"),
  t3kLink: () => call<string | null>("t3k_link"),
  t3kOpenLinkAgain: () => call<null>("t3k_open_link_again"),
  /** tone3000.com in the system browser. */
  t3kOpenSite: () => call<null>("t3k_open_site"),
  t3kCancelLink: () => call<null>("t3k_cancel_link"),
  t3kUnlink: () => call<null>("t3k_unlink"),
  t3kTones: () => call<T3kTone[]>("t3k_tones"),
  t3kInstall: (picks: T3kPick[]) => call<AddOutcome>("t3k_install", { picks }),

  sdEnvironment: () => call<SdEnvironment>("sd_environment"),
  sdCheckFirmware: (path: string) =>
    call<FirmwareCheck>("sd_check_firmware", { path }),
  sdListDisks: () => call<Disk[]>("sd_list_disks"),
  sdWriteCard: (firmware: string, device: string) =>
    call<null>("sd_write_card", { firmware, device }),
  /** System Settings › Privacy & Security › Files and Folders (macOS). */
  sdOpenPrivacySettings: () => call<null>("sd_open_privacy_settings"),
  /** Versions, platform, unit and the end of the log, for bug reports. */
  diagnostics: () => call<string>("diagnostics"),
  /** While on, closing the window or quitting asks first. */
  setQuitGuard: (on: boolean) => call<null>("set_quit_guard", { on }),
  quitNow: () => call<null>("quit_now"),
};

/** Paths dropped onto the window (Tauri drag-drop); no-op outside Tauri. */
export function onFileDrop(cb: (paths: string[]) => void): () => void {
  if (!isTauri()) return () => undefined;
  let un: (() => void) | undefined;
  let gone = false;
  void import("@tauri-apps/api/webview").then(({ getCurrentWebview }) =>
    getCurrentWebview()
      .onDragDropEvent((e) => {
        if (e.payload.type === "drop") cb(e.payload.paths);
      })
      .then((f) => {
        if (gone) f();
        else un = f;
      }),
  );
  return () => {
    gone = true;
    un?.();
  };
}

export const NAM_FILTER = [{ name: "NAM captures", extensions: ["nam"] }];

/** Native file pickers (tauri-plugin-dialog); `null` when cancelled. The mock picks sample files. */
export async function pickFiles(
  filters: { name: string; extensions: string[] }[],
  multiple: boolean,
): Promise<string[] | null> {
  if (!isTauri()) return mockPick(filters.flatMap((f) => f.extensions));
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ multiple, filters, directory: false });
  if (r === null) return null;
  return Array.isArray(r) ? r : [r];
}
