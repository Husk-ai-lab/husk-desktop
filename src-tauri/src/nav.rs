//! What a webview may navigate to. The Husk window shows a backend page served from
//! `http://127.0.0.1:8765`; anything else a link points at must not replace it:
//!   * our own backends stay in a webview (Studio links go to the Studio window),
//!   * ordinary web links open in the system browser,
//!   * everything else is blocked.

use url::Url;

use crate::config::{HUSK_PORT, STUDIO_PORT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Nav {
    /// Navigate inside this webview.
    Allow,
    /// Open (or focus) the Studio window instead.
    Studio,
    /// Hand over to the system browser.
    External,
    Block,
}

/// A link on this path (any Husk-port origin) is the Studio window's "back to Husk" button: the
/// shell intercepts it and focuses the Husk window instead of loading anything.
pub const BACK_TO_HUSK_PATH: &str = "/__shell/husk";

/// True for a page of the Husk backend itself (`http://127.0.0.1:8765/...`).
pub fn is_husk_url(url: &Url) -> bool {
    url.scheme() == "http"
        && is_loopback(url.host_str().unwrap_or_default())
        && url.port_or_known_default() == Some(HUSK_PORT)
}

fn is_loopback(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "[::1]" | "::1")
}

pub fn classify(url: &Url, dev: bool) -> Nav {
    match url.scheme() {
        // the shell's own bundled UI
        "tauri" | "ipc" | "asset" | "about" | "blob" | "data" => return Nav::Allow,
        "mailto" | "tel" => return Nav::External,
        "http" | "https" => {}
        _ => return Nav::Block,
    }
    let host = url.host_str().unwrap_or_default();
    // Windows/Android serve the bundled UI from http(s)://tauri.localhost
    if host == "tauri.localhost" {
        return Nav::Allow;
    }
    if is_loopback(host) && url.scheme() == "http" {
        match url.port_or_known_default() {
            Some(HUSK_PORT) => return Nav::Allow,
            Some(STUDIO_PORT) => return Nav::Studio,
            Some(1420) if dev => return Nav::Allow, // Vite dev server of the shell's own UI
            _ => {}
        }
    }
    Nav::External
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(u: &str) -> Nav {
        classify(&Url::parse(u).unwrap(), false)
    }

    #[test]
    fn backends_and_bundled_ui() {
        assert_eq!(c("http://127.0.0.1:8765/"), Nav::Allow);
        assert_eq!(c("http://localhost:8765/discovery"), Nav::Allow);
        assert_eq!(c("http://127.0.0.1:7654/runs/abc"), Nav::Studio);
        assert_eq!(c("tauri://localhost/index.html"), Nav::Allow);
        assert_eq!(c("http://tauri.localhost/"), Nav::Allow);
    }

    #[test]
    fn web_links_go_to_the_system_browser() {
        assert_eq!(c("https://example.com/"), Nav::External);
        assert_eq!(c("https://clerk.example/oauth/authorize"), Nav::External);
        assert_eq!(c("mailto:a@b.it"), Nav::External);
        // some other local service is not ours
        assert_eq!(c("http://127.0.0.1:9999/"), Nav::External);
        // https on our port is not our backend
        assert_eq!(c("https://127.0.0.1:8765/"), Nav::External);
    }

    #[test]
    fn dangerous_schemes_are_blocked() {
        assert_eq!(c("file:///etc/passwd"), Nav::Block);
        assert_eq!(c("javascript:alert(1)"), Nav::Block);
        assert_eq!(c("ftp://x/"), Nav::Block);
    }

    #[test]
    fn husk_pages_are_recognised() {
        assert!(is_husk_url(
            &Url::parse("http://127.0.0.1:8765/__shell/husk").unwrap()
        ));
        assert!(is_husk_url(
            &Url::parse("http://localhost:8765/discovery").unwrap()
        ));
        assert!(!is_husk_url(&Url::parse("http://127.0.0.1:7654/").unwrap()));
        assert!(!is_husk_url(
            &Url::parse("https://127.0.0.1:8765/").unwrap()
        ));
    }

    #[test]
    fn dev_server_only_in_debug() {
        let u = Url::parse("http://localhost:1420/").unwrap();
        assert_eq!(classify(&u, true), Nav::Allow);
        assert_eq!(classify(&u, false), Nav::External);
    }
}
