# NAM A1/A2 on the TMP SD card

The card builder (`crates/sdcard`, used by the TMP NAM app) builds the firmware 1.8.58 SD-root card with the USB root console
and one pinned NAM dispatcher. There is no player selector, automatic model
downgrade, or startup NAM preset.

## Supported scope

The player reads mono NAM A1 and A2 models at 44.1 or 48 kHz, including A2
slimmable containers and channel-sliced models. Firmware processing is 44.1
kHz. Different-rate models use streaming SpeexDSP Q5 conversion; equal-rate
models bypass conversion.

File-format support does not imply that every network fits the TMP callback
budget: 32 frames at 44.1 kHz, a 725.6-microsecond period. A capture fits when,
with one active instance, p99.9 stays below 75% of that period with zero
deadline misses, processing errors and conversion underflows. Known results:

| Network | Fits |
| --- | --- |
| A1 WaveNet 8/4 | Yes |
| A1 WaveNet 16/8 (Matchless DC30) | No |
| A2 three-channel child (Matchless DC30, size `0.0`) | Yes, also with a 4x12 cabinet IR |
| A2 eight-channel child (Matchless DC30, size `0.5` or `1.0`) | No |
| A2 eight-channel child (Marshall JCM800, 48 kHz, resampled) | Yes (avg 335 µs, max 614 µs) |

“A1,” “A2,” or “Lite” is therefore insufficient as a general admission rule;
check each capture and configuration on the unit before relying on it. Dense
presets, two instances and sustained thermal load are not covered.

The player keeps the full network as the default and never changes a capture
silently. Optional `/data/nam/player.json` selects a size by model hash on the
next load:

```json
{"models":{"<sha256-of-model-bytes>":{"size":0.0}}}
```

Per-model `output_gain` (0.0–8.0, default 1.0) compensates captures that land
quieter than stock blocks (typically ~4.0). It is explicit configuration, never
a silent alteration — an absent key is bit-identical to the model:

```json
{"models":{"<sha256-of-model-bytes>":{"output_gain":4.0}}}
```

Provisioning preserves an existing `player.json` and does not create one.
For a `SlimmableContainer`, upstream uses exclusive `max_value` thresholds;
size is a selector rather than a percentage. Missing rate metadata requires an
explicit `sample_rate_hz` of 44100 or 48000, and conflicting known rates are
rejected.

The player uses upstream's fast tanh for A1. On the comparison fixture it
differs from exact tanh by at most `0.004881421`; reference and performance
builds configured with the same fast tanh are bit-identical.

## Model switching

A ControlProxy load submits preparation to the non-real-time `LatestLoader`
worker, waits for a fresh fully prepared `Player`, publishes that exact
generation, and only then returns success. During replacement the prior player
remains callback-visible; a stale completion cannot overwrite a newer load or
ordinary-IR clear.

Switching reuses the firmware's preset machinery. The firmware prepares the
inactive graph bank and performs its existing completion/fade transition; the
NAM hook does not add another fade. By holding the stock IR load until the
selected NAM is reset, prewarmed, activated, and published, the inactive bank
cannot report success with a late NAM swap still pending.

The player backports upstream Core lifecycle behavior from
`d65cf2114e4a9e083292b235af1a24789d6fe128`,
`e49c93e678549230d09efbb0beeb50511e387874`, and
`6e32dca12e998c139c624a8f1a1d277fbc02e83e` while retaining the pinned Core
revision. Factory construction suppresses automatic warming; the selected
final child is reset and prewarmed once, and pending slimmable activation is
consumed on the loader worker with the existing zero-frame call.

With the firmware's CPU1 audio-IRQ routing, switches produce four-channel
attenuation and exact-zero gaps, so the card moves the four audio IRQs and
their eight IRQ threads to CPU3. AudioProc stays CPU1/FIFO49 and the IRQ
threads FIFO98. A NAM recall is acknowledged in roughly 0.34–0.36 seconds,
versus about 0.20 seconds for stock slots.

## Rebuild NAM and SSH from source

[`BUILDING.md`](../BUILDING.md) covers rebuilding the dispatcher and the SSH
tools from pinned sources and publishing them into `device/`. Newly compiled
bytes get new hashes and need a check on the unit before they are trusted.

