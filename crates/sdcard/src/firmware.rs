//! Official firmware: validate the RAUC bundle, unpack it, inventory and extract
//! the rootfs tarball.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::release::Release;
use crate::run;
use crate::util::{bail, locate_tool, sha256_file, Error, Result};

pub fn validate_firmware(path: &Path, release: &Release) -> Result<String> {
    let fw = &release.firmware;
    if !path.is_file() {
        return bail(format!("firmware file does not exist: {}", path.display()));
    }
    if std::fs::metadata(path)?.len() != fw.bytes {
        return bail("firmware size does not match supported 1.8.58 image");
    }
    let digest = sha256_file(path, None, 0, None)?;
    if digest != fw.sha256 {
        return bail("firmware SHA-256 does not match supported 1.8.58 image");
    }
    Ok(digest)
}

/// Minimal INI reader for `manifest.raucm` (sections + `key=value`, no interpolation).
pub fn parse_ini(text: &str) -> Vec<(String, String, String)> {
    let mut out = vec![];
    let mut section = String::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].trim().to_string();
            continue;
        }
        if let Some(idx) = line.find(['=', ':']) {
            let key = line[..idx].trim().to_lowercase();
            let value = line[idx + 1..].trim().to_string();
            out.push((section.clone(), key, value));
        }
    }
    out
}

pub fn check_rauc_manifest(path: &Path, release: &Release) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let entries = parse_ini(&text);
    let get = |s: &str, k: &str| {
        entries
            .iter()
            .rev()
            .find(|(sec, key, _)| sec == s && key == k)
            .map(|(_, _, v)| v.as_str())
            .unwrap_or("")
    };
    let fw = &release.firmware;
    if get("update", "compatible") != fw.compatible {
        return bail("unexpected RAUC compatible value");
    }
    if get("image.rootfs", "filename") != fw.payload.name {
        return bail("unexpected RAUC rootfs payload name");
    }
    if get("image.rootfs", "size").parse::<u64>().ok() != Some(fw.payload.bytes) {
        return bail("unexpected RAUC rootfs payload size");
    }
    if get("image.rootfs", "sha256") != fw.payload.sha256 {
        return bail("unexpected RAUC rootfs payload hash");
    }
    Ok(())
}

/// Unpack the RAUC SquashFS and return the verified payload path.
pub fn unpack_bundle(firmware: &Path, work: &Path, release: &Release) -> Result<PathBuf> {
    let squashfs = work.join("squashfs-root");
    let unsquashfs = locate_tool("unsquashfs")?;
    run!(unsquashfs, "-no-progress", "-d", squashfs, firmware)?;
    let manifest = squashfs.join("manifest.raucm");
    let payload = squashfs.join(&release.firmware.payload.name);
    let mut names: Vec<String> = std::fs::read_dir(&squashfs)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut expected = vec![
        "manifest.raucm".to_string(),
        release.firmware.payload.name.clone(),
    ];
    expected.sort();
    if names != expected {
        return bail("unexpected files in RAUC SquashFS");
    }
    check_rauc_manifest(&manifest, release)?;
    if std::fs::metadata(&payload)?.len() != release.firmware.payload.bytes {
        return bail("extracted rootfs payload size mismatch");
    }
    Ok(payload)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dir,
    File,
    Symlink,
    Hardlink,
}

/// One tar member as the rootfs image must reproduce it.
#[derive(Clone, Debug)]
pub struct Record {
    pub path: String,
    pub kind: Kind,
    pub uid: u64,
    pub gid: u64,
    pub mode: u32,
    pub linkname: String,
}

/// Normalize a tar member name like Python's `PurePosixPath(...).parts`: drop empty
/// and `.` components; refuse absolute paths, `..`, newlines and NULs. `None` is the
/// archive root itself (Yocto's leading `./`).
pub fn safe_member_name(name: &str) -> Result<Option<String>> {
    let parts: Vec<&str> = name
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    if name.starts_with('/') || parts.contains(&"..") {
        return bail(format!("unsafe tar path: {name:?}"));
    }
    if parts.is_empty() {
        return Ok(None);
    }
    if parts.iter().any(|p| p.contains('\n') || p.contains('\0')) {
        return bail(format!("unsupported tar path: {name:?}"));
    }
    Ok(Some(parts.join("/")))
}

