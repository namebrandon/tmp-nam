# TMP NAM SD card — create + first boot

Boot a Fender Tone Master Pro (firmware **1.8.58 only**) from an SD card with a USB root console and the NAM player preinstalled. The card carries the ARM executables pinned in `device/release.json`.

> Owner hardware only. Internal firmware partitions are not patched; removing
> the card restores stock boot. While the card is booted, the USB console is an
> unauthenticated **root shell** — keep physical control of the unit.
> Changes under internal `/data` (registry entries, your own NAM files)
> **survive** card removal.
>
> **Disclaimer — read before use:** back up your Tone Master Pro
> (Pro Control backup) before booting this card. You install and use this
> kit entirely **at your own risk**. Neither Tony LaMarca nor pcavadas
> accept responsibility for any damage, data loss, or warranty impact
> to your unit.

## What goes on the card

| Source (`device/`) | On the card |
|---|---|
| `bin/nam_dispatch.so` | NAM player, preloaded into the stock audio engine. Single-channel A1/A2, Speex resampling for 48 kHz models (firmware runs **44.1 kHz**), prepare-before-handoff, per-capture size and output gain. |
| `bin/dropbear`, `bin/dropbearkey` | Static AArch64 SSH server + key generator, staged onto `/data/nam/bin` at boot. |
| `systemd/usb-console.service`, `scripts/usb_console_setup.sh` | CDC-ACM root console over the stock USB-C cable. |
| `systemd/wifi-always-on.service`, `scripts/wifi-always-on.sh` | Powers Wi-Fi after the engine starts. |
| `systemd/dropbear-nam.service` | SSH on the LAN. |
| `helpers/register_nam_ir.py`, `helpers/t3k_sync.py` | User-IR registry helper; on-device Tone3000 pull (the Wi-Fi/SSH fallback to the app). |

NOT included: the official `ToneMasterPro_v1_8_58.img` (564,635,158 B). Obtain it
yourself from Fender's official 1.8.58 release. The builder refuses anything
else (exact size + `sha256:392dd4a2…f41f38ec` enforced before it touches a card).

## 1. Prerequisites

- macOS (Intel or Apple Silicon). A blank SD card, **8 GB minimum** (16–32 GB
  recommended), in the Mac's built-in SD slot or a USB card reader.
- `brew install squashfs e2fsprogs mtools util-linux` (`unsquashfs`, `mke2fs`/`debugfs`/`e2fsck`, `mformat`/`mcopy`, `sfdisk`). The app's SD Card page lists anything missing.
- Linux: use the `tmp-sdcard` CLI (`squashfs-tools e2fsprogs mtools util-linux dosfstools parted udev`).

## 2. Build the card

In **TMP NAM → SD Card**:

1. **Firmware** — *Choose file…* and pick `ToneMasterPro_v1_8_58.img`. It must show *verified 1.8.58*.
2. **SD card** — insert the card; it appears in the list within a few seconds (refused disks show why). Select it.
3. **Create card** — confirm the erase (device, model and size are shown), then enter the macOS administrator password (macOS may first ask to allow TMP NAM access to removable volumes). The app verifies every pinned input, extracts the firmware, builds the 4 GiB rootfs, writes the partition table and the boot partition with the official boot files, writes the rootfs and hashes all 4 GiB back from the card, then ejects it.

Use an identifiable blank spare and keep the currently working card for recovery.

Headless equivalent (also the way to build a portable `.img` for another flashing tool):

```sh
cargo run --release -p tmp-sdcard -- image /path/to/ToneMasterPro_v1_8_58.img /path/to/card.img
sudo target/release/tmp-sdcard write /path/to/ToneMasterPro_v1_8_58.img --device /dev/diskN
```

A good build ends with a complete rootfs-partition readback hash match. A mismatch means: do not
boot it — try another reader/adapter, then another card.

## 3. First boot + checks

1. Unit **off**, insert card, power on, wait for the preset screen.
2. Connect stock USB-C to the computer. A serial device appears
   (`/dev/cu.usbmodem*` or `ttyACM*`). Open it at 115200
   (`screen` or similar) — you get a root shell, no login.
3. Run:
   ```sh
   id                                            # uid=0(root)
   cat /proc/cmdline                             # root=/dev/mmcblk1p2
   findmnt /                                     # may display /dev/root alias
   systemctl is-active usb-console.service        # active
   cat /etc/sd-root-build-id
   sha256sum /usr/local/lib/nam_dispatch.so
   sha256sum /data/nam/bin/dropbear /data/nam/bin/dropbearkey
   sha256sum /data/nam/t3k_sync.py /data/nam/register_nam_ir.py
   # Compare with device/release.json and the build ID (TMP NAM → Settings shows the player hash).
   systemctl is-active nam-lan-install.service dropbear-nam.service
   sha256sum /usr/local/bin/increase-priorities.sh
   pid=$(pidof tm-stomp-server)
   grep nam_dispatch.so /proc/"$pid"/maps         # installed path, no (deleted)
   for irq in 160 161 177 178; do
       printf 'IRQ %s: ' "$irq"
       cat /proc/irq/"$irq"/smp_affinity           # each must be 08 (CPU3)
   done
   systemctl show tm-stomp-server tone-master-stomp-client \
       -p ActiveState -p SubState -p NRestarts -p FragmentPath -p DropInPaths
   systemctl show wifi-always-on -p Type -p ActiveState -p SubState
   # Type=simple; active/running during the script, then active/exited
   sha256sum /etc/systemd/system/wifi-always-on.service
   ```
