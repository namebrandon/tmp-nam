//! The 4 GiB ext4 rootfs: `mke2fs -d`, ownership/mode fix-up with `debugfs`,
//! `e2fsck`, then a metadata and content audit of the finished image.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::Value;

use crate::firmware::{safe_member_name, Kind, Record};
use crate::release::{Pin, Release};
use crate::run;
use crate::templates::{self, LAN_SERVICE_DROPIN};
use crate::util::{assert_file, bail, locate_tool, sha256_file, Error, Result};

pub struct Ext4Image {
    pub bytes: u64,
    pub sha256: String,
}

fn quote(path: &str) -> String {
    format!("\"{}\"", path.replace('"', "\\\""))
}

fn debugfs_batch(
    debugfs: &Path,
    image: &Path,
    commands: impl Iterator<Item = String>,
) -> Result<()> {
    let mut child = Command::new(debugfs)
        .args(["-w", "-f", "-"])
        .arg(image)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| Error::msg("debugfs stdin unavailable"))?;
        let mut w = std::io::BufWriter::new(stdin);
        for c in commands {
            w.write_all(c.as_bytes())?;
            w.write_all(b"\n")?;
        }
        w.flush()?;
    }
    drop(child.stdin.take());
    if !child.wait()?.success() {
        return bail("debugfs metadata update failed");
    }
    Ok(())
}

fn normalize_ownership(
    debugfs: &Path,
    image: &Path,
    records: &[Record],
    inode_count: u64,
) -> Result<()> {
    let all = (11..=inode_count).flat_map(|i| {
        [
            format!("set_inode_field <{i}> uid 0"),
            format!("set_inode_field <{i}> gid 0"),
        ]
    });
    let owned = records
        .iter()
        .filter(|r| (r.uid, r.gid) != (0, 0))
        .flat_map(|r| {
            let p = quote(&format!("/{}", r.path));
            [
                format!("set_inode_field {p} uid {}", r.uid),
                format!("set_inode_field {p} gid {}", r.gid),
            ]
        });
    debugfs_batch(debugfs, image, all.chain(owned))
}

/// debugfs ownership updates clear setuid/setgid (kernel chown semantics); restore
/// them with the full i_mode (type bits included).
fn restore_special_modes(debugfs: &Path, image: &Path, records: &[Record]) -> Result<()> {
    let mut commands = vec![];
    for r in records.iter().filter(|r| r.mode & 0o7000 != 0) {
        let type_bits = match r.kind {
            Kind::File => 0o100000,
            Kind::Dir => 0o040000,
            Kind::Symlink => 0o120000,
            Kind::Hardlink => {
                return bail(format!(
                    "special mode on a hardlink is unsupported: {}",
                    r.path
                ))
            }
        };
        commands.push(format!(
            "set_inode_field {} mode 0{:o}",
            quote(&format!("/{}", r.path)),
            type_bits | r.mode
        ));
    }
    debugfs_batch(debugfs, image, commands.into_iter())
}

fn request(debugfs: &Path, image: &Path, req: &str) -> Result<String> {
    Ok(run!(debugfs, "-R", req, image)?.stdout)
}

#[derive(Debug, PartialEq, Eq)]
pub struct Stat {
    pub inode: u64,
    pub mode: u32,
    pub uid: u64,
    pub gid: u64,
}

/// Parse `debugfs stat` output (Inode / Mode / User / Group).
pub fn parse_stat(rendered: &str) -> Result<Stat> {
    fn after<'a>(s: &'a str, key: &str) -> Option<&'a str> {
        let i = s.find(key)?;
        Some(s[i + key.len()..].trim_start())
    }
    fn number(s: &str) -> &str {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        &s[..end]
    }
    let parse = || -> Option<Stat> {
        let inode = number(after(rendered, "Inode:")?).parse().ok()?;
        let mode_text = number(after(rendered, "Mode:")?);
        let mode = u32::from_str_radix(mode_text.trim_start_matches('0'), 8)
            .or_else(|_| {
                if mode_text.chars().all(|c| c == '0') {
                    Ok(0)
                } else {
                    Err(())
                }
            })
            .ok()?;
        let user = after(rendered, "User:")?;
        let uid = number(user).parse().ok()?;
        let gid = number(after(user, "Group:")?).parse().ok()?;
        Some(Stat {
            inode,
            mode,
            uid,
            gid,
        })
    };
    parse().ok_or_else(|| Error::msg("cannot parse debugfs stat output"))
}

