//! First-run end-to-end test: on a clean app-data folder, resolve the backend release published
//! on GitHub for *this* OS and CPU, download and verify it (SHA-256 against GitHub's digest),
//! install it, start it the way `launch` does and wait for `/health`.
//!
//! It talks to the real GitHub, so it is `#[ignore]`d and does nothing unless `HUSK_E2E=1`:
//!
//! ```text
//! HUSK_E2E=1 cargo test --manifest-path src-tauri/Cargo.toml first_run -- --ignored --nocapture
//! ```
//!
//! Knobs (all optional): `HUSK_E2E_CHANNEL=beta`, `HUSK_E2E_TAG=<backend tag>`,
//! `HUSK_E2E_GITHUB_TOKEN` (API client only: runners share IPs and hit the 60/h anonymous limit),
//! `HUSK_E2E_KEEP=<dir>` (use this app-data folder and keep it, for logs),
//! `HUSK_E2E_RESULT=<file>` (machine-readable result, read by `scripts/release.py collect`).
//!
//! What it proves: the shell's install / verify / supervise core, as compiled from this commit,
//! works on the native OS and CPU against the published backend. It does not launch the packaged
//! GUI app. A platform the backend does not publish (Linux today) is reported as `skipped`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::{start_component, AppState};
use crate::config::{self, HUSK, HUSK_PORT};
use crate::install::installer::is_intact;
use crate::paths::{is_safe_segment, Paths};
use crate::platform::host_triple;
use crate::process::ports;
use crate::release::github::{parse_latest, GitHubSource};
use crate::release::{Channel, ReleaseInfo};

enum Outcome {
    Passed {
        version: String,
        backend_version: String,
        health_seconds: f64,
    },
    Skipped(String),
}

/// `0.1.0-alpha.1`, `0.1.0a1` and `0.1.0` all start with the same `0.1.0`.
fn base_version(v: &str) -> &str {
    let end = v
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(v.len());
    v[..end].trim_end_matches('.')
}

fn api_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder()
        .user_agent(config::USER_AGENT)
        .connect_timeout(Duration::from_secs(15));
    if let Ok(token) = std::env::var("HUSK_E2E_GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            let mut value =
                reqwest::header::HeaderValue::from_str(&format!("Bearer {}", token.trim()))
                    .expect("token is not a valid header value");
            value.set_sensitive(true);
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(reqwest::header::AUTHORIZATION, value);
            builder = builder.default_headers(headers);
        }
    }
    builder.build().expect("http client")
}

async fn resolve(state: &AppState) -> Result<Option<ReleaseInfo>, String> {
    if let Ok(tag) = std::env::var("HUSK_E2E_TAG") {
        let tag = tag.trim().to_string();
        if !tag.is_empty() {
            if !is_safe_segment(&tag) {
                return Err(format!("HUSK_E2E_TAG {tag:?} is not a plain tag name"));
            }
            let url = format!(
                "{}/repos/{}/releases/tags/{tag}",
                config::github_api_base(),
                HUSK.repo
            );
            let resp = api_client()
                .get(&url)
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(|e| format!("GET {url}: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("GET {url}: {}", resp.status()));
            }
            let body = resp.text().await.map_err(|e| e.to_string())?;
            return parse_latest(&HUSK, &body, host_triple()).map_err(|e| e.to_string());
        }
    }
    let channel = state.config.lock().unwrap().channel;
    state
        .source
        .latest(&HUSK, channel)
        .await
        .map_err(|e| format!("release lookup failed: {e}"))
}

async fn health(port: u16) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .await
        .map_err(|e| format!("GET /health: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET /health answered {}", resp.status()));
    }
    resp.json()
        .await
        .map_err(|e| format!("/health is not JSON: {e}"))
}

