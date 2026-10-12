//! Orchestrates one component install: download -> verify -> extract -> place. Every step works
//! in a scratch location and the final `rename` is what makes a version "exist", so a crash can
//! never leave a half-installed `bin/<component>/<version>/`.

use std::path::{Path, PathBuf};

use super::download::{download, Progress};
use super::extract::extract;
use super::verify::Verifier;
use crate::config::ComponentSpec;
use crate::error::{Result, ShellError};
use crate::paths::Paths;
use crate::platform::{exe_file, locate_root, make_executable};
use crate::release::ReleaseInfo;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Downloading,
    Extracting,
    Finalizing,
}

pub struct Installer<'a> {
    pub paths: &'a Paths,
    pub client: &'a reqwest::Client,
    pub verifier: &'a dyn Verifier,
}

/// Path of the executable of an installed version (not checked for existence).
pub fn exe_path(paths: &Paths, spec: &ComponentSpec, version: &str) -> PathBuf {
    paths.component_dir(spec.id, version).join(exe_file(spec.exe_name))
}

/// "Ripara installazione" trigger: the executable must be there and so must the Python library
/// it loads at start. PyInstaller 6 keeps it in `_internal/`; older builds keep it beside the
/// executable. Anything weaker (say, "the folder has another file") lets an executable that was
/// copied without its folder pass as installed.
pub fn is_intact(paths: &Paths, spec: &ComponentSpec, version: &str) -> bool {
    let dir = paths.component_dir(spec.id, version);
    if !dir.join(exe_file(spec.exe_name)).is_file() {
        return false;
    }
    dir.join("_internal").is_dir() || has_python_library(&dir)
}

fn has_python_library(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten().any(|e| {
                let n = e.file_name().to_string_lossy().to_ascii_lowercase();
                n.starts_with("libpython") || (n.starts_with("python3") && n.ends_with(".dll"))
            })
        })
        .unwrap_or(false)
}

impl Installer<'_> {
    pub async fn install(
        &self,
        spec: &ComponentSpec,
        rel: &ReleaseInfo,
        progress: &mut (dyn FnMut(Phase, u64, u64) + Send),
    ) -> Result<()> {
        let dest = self.paths.component_dir(spec.id, &rel.version);
        if is_intact(self.paths, spec, &rel.version) {
            return Ok(()); // this exact version is already in place
        }

        self.paths.ensure_dirs()?;
        let archive = self.paths.downloads_dir().join(&rel.asset.name);
        let part = archive.with_file_name(format!("{}.part", rel.asset.name));

        {
            let mut cb = |d: u64, t: u64| progress(Phase::Downloading, d, t);
            let p: Progress<'_> = &mut cb;
            download(self.client, &rel.asset.url, &part, rel.asset.size, p).await?;
        }
        tokio::fs::rename(&part, &archive).await?;

        if let Err(e) = self.verifier.verify(&archive, rel) {
            tokio::fs::remove_file(&archive).await.ok();
            return Err(e);
        }

        progress(Phase::Extracting, 0, 0);
        let tmp = self.paths.component_root(spec.id).join(format!("{}.tmp", rel.version));
        let exe = exe_file(spec.exe_name);
        let (archive2, tmp2, dest2) = (archive.clone(), tmp.clone(), dest.clone());
        let placed = tokio::task::spawn_blocking(move || place(&archive2, &tmp2, &dest2, &exe))
            .await
            .map_err(|e| ShellError::Other(e.to_string()))?;

        if let Err(e) = placed {
            std::fs::remove_dir_all(&tmp).ok();
            // an archive that cannot be extracted is a bad download, not worth keeping
            tokio::fs::remove_file(&archive).await.ok();
            return Err(match e {
                ShellError::Extract(m) => ShellError::DownloadInvalid(m),
                other => other,
            });
        }

        progress(Phase::Finalizing, 0, 0);
        tokio::fs::remove_file(&archive).await.ok();
        Ok(())
    }
}

