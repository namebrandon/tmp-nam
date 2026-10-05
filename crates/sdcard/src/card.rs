//! Physical cards: detection, safety checks, partitioning, writing and readback.
//!
//! Acceptance rules: a whole, writable, removable, physical
//! disk on a USB (macOS) or USB/MMC (Linux) bus, never the startup disk and never
//! Fender-exposed storage. Writing needs root; the desktop app runs this module's
//! `write_prepared` through an administrator prompt.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::image::{boot_files, partition_2_sectors, sfdisk_table, verify_partition_layout};
use crate::release::Release;
use crate::run;
use crate::util::{
    assert_file, bail, format_bytes, locate_tool, sha256_file, write_all, Reporter, Result,
    CHUNK_BYTES,
};

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub logical: String,
    pub raw: String,
    pub bytes: u64,
    pub model: String,
    pub protocol: String,
}

/// A detected disk, accepted or not (with the reason), for the target picker.
#[derive(Serialize, Clone, Debug)]
pub struct Candidate {
    pub device: String,
    pub name: String,
    pub bytes: u64,
    pub protocol: String,
    pub rejected: Option<String>,
}

fn plist(args: &[&str]) -> Result<plist::Dictionary> {
    let o = Command::new("/usr/sbin/diskutil").args(args).output()?;
    if !o.status.success() {
        return bail(format!(
            "diskutil {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    plist::from_bytes(&o.stdout)
        .map_err(|e| crate::util::Error::msg(format!("diskutil plist: {e}")))
}

pub fn normalize_macos_device(device: &str) -> Result<(String, String)> {
    let name = device.strip_prefix("/dev/").unwrap_or("");
    let name = name.strip_prefix('r').unwrap_or(name);
    let digits = name.strip_prefix("disk").unwrap_or("");
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return bail("macOS target must be a whole disk such as /dev/disk2");
    }
    Ok((format!("/dev/disk{digits}"), format!("/dev/rdisk{digits}")))
}

fn macos_device_info(device: &str) -> Result<DeviceInfo> {
    let (logical, raw) = normalize_macos_device(device)?;
    let info = plist(&["info", "-plist", &logical])?;
    let flag = |k: &str| info.get(k).and_then(|v| v.as_boolean()).unwrap_or(false);
    let text = |k: &str| {
        info.get(k)
            .and_then(|v| v.as_string())
            .unwrap_or("")
            .to_string()
    };
    if [
        "WholeDisk",
        "Removable",
        "RemovableMedia",
        "Writable",
        "WritableMedia",
    ]
    .iter()
    .any(|k| !flag(k))
    {
        return bail("target is not a writable whole removable disk");
    }
    if flag("SystemImage") || text("VirtualOrPhysical") != "Physical" {
        return bail("target is not physical removable media");
    }
    if text("BusProtocol") != "USB" {
        return bail("target protocol is not USB");
    }
    let root = plist(&["info", "-plist", "/"])?;
    if root.get("ParentWholeDisk").and_then(|v| v.as_string())
        == Some(text("DeviceIdentifier").as_str())
    {
        return bail("refusing the macOS startup disk");
    }
    let listing = Command::new("/usr/sbin/diskutil")
        .args(["list", &logical])
        .output()?;
    if String::from_utf8_lossy(&listing.stdout).contains("FENDER_AMP") {
        return bail("refusing Fender-exposed storage");
    }
    Ok(DeviceInfo {
        logical,
        raw,
        bytes: info
            .get("TotalSize")
            .and_then(|v| v.as_unsigned_integer())
            .unwrap_or(0),
        model: info
            .get("MediaName")
            .and_then(|v| v.as_string())
            .unwrap_or("unknown")
            .to_string(),
        protocol: info
            .get("BusProtocol")
            .and_then(|v| v.as_string())
            .unwrap_or("unknown")
            .to_string(),
    })
}

fn lsblk(device: Option<&str>) -> Result<serde_json::Value> {
    let lsblk = locate_tool("lsblk")?;
    let mut cmd = Command::new(lsblk);
    cmd.args([
        "-J",
        "-b",
        "-o",
        "PATH,TYPE,SIZE,RM,RO,TRAN,MODEL,MOUNTPOINTS",
    ]);
    if let Some(d) = device {
        cmd.arg(d);
    }
    let o = cmd.output()?;
    if !o.status.success() {
        return bail(format!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        ));
    }
    Ok(serde_json::from_slice(&o.stdout)?)
}

fn int(v: &serde_json::Value) -> u64 {
    v.as_u64()
        .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        .or_else(|| v.as_bool().map(u64::from))
        .unwrap_or(0)
}

fn is_linux_whole(device: &str) -> bool {
    let name = device.strip_prefix("/dev/").unwrap_or("");
    let sd = name
        .strip_prefix("sd")
        .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_lowercase()));
    let mmc = name
        .strip_prefix("mmcblk")
        .is_some_and(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()));
    let nvme = name
        .strip_prefix("nvme")
        .and_then(|r| r.split_once('n'))
        .is_some_and(|(a, b)| {
            !a.is_empty()
                && !b.is_empty()
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
        });
    sd || mmc || nvme
}

