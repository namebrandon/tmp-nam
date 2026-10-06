//! Physical cards: detection, safety checks, partitioning, writing and readback.
//!
//! Acceptance rules: a whole, writable, removable, physical disk in a USB reader or
//! the built-in SD slot (macOS) or on a USB/MMC bus (Linux), never the startup disk
//! and never Fender-exposed storage. On macOS the builder runs as the user and
//! `open` gets a read/write descriptor on the raw disk from authopen(1) behind an
//! administrator prompt; on Linux the desktop app runs the whole helper as root
//! through pkexec.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::image::{
    assemble_head, boot_files, check_partition_layout, create_boot_fat, partition_2_sectors,
    sfdisk_table, verify_partition_layout, write_into,
};
use crate::release::Release;
use crate::run;
use crate::util::{
    assert_file, bail, format_bytes, locate_tool, sha256_file, sha256_reader, Reporter, Result,
};

/// The error when the administrator prompt is cancelled or refused.
pub const ACCESS_DENIED: &str = "Administrator access was not granted; nothing was written";

/// The start of the error when macOS refuses this app removable-volume access
/// (System Settings › Privacy & Security › Files and Folders › Removable Volumes).
pub const VOLUMES_BLOCKED: &str = "macOS refused access to the SD card; nothing was written";

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

