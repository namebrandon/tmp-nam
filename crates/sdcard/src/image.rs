//! Card layout: the FAT32 boot partition, the MBR, and the portable image file.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use crate::release::Release;
use crate::rootfs::resolve_inside;
use crate::run;
use crate::util::{assert_file, bail, locate_tool, write_all, Result, CHUNK_BYTES};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MbrEntry {
    pub active: bool,
    pub kind: u8,
    pub start: u64,
    pub count: u64,
}

pub fn parse_mbr(raw: &[u8]) -> Result<[MbrEntry; 4]> {
    if raw.len() != 512 || raw[510..512] != [0x55, 0xaa] {
        return bail("invalid MBR signature");
    }
    let entry = |i: usize| {
        let e = &raw[446 + i * 16..446 + (i + 1) * 16];
        MbrEntry {
            active: e[0] == 0x80,
            kind: e[4],
            start: u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as u64,
            count: u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as u64,
        }
    };
    Ok([entry(0), entry(1), entry(2), entry(3)])
}

/// The partition table the card must carry (partition 2 may run to the card's end).
pub fn verify_partition_layout(raw_device: &Path, release: &Release) -> Result<()> {
    let mut buf = [0u8; 512];
    std::fs::File::open(raw_device)?.read_exact(&mut buf)?;
    let e = parse_mbr(&buf)?;
    let (p1, p2) = (&release.layout.partition_1, &release.layout.partition_2);
    if e[0].kind != p1.mbr_type || e[0].start != p1.start_sector || e[0].count != p1.sector_count {
        return bail("partitioner produced unexpected MBR geometry");
    }
    if e[1].kind != p2.mbr_type || e[1].start != p2.start_sector {
        return bail("partitioner produced unexpected MBR geometry");
    }
    if e[1].count * release.layout.sector_bytes < release.ext4.bytes {
        return bail("partition 2 is too small for the generated rootfs");
    }
    Ok(())
}

pub fn partition_2_sectors(card_sectors: u64, release: &Release) -> Result<u64> {
    let l = &release.layout;
    if l.partition_1.start_sector + l.partition_1.sector_count > l.partition_2.start_sector {
        return bail("overlapping partition geometry");
    }
    if card_sectors > 0xFFFF_FFFF || card_sectors <= l.partition_2.start_sector {
        return bail("card geometry exceeds MBR limits");
    }
    let p2 = card_sectors - l.partition_2.start_sector;
    if p2 < release.ext4.bytes / l.sector_bytes {
        return bail("card is too small for the 4 GiB rootfs");
    }
    Ok(p2)
}

pub fn sfdisk_table(release: &Release, p2_count: u64) -> String {
    let l = &release.layout;
    format!(
        "label: dos\nunit: sectors\n\nstart={}, size={}, type=b\nstart={}, size={}, type=7\n",
        l.partition_1.start_sector,
        l.partition_1.sector_count,
        l.partition_2.start_sector,
        p2_count
    )
}

/// Verify and copy the official kernel + DTB out of the rootfs.
pub fn boot_files(rootfs: &Path, release: &Release) -> Result<Vec<(std::path::PathBuf, String)>> {
    let mut out = vec![];
    for f in [&release.boot.image, &release.boot.dtb] {
        let p = resolve_inside(rootfs, &f.rootfs_path)?;
        assert_file(&p, &f.pin())?;
        out.push((p, f.fat_name.clone()));
    }
    Ok(out)
}

pub fn create_boot_fat(rootfs: &Path, output: &Path, release: &Release) -> Result<()> {
    let p1 = &release.layout.partition_1;
    let files = boot_files(rootfs, release)?;
    let f = std::fs::File::create(output)?;
    f.set_len(p1.sector_count * release.layout.sector_bytes)?;
    drop(f);
    let mformat = locate_tool("mformat")?;
    let mcopy = locate_tool("mcopy")?;
    run!(
        mformat,
        "-i",
        output,
        "-F",
        "-c",
        p1.sectors_per_cluster.to_string(),
        "-n",
        p1.sectors_per_track.to_string(),
        "-h",
        p1.heads.to_string(),
        "-H",
        p1.hidden_sectors.to_string(),
        "-v",
        p1.label,
        "::",
    )?;
    for (source, fat_name) in files {
        run!(mcopy, "-o", "-i", output, source, format!("::{fat_name}"))?;
    }
    Ok(())
}

fn copy_into(source: &Path, destination: &Path, offset: u64) -> Result<()> {
    let mut src = std::fs::File::open(source)?;
    let mut dst = std::fs::OpenOptions::new().write(true).open(destination)?;
    dst.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; CHUNK_BYTES];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        write_all(&mut dst, &buf[..n])?;
    }
}

pub fn assemble(
    output: &Path,
    boot_fat: &Path,
    rootfs_image: &Path,
    card_sectors: u64,
    release: &Release,
) -> Result<()> {
    let l = &release.layout;
    let p2 = partition_2_sectors(card_sectors, release)?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let f = std::fs::File::create(output)?;
    f.set_len(card_sectors * l.sector_bytes)?;
    drop(f);
    let sfdisk = locate_tool("sfdisk")?;
    run!(Some(sfdisk_table(release, p2).as_str()); sfdisk, "--quiet", output)?;
    copy_into(
        boot_fat,
        output,
        l.partition_1.start_sector * l.sector_bytes,
    )?;
    copy_into(
        rootfs_image,
        output,
        l.partition_2.start_sector * l.sector_bytes,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mbr_round_trip() {
        let mut raw = [0u8; 512];
        raw[510] = 0x55;
        raw[511] = 0xaa;
        let e = &mut raw[446..462];
        e[4] = 0x0b;
        e[8..12].copy_from_slice(&2048u32.to_le_bytes());
        e[12..16].copy_from_slice(&1_000_000u32.to_le_bytes());
        let p = parse_mbr(&raw).unwrap();
        assert_eq!(
            p[0],
            MbrEntry {
                active: false,
                kind: 11,
                start: 2048,
                count: 1_000_000
            }
        );
        assert!(parse_mbr(&raw[..511]).is_err());
    }

    #[test]
    fn table_matches_layout() {
        let r = Release::embedded();
        let min = r.minimum_card_bytes() / 512;
        let p2 = partition_2_sectors(min, &r).unwrap();
        assert_eq!(p2 * 512, r.ext4.bytes);
        assert_eq!(
            sfdisk_table(&r, p2),
            format!("label: dos\nunit: sectors\n\nstart=2048, size=1000000, type=b\nstart=1003520, size={p2}, type=7\n")
        );
    }
}