/// Extract into `tmp`, find the folder holding the executable and move it to `dest` atomically.
fn place(archive: &Path, tmp: &Path, dest: &Path, exe: &str) -> Result<()> {
    if tmp.exists() {
        std::fs::remove_dir_all(tmp)?;
    }
    extract(archive, tmp)?;
    let root = locate_root(tmp, exe)
        .ok_or_else(|| ShellError::Extract(format!("{exe} not found in the archive")))?;
    make_executable(&root.join(exe))?;

    if dest.exists() {
        std::fs::remove_dir_all(dest)?; // leftover of an interrupted earlier install
    }
    if let Some(p) = dest.parent() {
        std::fs::create_dir_all(p)?;
    }
    std::fs::rename(&root, dest)?;
    if root != tmp {
        std::fs::remove_dir_all(tmp).ok();
    }
    Ok(())
}

/// Delete every version directory of a component except the ones in `keep`.
pub fn prune_versions(paths: &Paths, id: &str, keep: &[&str]) {
    let Ok(rd) = std::fs::read_dir(paths.component_root(id)) else { return };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !keep.contains(&name.as_str()) {
            std::fs::remove_dir_all(e.path()).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::download::test_server::serve;
    use super::super::verify::{sha256_hex_of, NoopVerifier, Sha256Verifier};
    use super::*;
    use crate::config::HUSK;
    use crate::release::Asset;

    fn fake_backend_tar_gz() -> Vec<u8> {
        let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        let mut b = tar::Builder::new(gz);
        for (name, data, mode) in [
            ("husk-backend/husk-backend", b"#!/bin/sh\nexit 0\n".as_slice(), 0o755),
            ("husk-backend/_internal/base_library.zip", b"zip".as_slice(), 0o644),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(mode);
            h.set_cksum();
            b.append_data(&mut h, name, data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn release(url: String, size: u64) -> ReleaseInfo {
        ReleaseInfo {
            component: "husk".into(),
            version: "1.0.0".into(),
            prerelease: false,
            asset: Asset { name: "husk-backend-1.0.0-test.tar.gz".into(), size, url, digest: None },
        }
    }

    #[tokio::test]
    async fn installs_a_release_end_to_end() {
        let archive = fake_backend_tar_gz();
        let srv = serve(archive.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let client = reqwest::Client::new();
        let inst = Installer { paths: &paths, client: &client, verifier: &NoopVerifier };

        let mut phases = Vec::new();
        inst.install(&HUSK, &release(srv.url.clone(), archive.len() as u64), &mut |p, _, _| {
            if phases.last() != Some(&p) {
                phases.push(p);
            }
        })
        .await
        .unwrap();

        assert_eq!(phases, vec![Phase::Downloading, Phase::Extracting, Phase::Finalizing]);
        assert!(exe_path(&paths, &HUSK, "1.0.0").is_file());
        assert!(is_intact(&paths, &HUSK, "1.0.0"));
        assert!(!paths.component_root("husk").join("1.0.0.tmp").exists());
        assert_eq!(std::fs::read_dir(paths.downloads_dir()).unwrap().count(), 0);
    }

    fn with_digest(mut rel: ReleaseInfo, bytes: &[u8]) -> ReleaseInfo {
        rel.asset.digest = Some(format!("sha256:{}", sha256_hex_of(bytes)));
        rel
    }

    #[tokio::test]
    async fn installs_when_the_checksum_matches() {
        let archive = fake_backend_tar_gz();
        let srv = serve(archive.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let client = reqwest::Client::new();
        let inst = Installer { paths: &paths, client: &client, verifier: &Sha256Verifier };

        let rel = with_digest(release(srv.url.clone(), archive.len() as u64), &archive);
        inst.install(&HUSK, &rel, &mut |_, _, _| {}).await.unwrap();
        assert!(is_intact(&paths, &HUSK, "1.0.0"));
    }

    #[tokio::test]
    async fn a_tampered_archive_of_the_same_size_installs_nothing() {
        let archive = fake_backend_tar_gz();
        let mut tampered = archive.clone();
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01; // same size, one bit different: the byte-count check cannot see it
        let srv = serve(tampered, false, None);
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let client = reqwest::Client::new();
        let inst = Installer { paths: &paths, client: &client, verifier: &Sha256Verifier };

        let rel = with_digest(release(srv.url.clone(), archive.len() as u64), &archive);
        let err = inst.install(&HUSK, &rel, &mut |_, _, _| {}).await.unwrap_err();
        assert_eq!(err.code(), "checksum_mismatch");
        assert!(!paths.component_dir("husk", "1.0.0").exists());
        assert!(!paths.component_root("husk").join("1.0.0.tmp").exists());
        assert_eq!(std::fs::read_dir(paths.downloads_dir()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn a_release_without_a_checksum_is_refused() {
        let archive = fake_backend_tar_gz();
        let srv = serve(archive.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let client = reqwest::Client::new();
        let inst = Installer { paths: &paths, client: &client, verifier: &Sha256Verifier };

        let rel = release(srv.url.clone(), archive.len() as u64); // digest: None
        let err = inst.install(&HUSK, &rel, &mut |_, _, _| {}).await.unwrap_err();
        assert_eq!(err.code(), "no_checksum");
        assert!(!paths.component_dir("husk", "1.0.0").exists());
        assert_eq!(std::fs::read_dir(paths.downloads_dir()).unwrap().count(), 0);
    }

    #[tokio::test]
    async fn bad_archive_is_reported_as_invalid_download_and_leaves_nothing() {
        let junk = b"not an archive at all".to_vec();
        let srv = serve(junk.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let client = reqwest::Client::new();
        let inst = Installer { paths: &paths, client: &client, verifier: &NoopVerifier };

        let err = inst
            .install(&HUSK, &release(srv.url.clone(), junk.len() as u64), &mut |_, _, _| {})
            .await
            .unwrap_err();
        assert_eq!(err.code(), "download_invalid");
        assert!(!paths.component_dir("husk", "1.0.0").exists());
        assert!(!paths.component_root("husk").join("1.0.0.tmp").exists());
        assert_eq!(std::fs::read_dir(paths.downloads_dir()).unwrap().count(), 0);
    }

    #[test]
    fn an_executable_without_its_library_is_not_intact() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let d = paths.component_dir("husk", "alpha-release");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("husk-backend"), b"x").unwrap();
        assert!(!is_intact(&paths, &HUSK, "alpha-release"));
        // a stray file, even a hidden one, does not make it intact
        std::fs::write(d.join(".bare-executable"), b"").unwrap();
        assert!(!is_intact(&paths, &HUSK, "alpha-release"));
        // older PyInstaller layout: the library sits beside the executable
        std::fs::write(d.join("libpython3.9.dylib"), b"x").unwrap();
        assert!(is_intact(&paths, &HUSK, "alpha-release"));
    }

    #[tokio::test]
    async fn already_installed_version_is_not_downloaded_again() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        let d = paths.component_dir("husk", "1.0.0");
        std::fs::create_dir_all(d.join("_internal")).unwrap();
        std::fs::write(d.join("husk-backend"), b"x").unwrap();
        let client = reqwest::Client::new();
        let inst = Installer { paths: &paths, client: &client, verifier: &NoopVerifier };
        // the URL is unreachable: success proves nothing was fetched
        inst.install(&HUSK, &release("http://127.0.0.1:1/x".into(), 5), &mut |_, _, _| {})
            .await
            .unwrap();
    }

    #[test]
    fn prune_keeps_only_requested_versions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        for v in ["1.0.0", "1.1.0", "1.2.0"] {
            std::fs::create_dir_all(paths.component_dir("husk", v)).unwrap();
        }
        prune_versions(&paths, "husk", &["1.2.0", "1.1.0"]);
        assert!(!paths.component_dir("husk", "1.0.0").exists());
        assert!(paths.component_dir("husk", "1.1.0").exists());
        assert!(paths.component_dir("husk", "1.2.0").exists());
    }
}
