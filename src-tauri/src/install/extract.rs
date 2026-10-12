//! Safe archive extraction (tar.gz and zip). Paths are validated entry by entry: absolute paths,
//! `..`, and symlinks that point outside the destination are rejected ("zip slip").

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::{Result, ShellError};
use crate::platform::{archive_kind, ArchiveKind};

pub fn extract(archive: &Path, dest: &Path) -> Result<()> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    fs::create_dir_all(dest)?;
    match archive_kind(name) {
        Some(ArchiveKind::TarGz) => extract_tar_gz(archive, dest),
        Some(ArchiveKind::Zip) => extract_zip(archive, dest),
        None => Err(ShellError::Extract(format!("unsupported archive type: {name}"))),
    }
}

fn bad(msg: impl Into<String>) -> ShellError {
    ShellError::Extract(msg.into())
}

/// A relative path made only of normal components; returns it joined onto `dest`.
fn safe_join(dest: &Path, rel: &Path) -> Result<PathBuf> {
    let mut out = dest.to_path_buf();
    for c in rel.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            _ => return Err(bad(format!("unsafe path in archive: {}", rel.display()))),
        }
    }
    Ok(out)
}

/// Lexically resolve `link_target` as seen from `link_path`'s directory and make sure the result
/// stays under `dest` (no filesystem access, so it also works for links not created yet).
fn symlink_stays_inside(dest: &Path, link_path: &Path, link_target: &Path) -> bool {
    if link_target.is_absolute() {
        return false;
    }
    let mut cur: Vec<_> = link_path
        .parent()
        .unwrap_or(dest)
        .strip_prefix(dest)
        .map(|p| p.components().map(|c| c.as_os_str().to_owned()).collect())
        .unwrap_or_default();
    for c in link_target.components() {
        match c {
            Component::Normal(p) => cur.push(p.to_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                if cur.pop().is_none() {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn extract_tar_gz(archive: &Path, dest: &Path) -> Result<()> {
    let f = fs::File::open(archive)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    ar.set_preserve_permissions(true);
    ar.set_overwrite(true);
    let entries = ar.entries().map_err(|e| bad(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| bad(e.to_string()))?;
        let rel = entry.path().map_err(|e| bad(e.to_string()))?.into_owned();
        let target = safe_join(dest, &rel)?;
        let kind = entry.header().entry_type();

        if kind.is_dir() {
            fs::create_dir_all(&target)?;
        } else if kind.is_file() {
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            entry.unpack(&target).map_err(|e| bad(e.to_string()))?;
        } else if kind.is_symlink() {
            let link = entry
                .link_name()
                .map_err(|e| bad(e.to_string()))?
                .ok_or_else(|| bad("symlink without target"))?
                .into_owned();
            if !symlink_stays_inside(dest, &target, &link) {
                return Err(bad(format!(
                    "symlink {} -> {} points outside the archive",
                    rel.display(),
                    link.display()
                )));
            }
            if let Some(p) = target.parent() {
                fs::create_dir_all(p)?;
            }
            entry.unpack(&target).map_err(|e| bad(e.to_string()))?;
        } else if kind.is_hard_link() {
            return Err(bad(format!("hard link in archive: {}", rel.display())));
        }
        // other entry kinds (pax headers, etc.) carry no files
    }
    Ok(())
}

fn extract_zip(archive: &Path, dest: &Path) -> Result<()> {
    let f = fs::File::open(archive)?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| bad(e.to_string()))?;
    for i in 0..ar.len() {
        let mut entry = ar.by_index(i).map_err(|e| bad(e.to_string()))?;
        let rel = entry
            .enclosed_name()
            .ok_or_else(|| bad(format!("unsafe path in archive: {}", entry.name())))?;
        let target = safe_join(dest, &rel)?;
        let mode = entry.unix_mode();
        let is_symlink = mode.map(|m| m & 0o170000 == 0o120000).unwrap_or(false);

        if entry.is_dir() {
            fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(p) = target.parent() {
            fs::create_dir_all(p)?;
        }
        if is_symlink {
            let mut link = String::new();
            io::Read::read_to_string(&mut entry, &mut link).map_err(|e| bad(e.to_string()))?;
            if !symlink_stays_inside(dest, &target, Path::new(&link)) {
                return Err(bad(format!("symlink {} points outside the archive", rel.display())));
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&link, &target)?;
            #[cfg(not(unix))]
            return Err(bad("symlinks in zip archives are not supported on this platform"));
        } else {
            let mut out = fs::File::create(&target)?;
            io::copy(&mut entry, &mut out)?;
            #[cfg(unix)]
            if let Some(m) = mode {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&target, fs::Permissions::from_mode(m & 0o7777))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tar_gz(entries: &[(&str, &[u8], u32)], symlinks: &[(&str, &str)]) -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, data, mode) in entries {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(*mode);
            h.set_entry_type(tar::EntryType::Regular);
            // set_path rejects ".." itself, so write the raw name bytes for hostile cases
            let raw = name.as_bytes();
            h.as_old_mut().name[..raw.len()].copy_from_slice(raw);
            h.set_cksum();
            b.append(&h, *data).unwrap();
        }
        for (name, target) in symlinks {
            let mut h = tar::Header::new_gnu();
            h.set_size(0);
            h.set_entry_type(tar::EntryType::Symlink);
            h.set_mode(0o777);
            b.append_link(&mut h, name, target).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn write_archive(dir: &Path, name: &str, bytes: Vec<u8>) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn extracts_tar_gz_with_permissions() {
        let t = tempfile::tempdir().unwrap();
        let a = write_archive(
            t.path(),
            "a.tar.gz",
            tar_gz(
                &[
                    ("husk-backend/husk-backend", b"#!/bin/sh\n", 0o755),
                    ("husk-backend/_internal/lib.txt", b"data", 0o644),
                ],
                &[],
            ),
        );
        let out = t.path().join("out");
        extract(&a, &out).unwrap();
        assert_eq!(fs::read(out.join("husk-backend/_internal/lib.txt")).unwrap(), b"data");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = fs::metadata(out.join("husk-backend/husk-backend")).unwrap().permissions().mode();
            assert_eq!(m & 0o111, 0o111, "executable bit must survive");
        }
    }

    #[test]
    fn rejects_path_traversal_in_tar() {
        let t = tempfile::tempdir().unwrap();
        let a = write_archive(t.path(), "a.tar.gz", tar_gz(&[("../escape.txt", b"x", 0o644)], &[]));
        let out = t.path().join("out");
        assert!(extract(&a, &out).is_err());
        assert!(!t.path().join("escape.txt").exists());
    }

    #[test]
    fn rejects_absolute_paths_in_tar() {
        let t = tempfile::tempdir().unwrap();
        let a = write_archive(t.path(), "a.tar.gz", tar_gz(&[("/tmp/abs.txt", b"x", 0o644)], &[]));
        assert!(extract(&a, &t.path().join("out")).is_err());
    }

    #[test]
    fn rejects_symlink_escaping_the_destination() {
        let t = tempfile::tempdir().unwrap();
        let a = write_archive(
            t.path(),
            "a.tar.gz",
            tar_gz(&[], &[("link", "../../etc/passwd"), ("abs", "/etc/passwd")]),
        );
        assert!(extract(&a, &t.path().join("out")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn allows_symlink_inside_the_destination() {
        let t = tempfile::tempdir().unwrap();
        let a = write_archive(
            t.path(),
            "a.tar.gz",
            tar_gz(&[("pkg/lib.so.1", b"x", 0o644)], &[("pkg/lib.so", "lib.so.1")]),
        );
        let out = t.path().join("out");
        extract(&a, &out).unwrap();
        assert_eq!(fs::read_link(out.join("pkg/lib.so")).unwrap(), PathBuf::from("lib.so.1"));
    }

    #[test]
    fn symlink_check_is_lexical() {
        let d = Path::new("/d");
        assert!(symlink_stays_inside(d, Path::new("/d/a/l"), Path::new("../b")));
        assert!(!symlink_stays_inside(d, Path::new("/d/a/l"), Path::new("../../b")));
        assert!(!symlink_stays_inside(d, Path::new("/d/l"), Path::new("/etc")));
    }

    #[test]
    fn extracts_zip_and_rejects_slip() {
        let t = tempfile::tempdir().unwrap();
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("husk-backend/husk-backend.exe", o).unwrap();
            w.write_all(b"MZ").unwrap();
            w.finish().unwrap();
        }
        let a = write_archive(t.path(), "a.zip", buf.into_inner());
        let out = t.path().join("out");
        extract(&a, &out).unwrap();
        assert!(out.join("husk-backend/husk-backend.exe").is_file());

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("../evil.txt", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let b = write_archive(t.path(), "b.zip", buf.into_inner());
        assert!(extract(&b, &t.path().join("out2")).is_err());
        assert!(!t.path().join("evil.txt").exists());
    }

    #[test]
    fn corrupt_archive_is_an_error_not_a_panic() {
        let t = tempfile::tempdir().unwrap();
        let a = write_archive(t.path(), "a.tar.gz", b"this is not gzip".to_vec());
        assert!(extract(&a, &t.path().join("out")).is_err());
    }
}