async fn run(state: &AppState) -> Result<Outcome, String> {
    let triple = host_triple();
    let Some(rel) = resolve(state).await? else {
        return Ok(Outcome::Skipped(format!("no asset for {triple}")));
    };
    eprintln!(
        "e2e: {} {} ({} bytes) for {triple}",
        rel.asset.name, rel.version, rel.asset.size
    );

    // download + SHA-256 against GitHub's digest + extract + finalise
    state
        .installer()
        .install(&HUSK, &rel, &mut |_, _, _| {})
        .await
        .map_err(|e| format!("install failed: {e}"))?;
    if !is_intact(&state.paths, &HUSK, &rel.version) {
        return Err(format!("installed {} is not intact", rel.version));
    }

    // started exactly as `launch` does: same env, --data-dir and HEALTH_TIMEOUT
    let started = Instant::now();
    start_component(state, &HUSK, &rel.version, false)
        .await
        .map_err(|e| format!("start failed: {e} (see logs/husk.log)"))?;
    let health_seconds = started.elapsed().as_secs_f64();
    eprintln!("e2e: /health answered after {health_seconds:.1} s");
    if health_seconds > config::HEALTH_TIMEOUT.as_secs_f64() * 0.75 {
        eprintln!(
            "e2e: WARNING close to the shell's {} s wait",
            config::HEALTH_TIMEOUT.as_secs()
        );
    }

    let body = health(HUSK_PORT).await?;
    if body["status"] != "ok" {
        return Err(format!(
            "/health status is {}, expected \"ok\"",
            body["status"]
        ));
    }
    let backend_version = body["version"]
        .as_str()
        .filter(|v| !v.is_empty())
        .ok_or("/health has no version")?
        .to_string();
    if base_version(&backend_version) != base_version(&rel.version) {
        return Err(format!(
            "backend reports version {backend_version:?}, the release is {:?}",
            rel.version
        ));
    }
    Ok(Outcome::Passed {
        version: rel.version,
        backend_version,
        health_seconds,
    })
}

fn write_result(status: &str, reason: &str, extra: serde_json::Value) {
    let Some(path) = std::env::var_os("HUSK_E2E_RESULT").map(PathBuf::from) else {
        return;
    };
    let mut v = serde_json::json!({
        "status": status,
        "reason": reason,
        "triple": host_triple(),
    });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap())
        .expect("write HUSK_E2E_RESULT");
}

fn leftover_after_stop(paths: &Paths) -> Option<String> {
    if !ports::is_free(HUSK_PORT) {
        return Some(format!("port {HUSK_PORT} is still busy after stop"));
    }
    let pid = paths.run_dir().join("husk.pid");
    pid.exists()
        .then(|| format!("{} was left behind", pid.display()))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "talks to the real GitHub: HUSK_E2E=1 cargo test first_run -- --ignored"]
async fn first_run_installs_and_starts_the_published_backend() {
    if std::env::var("HUSK_E2E").as_deref() != Ok("1") {
        eprintln!("HUSK_E2E is not 1: first-run e2e not run");
        return;
    }

    let scratch = tempfile::tempdir().unwrap();
    let root = match std::env::var_os("HUSK_E2E_KEEP") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => scratch.path().to_path_buf(),
    };
    std::fs::create_dir_all(&root).unwrap();

    let paths = Paths::new(root);
    let mut state = AppState::new(paths.clone()).expect("app state");
    state.source = Arc::new(GitHubSource::new(api_client(), config::github_api_base()));
    if std::env::var("HUSK_E2E_CHANNEL").as_deref() == Ok("beta") {
        state.config.lock().unwrap().channel = Channel::Beta;
    }

    let outcome = run(&state).await;
    // whatever happened above, nothing may outlive the test
    state.supervisor.stop(&paths, HUSK.id).await;
    let leftover = leftover_after_stop(&paths);

    match outcome {
        Ok(Outcome::Skipped(reason)) => {
            eprintln!("e2e: SKIPPED ({reason})");
            write_result("skipped", &reason, serde_json::json!({}));
        }
        Ok(Outcome::Passed {
            version,
            backend_version,
            health_seconds,
        }) => {
            if let Some(problem) = leftover {
                write_result(
                    "failed",
                    &problem,
                    serde_json::json!({ "version": version }),
                );
                panic!("{problem}");
            }
            eprintln!("e2e: PASSED ({version}, backend reports {backend_version})");
            write_result(
                "passed",
                "",
                serde_json::json!({
                    "version": version,
                    "backend_version": backend_version,
                    "health_seconds": health_seconds,
                }),
            );
        }
        Err(reason) => {
            write_result("failed", &reason, serde_json::json!({}));
            panic!("first-run e2e failed: {reason}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::base_version;

    #[test]
    fn base_version_ignores_prerelease_spelling() {
        assert_eq!(base_version("0.1.0"), "0.1.0");
        assert_eq!(base_version("0.1.0-alpha.1"), "0.1.0");
        assert_eq!(base_version("0.1.0a1"), "0.1.0");
        assert_eq!(base_version("1.2.3+build.5"), "1.2.3");
        assert_eq!(base_version("alpha-release"), "");
    }
}
