//! The bounded change set applied to the extracted official rootfs.
//!
//! Adds the CDC-ACM root console to the stock USB gadget, drops `data=journal`,
//! pins the audio IRQs to CPU3, installs the NAM player + LAN tools, and
//! writes the build marker. Every stock file it edits is matched by exact anchors
//! (or by hash) so a different firmware fails instead of being half-patched.

use std::os::unix::fs::{symlink, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::release::{Asset, DeviceDir, PatchedFile, Release};
use crate::templates::{self, LAN_SERVICE_DROPIN};
use crate::util::{assert_file, bail, Error, Result};

fn chmod(path: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .map_err(|e| Error::msg(format!("chmod {}: {e}", path.display())))
}

fn exists_or_link(p: &Path) -> bool {
    p.exists() || p.is_symlink()
}

/// Resolve `relative` under `root`, refusing anything that escapes it via symlinks.
pub fn resolve_inside(root: &Path, relative: &str) -> Result<PathBuf> {
    let resolved = root.join(relative).canonicalize()?;
    let root_resolved = root.canonicalize()?;
    if !resolved.starts_with(&root_resolved) {
        return bail(format!("path resolves outside rootfs: {relative}"));
    }
    Ok(resolved)
}

fn write_text_preserving_mode(path: &Path, text: &str) -> Result<()> {
    let mode = std::fs::metadata(path)?.permissions().mode() & 0o7777;
    std::fs::write(path, text)?;
    chmod(path, mode)
}

fn write_new_file(path: &Path, text: &str, mode: u32) -> Result<()> {
    if exists_or_link(path) {
        return bail(format!(
            "unexpected existing rootfs file: {}",
            path.display()
        ));
    }
    std::fs::create_dir_all(path.parent().expect("rootfs path has a parent"))?;
    std::fs::write(path, text)?;
    chmod(path, mode)
}

fn copy_asset(rootfs: &Path, device: &DeviceDir, asset: &Asset, mode: u32) -> Result<()> {
    let source = device.path(asset);
    assert_file(&source, &asset.pin())?;
    let target = rootfs.join(&asset.target);
    if exists_or_link(&target) {
        return bail(format!("unexpected existing rootfs file: {}", asset.target));
    }
    std::fs::create_dir_all(target.parent().expect("rootfs path has a parent"))?;
    std::fs::copy(&source, &target)?;
    chmod(&target, mode)?;
    assert_file(&target, &asset.pin())?;
    Ok(())
}

fn replace_once(text: &str, old: &str, new: &str) -> String {
    text.replacen(old, new, 1)
}

fn patch_audio_irq_priority_script(rootfs: &Path, script: &PatchedFile) -> Result<()> {
    let target = resolve_inside(rootfs, &script.target)?;
    assert_file(&target, &script.source)?;
    let original = std::fs::read_to_string(&target)?;
    let replacements = [
        ("AUDIO_IRQ_CPU=02", "AUDIO_IRQ_CPU=08"),
        ("AUDIO_BOTTOM_HALF_CPU=1", "AUDIO_BOTTOM_HALF_CPU=3"),
        (
            "# Move the audio-related top halves to Core 1",
            "# Move the audio-related top halves to Core 3",
        ),
    ];
    for (old, _) in replacements {
        if original.matches(old).count() != 1 {
            return bail(format!(
                "audio priority script anchor does not match exactly once: {old}"
            ));
        }
    }
    let mut patched = original;
    for (old, new) in replacements {
        patched = replace_once(&patched, old, new);
    }
    write_text_preserving_mode(&target, &patched)?;
    assert_file(&target, &script.patched)?;
    Ok(())
}

/// The build marker written to `/etc/sd-root-build-id`.
pub fn build_marker(
    release: &Release,
    build_id: &str,
    rootfs_uuid: &str,
    firmware_hash: &str,
) -> Value {
    let mut m = Map::new();
    m.insert("schema".into(), "tmp-sd-root-card/build-marker-v1".into());
    m.insert("build_id".into(), build_id.into());
    m.insert("firmware_sha256".into(), firmware_hash.into());
    m.insert("rootfs_uuid".into(), rootfs_uuid.into());
    m.insert(
        "nam_dispatch_sha256".into(),
        release.assets.nam_dispatch.sha256.clone().into(),
    );
    m.insert(
        "lan_dropbear_sha256".into(),
        release.assets.dropbear.sha256.clone().into(),
    );
    m.insert(
        "lan_dropbearkey_sha256".into(),
        release.assets.dropbearkey.sha256.clone().into(),
    );
    m.insert(
        "audio_priority_script_sha256".into(),
        release
            .rootfs
            .audio_priority_script
            .patched
            .sha256
            .clone()
            .into(),
    );
    Value::Object(m)
}

/// Python `json.dumps(obj, sort_keys=True)` for a flat object of strings.
pub fn python_json(v: &Value) -> String {
    let obj = v.as_object().expect("marker is an object");
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    let parts: Vec<String> = keys
        .iter()
        .map(|k| format!("{}: {}", Value::String((*k).clone()), obj[*k]))
        .collect();
    format!("{{{}}}", parts.join(", "))
}

pub fn apply(
    rootfs: &Path,
    release: &Release,
    device: &DeviceDir,
    build_id: &str,
    rootfs_uuid: &str,
    firmware_hash: &str,
) -> Result<Value> {
    let layout = &release.rootfs;
    let assets = &release.assets;

    // USB gadget: stage the CDC-ACM function beside MIDI before the shared bind.
    let gadget = rootfs.join(&layout.gadget_script);
    let original = std::fs::read_to_string(&gadget)?;
    let (first, tail) = (&layout.gadget_anchor, &layout.gadget_tail_anchor);
    if original.matches(first.as_str()).count() != 1 || original.matches(tail.as_str()).count() != 1
    {
        return bail("gadget script anchors do not match exactly once");
    }
    if original.contains("usb_f_acm") || original.contains("functions/acm.usb0") {
        return bail("gadget script is already modified");
    }
    let patched = replace_once(&original, first, &format!("{first}modprobe usb_f_acm\n"));
    let patched = replace_once(
        &patched,
        tail,
        &format!(
            "{tail}\n# Add the owner-side CDC console before the shared gadget is bound.\n\
             mkdir functions/acm.usb0\nln -s functions/acm.usb0 configs/config.1/\n"
        ),
    );
    write_text_preserving_mode(&gadget, &patched)?;

    let fstab = rootfs.join(&layout.fstab);
    let fstab_text = std::fs::read_to_string(&fstab)?;
    if fstab_text.matches(",data=journal").count() != 2 {
        return bail("expected exactly two data=journal mount options");
    }
    write_text_preserving_mode(&fstab, &fstab_text.replace(",data=journal", ""))?;

    patch_audio_irq_priority_script(rootfs, &layout.audio_priority_script)?;

    // USB console service + setup (copied, then enabled).
    for (asset, mode) in [
        (&assets.console_service, 0o644),
        (&assets.console_setup, 0o755),
    ] {
        let source = device.path(asset);
        assert_file(&source, &asset.pin())?;
        let target = rootfs.join(&asset.target);
        std::fs::create_dir_all(target.parent().expect("parent"))?;
        std::fs::copy(&source, &target)?;
        chmod(&target, mode)?;
    }
    let enable = rootfs.join(&layout.console_enable_target);
    std::fs::create_dir_all(enable.parent().expect("parent"))?;
    if exists_or_link(&enable) {
        return bail("usb-console service is already enabled in source rootfs");
    }
    symlink("../usb-console.service", &enable)?;

    copy_asset(rootfs, device, &assets.nam_dispatch, 0o755)?;

    let registry = &assets.registry_helper;
    let registry_source = device.path(registry);
    assert_file(&registry_source, &registry.pin())?;
    let registry_target = rootfs.join(&registry.target);
    if exists_or_link(&registry_target) {
        return bail(format!(
            "unexpected existing rootfs file: {}",
            registry.target
        ));
    }
    std::fs::create_dir_all(registry_target.parent().expect("parent"))?;
    std::fs::copy(&registry_source, &registry_target)?;
    chmod(&registry_target, 0o755)?;

    write_new_file(
        &rootfs.join(&layout.nam_dropin_target),
        templates::NAM_SERVICE_DROPIN,
        0o644,
    )?;

    copy_asset(rootfs, device, &assets.dropbear, 0o755)?;
    copy_asset(rootfs, device, &assets.dropbearkey, 0o755)?;
    copy_asset(rootfs, device, &assets.wifi_setup, 0o755)?;
    copy_asset(rootfs, device, &assets.wifi_service, 0o644)?;
    copy_asset(rootfs, device, &assets.dropbear_service, 0o644)?;
    copy_asset(rootfs, device, &assets.t3k, 0o755)?;
    write_new_file(
        &rootfs.join(&layout.lan_setup_target),
        templates::NAM_LAN_SETUP,
        0o755,
    )?;
    write_new_file(
        &rootfs.join(&layout.lan_service_target),
        templates::NAM_LAN_INSTALL_SERVICE,
        0o644,
    )?;
    write_new_file(
        &rootfs.join(LAN_SERVICE_DROPIN),
        templates::NAM_LAN_SERVICE_DROPIN,
        0o644,
    )?;
    chmod(
        rootfs.join(LAN_SERVICE_DROPIN).parent().expect("parent"),
        0o755,
    )?;
    for service in [
        &assets.wifi_service.target,
        &assets.dropbear_service.target,
        &layout.lan_service_target,
    ] {
        let service = Path::new(service);
        let name = service
            .file_name()
            .expect("service name")
            .to_string_lossy()
            .into_owned();
        let link = rootfs
            .join(service.parent().expect("parent"))
            .join("multi-user.target.wants")
            .join(&name);
        std::fs::create_dir_all(link.parent().expect("parent"))?;
        if exists_or_link(&link) {
            return bail(format!(
                "LAN service is already enabled in source rootfs: {}",
                service.display()
            ));
        }
        symlink(format!("../{name}"), &link)?;
    }
    // Explicit modes avoid inheriting a restrictive host umask.
    let dropin_dir = Path::new(&layout.nam_dropin_target)
        .parent()
        .expect("parent");
    for relative in [Path::new("usr/local/share/nam"), dropin_dir] {
        chmod(&rootfs.join(relative), 0o755)?;
    }

    enum Want {
        Dir(u32),
        Link(&'static str),
    }
    let required = [
        ("dev", Want::Dir(0o755)),
        ("proc", Want::Dir(0o755)),
        ("sys", Want::Dir(0o755)),
        ("run", Want::Dir(0o1777)),
        ("data", Want::Dir(0o755)),
        ("var/lib", Want::Dir(0o755)),
        ("tmp", Want::Link("/var/tmp")),
        ("var/run", Want::Link("/run")),
        ("var/tmp", Want::Link("/var/volatile/tmp")),
    ];
    for (relative, expected) in required {
        let target = rootfs.join(relative);
        match expected {
            Want::Dir(mode) => {
                if !target.is_dir() || target.is_symlink() {
                    return bail(format!("required mountpoint directory missing: {relative}"));
                }
                chmod(&target, mode)?;
            }
            Want::Link(link) => {
                let ok = target.is_symlink()
                    && std::fs::read_link(&target)
                        .map(|l| l == Path::new(link))
                        .unwrap_or(false);
                if !ok {
                    return bail(format!("unexpected rootfs symlink: {relative}"));
                }
            }
        }
    }

    let marker = build_marker(release, build_id, rootfs_uuid, firmware_hash);
    let marker_target = rootfs.join(&layout.build_marker);
    std::fs::write(&marker_target, format!("{}\n", python_json(&marker)))?;
    chmod(&marker_target, 0o644)?;
    Ok(marker)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marker_json_matches_python_dumps() {
        let mut m = Map::new();
        m.insert("b".into(), "2".into());
        m.insert("a".into(), "x\"y".into());
        assert_eq!(python_json(&Value::Object(m)), r#"{"a": "x\"y", "b": "2"}"#);
    }
}
