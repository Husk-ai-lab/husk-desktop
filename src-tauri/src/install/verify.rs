//! Integrity check of a downloaded archive, before anything is extracted or executed.
//!
//! The expected checksum is the `digest` GitHub publishes with every release asset
//! (`sha256:<64 hex>`). A release whose asset has no usable digest is **not installed**: a missing
//! checksum must never silently switch the check off.
//!
//! What this does and does not prove. GitHub computes the digest when the asset is uploaded, so a
//! match shows the file is exactly what the release holds: no truncation, corruption, swapped file
//! or tampering on the way. It does not protect against someone who can edit the release itself,
//! because they can change the asset and its digest together. Authenticity needs a signed manifest
//! (plan rule 5, still open: see the README release checklist).

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use crate::error::{Result, ShellError};
use crate::release::ReleaseInfo;

pub trait Verifier: Send + Sync {
    /// Called with the fully downloaded archive, before anything is extracted or executed.
    fn verify(&self, archive: &Path, release: &ReleaseInfo) -> Result<()>;
}

/// Compares the archive's SHA-256 with the digest published for the release asset.
pub struct Sha256Verifier;

impl Verifier for Sha256Verifier {
    fn verify(&self, archive: &Path, release: &ReleaseInfo) -> Result<()> {
        let name = &release.asset.name;
        let expected = release
            .asset
            .digest
            .as_deref()
            .and_then(parse_sha256_digest)
            .ok_or_else(|| ShellError::NoChecksum(name.clone()))?;
        let actual = sha256_file(archive)?;
        if actual == expected {
            tracing::info!(asset = %name, sha256 = %actual, "checksum verified");
            Ok(())
        } else {
            tracing::error!(asset = %name, expected = %expected, actual = %actual, "checksum mismatch");
            Err(ShellError::ChecksumMismatch(format!(
                "{name}: expected {}…, got {}…",
                &expected[..12],
                &actual[..12]
            )))
        }
    }
}

/// `sha256:<64 hex>` (any case) -> lowercase hex. Other algorithms, short or non-hex values: `None`.
fn parse_sha256_digest(digest: &str) -> Option<String> {
    let (algo, hex) = digest.trim().split_once(':')?;
    if !algo.eq_ignore_ascii_case("sha256")
        || hex.len() != 64
        || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(hex.to_ascii_lowercase())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Accepts every archive. Tests only: the shell always runs [`Sha256Verifier`].
#[cfg(test)]
pub struct NoopVerifier;

#[cfg(test)]
impl Verifier for NoopVerifier {
    fn verify(&self, _archive: &Path, _release: &ReleaseInfo) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
pub fn sha256_hex_of(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::Asset;

    // sha256("abc"), the standard test vector
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn release(digest: Option<&str>) -> ReleaseInfo {
        ReleaseInfo {
            component: "husk".into(),
            version: "1.0.0".into(),
            prerelease: false,
            asset: Asset {
                name: "husk-backend-1.0.0-x.tar.gz".into(),
                size: 3,
                url: "https://x/a".into(),
                digest: digest.map(String::from),
            },
        }
    }

    fn file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.tar.gz");
        std::fs::write(&p, bytes).unwrap();
        (d, p)
    }

    #[test]
    fn hashing_matches_the_standard_vector() {
        let (_d, p) = file(b"abc");
        assert_eq!(sha256_file(&p).unwrap(), ABC);
        assert_eq!(sha256_hex_of(b"abc"), ABC);
    }

    #[test]
    fn matching_digest_passes_in_any_case() {
        let (_d, p) = file(b"abc");
        Sha256Verifier
            .verify(&p, &release(Some(&format!("sha256:{ABC}"))))
            .unwrap();
        Sha256Verifier
            .verify(
                &p,
                &release(Some(&format!("SHA256:{}", ABC.to_uppercase()))),
            )
            .unwrap();
    }

    #[test]
    fn a_different_file_is_a_mismatch() {
        let (_d, p) = file(b"abd"); // one byte off, same size
        let err = Sha256Verifier
            .verify(&p, &release(Some(&format!("sha256:{ABC}"))))
            .unwrap_err();
        assert_eq!(err.code(), "checksum_mismatch");
    }

    #[test]
    fn no_usable_checksum_means_no_install() {
        let (_d, p) = file(b"abc");
        for bad in [
            None,
            Some("md5:900150983cd24fb0d6963f7d28e17f72"),
            Some("sha256:abc"),
            Some(&format!("sha256:{}", "z".repeat(64))[..]),
            Some(ABC), // no algorithm prefix
            Some(""),
        ] {
            let err = Sha256Verifier.verify(&p, &release(bad)).unwrap_err();
            assert_eq!(err.code(), "no_checksum", "{bad:?}");
        }
    }

    #[test]
    fn a_missing_file_is_an_io_error_not_a_pass() {
        let err = Sha256Verifier
            .verify(
                Path::new("/nonexistent/archive"),
                &release(Some(&format!("sha256:{ABC}"))),
            )
            .unwrap_err();
        assert_eq!(err.code(), "io");
    }
}