fn linux_device_info(device: &str) -> Result<DeviceInfo> {
    if !is_linux_whole(device) {
        return bail("Linux target must be an explicit whole block device");
    }
    let payload = lsblk(Some(device))?;
    let list = payload["blockdevices"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if list.len() != 1 {
        return bail("cannot resolve Linux block device");
    }
    let info = &list[0];
    if info["type"] != "disk" || int(&info["rm"]) != 1 || int(&info["ro"]) != 0 {
        return bail("target is not a writable removable whole disk");
    }
    let tran = info["tran"].as_str().unwrap_or("");
    let is_mmc = device.starts_with("/dev/mmcblk");
    if tran != "usb" && tran != "mmc" && !is_mmc {
        return bail("target transport is not USB/MMC removable media");
    }
    let root = run!(locate_tool("findmnt")?, "-no", "SOURCE", "/")?.stdout;
    if root.trim().starts_with(device) {
        return bail("refusing the Linux startup disk");
    }
    Ok(DeviceInfo {
        logical: device.into(),
        raw: device.into(),
        bytes: int(&info["size"]),
        model: info["model"]
            .as_str()
            .unwrap_or("unknown")
            .trim()
            .to_string(),
        protocol: if tran.is_empty() {
            "unknown".into()
        } else {
            tran.into()
        },
    })
}

pub fn device_info(device: &str) -> Result<DeviceInfo> {
    if cfg!(target_os = "macos") {
        macos_device_info(device)
    } else if cfg!(target_os = "linux") {
        linux_device_info(device)
    } else {
        bail("direct card writing is supported only on macOS and Linux")
    }
}

/// Every removable disk with the reason it would be refused, for a picker UI.
pub fn candidates() -> Result<Vec<Candidate>> {
    let whole: Vec<String> = if cfg!(target_os = "macos") {
        let listing = plist(&["list", "-plist", "physical"])?;
        listing
            .get("WholeDisks")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_string())
                    .map(|s| format!("/dev/{s}"))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        lsblk(None)?["blockdevices"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter(|d| d["type"] == "disk" && int(&d["rm"]) == 1)
            .filter_map(|d| d["path"].as_str().map(str::to_string))
            .collect()
    };
    let mut out = vec![];
    for dev in whole {
        let (name, bytes, protocol, removable) = if cfg!(target_os = "macos") {
            let Ok(info) = plist(&["info", "-plist", &dev]) else {
                continue;
            };
            let flag = |k: &str| info.get(k).and_then(|v| v.as_boolean()).unwrap_or(false);
            (
                info.get("MediaName")
                    .and_then(|v| v.as_string())
                    .unwrap_or("")
                    .to_string(),
                info.get("TotalSize")
                    .and_then(|v| v.as_unsigned_integer())
                    .unwrap_or(0),
                info.get("BusProtocol")
                    .and_then(|v| v.as_string())
                    .unwrap_or("")
                    .to_string(),
                flag("Removable") || flag("RemovableMedia") || flag("Ejectable"),
            )
        } else {
            (String::new(), 0, String::new(), true)
        };
        if !removable {
            continue;
        }
        match device_info(&dev) {
            Ok(i) => out.push(Candidate {
                device: dev,
                name: i.model,
                bytes: i.bytes,
                protocol: i.protocol,
                rejected: None,
            }),
            Err(e) => out.push(Candidate {
                device: dev,
                name,
                bytes,
                protocol,
                rejected: Some(e.0),
            }),
        }
    }
    Ok(out)
}

pub fn validate_capacity(info: &DeviceInfo, release: &Release) -> Result<()> {
    let required = release.minimum_card_bytes();
    if info.bytes < required {
        return bail(format!(
            "SD card is too small: {} available, {} required",
            format_bytes(info.bytes),
            format_bytes(required)
        ));
    }
    if !info.bytes.is_multiple_of(release.layout.sector_bytes) {
        return bail("SD card size is not a whole number of 512-byte sectors");
    }
    Ok(())
}

fn unmount(info: &DeviceInfo) -> Result<()> {
    if cfg!(target_os = "macos") {
        // Force: Spotlight dissents plain unmounts and macOS remounts ejected cards.
        let mut last = String::new();
        for _ in 0..6 {
            let o = Command::new("/usr/sbin/diskutil")
                .args(["unmountDisk", "force", &info.logical])
                .output()?;
            if o.status.success() {
                return Ok(());
            }
            last = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        return bail(format!(
            "command failed: diskutil unmountDisk force {}\n{}",
            info.logical,
            last.trim_end()
        ));
    }
    let payload = lsblk(Some(&info.logical))?;
    let mut paths = vec![];
    fn collect(node: &serde_json::Value, paths: &mut Vec<String>) {
        if node["mountpoints"]
            .as_array()
            .is_some_and(|m| m.iter().any(|v| v.as_str().is_some_and(|s| !s.is_empty())))
        {
            if let Some(p) = node["path"].as_str() {
                paths.push(p.to_string());
            }
        }
        for c in node["children"].as_array().into_iter().flatten() {
            collect(c, paths);
        }
    }
    for root in payload["blockdevices"].as_array().into_iter().flatten() {
        collect(root, &mut paths);
    }
    for p in paths.iter().rev() {
        run!(locate_tool("umount")?, p)?;
    }
    Ok(())
}

fn write_rootfs_partition(source: &Path, destination: &Path, r: &dyn Reporter) -> Result<()> {
    use std::io::Read;
    let total = std::fs::metadata(source)?.len();
    let mut src = std::fs::File::open(source)?;
    let mut dst = std::fs::OpenOptions::new().write(true).open(destination)?;
    let mut buf = vec![0u8; CHUNK_BYTES];
    let mut copied = 0u64;
    let interval = 256 * 1024 * 1024;
    let mut next = 0;
    r.progress("Writing rootfs partition", 0, total);
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        write_all(&mut dst, &buf[..n])?;
        copied += n as u64;
        if copied >= next || copied == total {
            r.progress("Writing rootfs partition", copied, total);
            next = copied + interval;
        }
    }
    dst.sync_all()?;
    Ok(())
}

fn verify_written_rootfs(
    path: &Path,
    rootfs_image: &Path,
    expected: &str,
    r: &dyn Reporter,
) -> Result<()> {
    let len = std::fs::metadata(rootfs_image)?.len();
    if sha256_file(path, Some(len), 0, Some((r, "Verifying rootfs")))? != expected {
        return bail("rootfs partition readback hash mismatch");
    }
    Ok(())
}

fn copy_boot_files(rootfs: &Path, mountpoint: &Path, release: &Release) -> Result<()> {
    for (source, fat_name) in boot_files(rootfs, release)? {
        let dest = mountpoint.join(&fat_name);
        std::fs::copy(&source, &dest)?;
        let pin = if fat_name == release.boot.image.fat_name {
            release.boot.image.pin()
        } else {
            release.boot.dtb.pin()
        };
        assert_file(&dest, &pin)?;
    }
    Ok(())
}

fn prepare_macos(
    target: &DeviceInfo,
    rootfs: &Path,
    image: &Path,
    sha: &str,
    release: &Release,
    r: &dyn Reporter,
) -> Result<()> {
    let label = &release.layout.partition_1.label;
    r.progress("Preparing partitions", 0, 1);
    unmount(target)?;
    let diskutil = Path::new("/usr/sbin/diskutil");
    run!(
        diskutil,
        "partitionDisk",
        target.logical,
        "MBR",
        "MS-DOS FAT32",
        label,
        "512M",
        "ExFAT",
        "ROOTFS",
        "R"
    )?;
    let current = device_info(&target.logical)?;
    if current.bytes != target.bytes || current.model != target.model {
        return bail("SD-card identity changed during partitioning");
    }
    verify_partition_layout(Path::new(&current.raw), release)?;
    let partition = format!("{}s1", current.logical);
    let mount_point = |p: &str| -> Result<Option<String>> {
        Ok(plist(&["info", "-plist", p])?
            .get("MountPoint")
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
            .map(str::to_string))
    };
    let mut mp = mount_point(&partition)?;
    if mp.is_none() {
        run!(diskutil, "mount", partition)?;
        mp = mount_point(&partition)?;
    }
    let mp = mp.ok_or_else(|| crate::util::Error::msg("BOOT partition did not mount"))?;
    copy_boot_files(rootfs, Path::new(&mp), release)?;
    run!("sync")?;
    unmount(&current)?;
    r.progress("Preparing partitions", 1, 1);
    let raw_rootfs = PathBuf::from(format!("{}s2", current.raw));
    write_rootfs_partition(image, &raw_rootfs, r)?;
    verify_written_rootfs(&raw_rootfs, image, sha, r)?;
    run!(diskutil, "eject", current.logical)?;
    r.status("Card verified and ejected — ready to boot");
    Ok(())
}

fn linux_partition(device: &str, n: u32) -> String {
    let tail = device.trim_end_matches(|c: char| c.is_ascii_digit());
    let ends_digit = tail.len() != device.len();
    let sep = if ends_digit && (device.contains("mmcblk") || device.contains("nvme")) {
        "p"
    } else {
        ""
    };
    format!("{device}{sep}{n}")
}

fn prepare_linux(
    target: &DeviceInfo,
    rootfs: &Path,
    image: &Path,
    sha: &str,
    release: &Release,
    r: &dyn Reporter,
) -> Result<()> {
    let p1 = &release.layout.partition_1;
    let card_sectors = target.bytes / release.layout.sector_bytes;
    let table = sfdisk_table(release, partition_2_sectors(card_sectors, release)?);
    r.progress("Preparing partitions", 0, 1);
    unmount(target)?;
    run!(Some(table.as_str()); locate_tool("sfdisk")?, "--quiet", "--wipe", "always", target.logical)?;
    run!(locate_tool("partprobe")?, target.logical)?;
    run!(locate_tool("udevadm")?, "settle")?;
    verify_partition_layout(Path::new(&target.raw), release)?;
    let boot = linux_partition(&target.logical, 1);
    let root = PathBuf::from(linux_partition(&target.logical, 2));
    run!(
        locate_tool("mkfs.vfat")?,
        "-F",
        "32",
        "-S",
        "512",
        "-s",
        "8",
        "-h",
        "2048",
        "-g",
        "54/32",
        "-n",
        p1.label,
        boot
    )?;
    let mp = tempfile::Builder::new()
        .prefix("tmp-boot-mount-")
        .tempdir()?;
    run!(locate_tool("mount")?, boot, mp.path())?;
    let copied = copy_boot_files(rootfs, mp.path(), release).and_then(|_| run!("sync").map(|_| ()));
    run!(locate_tool("umount")?, mp.path())?;
    copied?;
    r.progress("Preparing partitions", 1, 1);
    write_rootfs_partition(image, &root, r)?;
    verify_written_rootfs(&root, image, sha, r)?;
    run!("sync")?;
    r.status("Card verified — safe to remove and ready to boot");
    Ok(())
}

/// Write an already-built rootfs to a card (needs root). `expect` is the identity the
/// user confirmed; the card is re-validated and must still match it.
pub fn write_prepared(
    device: &str,
    expect: Option<&DeviceInfo>,
    rootfs: &Path,
    rootfs_image: &Path,
    rootfs_sha256: &str,
    release: &Release,
    r: &dyn Reporter,
) -> Result<()> {
    let target = device_info(device)?;
    if let Some(e) = expect {
        if e.logical != target.logical || e.bytes != target.bytes || e.model != target.model {
            return bail("SD card changed since it was confirmed; nothing was written");
        }
    }
    validate_capacity(&target, release)?;
    if cfg!(target_os = "macos") {
        prepare_macos(&target, rootfs, rootfs_image, rootfs_sha256, release, r)
    } else {
        prepare_linux(&target, rootfs, rootfs_image, rootfs_sha256, release, r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_device_names() {
        assert_eq!(
            normalize_macos_device("/dev/rdisk4").unwrap(),
            ("/dev/disk4".into(), "/dev/rdisk4".into())
        );
        assert!(normalize_macos_device("/dev/disk4s1").is_err());
        assert!(normalize_macos_device("disk4").is_err());
    }

    #[test]
    fn linux_names() {
        assert!(is_linux_whole("/dev/sdb"));
        assert!(is_linux_whole("/dev/mmcblk0"));
        assert!(is_linux_whole("/dev/nvme0n1"));
        assert!(!is_linux_whole("/dev/sdb1"));
        assert_eq!(linux_partition("/dev/mmcblk0", 2), "/dev/mmcblk0p2");
        assert_eq!(linux_partition("/dev/sdb", 1), "/dev/sdb1");
    }
}
