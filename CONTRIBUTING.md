# Contributing

TMP NAM is a Tauri 2 app (Rust backend + React/TypeScript frontend) plus an SD-card builder and
an ARM NAM player for the Fender Tone Master Pro (firmware 1.8.58 only). It runs on **macOS and
Linux**; Windows is not supported.

- **Start here:** [`CLAUDE.md`](CLAUDE.md) — architecture, the unit protocol, and the invariants
  that break silently (engine restarts, `userIRs.json`, console line limits).
- **Building, release pins, ARM binaries:** [`BUILDING.md`](BUILDING.md).
- **The card and the device:** [`docs/`](docs/).
- **Legal posture:** [`NOTICE`](NOTICE). Never commit firmware images, extracted firmware files,
  or NAM captures (`build/`, `*.img`, `*.nam` are ignored).

## Build & test

Requires stable Rust (pinned by `rust-toolchain.toml`) and [Bun](https://bun.sh). On Linux also:

```bash
sudo apt-get install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libudev-dev libxdo-dev
```

The card builder needs `squashfs e2fsprogs mtools util-linux` (Homebrew) or
`squashfs-tools e2fsprogs mtools util-linux dosfstools parted udev` (Linux).

```bash
scripts/check.sh quick     # syntax, release-pin integrity, no-exploit grep
scripts/check.sh all       # + cargo test/clippy -D warnings + typecheck/lint/vitest
cd apps/desktop && TMP_NAM_SIM=1 bun run tauri dev   # simulated unit, no hardware
```

CI (`.github/workflows/ci.yml`) runs the same checks on macOS and Linux, builds unsigned bundles,
and lints every commit message.

## Pull requests

`main` is protected: every change lands through a squash-merged PR with green CI and an approving
review. [CodeRabbit](https://coderabbit.ai) reviews every ready PR; @pcavadas reviews and merges
external contributions.

- Open as **draft** while iterating (CodeRabbit skips drafts), mark ready when settled.
- Every commit and the PR title follow [Conventional Commits](https://www.conventionalcommits.org)
  (`feat:`, `fix:`, `docs:`, `chore:`, …). The squashed title drives the release version.
- Address CodeRabbit findings with a fix or a reasoned reply; don't resolve its threads by hand.
- Keep formatting changes to touched files.

## Official documentation first

Use each tool's documented setup (its guide, recipe or preset) rather than custom scripts or
low-maintenance wrappers, and adapt it only where this repo requires it, with a comment saying
why. Prefer documented defaults and official dependencies/actions. Details: the "Official
documentation first" section of [`CLAUDE.md`](CLAUDE.md).

## Rules that protect the release

- `device/release.json` pins every byte that lands on the card. Never hand-edit a hash to make
  `cargo test -p tmp-sdcard --test release` pass. ARM binaries change only through
  `tools/release/build_player.py` → `publish_release.py`; see BUILDING.md.
- Device-side Python is 3.5 (no f-strings) and the device shell is BusyBox ash.
- Changes to the player, the card builder, or the unit protocol need a check on a real unit
  (boot, audio, capture switching) before they ship; say in the PR what was verified.