fn parents(path: &str) -> impl Iterator<Item = &str> {
    path.match_indices('/').map(move |(i, _)| &path[..i])
}

/// Inventory the rootfs tarball: types, ownership, modes and link targets. Refuses
/// anything the image can't faithfully reproduce (devices, PAX metadata, unsafe
/// paths, members under symlinks, dangling hardlinks).
pub fn inventory(archive: &Path) -> Result<Vec<Record>> {
    let file = std::fs::File::open(archive)?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
    let mut records = vec![];
    let mut symlinks: HashSet<String> = HashSet::new();
    for entry in tar.entries()? {
        let mut entry = entry?;
        let header_type = entry.header().entry_type();
        if header_type.is_pax_global_extensions() {
            return bail("unsupported PAX metadata in archive");
        }
        let raw = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        if entry.pax_extensions()?.is_some() {
            return bail(format!("unsupported PAX metadata in {raw}"));
        }
        let kind = if header_type.is_dir() {
            Kind::Dir
        } else if header_type.is_file()
            || header_type.is_contiguous()
            || header_type.is_gnu_sparse()
        {
            Kind::File
        } else if header_type.is_symlink() {
            Kind::Symlink
        } else if header_type.is_hard_link() {
            Kind::Hardlink
        } else {
            return bail(format!("unsupported tar member type: {raw}"));
        };
        let Some(path) = safe_member_name(&raw)? else {
            if kind != Kind::Dir {
                return bail("archive root is not a directory");
            }
            continue;
        };
        if kind == Kind::Symlink {
            symlinks.insert(path.clone());
        }
        let h = entry.header();
        let linkname = entry
            .link_name_bytes()
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        records.push(Record {
            path,
            kind,
            uid: h.uid()?,
            gid: h.gid()?,
            mode: h.mode()?,
            linkname,
        });
    }
    for r in &records {
        if parents(&r.path).any(|p| symlinks.contains(p)) {
            return bail(format!("tar member traverses a symlink: {}", r.path));
        }
    }
    let paths: HashSet<&str> = records.iter().map(|r| r.path.as_str()).collect();
    for r in records.iter().filter(|r| r.kind == Kind::Hardlink) {
        match safe_member_name(&r.linkname)? {
            Some(t) if paths.contains(t.as_str()) => {}
            _ => return bail(format!("unsafe or missing hardlink target: {}", r.linkname)),
        }
    }
    Ok(records)
}

pub fn extract(archive: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)?;
    let tar = locate_tool("tar")?;
    run!(tar, "-xzpf", archive, "-C", destination)
        .map(|_| ())
        .map_err(|e| Error::msg(e.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn member_names_normalize_like_purepath() {
        assert_eq!(safe_member_name("./").unwrap(), None);
        assert_eq!(safe_member_name(".").unwrap(), None);
        assert_eq!(
            safe_member_name("./usr//bin/").unwrap().as_deref(),
            Some("usr/bin")
        );
        assert!(safe_member_name("/etc").is_err());
        assert!(safe_member_name("a/../b").is_err());
    }

    #[test]
    fn ini_reads_rauc_sections() {
        let e = parse_ini("[update]\ncompatible=tone-master-stomp\n\n[image.rootfs]\nfilename = x.tar.gz\nsize=12\n");
        assert!(e.contains(&(
            "update".into(),
            "compatible".into(),
            "tone-master-stomp".into()
        )));
        assert!(e.contains(&("image.rootfs".into(), "filename".into(), "x.tar.gz".into())));
    }

    #[test]
    fn parents_lists_ancestors() {
        assert_eq!(parents("a/b/c").collect::<Vec<_>>(), vec!["a", "a/b"]);
    }
}
