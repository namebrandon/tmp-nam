# LAN access on an SD-booted NAM card (Wi-Fi, SSH, TONE3000)

Card creation and boot checks are in [the SD-card guide](../sd-card.md). The **TMP NAM** desktop app manages captures and Tone3000 sync over the USB cable and needs none of this; Wi-Fi + SSH is the computer-free path (an SSH app on a phone works). Maintainer compilation and publication are described in [BUILDING.md](../../BUILDING.md).

Stock 1.8.58 has ConnMan and an RTL8822CU Wi-Fi/Bluetooth radio but no SSH daemon. The normal device UI has no Wi-Fi screen; the factory-test app can toggle the radio. Avahi advertises `_ssh._tcp` even without a daemon listening.

The card configures Wi-Fi power after `tm-stomp-server` starts, Dropbear SSH under internal `/data`, and TONE3000 downloads into the User IR store. Network association is a separate prerequisite.

## 1. First join

ConnMan cannot auto-join a network it has never seen. Join once through either:

- Factory-test UI → Wi-Fi Test: hold the leftmost mag-encoder push at power-on (startup button mask `1`).
- USB HID `WifiMessage` / `wifiConnect` from the host.

This writes `/var/lib/connman/wifi_*_managed_psk/` on internal eMMC `/data` (`Favorite=true`, `AutoConnect=true`) and sets `/data/settings.json` `wifiEnabled: true`. The passphrase stays in ConnMan's settings file; keep it out of version control.

## 2. Automatic Wi-Fi power

The card builder installs `wifi-always-on.sh` and its systemd service in the SD root. No manual copy is needed. The unit runs after `tm-stomp-server` and sets Wi-Fi `Powered=true` over D-Bus for about 15 seconds so ConnMan can associate with a remembered network. Subsequent boots do not need the factory-test encoder gesture.

```sh
systemctl status wifi-always-on.service
```

A running Wi-Fi service does not itself prove association, an assigned address or remote connectivity.

## 3. Exact SSH and helper installation

The distributed release supplies `dropbear` and `dropbearkey`, built from pinned Dropbear 2024.86 sources as static, stripped AArch64/musl binaries. Their current hashes are pinned in `device/release.json`. Card writing does not compile them.

On every boot, `nam-lan-install.service` installs each packaged file atomically at its persistent destination, replacing an older copy when necessary:

```text
/data/nam/bin/dropbear
/data/nam/bin/dropbearkey
/data/nam/t3k_sync.py
/data/nam/register_nam_ir.py
```

`dropbear-nam.service` requires successful installer completion before it starts. Existing SSH host keys, NAM captures, player settings, network settings and TONE3000 tokens are retained. A missing host key is generated on-device at `/data/nam/ssh/ed25519`. The service configuration lives on the SD root; `/data/nam` persists across card changes. Put client public keys in `/home/root/.ssh/authorized_keys` on the SD root. On the device, `/home/root` is part of that read-only root filesystem; remount it writable before changing authorized keys (`mount -o remount,rw /`). This device-side step does not mount anything on the host computer.

Check a new card from its USB console before relying on network access:

```sh
systemctl status nam-lan-install.service dropbear-nam.service
cat /etc/sd-root-build-id
sha256sum /data/nam/bin/dropbear /data/nam/bin/dropbearkey
sha256sum /data/nam/t3k_sync.py /data/nam/register_nam_ir.py
```

Compare the active files with that card's manifest and build ID. If installation failed, inspect `journalctl -u nam-lan-install.service` and fix the failed installation before starting SSH. Package checks alone do not establish remote network access.

Connect with `ssh root@fmic-tm-pro.local`. Dropbear uses `-B`, which allows a blank password; install a client key or set a password, and do not forward port 22 beyond the LAN. OpenSSH `scp` defaults to SFTP, but this image has no `sftp-server`. Transfer a model with:

```sh
ssh root@fmic-tm-pro.local 'cat > /data/userIRs/My_Model.nam.wav' < My_Model.nam
```

Register it using `python3 /data/nam/register_nam_ir.py` and restart `tm-stomp-server` when ready to reload the picker.

## 4. TONE3000

The installer supplies the Python helpers. TONE3000 configuration and tokens remain under `/data/nam`:

```text
/data/nam/t3k_pub          # publishable t3k_pub_… key, mode 600
/data/nam/t3k_tokens.json  # OAuth credentials; keep private
```

On the pedal, run:

```sh
python3 /data/nam/t3k_sync.py
```

The first run uses LAN-relay OAuth: open the printed URL on a phone on the same Wi-Fi. Later runs refresh the saved token. The site UI calls the collection **Bookmarks**; its API endpoint is `/tones/favorited`.

The helper picks one model per tone: A2 if published (one file holding its sizes; TONE3000 lists A2 with no size), else A1 Feather, then A1 Nano; it skips A1 Standard/Lite. It asks the API for A2 and A1 separately, since a request without `architecture` never returns A2. These labels are a download filter, not proof that a capture fits the approximately 725 µs IR-slot deadline; validate its actual configuration on the unit. After a download the script restarts `tm-stomp-server` so the User IR picker reloads (`--no-restart` skips that restart). Bypass the Fender amp block when auditioning an amp-and-cab NAM capture.
