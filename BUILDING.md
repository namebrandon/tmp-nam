# Building TMP NAM

## Desktop app (`apps/desktop`)

Prerequisites: Rust (stable), [Bun](https://bun.sh), Xcode Command Line Tools.

```sh
cd apps/desktop
bun install
bun run tauri dev                  # real unit over USB
TMP_NAM_SIM=1 bun run tauri dev    # simulated unit
bun run dev                        # UI only, in a browser, against the in-page mock (src/lib/mock.ts)
bun run tauri build                # .app + .dmg (bundles device/ as a resource)
```

Frontend checks: `bun run typecheck`, `bun run lint`, `bun run test`. Rust:
`cargo test --workspace`, `cargo clippy --workspace --all-targets`. Everything:
`scripts/check.sh all`.

Backend modules (`apps/desktop/src-tauri/src/`): `console.rs` (USB root console
transport, marker-framed commands, base64 file push), `unit.rs` + `unit_helper.py`
(captures on the unit), `t3k.rs` + `variants.rs` (Tone3000), `sdcard.rs` (card
builder driver), `settings.rs`. The binary doubles as the card builder's helper:
`TMP NAM --sdcard-helper image|write|list|verify …` (see `crates/sdcard/src/cli.rs`);
the app runs it unprivileged for image files and through an administrator prompt
for physical cards.

App icon (the gain knob): `src-tauri/icons/` holds the pack's ready-made files, copied byte for
byte (`icon.icns` ← `macos/AppIcon.icns`, `icon.ico` ← `windows/TmpNam.ico`, the PNGs ← the
`linux/hicolor` 32/128/256 sizes). Don't run `tauri icon`: it downscales the 1024 master, while
sizes ≤ 32 px use their own simplified drawing. Sources and regeneration (`knob.py`,
`build_icons.py`): `apps/desktop/design/icons/`.

Logs go to the platform log directory (macOS: `~/Library/Logs/dev.tmpnam.app/tmp-nam.log`,
2 MB × 3 files); Settings › Unit › Copy Diagnostics copies versions, the unit's details
and the end of the log (never the Tone3000 key or tokens).

### Releases (signed + notarized macOS, Linux packages)

Releases are cut by CI (`.github/workflows/release.yml`): on every merge to `main`,
release-please keeps a release PR open whose version follows the Conventional Commits since the
last tag. Merging that PR tags `v<version>` and creates the GitHub release;
`tauri-apps/tauri-action` then adds signed and notarized macOS DMGs (Apple Silicon and Intel)
and Linux `.deb`/`.AppImage`. Windows is not
supported (the unit's console transport is Unix-only, and card building needs e2fsprogs/mtools).

To reproduce a signed build locally, on a Mac with your Developer ID certificate in the login
keychain:

```sh
security find-identity -v -p codesigning          # copy the "Developer ID Application: …" name
export APPLE_SIGNING_IDENTITY="Developer ID Application: <Name> (<TEAMID>)"
export APPLE_ID="<apple id email>"
export APPLE_PASSWORD="<app-specific password>"   # appleid.apple.com › Sign-In and Security
export APPLE_TEAM_ID="<TEAMID>"
cd apps/desktop && bun run tauri build             # signs, notarizes and staples the .app/.dmg
```

`tauri.conf.json` reads its version from `apps/desktop/package.json`; release-please bumps
it (and the root `package.json`) in the release PR. Keep the bundle `identifier`
(`dev.tmpnam.app`) stable: notarization, the log directory and the settings location are
tied to it. Tauri enables the hardened runtime by default; no extra entitlements are needed
(the app uses no sandbox, and the card writer re-runs the app's own binary through the
administrator prompt).

Check the result on a second Mac (or after clearing quarantine caches):

```sh
spctl -a -vvv -t install "../../target/release/bundle/dmg/TMP NAM_<version>_aarch64.dmg"
xcrun stapler validate "../../target/release/bundle/macos/TMP NAM.app"
```

Then open it, connect the unit, add and remove a capture, and build an image-file card
(`--sdcard-helper image`) before publishing.

## Card builder (`crates/sdcard`)

Host tools: `brew install squashfs e2fsprogs mtools util-linux` (macOS) or
`squashfs-tools e2fsprogs mtools util-linux dosfstools parted udev` (Linux). The
builder searches PATH plus the Homebrew keg-only locations.

```sh
cargo run --release -p tmp-sdcard -- verify
cargo run --release -p tmp-sdcard -- image <ToneMasterPro_v1_8_58.img> <new-output.img>
sudo target/release/tmp-sdcard write <firmware.img> --device /dev/diskN
```

Validate on a machine where physical media must not be touched with `image` only.

## The release pins (`device/release.json`)

`device/release.json` is the single source of truth for what goes on the card:
the firmware and its rootfs payload, the stock files the builder patches, every
asset under `device/` (size + SHA-256), the ext4/partition layout, the player's
compiled-input hashes (`source_sha256`), build provenance, and license hashes.

- It is compiled into the app and `tmp-sdcard` (`include_str!`), and every asset
  is re-verified before a build — a bundled `device/` can't select other bytes.
- `cargo test -p tmp-sdcard --test release` fails when an asset, a license, or any
  compiled input under `player/` (list: `tools/release/compiled_inputs.json`)
  differs from the pins.
- Never hand-edit hashes to get past a failure. Asset files under `device/` other
  than the three binaries (units, scripts, helpers, models) can be re-pinned by
  updating their entry together with the file; binaries and `source_sha256` are
  only updated by `tools/release/publish_release.py`.
- After changing `release.json`, rebuild the app/CLI.

## Rebuilding the ARM binaries (`player/`, `tools/release/`)

Requires macOS on Apple Silicon, Python 3.11+, Git, make, curl and network access
(pinned compiler and dependency downloads).

```sh
python3 tools/release/build_player.py --output build/player --work-dir build/player-work
python3 tools/release/publish_release.py --build build/player
cargo test -p tmp-sdcard            # confirms the new pins
```

`build_player.py` bootstraps pinned NAM Core/Eigen/SpeexDSP
(`player/stubs/vendor/VERSION`) and the messense aarch64 GCC 10.2 toolchain, builds
`nam_dispatch.so` (`NAM_BUILD_PASS=perf`, Cortex-A57) and static musl Dropbear
2024.86 with pinned Zig, and records provenance. Output and work directories must be
new/empty. `publish_release.py` copies the binaries and licenses into `device/` and
rewrites `release.json` atomically. This is source reconstruction, not
byte-reproducibility: new binaries get new hashes and need a fresh owner check on
the unit (boot, audio, switching) before they are trusted.

Individual player builds (all honor `TMP_NAM_VENDOR_DIR`; use one per worktree):

```sh
bash player/bootstrap_nam_deps.sh && bash player/check_nam_deps.sh   # check applies patches/model.cpp.patch
bash player/bootstrap_nam_toolchain.sh
bash player/build_nam_dispatch.sh
bash player/build_dropbear.sh
bash player/run_nam_native_tests.sh                                  # host tests, parity + perf
```

