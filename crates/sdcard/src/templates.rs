//! Files the builder generates into the rootfs; the image verifier compares against
//! these byte for byte.

pub const NAM_SERVICE_DROPIN: &str = r#"[Service]
Environment="LD_PRELOAD=/usr/local/lib/nam_dispatch.so"
Environment="TMP_NAM_DISPATCH_LOG=/tmp/nam_dispatch.log"
Environment="TMP_NAM_DISPATCH_NO_MLOCK=1"
"#;

pub const NAM_LAN_SETUP: &str = r#"#!/bin/sh
set -eu

# Synchronize only kit-owned code. Preserve models, settings and SSH host keys.
staged=
trap '[ -z "$staged" ] || rm -f "$staged"' EXIT
trap 'exit 1' HUP INT TERM
sync_code() {
    source=$1
    target=$2
    if [ -L "$target" ] || [ -d "$target" ]; then
        echo "Refusing symlink or directory at code destination: $target" >&2
        return 1
    fi
    if [ ! -L "$target" ] && [ -f "$target" ] && cmp -s "$source" "$target"; then
        chmod 755 "$target"
        return
    fi
    staged=$(mktemp "${target}.tmp.XXXXXX")
    cp "$source" "$staged"
    chmod 755 "$staged"
    cmp -s "$source" "$staged"
    mv -f "$staged" "$target"
    staged=
}
mkdir -p /data/nam/bin /data/nam/ssh
for tool in dropbear dropbearkey; do
    sync_code /usr/local/share/nam/$tool /data/nam/bin/$tool
done
for helper in t3k_sync.py register_nam_ir.py; do
    sync_code /usr/local/bin/$helper /data/nam/$helper
done
if [ ! -e /data/nam/ssh/ed25519 ]; then
    /data/nam/bin/dropbearkey -t ed25519 -f /data/nam/ssh/ed25519
    chmod 600 /data/nam/ssh/ed25519
fi
sync
"#;

pub const NAM_LAN_SERVICE_DROPIN: &str = "[Unit]
Requires=nam-lan-install.service
After=nam-lan-install.service
";

pub const LAN_SERVICE_DROPIN: &str = "etc/systemd/system/dropbear-nam.service.d/10-install.conf";

pub const NAM_LAN_INSTALL_SERVICE: &str = "[Unit]
Description=Stage NAM LAN access onto /data
After=local-fs.target
Before=dropbear-nam.service

[Service]
Type=oneshot
ExecStart=/usr/local/bin/install-tmp-nam-lan.sh
RemainAfterExit=yes
";
