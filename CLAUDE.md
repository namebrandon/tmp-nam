# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Scope

NAM (neural amp model) playback for the owner's Fender Tone Master Pro (firmware 1.8.58
only) and the **TMP NAM** desktop app: manage captures on the unit, Tone3000 sync, and
the bootable SD-card builder. There is no exploit-development, firmware reverse-engineering
or binary-analysis code here; `scripts/check.sh` enforces that with a grep.

## Routing

| Area | Entry point |
|---|---|
| Desktop app UI (React/TS; TMP NAM design system in `src/ds`, tokens in `src/theme/tokens.css`) | `apps/desktop/src/App.tsx`, `apps/desktop/src/views/`, app store in `src/state/` |
| Design source of truth (spec, tokens, screens, prototype from Claude Design) | `apps/desktop/design/HANDOFF.md` |
| App icon (gain knob): bundled files / sources and regeneration | `apps/desktop/src-tauri/icons/` / `apps/desktop/design/icons/` (never `tauri icon`) |
| App backend: unit console, captures, Tone3000, settings | `apps/desktop/src-tauri/src/{console,unit,t3k,variants,settings}.rs`, `unit_helper.py` |
| SD-card builder | `crates/sdcard/src/` — `lib.rs` pipeline, `cli.rs` commands |
| Everything copied onto the card + all pins | `device/`, `device/release.json` |
| NAM player source and ARM build | `player/`; `docs/nam-player.md` |
| Rebuild/publish the ARM binaries | `tools/release/`; `BUILDING.md` |
| Card guide, USB console, Wi-Fi/SSH | `docs/sd-card.md`, `docs/device/` |
| CI, releases, review/merge automation | `.github/workflows/`, `.coderabbit.yaml`, `.github/dependabot.yml`, `.github/rulesets/protect-main.json` |

## Commands

```bash
scripts/check.sh quick                     # syntax + release-pin integrity + no-exploit grep
scripts/check.sh all                       # + cargo test/clippy -D warnings + desktop typecheck/lint/vitest
cargo test --workspace                     # Rust (crates/sdcard + app backend, incl. pty console tests)
cargo test -p tmp-sdcard --test release    # device/ assets, player sources, licenses vs release.json
cd apps/desktop && bun run tauri dev       # TMP_NAM_SIM=1 → simulated unit (TMP_NAM_SIM_FAIL=disconnect|restart|drop, TMP_NAM_SIM_RESTART=1)
cd apps/desktop && bun run dev             # browser + mock; ?fail= ?flags=1 ?t3k= ?sd= reach error screens (src/lib/mock.ts)
node apps/desktop/scripts/gen-tokens.mjs   # regenerate src/theme/tokens.css after editing tokens.json
cargo run --release -p tmp-sdcard -- image <ToneMasterPro_v1_8_58.img> <new.img>
```

- Frontend lint is strict type-aware eslint with `noInlineConfig` (no disable comments) and
  `react-hooks/set-state-in-effect`: kick async loaders from effects with `defer()`
  (`src/lib/format.ts`), not a direct call.
- `player/run_nam_native_tests.sh` runs the host NAM tests (not part of check.sh).

## Architecture: the release pins

`device/release.json` is the single source of truth: firmware + payload hashes, the engine
hash, stock files the builder patches (anchors + before/after hashes), every `device/` asset
(size + SHA-256 + rootfs target), layout/ext4 parameters, the player's compiled-input hashes
(`source_sha256`), build provenance and license hashes.

- It is `include_str!`'d into `tmp-sdcard` (and therefore the app): changing it requires a
  rebuild, and a bundled `device/` is re-verified against the compiled-in pins before every
  build.
- `cargo test -p tmp-sdcard --test release` fails if any asset, license or compiled input
  (`tools/release/compiled_inputs.json`) differs. Never hand-edit hashes to pass it. Binaries
  and `source_sha256` change only via `tools/release/build_player.py` →
  `publish_release.py` (Apple Silicon, network); new binaries need an owner check on the
  unit (boot, audio, switching). Non-binary assets (units, scripts, helpers) are re-pinned by
  updating their `release.json` entry with the file.

