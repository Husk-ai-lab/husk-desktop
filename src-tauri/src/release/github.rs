//! GitHub Releases as a [`ReleaseSource`]. Both repositories are public, so the API needs no
//! token (60 requests/hour per IP is plenty: one check per component per launch).

use futures_util::future::BoxFuture;
use serde::Deserialize;

use super::{normalize_version, select_asset, Asset, Channel, ReleaseInfo, ReleaseSource};
use crate::config::ComponentSpec;
use crate::error::{Result, ShellError};
use crate::platform::host_triple;

pub struct GitHubSource {
    client: reqwest::Client,
    api_base: String,
}

impl GitHubSource {
    pub fn new(client: reqwest::Client, api_base: String) -> Self {
        Self { client, api_base }
    }

    async fn fetch(&self, url: &str) -> Result<Option<String>> {
        let resp = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None); // repository has no release yet
        }
        if !resp.status().is_success() {
            return Err(ShellError::Network(format!(
                "GitHub answered {}",
                resp.status()
            )));
        }
        Ok(Some(resp.text().await?))
    }
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

fn to_info(spec: &ComponentSpec, r: GhRelease, triple: &str) -> Option<ReleaseInfo> {
    if r.draft {
        return None;
    }
    let version = normalize_version(&r.tag_name).ok()?;
    let assets: Vec<Asset> = r
        .assets
        .into_iter()
        .map(|a| Asset {
            name: a.name,
            size: a.size,
            url: a.browser_download_url,
            digest: a.digest,
        })
        .collect();
    let asset = select_asset(&assets, spec.asset_prefix, triple)?.clone();
    Some(ReleaseInfo {
        component: spec.id.into(),
        version,
        prerelease: r.prerelease,
        asset,
    })
}

/// `releases/latest` body -> the release for this machine (stable channel).
pub fn parse_latest(spec: &ComponentSpec, body: &str, triple: &str) -> Result<Option<ReleaseInfo>> {
    let r: GhRelease = serde_json::from_str(body)?;
    Ok(to_info(spec, r, triple))
}

/// `releases` list body -> the newest release (pre-releases included) that has our asset.
pub fn parse_list(spec: &ComponentSpec, body: &str, triple: &str) -> Result<Option<ReleaseInfo>> {
    let rs: Vec<GhRelease> = serde_json::from_str(body)?;
    Ok(rs.into_iter().find_map(|r| to_info(spec, r, triple)))
}

impl ReleaseSource for GitHubSource {
    fn latest<'a>(
        &'a self,
        spec: &'a ComponentSpec,
        channel: Channel,
    ) -> BoxFuture<'a, Result<Option<ReleaseInfo>>> {
        Box::pin(async move {
            let triple = host_triple();
            match channel {
                Channel::Stable => {
                    let url = format!("{}/repos/{}/releases/latest", self.api_base, spec.repo);
                    match self.fetch(&url).await? {
                        Some(body) => parse_latest(spec, &body, triple),
                        None => Ok(None),
                    }
                }
                Channel::Beta => {
                    let url = format!("{}/repos/{}/releases?per_page=10", self.api_base, spec.repo);
                    match self.fetch(&url).await? {
                        Some(body) => parse_list(spec, &body, triple),
                        None => Ok(None),
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{HUSK, STUDIO};

    const LATEST: &str = r#"{
      "tag_name": "v1.4.0", "draft": false, "prerelease": false,
      "assets": [
        {"name":"husk-backend-1.4.0-aarch64-apple-darwin.tar.gz","size":98213345,
         "digest":"sha256:2d201ef07b21d5652860bce4370faef46e3ff158c205ceedcd9d457880fd2c20",
         "browser_download_url":"https://github.com/Husk-ai-lab/HuskAI-core/releases/download/v1.4.0/husk-backend-1.4.0-aarch64-apple-darwin.tar.gz"},
        {"name":"husk-backend-1.4.0-x86_64-pc-windows-msvc.zip","size":120000000,
         "browser_download_url":"https://example/w.zip"},
        {"name":"SHA256SUMS","size":300,"browser_download_url":"https://example/s"}
      ]}"#;

    #[test]
    fn parses_latest_for_the_right_triple() {
        let r = parse_latest(&HUSK, LATEST, "aarch64-apple-darwin")
            .unwrap()
            .unwrap();
        assert_eq!(r.version, "1.4.0");
        assert_eq!(r.asset.size, 98213345);
        assert_eq!(
            r.asset.digest.as_deref(),
            Some("sha256:2d201ef07b21d5652860bce4370faef46e3ff158c205ceedcd9d457880fd2c20")
        );
        assert!(r
            .asset
            .url
            .starts_with("https://github.com/Husk-ai-lab/HuskAI-core/"));
        assert!(parse_latest(&HUSK, LATEST, "x86_64-unknown-linux-gnu")
            .unwrap()
            .is_none());
    }

    #[test]
    fn an_asset_without_a_digest_is_parsed_with_none() {
        let body = r#"{"tag_name":"v1.0.0","assets":[
          {"name":"husk-backend-1.0.0-aarch64-apple-darwin.tar.gz","size":5,"browser_download_url":"https://e/a"}]}"#;
        let r = parse_latest(&HUSK, body, "aarch64-apple-darwin")
            .unwrap()
            .unwrap();
        assert_eq!(r.asset.digest, None);
    }

    #[test]
    fn no_matching_asset_means_unavailable() {
        // Studio has no release assets of its own in this fixture
        assert!(parse_latest(&STUDIO, LATEST, "aarch64-apple-darwin")
            .unwrap()
            .is_none());
    }

    #[test]
    fn drafts_are_ignored_and_list_picks_first_usable() {
        let body = r#"[
          {"tag_name":"v1.6.0","draft":true,"prerelease":false,"assets":[]},
          {"tag_name":"v1.5.0-beta.1","draft":false,"prerelease":true,"assets":[
            {"name":"husk-backend-1.5.0-beta.1-aarch64-apple-darwin.tar.gz","size":5,"browser_download_url":"https://e/b"}]},
          {"tag_name":"v1.4.0","draft":false,"prerelease":false,"assets":[
            {"name":"husk-backend-1.4.0-aarch64-apple-darwin.tar.gz","size":5,"browser_download_url":"https://e/a"}]}
        ]"#;
        let r = parse_list(&HUSK, body, "aarch64-apple-darwin")
            .unwrap()
            .unwrap();
        assert_eq!(r.version, "1.5.0-beta.1");
        assert!(r.prerelease);
    }

    #[test]
    fn a_hostile_tag_is_rejected_not_used_as_a_path() {
        let body = r#"{"tag_name":"v../../x","assets":[
          {"name":"husk-backend-1-aarch64-apple-darwin.tar.gz","size":1,"browser_download_url":"https://e/a"}]}"#;
        assert!(parse_latest(&HUSK, body, "aarch64-apple-darwin")
            .unwrap()
            .is_none());
    }
}
