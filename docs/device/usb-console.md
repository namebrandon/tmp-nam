# Bootable SD root access for the Tone Master Pro

The card builder (the TMP NAM app's SD Card page, or `tmp-sdcard`) builds a removable NAM-ready SD system for a Fender
Tone Master Pro (TMP). The unit keeps its factory bootloader and runs Fender's
own 1.8.58 kernel, device tree, and userspace from the card. Minimal rootfs
changes add an unauthenticated root shell to the existing USB composite gadget
and load the firmware-matched NAM runtime. It ships no NAM captures; add your own
from the app.

This is a factory service-boot path plus a controlled Linux root filesystem.
It is not a kernel exploit, a RAUC-signature bypass, or a modified bootloader.
The builder never writes the TMP: it writes only the removable card selected
by the operator.

> **Security and persistence:** while this card is booted, anyone with USB
> access receives a UID-0 shell. The shell can access the whole device.
> `/data` remains the internal eMMC partition and is mounted read-write, so
> commands run as root can make persistent changes. Removing the card restores
> the stock eMMC *root filesystem* on the next cold boot, but it does not undo
> changes deliberately made under `/data` or to other internal storage.

Wi-Fi auto-join, Dropbear SSH, and TONE3000 pull are baked in by the card builder
(first-boot installer stages Dropbear onto `/data` and generates the SSH host
key on-device). Background: [`lan-access.md`](lan-access.md).

## Root access in one diagram
```text
Internal eMMC                          Removable SD
┌─────────────────────┐               ┌────────────────────────────┐
│ BL2 + factory U-Boot│               │ p1 FAT32                   │
│ fender_boot_logic   │──loads───────>│ official Image + Fender DTB│
└──────────┬──────────┘               └────────────────────────────┘
           │ sets root=/dev/mmcblk1p2
           v                            ┌────────────────────────────┐
┌─────────────────────┐               │ p2 ext4                   │
│ Fender Linux kernel │──mounts /────>│ official rootfs + root     │
└──────────┬──────────┘               │ console + NAM runtime      │
           │                           └────────────────────────────┘
           v
┌─────────────────────┐    /dev/ttyGS0    ┌────────────────────────┐
│ systemd service     │<─────────────────>│ CDC-ACM USB function   │
│ /bin/sh -i as UID 0 │                   │ cu.usbmodem / ttyACM   │
└─────────────────────┘                   └────────────────────────┘
```

The load-bearing transition is the kernel command line. U-Boot selects
`/dev/mmcblk1p2` as `/`, so systemd reads units from the card rather than from
an eMMC A/B root slot. The card rootfs can therefore define system services.
The generated card loads the tracked NAM runtime into `tm-stomp-server`; it
installs no captures. The USB shell still runs as UID 0 without a login or password prompt.

## What remains stock and what changes

The builder requires an unmodified `ToneMasterPro_v1_8_58.img`.
It verifies every boot-critical input before building:

| Artifact | Bytes | SHA-256 |
| --- | ---: | --- |
| Official 1.8.58 bundle | 564,635,158 | `392dd4a2a1109b2ff9590610fa4c0f403c9d702d049692476d7f7b84f41f38ec` |
| Rootfs tar payload | 566,177,742 | `584ae1b7559ecc66487234d371773d7da184c8604d5d6bb4abec994542a8f4e6` |
| `Image` | 15,948,288 | `7ea4b5c96a065e24cbba033568068355425ea672e57316d3e325c469014a9605` |
| `fmic-tm-stomp.dtb` | 60,066 | `f768ebe2fc97bb00a2c6a7d5afd91e3d94bcc60ece4c16011603b9f5a62bae53` |
| `nam_dispatch.so` | 2,508,128 | `669c317e59fd479fd407762c93ef0d4e463ad34e33c2af13d82b6467a30bf525` |

The build transformation is:

```text
official .img
  -> verify exact size and SHA-256
  -> unsquashfs the RAUC container
  -> verify manifest identity, payload name, size, and SHA-256
  -> inventory and safely extract the rootfs tar with Unix metadata
  -> apply the bounded rootfs changes below
  -> build and check a fresh ext4 filesystem
  -> partition the selected card and copy the official boot files
  -> write 4 GiB of rootfs and hash all 4 GiB back from the card
```

The RAUC-compatible bundle is used only as an authenticated source archive.
The builder does not alter it or ask RAUC to install it. It extracts the
official rootfs, applies the following changes to a temporary copy, and builds
a fresh ext4 filesystem.

### 1. Add CDC-ACM before the stock gadget is bound

The stock `create-midi-usb-gadget.sh` stages MIDI in configfs. The builder adds
these operations at fixed, verified anchors:

```sh
modprobe usb_f_acm
mkdir functions/acm.usb0
ln -s functions/acm.usb0 configs/config.1/
```

The stock kernel already ships `usb_f_acm.ko` and `u_serial.ko`. No kernel
module is supplied by this repository.

The stock service ordering is important:

```text
setup-composite-gadget.service
          │
          ├── create-hid-usb-gadget.service
          ├── create-midi-usb-gadget.service  (patched to stage ACM too)
          │
          v
bind-composite-gadget.service -> echo e6590000.usb > gadget0/UDC
```

`create-midi-usb-gadget.service` runs as root, requires the setup and bind
units, starts after setup, and is ordered before bind. Therefore ACM joins HID
and MIDI before the host sees the composite device. The separate
`usb_console_setup.sh` is an idempotent guard: it waits for `gadget0`, ensures
the module/function/link exist, binds only if the UDC is still unbound, and
waits for `/dev/ttyGS0`. It never performs an unconditional rebind.

### 2. Install a root-owned shell service

The card installs `usb-console.service` and enables it through
`multi-user.target.wants`:

```ini
[Service]
Type=simple
ExecStartPre=/usr/local/bin/usb_console_setup.sh
ExecStart=/bin/sh -c 'exec /bin/sh -i < /dev/ttyGS0 > /dev/ttyGS0 2>&1'
Restart=always
RestartSec=5
```

There is deliberately no `User=` setting, so the system manager starts both
commands as root. The shell's stdin, stdout, and stderr all use `ttyGS0`.
`Restart=always` creates a replacement shell after the host disconnects.

Do not start a second reader or shell on `ttyGS0`: two readers consume each
other's bytes. Do not force a UDC unbind/rebind while the shell is open: the
kernel removes `ttyGS0`, killing that session and potentially creating a
restart loop.

### 3. Drop `data=journal` from the mounts

The official `fstab` declares both `/dev/root` and internal `/data` with
`data=journal`. The card removes those two option tokens:

```text
/dev/root      /       auto  ro,...,sync,noatime,errors=continue  1 0
/dev/mmcblk0p3 /data   auto  rw,...,sync,noatime,errors=continue  0 2
```

Without this change the SD root fails to mount. It enables a reliable SD boot; it is not what makes the shell root. The generated ext4
filesystem itself has a journal and a pinned feature set.

### 4. Enable NAM

The builder copies the tracked `nam_dispatch.so` to
`/usr/local/lib/nam_dispatch.so` and adds this drop-in to the card rootfs:

```ini
[Service]
Environment="LD_PRELOAD=/usr/local/lib/nam_dispatch.so"
Environment="TMP_NAM_DISPATCH_LOG=/tmp/nam_dispatch.log"
Environment="TMP_NAM_DISPATCH_NO_MLOCK=1"
```

The library is firmware-specific and is pinned to the official 1.8.58
userspace. The process hook is installed lazily when a NAM User IR is selected;
the `NO_MLOCK` setting avoids a boot-time memory-locking problem. No NAM capture is loaded automatically at boot.

The card does not include the SD auto-watcher or any NAM captures. Add models
from the TMP NAM app (or the USB/User IR workflow) after the card has booted.
Because User IRs live in internal `/data`, they remain listed after the card is
removed, although the NAM runtime is then absent and the stock engine cannot
execute them.

### 5. Preserve rootfs semantics and add provenance

The ext4 build preserves archive ownership, permission bits, symlinks, and
hardlinks. The builder checks required mountpoint directories, runs `e2fsck`,
verifies the ext4 feature set, and confirms critical metadata through
`debugfs`.

The filesystem is 4,294,967,296 bytes with 4,096-byte blocks, 262,144 inodes,
zero percent reserved blocks, label `rootfs.1`, and a new UUID per build. Its
features are pinned to:

```text
has_journal ext_attr resize_inode dir_index filetype extent 64bit flex_bg
metadata_csum_seed sparse_super large_file huge_file dir_nlink extra_isize
metadata_csum
```

Lazy inode-table and journal initialization are disabled before the image is
hashed. `orphan_file` is explicitly disabled so a newer host's e2fsprogs does
not silently add a feature outside the pinned set.

`/etc/sd-root-build-id` contains:

```json
{"build_id":"<UUID>","firmware_sha256":"<official bundle hash>","rootfs_uuid":"<UUID>","schema":"tmp-sd-root-card/build-marker-v1"}
```

The two UUIDs are generated for each run. They identify which built
filesystem actually booted.

## Card layout and why U-Boot accepts it

The factory `fender_boot_logic` contains a live `Flasher SD card` branch. Card
presence alone selects it; no rear-panel button,
`boot.scr`, environment edit, or eMMC write was required. Factory U-Boot itself
continues to run from internal storage. The version reported through
`fw_printenv ver` is `U-Boot 2021.10-1.0.32-b6bb129 (May 11 2023 - 00:28:51
+0000)`.

The card uses the factory service-card layout:

| Partition | Start | Partition size | MBR type | Filesystem and contents |
| --- | ---: | ---: | ---: | --- |
| p1 `BOOT` | sector 2,048 | 1,000,000 sectors | `0x0b` | FAT32; `Image` and `fmic-tm-stomp.dtb` |
| p2 `rootfs.1` | sector 1,003,520 | Remaining card | `0x07` | A fixed 4 GiB ext4 filesystem at its start |

Both MBR entries are non-active. The factory branch addresses the known
partitions directly rather than using the PC active flag. Type `0x07` is
intentional: Linux identifies the actual filesystem from its ext4 superblock,
and keeping the known-good byte is safer than normalizing an MBR field that the
bootloader may inspect.

The FAT32 volume uses 512-byte sectors, eight sectors per cluster, 32 sectors
per track, 54 heads, and 2,048 hidden sectors, set explicitly rather than
relying on host defaults.

On cards larger than the minimum, partition 2 spans the remaining media but
the ext4 filesystem inside it remains exactly 4 GiB. The unused tail is not
needed by the device. In a portable minimum-size image, partition 2 and its
ext4 filesystem are both exactly 4 GiB.

### Why the official kernel can continue from SD

The Fender DTB maps the removable slot to the enabled SDHI0 controller at
`ee100000` with a 4-bit bus and card-detect GPIO. Internal eMMC uses SDHI3 at
`ee160000`, an 8-bit non-removable interface. Linux consequently exposes
internal eMMC as `mmcblk0` and the inserted card as `mmcblk1`.

The shipped kernel has MMC block, Renesas SDHI, DOS partition parsing, VFAT,
and ext4 support built in, so it can read both card partitions without an
initramfs. U-Boot includes `rootwait` with `root=/dev/mmcblk1p2`, allowing the
kernel to wait for card enumeration before mounting `/`. The stock `fstab`
also records the intended design: “Either eMMC or SD card rootfs partition,
determined by u-boot and passed to the kernel command line.”

Bootloader MMC numbering is independent from Linux's `mmcblkN` numbering. The
card layout depends on the Fender boot branch and Linux root argument, not on assumptions taken from a generic Renesas board configuration.

## Storage boundaries while booted

| Path or component | Backing storage | Consequence |
| --- | --- | --- |
| BL2, U-Boot, environment | Internal eMMC boot areas | Not supplied or rewritten by the card builder |
| Normal system A/B roots | Internal `mmcblk0p1` / `mmcblk0p2` | Bypassed while SD root is selected |
| `/` | SD `mmcblk1p2` | Contains the USB-console service and returns to stock when card is removed |
| `/usr/local/lib/nam_dispatch.so` | SD `mmcblk1p2` | Loaded only while the card root is selected |
| `/data` | Internal `mmcblk0p3` | Read-write and persistent across SD removal |
| `/data/userIRs/*.nam.wav` | Internal `mmcblk0p3` | NAM captures you add persist after the card is removed |
| `/var/lib` | Bind mount from `/data/var/lib` | Also persistent internal state |
| `/run`, `/var/volatile` | tmpfs | Lost at reboot |

The normal app stack still runs and may update ordinary persistent state. If a
strictly read-only internal-storage experiment is required, this card is not
such an environment.

## Requirements

The only proprietary input is the official 1.8.58 bundle. NAM captures
are not build inputs and are never tracked in this repository. The A1/A2
`nam_dispatch.so` and the registry helper are.

Temporary construction requires about 6 GiB of free space. The card must be at
least 4,808,769,536 bytes; use an 8 GB or larger card in practice.

### macOS Intel and Apple Silicon

Install Homebrew dependencies:

```sh
brew install squashfs e2fsprogs mtools util-linux
```

`diskutil` and `screen` are supplied by macOS. The builder locates Homebrew's
`mke2fs`, `debugfs`, `e2fsck`, and `sfdisk` on both Intel and Apple Silicon paths.

### Debian or Ubuntu Linux

```sh
sudo apt-get update
sudo apt-get install squashfs-tools e2fsprogs dosfstools fdisk parted udev mtools exfatprogs screen
```

`fdisk` supplies `sfdisk`; `parted` supplies `partprobe`; `dosfstools` supplies
`mkfs.vfat`; and `mtools` is required only for portable-image creation.

## Build a physical card on macOS

Use the **TMP NAM** desktop app → **SD Card**: choose the official firmware,
pick the detected card, and confirm. The app accepts only a writable removable
whole disk in a USB card reader or the built-in SD slot, rejects the startup
disk and Fender-exposed storage, checks capacity, shows device/model/size, and asks for
an explicit erase confirmation plus the macOS administrator password before
writing.

Headless (CI, Linux, scripting), the same builder ships as `tmp-sdcard`:

```sh
cargo run --release -p tmp-sdcard -- list
sudo target/release/tmp-sdcard write /path/to/ToneMasterPro_v1_8_58.img --device /dev/diskN   # macOS
sudo target/release/tmp-sdcard write /path/to/ToneMasterPro_v1_8_58.img --device /dev/sdX     # Linux
```

Always pass a whole disk, never a partition such as `disk2s1` or `sdb1`.

The final verification reads and hashes the complete 4 GiB root filesystem
from the card. It can take about as long as the write, but detects incomplete
writes and failing or counterfeit media. On macOS the verified card is ejected
automatically; on Linux wait for the success message and remove it only after
all activity has stopped.

## Boot and connect

1. Power the TMP off fully.
2. Insert the prepared card.
3. Power it on normally; do not hold a button.
4. Wait for the normal preset screen.
5. Connect the standard USB-C cable to the computer.

Keep a stock/non-NAM preset selected during power-on. Select a NAM User IR only after the preset screen and USB console are
ready; this keeps model loading out of the startup window.

### macOS

```sh
ls /dev/cu.usbmodem*
screen /dev/cu.usbmodem123454 115200
```

Use the exact path returned on your host. Press Return if the prompt is not
immediately visible.

### Linux

```sh
ls /dev/ttyACM*
sudo screen /dev/ttyACM0 115200
```

Using `sudo` avoids distribution-specific serial-group setup for this one-off
root session. Alternatively, configure the normal `dialout`/`uucp` group for
your distribution and reconnect the device.

Only one terminal or transfer process may hold the port at a time.

For `screen`, exit with `Ctrl-A`, then `K`, then confirm with `y`. Closing the
terminal window without exiting may leave the port busy temporarily.

## Internal UART console for an opened unit

The bootable card removes the need to open a unit, but it does not make the
board's physical console obsolete. An already-opened device can use the P300
UART for stock-root administration, passive boot capture, SD-boot diagnosis,
and recovery when Linux or the USB gadget does not start. Unlike CDC-ACM, this
console is present during BL2, U-Boot, and early kernel boot.

The P300 connection is 3.3 V UART at 115200 8N1:

| P300 pin | Direction | USB-TTL connection |
| ---: | --- | --- |
| 5 | TMP transmit | Adapter RX |
| 4 | TMP receive | Adapter TX |
| 1 or 6 | Ground | Adapter GND |

Do not connect the adapter's VCC/5 V lead and never use 5 V logic. The TMP is
self-powered. The stock serial getty accepts user `root` with an empty password.

The two consoles serve different layers:

- P300 UART is the best source for the bootloader's `Flasher SD card` messages
  and remains useful with no valid rootfs;
- USB CDC-ACM is convenient after the prepared SD root reaches systemd and
  does not require opening the enclosure.

## Prove that the card provided root

Run these commands in the device shell:

```sh
id
findmnt /
findmnt /data
cat /proc/cmdline
cat /etc/sd-root-build-id
systemctl is-active usb-console.service
```

The expected results are:

- `id` reports UID and GID 0;
- `/` is `/dev/mmcblk1p2`;
- `/data` is internal `/dev/mmcblk0p3` and read-write;
- the command line names `root=/dev/mmcblk1p2`;
- the marker parses as JSON and its firmware hash matches the table above;
- `usb-console.service` is `active`.

This combination distinguishes a newly built SD root from a root service left
on another filesystem.

For NAM readiness, also run:

```sh
sha256sum /usr/local/lib/nam_dispatch.so
pid=$(pidof tm-stomp-server)
tr '\000' '\n' < /proc/$pid/environ | grep -E 'LD_PRELOAD|TMP_NAM_DISPATCH'
grep -E 'MATCH 1.8.58|ARMED|NAM ready|NAM stats' /tmp/nam_dispatch.log
```

The library hash must be `669c317e59fd479fd407762c93ef0d4e463ad34e33c2af13d82b6467a30bf525`.
Add a capture from the app, then select it only after the device has reached the
preset screen and the log reports `NAM dispatch ARMED`. Loading should then
show `NAM ready` with the expected model hash and size, followed by `NAM stats`
with increasing block counts and zero errors/underflows. Follow the
[A1/A2 device checklist](../nam-player.md#device-use).

## Power-cycle and return to stock

To check persistence, exit the terminal, power the unit off, leave the card
inserted, power on, reconnect USB, and repeat the six checks. The rootfs UUID
and build ID must match the first boot. Captures you added and their User IR registry
entries should remain available, and selecting one after startup should
again produce `NAM dispatch ACTIVE`.

To return to the stock rootfs:

1. exit the USB shell;
2. power the unit off completely;
3. remove the SD card;
4. power it on normally.

Do not remove the card while the SD-root system is running. Remember that
changes under internal `/data`, including the captures you added, survive this
operation. The stock rootfs does not load the SD-resident NAM library, so they
are then only ordinary stored User IRs.

## Resetting a card to exFAT

Resetting to exFAT is not required for ordinary use because the builder
repartitions the whole card. It confirms that no residue from an earlier
bootable card is required.

First identify the whole card carefully, then create one MBR/exFAT partition:

```sh
# macOS
diskutil eraseDisk ExFAT TMP_FRESH MBRFormat /dev/diskN

# Linux USB reader
sudo wipefs --all /dev/sdX
sudo parted --script /dev/sdX mklabel msdos mkpart primary 1MiB 100%
sudo mkfs.exfat -n TMP_FRESH /dev/sdX1
```

For an MMC reader, the Linux partition path is normally `/dev/mmcblkNp1`.
After confirming the one-partition state, write the card from the app,
perform the first-boot checks, power-cycle checks, and card-removal stock boot.

## Troubleshooting

- **No removable card detected:** reconnect the reader, disconnect other
  removable disks, and rerun. Use `--device` only after checking the model and
  capacity independently.
- **Firmware rejected:** use the unmodified official 1.8.58 image. Renaming a
  different bundle cannot satisfy the pinned size and hash.
- **Required tool not found:** install the complete package set for the host
  above and start a new terminal so Homebrew or package-manager paths refresh.
- **Readback hash mismatch:** do not boot the card. Try a different
  reader/adapter, then a different card. A card may still boot with damaged
  unused blocks, but it is not a good card.
- **Build interrupted:** rerun the tool. It recreates the full partition layout
  only after showing the target and receiving confirmation.
- **Unit does not boot with the card:** power off, remove the card, and repower
  to recover stock boot. Rebuild only after checking the firmware hash, card
  layout, and complete readback.
- **NAM sounds like only a cab:** verify that `LD_PRELOAD` contains
  `/usr/local/lib/nam_dispatch.so`, that the library is mapped in the
  `tm-stomp-server` process, and that `/tmp/nam_dispatch.log` reports
  `NAM dispatch ARMED` before selecting the User IR.
- **Capture is not listed:** check `/data/userIRs.json`. A name must be
  registered before its file lands, or the firmware's User IR pruner removes it.
- **No USB serial device:** wait for the preset screen, reconnect USB-C once,
  and check `cu.usbmodem*` or `ttyACM*` again.
- **Port busy or commands lose characters:** close Pro Control, `screen`, serial
  transfer tools, and every second reader. Only the service-owned device shell
  and one host client should access the tty.
- **Shell disappears repeatedly:** do not restart gadget services or rebind
  `gadget0/UDC`; power-cycle cleanly with the card still inserted.

## Re-derive and audit the mechanism

The guide is self-contained, but the important inputs can be independently
re-derived from the official firmware and the physical unit.

Verify and inspect the outer bundle:

```sh
shasum -a 256 ToneMasterPro_v1_8_58.img
unsquashfs -ll ToneMasterPro_v1_8_58.img
unsquashfs -cat ToneMasterPro_v1_8_58.img manifest.raucm
```

Inspect stock files without retaining an extracted rootfs:

```sh
PAYLOAD=fmic-image-manufacturing-tone-master-stomp-hihope-rzg2h.tar.gz
unsquashfs -cat ToneMasterPro_v1_8_58.img "$PAYLOAD" | tar -xzOf - ./etc/fstab
unsquashfs -cat ToneMasterPro_v1_8_58.img "$PAYLOAD" | \
  tar -xzOf - ./usr/local/bin/create-midi-usb-gadget.sh
unsquashfs -cat ToneMasterPro_v1_8_58.img "$PAYLOAD" | \
  tar -xzOf - ./etc/systemd/system/create-midi-usb-gadget.service
```

On a booted card, collect the boot decision and storage facts read-only:

```sh
fw_printenv ver
fw_printenv fender_boot_logic
cat /proc/cmdline
lsblk
findmnt /
findmnt /data
systemctl cat create-midi-usb-gadget.service
systemctl cat bind-composite-gadget.service
systemctl cat usb-console.service
```

For strongest bootloader evidence, capture a passive UART cold-boot log and
look for the `Flasher SD card is present`, FAT load, `booti`, and Linux command
line messages. The firmware update bundle contains the Linux payload but not
factory U-Boot, so bootloader selection policy cannot be reconstructed from
the `.img` alone.

Finally, audit the generated card without trusting its labels: parse sector 0,
hash p1 files, inspect the ext4 superblock at sector 1,003,520, read
`/etc/sd-root-build-id`, and compare the generated rootfs against the official
archive. These are the same invariants enforced by the builder
(`crates/sdcard`) before it reports success.