fn stat(debugfs: &Path, image: &Path, path: &str) -> Result<Stat> {
    parse_stat(&request(debugfs, image, &format!("stat /{path}"))?)
}

fn verify(
    debugfs: &Path,
    image: &Path,
    records: &[Record],
    release: &Release,
    marker: &Value,
) -> Result<()> {
    let by_path: BTreeMap<&str, &Record> = records.iter().map(|r| (r.path.as_str(), r)).collect();
    let mut checked = HashSet::new();
    let checks = records
        .iter()
        .filter(|r| (r.uid, r.gid) != (0, 0))
        .chain(records.iter().filter(|r| r.mode & 0o7000 != 0));
    for r in checks {
        if !checked.insert(r.path.as_str()) {
            continue;
        }
        let actual = stat(debugfs, image, &r.path)?;
        if actual.uid != r.uid || actual.gid != r.gid {
            return bail(format!("ownership mismatch in ext4 image: {}", r.path));
        }
        if actual.mode != r.mode {
            return bail(format!("mode mismatch in ext4 image: {}", r.path));
        }
    }
    for r in records.iter().filter(|r| r.kind == Kind::Hardlink) {
        let target = safe_member_name(&r.linkname)?
            .ok_or_else(|| Error::msg("hardlink cannot target archive root"))?;
        if stat(debugfs, image, &r.path)?.inode != stat(debugfs, image, &target)?.inode {
            return bail(format!(
                "hardlink relationship was not preserved: {}",
                r.path
            ));
        }
    }
    let layout = &release.rootfs;
    let marker_text = request(debugfs, image, &format!("cat /{}", layout.build_marker))?;
    let parsed: Value = serde_json::from_str(&marker_text)?;
    if &parsed != marker {
        return bail("build marker mismatch in ext4 image");
    }
    let priority = &layout.audio_priority_script;
    let rec = by_path
        .get(priority.target.as_str())
        .filter(|r| r.kind == Kind::File)
        .ok_or_else(|| Error::msg("audio priority script metadata inventory is incomplete"))?;
    let actual = stat(debugfs, image, &priority.target)?;
    if (actual.uid, actual.gid, actual.mode) != (rec.uid, rec.gid, rec.mode) {
        return bail(format!(
            "audio priority script ownership/mode mismatch: {}",
            priority.target
        ));
    }

    let a = &release.assets;
    let dropin_dir = Path::new(&layout.nam_dropin_target)
        .parent()
        .expect("parent")
        .to_string_lossy()
        .into_owned();
    let lan_dropin_dir = Path::new(LAN_SERVICE_DROPIN)
        .parent()
        .expect("parent")
        .to_string_lossy()
        .into_owned();
    let mut modes: BTreeMap<String, u32> = BTreeMap::new();
    for (p, m) in [
        (a.nam_dispatch.target.clone(), 0o755),
        (layout.nam_dropin_target.clone(), 0o644),
        (a.registry_helper.target.clone(), 0o755),
        ("usr/local/share/nam".to_string(), 0o755),
        (dropin_dir, 0o755),
        (a.dropbear.target.clone(), 0o755),
        (a.dropbearkey.target.clone(), 0o755),
        (a.wifi_setup.target.clone(), 0o755),
        (a.wifi_service.target.clone(), 0o644),
        (a.dropbear_service.target.clone(), 0o644),
        (a.t3k.target.clone(), 0o755),
        (layout.lan_setup_target.clone(), 0o755),
        (layout.lan_service_target.clone(), 0o644),
        (LAN_SERVICE_DROPIN.to_string(), 0o644),
        (lan_dropin_dir, 0o755),
    ] {
        modes.insert(p, m);
    }
    for (p, m) in &modes {
        let actual = stat(debugfs, image, p)?;
        if (actual.uid, actual.gid, actual.mode) != (0, 0, *m) {
            return bail(format!("NAM ownership/mode mismatch: {p}"));
        }
    }

    let dir = tempfile::Builder::new()
        .prefix("nam-image-verify-")
        .tempdir()?;
    let mut pinned: Vec<(&str, Pin)> = vec![(&a.nam_dispatch.target, a.nam_dispatch.pin())];
    for x in [
        &a.dropbear,
        &a.dropbearkey,
        &a.wifi_setup,
        &a.wifi_service,
        &a.dropbear_service,
        &a.t3k,
    ] {
        pinned.push((&x.target, x.pin()));
    }
    pinned.push((&priority.target, priority.patched.clone()));
    for (target, pin) in pinned {
        let name = Path::new(target).file_name().expect("file name");
        let extracted = dir.path().join(name);
        request(
            debugfs,
            image,
            &format!("dump /{target} \"{}\"", extracted.display()),
        )?;
        assert_file(&extracted, &pin)?;
    }

    for (path, expected, what) in [
        (
            layout.nam_dropin_target.as_str(),
            templates::NAM_SERVICE_DROPIN,
            "installed NAM service mismatch",
        ),
        (
            layout.lan_setup_target.as_str(),
            templates::NAM_LAN_SETUP,
            "installed LAN setup script mismatch",
        ),
        (
            layout.lan_service_target.as_str(),
            templates::NAM_LAN_INSTALL_SERVICE,
            "installed LAN service mismatch",
        ),
        (
            LAN_SERVICE_DROPIN,
            templates::NAM_LAN_SERVICE_DROPIN,
            "installed LAN dependency mismatch",
        ),
    ] {
        if request(debugfs, image, &format!("cat /{path}"))?.trim() != expected.trim() {
            return bail(what);
        }
    }
    if !by_path.contains_key("boot/Image") {
        return bail("rootfs metadata inventory is incomplete");
    }
    Ok(())
}