4. Add a NAM capture from the app (Captures or Tone3000), then select it in the
   User IR picker **after** the preset screen is up. It should play as a gained amp.
   If it sounds like only a cab, check `/data/userIRs.json` and
   `/tmp/nam_dispatch.log`.

## 4. Switching check

Play through the normal instrument input and switch stock → NAM → stock,
then between distinct NAM captures, including a rapid sequence. Listen for
gaps, clicks, a brief wrong tone, missing sound or abnormal noise. Repeat after
a full power cycle. Do not treat a noisy first switch as normal.

Check service health and `/tmp/nam_dispatch.log` during the run for restarts,
load failures, processing errors or underflows. The mapped player and unit
configuration must come from the installed card, without temporary `/run`
service overrides. Keep a known-good card for recovery if any check fails.

## 5. Your own NAM files

Use **TMP NAM → Captures** with the unit connected over USB: **Add captures**
picks `.nam` files, validates them on the Mac and installs them as
`/data/userIRs/<name>.nam.wav`. The running engine adds and removes them over the
unit's USB control channel (Pro Control's), so nothing restarts; with Pro Control
open the app falls back to registering in `/data/userIRs.json` and restarting the
audio engine briefly. Pick them in the unit's User
IR picker and bypass the amp block. **Tone3000** installs your bookmarked and
created tones the same way.

Notes that will save you time:

- A NAM capture in an IR block sounds about 12 dB quieter than a stock amp block
  because the IR block's level defaults to -12 dB. Set the level on the block, per
  preset, on the unit; the app has no gain control. (The player still honours a
  per-capture `output_gain` in `/data/nam/player.json`, see `docs/nam-player.md`.)
- Model size labels do not establish whether a capture fits the approximately
  725 µs callback period. Validate each capture and configuration on the unit;
  stop using one that overruns the budget or produces glitches.
- A2 SlimmableContainers work natively; the Captures page's Feather … Full switch
  writes the size into `/data/nam/player.json` (see [nam-player.md](nam-player.md)).
  It applies the next time the capture is selected.

## 6. Wi-Fi + SSH (no computer needed)

The card powers Wi-Fi and starts SSH automatically. Network association is a
separate prerequisite for pushing `.nam` files:

1. Join your Wi-Fi **once** (factory-test UI → Wi-Fi Test at power-on, or Pro
   Control). ConnMan remembers it; the card auto-joins on later boots.
2. On each boot the LAN installer atomically refreshes `dropbear`, `dropbearkey`, `t3k_sync.py` and `register_nam_ir.py` under internal `/data` to match the card. SSH requires a successful installation. Existing host keys, models, settings and tokens remain; a missing host key is generated on-device. Connect with `ssh root@fmic-tm-pro.local` (blank password until you set one — do not expose port 22 beyond your LAN).
3. Push a capture straight into the User IR store:
   ```sh
   ssh root@fmic-tm-pro.local 'cat > /data/userIRs/My_Model.nam.wav' < My_Model.nam
   ```
   then register it (`python3 /data/nam/register_nam_ir.py`) and restart
   `tm-stomp-server` so the picker reloads. Pulling TONE3000 bookmarks over
   Wi-Fi: `python3 /data/nam/t3k_sync.py` on the unit (an SSH app on a phone is enough).

## 7. Power-cycle and back to stock

Power off, leave the card in, power on, re-check §3 — the install persists.
To return to factory: power off, **remove the card**, power on. Remember that
anything written to internal `/data` stays until you delete it on-device.

## 8. Troubleshooting

- **No serial device / app says no unit:** wait for the preset screen, reconnect USB-C once, recheck.
  Close screen/serial tools that hold the port (the app needs it exclusively).
- **Unit won't boot with the card:** power off, remove card, repower for stock.
  Rebuild after re-verifying the firmware hash and card layout.
- **"macOS blocked access to the SD card":** TMP NAM was refused removable-volume
  access, and macOS doesn't ask again. Click **Open Privacy & Security** (or open System
  Settings › Privacy & Security › Files and Folders), turn on **Removable Volumes** under
  TMP NAM, then create the card again.
- **Build interrupted:** rerun — it only partitions after you confirm the target.
- **Shell disappears on reboot:** normal if gadget services were restarted; clean
  power-cycle with the card inserted.
- **Severe input noise on stock and NAM presets:** unplug the instrument cable at
  the unit and reconnect it.
- **Soft reboot is slow (~90 s):** known, the console shell holds the stop job.
  Prefer full power cycles.