/// USB card readers, and the built-in SDXC slot of recent Macs, which reports
/// `Secure Digital` with an Internal location. The Mac's own drives report other
/// buses (`Apple Fabric`, `PCI-Express`, `SATA`) and are not removable media.
fn is_macos_card_bus(protocol: &str) -> bool {
    matches!(protocol, "USB" | "Secure Digital")
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
    if ["WholeDisk", "Removable", "RemovableMedia"]
        .iter()
        .any(|k| !flag(k))
    {
        return bail("target is not a whole removable disk");
    }
    if !flag("Writable") || !flag("WritableMedia") {
        return bail("SD card is write-protected; slide its lock switch up");
    }
    if flag("SystemImage") || text("VirtualOrPhysical") != "Physical" {
        return bail("target is not physical removable media");
    }
    if !is_macos_card_bus(&text("BusProtocol")) {
        return bail("target is not on a USB card reader or SD slot");
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
    let mut dst = std::fs::OpenOptions::new().write(true).open(destination)?;
    write_into(source, &mut dst, 0, Some((r, "Writing rootfs partition")))?;
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

/// A read/write descriptor on the whole raw disk, from authopen(1): it asks for the
/// `sys.openfile.readwrite.<device>` right (the administrator prompt) and sends the
/// open descriptor back over a socket (`-stdoutpipe`, SCM_RIGHTS), so the builder
/// never runs as root. macOS checks removable-volume access against the responsible
/// app, which authopen passes through and a root process from
/// `do shell script … with administrator privileges` does not have. The card must
/// be unmounted first: a read/write open of a disk with mounted volumes fails.
#[cfg(target_os = "macos")]
fn authopen(raw: &str) -> Result<File> {
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::Stdio;
    let (ours, theirs) = UnixStream::pair()?;
    let mut cmd = Command::new("/usr/libexec/authopen");
    cmd.args(["-stdoutpipe", "-o", &libc::O_RDWR.to_string(), raw])
        .stdin(Stdio::null())
        .stdout(Stdio::from(OwnedFd::from(theirs)))
        .stderr(Stdio::piped());
    let child = cmd.spawn()?;
    // The command holds a copy of authopen's end; drop it so a failed authopen
    // closes the socket instead of leaving `receive_fd` waiting.
    drop(cmd);
    let (fd, sent) = receive_fd(&ours);
    let out = child.wait_with_output()?;
    if let Some(fd) = fd {
        return Ok(File::from(fd));
    }
    let said = format!("{sent} {}", String::from_utf8_lossy(&out.stderr))
        .trim()
        .to_string();
    if said.contains("AuthorizationCopyRights failed") {
        return bail(ACCESS_DENIED);
    }
    if said.contains("Operation not permitted") {
        return bail(format!(
            "{VOLUMES_BLOCKED}. Allow this app in System Settings › Privacy & Security › \
             Files and Folders › Removable Volumes, then try again. ({said})"
        ));
    }
    bail(format!(
        "authopen couldn't open {raw} ({}{}{said})",
        out.status,
        if said.is_empty() { "" } else { ": " }
    ))
}

/// Read authopen's socket until it sends a descriptor (SCM_RIGHTS) or closes it;
/// returns the descriptor, or the text it wrote instead.
#[cfg(target_os = "macos")]
fn receive_fd(socket: &std::os::unix::net::UnixStream) -> (Option<std::os::fd::OwnedFd>, String) {
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    let mut text = Vec::new();
    loop {
        let mut data = [0u8; 256];
        let mut iov = libc::iovec {
            iov_base: data.as_mut_ptr().cast(),
            iov_len: data.len(),
        };
        // u64 keeps the control buffer aligned for `cmsghdr`.
        let mut control = [0u64; 8];
        // SAFETY: a zeroed msghdr is valid; every pointer set below outlives recvmsg.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control) as libc::socklen_t;
        // SAFETY: `msg` points at live buffers of the sizes it states.
        let n = unsafe { libc::recvmsg(socket.as_raw_fd(), &mut msg, 0) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        // SAFETY: the CMSG_* macros walk the control buffer recvmsg filled; an
        // SCM_RIGHTS message carries a file descriptor now owned by this process.
        unsafe {
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            if !cmsg.is_null()
                && (*cmsg).cmsg_level == libc::SOL_SOCKET
                && (*cmsg).cmsg_type == libc::SCM_RIGHTS
            {
                let fd = std::ptr::read_unaligned(libc::CMSG_DATA(cmsg).cast::<libc::c_int>());
                return (Some(OwnedFd::from_raw_fd(fd)), String::new());
            }
        }
        if n == 0 {
            break;
        }
        text.extend_from_slice(&data[..n as usize]);
    }
    (None, String::from_utf8_lossy(&text).trim().to_string())
}

/// What the card writer needs opened before the build: on macOS the unmounted raw
/// disk through the administrator prompt, so a refusal costs nothing and the long
/// build runs unattended. On Linux the helper already runs as root.
pub fn open(target: &DeviceInfo) -> Result<Option<File>> {
    #[cfg(target_os = "macos")]
    {
        unmount(target)?;
        authopen(&target.raw).map(Some)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = target;
        Ok(None)
    }
}

/// The whole card through one raw descriptor: the MBR and boot partition as the
/// portable image lays them out, then the rootfs, each read back.
fn prepare_macos(
    target: &DeviceInfo,
    mut disk: File,
    rootfs: &Path,
    image: &Path,
    sha: &str,
    release: &Release,
    r: &dyn Reporter,
) -> Result<()> {
    let l = &release.layout;
    r.progress("Preparing partitions", 0, 1);
    let boot_fat = image.with_file_name("boot.fat");
    create_boot_fat(rootfs, &boot_fat, release)?;
    let head = image.with_file_name("card-head.img");
    assemble_head(&head, &boot_fat, target.bytes / l.sector_bytes, release)?;
    let head_bytes = l.partition_2.start_sector * l.sector_bytes;
    File::options()
        .write(true)
        .open(&head)?
        .set_len(head_bytes)?;
    unmount(target)?;
    // Raw-disk writes bypass the OS cache, and macOS rejects `sync_all` (F_FULLFSYNC)
    // on a raw disk; the readbacks read the card and `diskutil eject` flushes it.
    write_into(&head, &mut disk, 0, None)?;
    check_partition_layout(&mut disk, release)?;
    if sha256_reader(&mut disk, Some(head_bytes), 0, None)? != sha256_file(&head, None, 0, None)? {
        return bail("boot partition readback hash mismatch");
    }
    r.progress("Preparing partitions", 1, 1);
    write_into(
        image,
        &mut disk,
        head_bytes,
        Some((r, "Writing rootfs partition")),
    )?;
    let len = std::fs::metadata(image)?.len();
    if sha256_reader(
        &mut disk,
        Some(len),
        head_bytes,
        Some((r, "Verifying rootfs")),
    )? != sha
    {
        return bail("rootfs partition readback hash mismatch");
    }
    // Closing the disk brings its partitions back and macOS mounts BOOT, so the
    // eject can be refused (Spotlight); the card is complete either way.
    drop(disk);
    if run!("/usr/sbin/diskutil", "eject", target.logical).is_ok() {
        r.status("Card verified and ejected — ready to boot");
    } else {
        r.status("Card verified — eject it in Finder before removing it");
    }
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

/// Write an already-built rootfs to a card, through `disk` from `open` on macOS and
/// as root on Linux. `expect` is the identity the user confirmed; the card is
/// re-validated and must still match it.
pub fn write_prepared(
    device: &str,
    expect: Option<&DeviceInfo>,
    disk: Option<File>,
    built: &crate::Prepared,
    release: &Release,
    r: &dyn Reporter,
) -> Result<()> {
    let (rootfs, rootfs_image, rootfs_sha256) = (&built.rootfs, &built.image, &built.sha256);
    let target = device_info(device)?;
    if let Some(e) = expect {
        if e.logical != target.logical || e.bytes != target.bytes || e.model != target.model {
            return bail("SD card changed since it was confirmed; nothing was written");
        }
    }
    validate_capacity(&target, release)?;
    if cfg!(target_os = "macos") {
        let disk = disk.ok_or_else(|| crate::util::Error::msg("SD card was not opened"))?;
        prepare_macos(
            &target,
            disk,
            rootfs,
            rootfs_image,
            rootfs_sha256,
            release,
            r,
        )
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
    fn macos_card_buses() {
        assert!(is_macos_card_bus("USB"));
        assert!(is_macos_card_bus("Secure Digital"));
        for internal in ["Apple Fabric", "PCI-Express", "SATA", "Thunderbolt", ""] {
            assert!(!is_macos_card_bus(internal), "{internal}");
        }
    }

    /// What authopen does with `-stdoutpipe`: one byte carrying the descriptor.
    #[cfg(target_os = "macos")]
    #[test]
    fn receives_a_descriptor_or_the_text_sent_instead() {
        use std::io::{Read, Seek, Write};
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixStream;

        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"card").unwrap();
        let (ours, theirs) = UnixStream::pair().unwrap();
        let mut byte = [0u8; 1];
        let mut iov = libc::iovec {
            iov_base: byte.as_mut_ptr().cast(),
            iov_len: 1,
        };
        let mut control = [0u64; 8];
        // SAFETY: test-only sendmsg of one SCM_RIGHTS descriptor over live buffers.
        unsafe {
            let mut msg: libc::msghdr = std::mem::zeroed();
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            msg.msg_control = control.as_mut_ptr().cast();
            msg.msg_controllen = libc::CMSG_SPACE(4) as libc::socklen_t;
            let cmsg = libc::CMSG_FIRSTHDR(&msg);
            (*cmsg).cmsg_level = libc::SOL_SOCKET;
            (*cmsg).cmsg_type = libc::SCM_RIGHTS;
            (*cmsg).cmsg_len = libc::CMSG_LEN(4) as libc::socklen_t;
            std::ptr::write_unaligned(libc::CMSG_DATA(cmsg).cast(), file.as_raw_fd());
            assert_eq!(libc::sendmsg(theirs.as_raw_fd(), &msg, 0), 1);
        }
        let (fd, text) = receive_fd(&ours);
        let mut received = File::from(fd.expect("descriptor"));
        received.rewind().unwrap();
        let mut back = String::new();
        received.read_to_string(&mut back).unwrap();
        assert_eq!((back.as_str(), text.as_str()), ("card", ""));

        let (ours, mut theirs) = UnixStream::pair().unwrap();
        theirs.write_all(b"couldn't open /dev/rdisk9\n").unwrap();
        drop(theirs);
        let (fd, text) = receive_fd(&ours);
        assert!(fd.is_none());
        assert_eq!(text, "couldn't open /dev/rdisk9");
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
