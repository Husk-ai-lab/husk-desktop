//! Host target triple and archive helpers. Release assets use the plan's naming:
//! `husk-backend-1.4.0-aarch64-apple-darwin.tar.gz`.

use std::path::{Path, PathBuf};

pub fn host_triple() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    return "aarch64-apple-darwin";
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    return "x86_64-apple-darwin";
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    return "x86_64-pc-windows-msvc";
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    return "x86_64-unknown-linux-gnu";
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
    )))]
    return "unsupported";
}

/// `husk-backend` -> `husk-backend.exe` on Windows.
pub fn exe_file(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    TarGz,
    Zip,
}

pub fn archive_kind(file_name: &str) -> Option<ArchiveKind> {
    let n = file_name.to_ascii_lowercase();
    if n.ends_with(".tar.gz") || n.ends_with(".tgz") {
        Some(ArchiveKind::TarGz)
    } else if n.ends_with(".zip") {
        Some(ArchiveKind::Zip)
    } else {
        None
    }
}

/// PyInstaller one-folder archives may hold `husk-backend/...` as a single top folder or the
/// files directly. Returns the directory that contains the executable, if any.
pub fn locate_root(extracted: &Path, exe: &str) -> Option<PathBuf> {
    if extracted.join(exe).is_file() {
        return Some(extracted.to_path_buf());
    }
    let mut dirs = std::fs::read_dir(extracted)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path());
    let only = dirs.next()?;
    if dirs.next().is_some() {
        return None;
    }
    only.join(exe).is_file().then_some(only)
}

#[cfg(unix)]
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perm = std::fs::metadata(path)?.permissions();
    perm.set_mode(perm.mode() | 0o755);
    std::fs::set_permissions(path, perm)
}

#[cfg(not(unix))]
pub fn make_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_archive_kinds() {
        assert_eq!(archive_kind("a-1.0.0-x.tar.gz"), Some(ArchiveKind::TarGz));
        assert_eq!(archive_kind("a-1.0.0-x.ZIP"), Some(ArchiveKind::Zip));
        assert_eq!(archive_kind("SHA256SUMS"), None);
    }

    #[test]
    fn locates_flat_and_nested_roots() {
        let t = tempfile::tempdir().unwrap();
        let flat = t.path().join("flat");
        std::fs::create_dir_all(&flat).unwrap();
        std::fs::write(flat.join("husk-backend"), b"x").unwrap();
        assert_eq!(locate_root(&flat, "husk-backend"), Some(flat.clone()));

        let nested = t.path().join("nested");
        std::fs::create_dir_all(nested.join("husk-backend-dir")).unwrap();
        std::fs::write(nested.join("husk-backend-dir").join("husk-backend"), b"x").unwrap();
        assert_eq!(
            locate_root(&nested, "husk-backend"),
            Some(nested.join("husk-backend-dir"))
        );

        let empty = t.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert_eq!(locate_root(&empty, "husk-backend"), None);
    }
}