## Build the SD-card image

Supply the unmodified official `ToneMasterPro_v1_8_58.img`. Portable mode uses
standard host filesystem tools and requires neither Docker nor disk mounting:

```sh
cargo run --release -p tmp-sdcard -- image ~/Downloads/ToneMasterPro_v1_8_58.img \
  ~/Downloads/TMP-NAM-A1-A2.img
shasum -a 256 ~/Downloads/TMP-NAM-A1-A2.img
```

Use a new output filename. The builder verifies the firmware, official boot
files, pinned dispatcher, filesystem metadata, permissions, and installed-file
readback. It also verifies and patches the official
`increase-priorities.sh` from source SHA-256
`7383cd653533a667b7318289621e00794fa7ad9035d0253f165459d82acd6b0f` to
`2563af86a443f66e9af702a697d9b8fcb21fabf199c6195a29ab08925af3e931`:
the existing dynamic audio-IRQ discovery is retained, while its four IRQs and
eight primary/secondary threads move from CPU1 to CPU3. The original tar
inventory ownership and mode are preserved and checked after ext4 creation.
Card-writing and restoration instructions are in
[the SD console guide](device/usb-console.md).

The checked-in dispatcher (2,508,128 bytes, SHA-256
`669c317e59fd479fd407762c93ef0d4e463ad34e33c2af13d82b6467a30bf525`) is built
from the revisions pinned in `player/stubs/vendor/VERSION`, for ARMv8-A tuned
for Cortex-A57. It uses the generic model path with the retained AArch64
Dense8x8 NEON optimization, C++17, and no fast-math or `NAM_ENABLE_A2_FAST`
specialization. `player/build_nam_dispatch.sh` first rebuilds Core from
canonical sources, then performs the release strip and verifies ELF64
AArch64, allowed dynamic dependencies, and GLIBC no newer than 2.28.

Native dispatcher coverage always runs five hash-pinned upstream example
models from the pinned NeuralAmpModelerCore checkout (including the full-size
`wavenet_a2_max.nam`); no NAM capture is tracked in this
repository.

## Device use

1. Boot with a non-NAM preset and wait for `NAM dispatch ARMED` in
   `/tmp/nam_dispatch.log`.
2. Verify `/usr/local/lib/nam_dispatch.so` is
   `669c317e59fd479fd407762c93ef0d4e463ad34e33c2af13d82b6467a30bf525`.
3. Add a capture from the app and load it through the normal User IR
   picker. If it needs a smaller A2 size, set it in `player.json` before
   loading it.
4. Confirm `NAM ready before load return` reports the intended hash, model
   rate, latency, and generation, then confirm increasing block counts with zero
   errors and underflows.
5. Enable `TMP_NAM_PROFILE=1` only for measurements. Compare p99.9 against the
   budget above and keep the profiler setting identical between captures.

Failed replacements retain the prior ready player. A failed initial NAM load
removes its empty registry slot and leaves the unity placeholder on Fender's
path, producing explicit bypass rather than silence. The graph-load visitor
converts the dispatcher's runtime error to load failure; the stock live
string-edit path may only log the error and report success, an inherited
firmware limitation. Ordinary WAV selection cancels pending NAM work and
restores Fender IR processing. Stop using a capture if
it produces a deadline miss, processing error, conversion underflow, corrupted
audio, crash, or hang.

When a rooted test needs a full application restart, stop
`fmic-app-started.target`, then `fmic-platform-ready.target`. Reconnect the USB
console and start `fmic-app-started.target`; never *stop* and then *start* only
`tm-stomp-server.service` — `fmic-platform-ready.target` is `BindsTo=` it and the UI
client `Requires=` that target, so the client stays down. `systemctl restart
tm-stomp-server` does propagate to the target and client, and the player
re-arms; the TMP NAM app and `t3k_sync.py` use it to reload the IR picker.
Verify the client and advancing ALSA pointers before recording.

The SD root is normally read-only. Removing the card returns to the stock root
filesystem after power-off, but does not undo persistent `/data` changes such
as User IR registration, model files, or `player.json`.