Card builder pipeline (`crates/sdcard`): validate firmware → `unsquashfs` RAUC → inventory the
rootfs tar (`firmware.rs`) → system `tar -xzpf` → `rootfs.rs` delta (CDC-ACM console in the
USB gadget, drop `data=journal`, audio IRQs to CPU3, NAM/LAN assets, build marker) →
`mke2fs -d` + `debugfs` ownership/mode fix-up + `e2fsck` + audit (`ext4.rs`) → image
(`image.rs`: mformat/mcopy FAT, sfdisk MBR, readback) or card (`card.rs`: diskutil/sfdisk,
raw write, 4 GiB readback). Generated text files are in `templates.rs` and must stay
byte-identical (the audit compares them).

App ↔ unit: the card's root shell — the CDC-ACM tty under the USB device with the TMP's
VID/PID `0x1ED8`/`0x44` (same IDs TMP Companion uses), found via `ioreg`/sysfs (`console.rs`). Commands are
marker-framed; `stty -echo` puts ash's line editor in fgets mode (≤1 KiB lines); file pushes
are 30-line base64 heredoc batches each acknowledged by exit code, because a tty input queue
drops overflow. Adds and removes go through the running engine over the USB HID channel
(`hid.rs`, Pro Control's protocol), so the picker updates with no restart: bytes are pushed
over the console first, `AddUserIR` registers the name with a placeholder WAV (one message is
capped at 65,535 bytes), then `install` renames the real file over it; `RemoveUserIR` (1-based
slot) deletes entry and file. The engine drops a HID client after 0.75 s without traffic and
then ignores it silently (heartbeat thread + in-pump heartbeat); the session is kept open for
the connection because macOS refuses exclusive re-opens for tens of seconds after a close.
The engine persists `userIRs.json` ~0.5–0.8 s after a change; never edit it while HID-added
changes are pending (both writers use `userIRs.json.tmp`). Fallback when HID can't open (Pro
Control holds it, no hidraw access): register the IR name before the file lands (the firmware
prunes unregistered files), then one `systemctl restart tm-stomp-server` reloads the picker. Never
stop/start it separately: `fmic-platform-ready.target` is `BindsTo=` the server and the UI
client `Requires=` that target, so a stop kills the client and a start doesn't revive it;
`restart` propagates, but only to a client that is running:
a client stopped by a failed start stays down, so `wait_engine_active` starts it. The engine
allows **2 starts per minute** (`StartLimitBurst=2`); a third leaves the unit silent with
`start-limit-hit`, so `reload_engine` paces restarts. Never `systemctl restart` the engine by
hand in probes or scripts. The engine rewrites `userIRs.json` ~1 s after it starts, undoing
edits made before that, so `reload_engine` waits for that write. `systemctl is-active A B`
exits 0 if *any* unit is active: read each state. After
any engine restart the NAM player is loaded but inactive until a capture is selected again
on the unit. Player timing: `systemctl set-environment TMP_NAM_PROFILE=1` + one paced
restart, reselect, read `NAM stats … avg_us max_us deadline_misses` in
`/tmp/nam_dispatch.log`, then `unset-environment` + restart (volatile; no unit files).
A2 sizes and output gain go in
`/data/nam/player.json` keyed by the SHA-256 of the installed bytes.

Sends and installs never fail as a whole: `Unit::add` returns an `AddOutcome` (added, not
loaded, interrupted, not sent, stop reason, `needs_restart` when the fallback ran) and streams
`op://event` steps the UI folds into per-file rows (`src/state/operation.ts`); remove/register
return whether the engine restarted, and only then does the UI ask for a reselect. Whatever an
interrupted batch left registered without its real file (no file, or still the placeholder)
is dropped by `discard_unsent` on the next connection, which is what makes "the interrupted
file was not kept" true. While a transfer holds the unit mutex, `unit_connect`
answers `busy` from cache instead of queueing. Tone3000 installs are recorded by SHA-256 in
`<app config>/installs.json` (A1 size, "Show in Tone3000", the Installed column). The UI keeps
one long operation at a time (send, install, SD build) in the app store above the page switch.

