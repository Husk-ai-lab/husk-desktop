//! Streaming download with resume. The archive is written to `<name>.part`; a dropped
//! connection leaves the partial file in place and the next attempt continues with an HTTP
//! `Range` request. A finished download must have exactly the size GitHub reports.

use std::path::Path;

use futures_util::StreamExt;
use reqwest::StatusCode;
use tokio::io::AsyncWriteExt;

use crate::error::{Result, ShellError};

/// `progress(downloaded, total)`.
pub type Progress<'a> = &'a mut (dyn FnMut(u64, u64) + Send);

pub async fn download(
    client: &reqwest::Client,
    url: &str,
    part: &Path,
    expected_size: u64,
    progress: Progress<'_>,
) -> Result<()> {
    if let Some(parent) = part.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let mut have = tokio::fs::metadata(part)
        .await
        .map(|m| m.len())
        .unwrap_or(0);
    if have > expected_size {
        // not a prefix of this asset (a different release was being downloaded): start over
        tokio::fs::remove_file(part).await.ok();
        have = 0;
    }

    if have < expected_size {
        let mut req = client.get(url);
        if have > 0 {
            req = req.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let resp = req.send().await?;
        let status = resp.status();

        let mut file;
        match status {
            StatusCode::PARTIAL_CONTENT if have > 0 => {
                file = tokio::fs::OpenOptions::new()
                    .append(true)
                    .open(part)
                    .await?;
            }
            s if s.is_success() => {
                // server ignored the Range header (or this is a fresh download): write from 0
                have = 0;
                file = tokio::fs::File::create(part).await?;
            }
            StatusCode::RANGE_NOT_SATISFIABLE => {
                tokio::fs::remove_file(part).await.ok();
                return Err(ShellError::DownloadInvalid(
                    "server refused the resume range".into(),
                ));
            }
            s => return Err(ShellError::Network(format!("download answered {s}"))),
        }

        progress(have, expected_size);
        let mut stream = resp.bytes_stream();
        let mut written = have;
        while let Some(chunk) = stream.next().await {
            // a mid-stream failure keeps the .part file so the retry can resume
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            written += chunk.len() as u64;
            progress(written.min(expected_size), expected_size);
        }
        file.flush().await?;
        file.sync_all().await.ok();
    }

    let size = tokio::fs::metadata(part).await?.len();
    if size != expected_size {
        tokio::fs::remove_file(part).await.ok();
        return Err(ShellError::DownloadInvalid(format!(
            "expected {expected_size} bytes, got {size}"
        )));
    }
    progress(expected_size, expected_size);
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_server {
    //! A tiny HTTP server with `Range` support, standing in for GitHub's download CDN.
    use std::sync::Arc;

    pub struct Served {
        pub url: String,
        _server: Arc<tiny_http::Server>,
    }

    /// `ignore_range`: answer 200 with the whole body even when a Range is requested.
    /// `cut_after`: close the connection after that many body bytes (simulates a dropped link).
    pub fn serve(body: Vec<u8>, ignore_range: bool, cut_after: Option<usize>) -> Served {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let port = server.server_addr().to_ip().unwrap().port();
        let s = server.clone();
        std::thread::spawn(move || {
            for req in s.incoming_requests() {
                let range = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Range"))
                    .and_then(|h| {
                        h.value
                            .as_str()
                            .strip_prefix("bytes=")
                            .map(|v| v.to_string())
                    });
                let (status, data): (u16, Vec<u8>) = match (range, ignore_range) {
                    (Some(r), false) => {
                        let start: usize = r.trim_end_matches('-').parse().unwrap_or(0);
                        (206, body[start.min(body.len())..].to_vec())
                    }
                    _ => (200, body.clone()),
                };
                let data = match cut_after {
                    Some(n) if status == 200 || n < data.len() => {
                        data[..n.min(data.len())].to_vec()
                    }
                    _ => data,
                };
                // Content-Length is computed from the data actually sent, so a "cut" is a
                // short-but-well-formed body: the shell must catch it via the size check.
                let resp = tiny_http::Response::from_data(data).with_status_code(status);
                let _ = req.respond(resp);
            }
        });
        Served {
            url: format!("http://127.0.0.1:{port}/asset"),
            _server: server,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_server::serve;
    use super::*;

    fn body(n: usize) -> Vec<u8> {
        (0..n).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn downloads_whole_file_and_reports_progress() {
        let data = body(300_000);
        let srv = serve(data.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        let mut last = (0, 0);
        let mut cb = |d: u64, t: u64| last = (d, t);
        download(
            &reqwest::Client::new(),
            &srv.url,
            &part,
            data.len() as u64,
            &mut cb,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), data);
        assert_eq!(last, (data.len() as u64, data.len() as u64));
    }

    #[tokio::test]
    async fn resumes_from_a_partial_file() {
        let data = body(100_000);
        let srv = serve(data.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        std::fs::write(&part, &data[..40_000]).unwrap();
        let mut cb = |_: u64, _: u64| {};
        download(
            &reqwest::Client::new(),
            &srv.url,
            &part,
            data.len() as u64,
            &mut cb,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), data);
    }

    #[tokio::test]
    async fn server_that_ignores_range_restarts_cleanly() {
        let data = body(50_000);
        let srv = serve(data.clone(), true, None);
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        std::fs::write(&part, &data[..10_000]).unwrap();
        let mut cb = |_: u64, _: u64| {};
        download(
            &reqwest::Client::new(),
            &srv.url,
            &part,
            data.len() as u64,
            &mut cb,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), data);
    }

    #[tokio::test]
    async fn truncated_download_is_rejected_and_removed() {
        let data = body(80_000);
        let srv = serve(data.clone(), false, Some(30_000));
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        let mut cb = |_: u64, _: u64| {};
        let err = download(
            &reqwest::Client::new(),
            &srv.url,
            &part,
            data.len() as u64,
            &mut cb,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), "download_invalid");
        assert!(!part.exists(), "a bad download must not be kept");
    }

    #[tokio::test]
    async fn oversized_part_file_is_discarded() {
        let data = body(10_000);
        let srv = serve(data.clone(), false, None);
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("a.part");
        std::fs::write(&part, body(99_999)).unwrap();
        let mut cb = |_: u64, _: u64| {};
        download(
            &reqwest::Client::new(),
            &srv.url,
            &part,
            data.len() as u64,
            &mut cb,
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read(&part).unwrap(), data);
    }
}
