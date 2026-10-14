//! Where the shell learns which version of a component to install.
//!
//! Today that is the GitHub Releases API (`github.rs`). The plan wants `GET /v1/status` on the
//! gateway (versions come from Neon, with one switch to promote or pull a release), but that
//! endpoint is not built yet. `ReleaseSource` is the seam: swapping the source changes nothing
//! else in the shell.

pub mod github;

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::config::ComponentSpec;
use crate::error::{Result, ShellError};
use crate::paths::is_safe_segment;
use crate::platform::archive_kind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Stable,
    Beta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub url: String,
    /// GitHub's checksum of the asset, as published: `sha256:<64 hex>`. `None` for assets
    /// uploaded before GitHub computed one; such a release is not installed (see `verify.rs`).
    pub digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReleaseInfo {
    pub component: String,
    /// Tag without the leading `v`; safe to use as a directory name.
    pub version: String,
    pub prerelease: bool,
    pub asset: Asset,
}

/// A published release of the shell itself. It cannot be applied in place: the update popup
/// sends the user to `url` (the release page, where the installers are).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellRelease {
    /// Tag without the leading `v`.
    pub version: String,
    pub url: String,
}

pub trait ReleaseSource: Send + Sync {
    /// Latest release of `spec` that has an asset for this machine; `Ok(None)` when there is no
    /// release yet or none for this platform (the UI shows "non ancora disponibile").
    fn latest<'a>(
        &'a self,
        spec: &'a ComponentSpec,
        channel: Channel,
    ) -> BoxFuture<'a, Result<Option<ReleaseInfo>>>;

    /// Latest published (stable) release of the shell itself; `Ok(None)` when there is none.
    /// Defaults to "none" so a source that only knows the backends needs no change.
    fn latest_shell(&self) -> BoxFuture<'_, Result<Option<ShellRelease>>> {
        Box::pin(async { Ok(None) })
    }
}

/// `v1.4.0` -> `1.4.0`, rejecting anything that could not be a plain directory name.
pub fn normalize_version(tag: &str) -> Result<String> {
    let v = tag.trim().trim_start_matches(['v', 'V']);
    if is_safe_segment(v) {
        Ok(v.to_string())
    } else {
        Err(ShellError::Other(format!("unusable release tag {tag:?}")))
    }
}

/// `<prefix>-<version>-<triple>.(tar.gz|zip)`; the version part is not parsed, so a pre-release
/// suffix like `1.5.0-beta.1` works.
pub fn select_asset<'a>(assets: &'a [Asset], prefix: &str, triple: &str) -> Option<&'a Asset> {
    let start = format!("{prefix}-");
    let marker = format!("-{triple}.");
    assets.iter().find(|a| {
        a.name.starts_with(&start) && a.name.contains(&marker) && archive_kind(&a.name).is_some()
    })
}

/// Newer-than comparison used by the update check. An unparseable candidate never counts as
/// newer; a real version does replace an installed non-version tag such as `alpha-release`.
pub fn is_newer(candidate: &str, installed: &str) -> bool {
    match (
        semver::Version::parse(candidate),
        semver::Version::parse(installed),
    ) {
        (Ok(c), Ok(i)) => c > i,
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> Asset {
        Asset {
            name: name.into(),
            size: 10,
            url: format!("https://x/{name}"),
            digest: None,
        }
    }

    #[test]
    fn version_normalisation() {
        assert_eq!(normalize_version("v1.4.0").unwrap(), "1.4.0");
        assert_eq!(normalize_version("1.5.0-beta.1").unwrap(), "1.5.0-beta.1");
        assert!(normalize_version("v../../etc").is_err());
        assert!(normalize_version("").is_err());
    }

    #[test]
    fn picks_the_asset_for_the_triple() {
        let assets = vec![
            asset("SHA256SUMS"),
            asset("manifest.json"),
            asset("husk-backend-1.4.0-x86_64-pc-windows-msvc.zip"),
            asset("husk-backend-1.4.0-aarch64-apple-darwin.tar.gz"),
            asset("husk-studio-2.1.0-aarch64-apple-darwin.tar.gz"),
        ];
        let a = select_asset(&assets, "husk-backend", "aarch64-apple-darwin").unwrap();
        assert_eq!(a.name, "husk-backend-1.4.0-aarch64-apple-darwin.tar.gz");
        let w = select_asset(&assets, "husk-backend", "x86_64-pc-windows-msvc").unwrap();
        assert!(w.name.ends_with(".zip"));
        assert!(select_asset(&assets, "husk-backend", "x86_64-unknown-linux-gnu").is_none());
        // the studio asset must never be picked for husk, nor the reverse
        assert!(select_asset(&assets, "husk-studio", "x86_64-pc-windows-msvc").is_none());
    }

    #[test]
    fn newer_is_semver_aware() {
        assert!(is_newer("1.10.0", "1.9.0"));
        assert!(!is_newer("1.4.0", "1.4.0"));
        assert!(!is_newer("1.4.0-beta.1", "1.4.0"));
        assert!(!is_newer("garbage", "1.0.0"));
        // an alpha tag is replaced by the first real version, never the other way round
        assert!(is_newer("0.1.0", "alpha-release"));
        assert!(!is_newer("alpha-release", "0.1.0"));
    }
}