The app binary doubles as the card builder: `--sdcard-helper image|write|list|verify`
(`apps/desktop/src-tauri/src/main.rs`), unprivileged for images, via osascript admin prompt
(pkexec on Linux) for physical cards; it prints JSON progress lines the GUI tails.

## Shell notes

- macOS ships Bash 3.2 and BSD userland; no `timeout`/`gtimeout` — use the tool's timeout.
- Device-side `/bin/sh` is BusyBox ash; device Python is 3.5 (no f-strings in
  `unit_helper.py` or `device/helpers/*`).

## Official documentation first

Applies to everything: CI, tooling config, dependencies, build setup and code.

- Before adding or changing a tool's setup, read its official docs and use the documented
  setup (guide example, recipe, preset). Copy it, then adapt only what this repo forces
  (paths, SHA pins, extra system packages, bun as the app's package manager), with a
  comment naming the guide and why each adaptation is needed.
- Rely on documented defaults and presets; don't restate defaults or reimplement a feature
  the tool already has (e.g. semantic-release's GitHub release, tauri-action's upload,
  Dependabot's grouping and cooldown).
- No custom scripts where a standard solution exists. If none fits, say so in the PR and why.
- New dependencies and GitHub Actions: prefer the official one (vendor org, e.g. `actions/`,
  `tauri-apps/`, `oven-sh/`); otherwise check maintenance (stars, maintainers, recent
  releases) and avoid thin single-maintainer wrappers around a CLI that can be run directly.
- When the docs are wrong or contradict themselves, follow the correct part and note it in a
  comment (e.g. `release.yml`'s Developer ID certificate check).

## Docs and comments

Docs and code comments describe the current state only: no history, dated experiments,
validation logs, milestones or "previously/formerly" notes. Git history and GitHub releases
are the record.

## PR flow, CI and releases

- Platforms: macOS and Linux only; Windows is not supported.
- `main` is protected by the "protect main" ruleset (`.github/rulesets/protect-main.json`, no
  bypass): squash-only PRs, 1 approval (CodeRabbit's counts; a push dismisses it), and the
  checks `checks`, `rust (macos-latest|ubuntu-22.04)`, `frontend`,
  `bundle (macos-latest|ubuntu-22.04)` (`ci.yml`) and `commitlint` (`commitlint.yml`).
  Renaming a job means updating the ruleset.
- Every commit is a Conventional Commit (commitlint); the squashed title drives the version.
- Dependabot (`.github/dependabot.yml`) opens weekly grouped updates with a 7-day cooldown;
  `dependabot-auto-approve.yml` approves them. It cannot parse Bun 1.4's `bun.lock` v2 yet
  (dependabot/dependabot-core#16026), so frontend deps get no Dependabot PRs or alerts; CI
  runs `bun audit` instead.
- The owner's and Dependabot's ready PRs get auto-merge armed by `auto-merge.yml` (GitHub
  App token, so the merge push triggers `release.yml`). External PRs are merged by hand by
  @pcavadas.
- Iterate in draft (CodeRabbit skips drafts). Fix or answer CodeRabbit findings; never post
  `@coderabbitai approve`, `resolve` or `full review`, and never resolve its threads by hand.
- `release.yml`: every merge to `main` runs semantic-release (default plugins); a `feat:`/`fix:`
  since the last tag releases automatically: it tags `v<version>` and creates the GitHub release,
  then tauri-action builds and uploads signed + notarized aarch64/x86_64 DMGs and the Linux
  `.deb`/`.AppImage`. Versions are never bumped by hand: the job stamps the release version into
  `apps/desktop/package.json` (read by `tauri.conf.json`) at build time only. Apple secrets live
  in the `release` environment.

## Conventions

- Keep firmware images and NAM captures out of version control (`build/`, `*.img`, `*.nam`);
  tests use the pinned NeuralAmpModelerCore `example_models/`.
- Use a worktree branch; changes land by squash-merge to `main`.