pub fn create(
    rootfs: &Path,
    records: &[Record],
    output: &Path,
    release: &Release,
    marker: &Value,
) -> Result<Ext4Image> {
    let ext4 = &release.ext4;
    let mke2fs = locate_tool("mke2fs")?;
    let debugfs = locate_tool("debugfs")?;
    let e2fsck = locate_tool("e2fsck")?;
    let mut features = ext4.features.clone();
    features.push("^orphan_file".into());
    let uuid = marker["rootfs_uuid"]
        .as_str()
        .ok_or_else(|| Error::msg("marker has no rootfs_uuid"))?;
    run!(
        mke2fs,
        "-t",
        "ext4",
        "-F",
        "-b",
        ext4.block_bytes.to_string(),
        "-N",
        ext4.inode_count.to_string(),
        "-m",
        ext4.reserved_percent.to_string(),
        "-L",
        release.layout.partition_2.label,
        "-U",
        uuid,
        "-O",
        features.join(","),
        "-E",
        "lazy_itable_init=0,lazy_journal_init=0",
        "-d",
        rootfs,
        output,
        (ext4.bytes / ext4.block_bytes).to_string(),
    )?;
    normalize_ownership(&debugfs, output, records, ext4.inode_count)?;
    restore_special_modes(&debugfs, output, records)?;
    let fsck = Command::new(&e2fsck).arg("-fy").arg(output).output()?;
    let code = fsck.status.code().unwrap_or(-1);
    if code != 0 && code != 1 {
        return bail(format!(
            "e2fsck failed with exit {code}\n{}{}",
            String::from_utf8_lossy(&fsck.stdout),
            String::from_utf8_lossy(&fsck.stderr)
        ));
    }
    verify(&debugfs, output, records, release, marker)?;
    let stats = request(&debugfs, output, "stats")?;
    let line = stats
        .lines()
        .find_map(|l| l.strip_prefix("Filesystem features:"))
        .ok_or_else(|| Error::msg("cannot read ext4 feature set"))?;
    let actual: HashSet<&str> = line.split_whitespace().collect();
    let expected: HashSet<&str> = ext4.features.iter().map(String::as_str).collect();
    if actual != expected {
        let mut a: Vec<_> = actual.into_iter().collect();
        let mut e: Vec<_> = expected.into_iter().collect();
        a.sort();
        e.sort();
        return bail(format!("ext4 features differ: expected {e:?}, got {a:?}"));
    }
    Ok(Ext4Image {
        bytes: ext4.bytes,
        sha256: sha256_file(output, None, 0, None)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_debugfs_stat() {
        let out = "Inode: 1234   Type: regular    Mode:  04755   Flags: 0x80000\n\
                   Generation: 0    Version: 0x00000000:00000000\n\
                   User:     0   Group:    50   Project:     0   Size: 1\n";
        assert_eq!(
            parse_stat(out).unwrap(),
            Stat {
                inode: 1234,
                mode: 0o4755,
                uid: 0,
                gid: 50
            }
        );
        let zero =
            "Inode: 7   Type: directory    Mode:  0000   Flags: 0x0\nUser:     5   Group:     6\n";
        assert_eq!(parse_stat(zero).unwrap().mode, 0);
    }
}
