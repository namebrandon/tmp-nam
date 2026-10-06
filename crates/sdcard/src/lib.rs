//! Tone Master Pro NAM SD-root card builder.
//!
//! Inputs: the owner's official firmware 1.8.58 bundle and the pinned assets in
//! `device/`. Output: a card (or portable image) whose p1 is a FAT32 copy of the
//! official kernel + DTB and whose p2 is a 4 GiB ext4 rootfs = official rootfs +
//! the bounded change set in `rootfs.rs`. The internal eMMC is never touched.
//!
//! Pipeline: validate firmware → unpack RAUC → inventory + extract rootfs → apply
//! delta → mke2fs/debugfs/e2fsck + audit → (image) FAT + MBR + readback, or
//! (card) the same MBR + FAT written raw (macOS) or partition + copy boot (Linux),
//! then raw rootfs write + readback.

pub mod card;
pub mod cli;
pub mod ext4;
pub mod firmware;
pub mod image;
pub mod release;
pub mod rootfs;
pub mod templates;
pub mod util;

use std::path::{Path, PathBuf};

use serde_json::Value;

pub use release::{DeviceDir, Release};
pub use util::{Error, PrintReporter, Reporter, Result};

use util::{bail, locate_tool, remove_tree, sha256_file};

pub const BUILD_TOOLS: &[&str] = &[
    "unsquashfs",
    "mformat",
    "mcopy",
    "mke2fs",
    "debugfs",
    "e2fsck",
    "sfdisk",
];

/// A work directory removed on drop, read-only firmware directories included.
pub struct WorkDir {
    path: PathBuf,
    keep: bool,
}

impl WorkDir {
    pub fn new() -> Result<WorkDir> {
        let dir = tempfile::Builder::new().prefix("tmp-sdcard-").tempdir()?;
        Ok(WorkDir {
            path: dir.keep(),
            keep: false,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Leave the directory in place (another process will consume and delete it).
    pub fn keep(mut self) -> PathBuf {
        self.keep = true;
        self.path.clone()
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if !self.keep {
            remove_tree(&self.path);
        }
    }
}

/// A built root filesystem, ready to be laid out as an image or written to a card.
pub struct Prepared {
    pub work: WorkDir,
    pub rootfs: PathBuf,
    pub image: PathBuf,
    pub sha256: String,
    pub marker: Value,
}

/// Which required host tools are missing (empty = ready).
pub fn missing_tools() -> Vec<&'static str> {
    BUILD_TOOLS
        .iter()
        .copied()
        .filter(|t| locate_tool(t).is_err())
        .collect()
}

pub fn build_rootfs(
    release: &Release,
    device: &DeviceDir,
    firmware: &Path,
    r: &dyn Reporter,
) -> Result<Prepared> {
    const TOTAL: u64 = 5;
    for t in ["unsquashfs", "mke2fs", "debugfs", "e2fsck", "tar"] {
        locate_tool(t)?;
    }
    device.verify(release)?;
    r.progress("Validating official firmware", 0, TOTAL);
    let firmware_hash = firmware::validate_firmware(firmware, release)?;
    let build_id = uuid::Uuid::new_v4().to_string();
    let rootfs_uuid = uuid::Uuid::new_v4().to_string();
    let work = WorkDir::new()?;

    r.progress("Extracting firmware", 1, TOTAL);
    let payload = firmware::unpack_bundle(firmware, work.path(), release)?;
    r.progress("Verifying firmware payload", 2, TOTAL);
    if sha256_file(&payload, None, 0, None)? != release.firmware.payload.sha256 {
        return bail("extracted rootfs payload hash mismatch");
    }
    let rootfs = work.path().join("rootfs");
    let records = firmware::inventory(&payload)?;
    firmware::extract(&payload, &rootfs)?;
    if sha256_file(&rootfs.join("home/root/tm-stomp-server"), None, 0, None)?
        != release.engine_sha256
    {
        return bail("NAM engine hash mismatch in extracted firmware");
    }
    // The payload and squashfs copy are no longer needed (≈1.1 GB).
    remove_tree(&work.path().join("squashfs-root"));

    r.progress("Adding root console and NAM support", 3, TOTAL);
    let marker = rootfs::apply(
        &rootfs,
        release,
        device,
        &build_id,
        &rootfs_uuid,
        &firmware_hash,
    )?;
    r.progress("Building root filesystem", 4, TOTAL);
    let image = work.path().join("rootfs.ext4");
    let built = ext4::create(&rootfs, &records, &image, release, &marker)?;
    r.progress("Build complete", 5, TOTAL);
    Ok(Prepared {
        work,
        rootfs,
        image,
        sha256: built.sha256,
        marker,
    })
}

/// Build a minimum-size portable card image at `output` (never overwritten).
pub fn build_image(
    release: &Release,
    device: &DeviceDir,
    firmware: &Path,
    output: &Path,
    r: &dyn Reporter,
) -> Result<()> {
    if output.exists() {
        return bail(format!(
            "refusing to overwrite existing output: {}",
            output.display()
        ));
    }
    for t in BUILD_TOOLS {
        locate_tool(t)?;
    }
    let result = (|| {
        let p = build_rootfs(release, device, firmware, r)?;
        let card_sectors = release.minimum_card_bytes() / release.layout.sector_bytes;
        r.progress("Building portable image", 0, 2);
        let boot_fat = p.work.path().join("boot.fat");
        image::create_boot_fat(&p.rootfs, &boot_fat, release)?;
        r.progress("Building portable image", 1, 2);
        image::assemble(output, &boot_fat, &p.image, card_sectors, release)?;
        r.progress("Building portable image", 2, 2);
        image::verify_partition_layout(output, release)?;
        let offset = release.layout.partition_2.start_sector * release.layout.sector_bytes;
        let len = std::fs::metadata(&p.image)?.len();
        if sha256_file(output, Some(len), offset, Some((r, "Verifying image")))? != p.sha256 {
            return bail("portable image rootfs verification failed");
        }
        Ok(())
    })();
    if result.is_err() && output.exists() {
        let _ = std::fs::remove_file(output);
    }
    result?;
    r.status(&format!("Portable image ready: {}", output.display()));
    Ok(())
}
