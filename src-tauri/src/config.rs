//! Compiled-in configuration. Nothing here can be changed by a config file or the environment
//! in a release build (plan: "The gateway URL and the signature public key are compiled into the
//! shell; no config file can change them"). Debug builds may redirect the release source to a
//! local server so the whole flow can be tested without publishing anything.

use std::time::Duration;

pub const USER_AGENT: &str = concat!("HuskDesktop/", env!("CARGO_PKG_VERSION"));

/// The Clerk redirect URI is exactly `http://127.0.0.1:8765/callback`, so Husk must stay on 8765.
pub const HUSK_PORT: u16 = 8765;
/// Studio's own defaults, which Husk probes (`HUSK_STUDIO_UI_URL` / `HUSK_STUDIO_MCP_URL`).
pub const STUDIO_PORT: u16 = 7654;
pub const STUDIO_MCP_PORT: u16 = 7655;

/// Reserved for the day `GET /v1/status` exists on the gateway (see `release::ReleaseSource`).
#[allow(dead_code)]
pub const GATEWAY_STATUS_URL: &str = "https://api.husk-ai.com/v1/status";

pub const GITHUB_API: &str = "https://api.github.com";

pub const HEALTH_TIMEOUT: Duration = Duration::from_secs(20);
pub const STOP_GRACE: Duration = Duration::from_secs(5);

/// One installable backend. Adding a component is adding an entry to [`COMPONENTS`].
#[derive(Debug, Clone, Copy)]
pub struct ComponentSpec {
    pub id: &'static str,
    pub display_name: &'static str,
    /// Husk is mandatory; Studio is optional.
    pub required: bool,
    /// Pre-checked on the first-run screen.
    pub recommended: bool,
    /// `owner/repo` on GitHub (public repositories, no token needed).
    pub repo: &'static str,
    /// Release assets are named `<asset_prefix>-<version>-<target triple>.tar.gz|zip`.
    pub asset_prefix: &'static str,
    /// Executable inside the PyInstaller one-folder build (without `.exe`).
    pub exe_name: &'static str,
    pub port: u16,
    pub health_path: &'static str,
}

pub const HUSK: ComponentSpec = ComponentSpec {
    id: "husk",
    display_name: "Husk AI",
    required: true,
    recommended: true,
    repo: "Husk-ai-lab/HuskAI-core",
    asset_prefix: "husk-backend",
    exe_name: "husk-backend",
    port: HUSK_PORT,
    health_path: "/health",
};

pub const STUDIO: ComponentSpec = ComponentSpec {
    id: "studio",
    display_name: "Husk Studio",
    required: false,
    recommended: true,
    repo: "Husk-ai-lab/husk-ai",
    asset_prefix: "husk-studio",
    exe_name: "husk-studio",
    port: STUDIO_PORT,
    health_path: "/health",
};

/// Start order is the plan's: Studio first (if installed), then Husk.
pub const COMPONENTS: [ComponentSpec; 2] = [HUSK, STUDIO];

pub fn spec(id: &str) -> Option<&'static ComponentSpec> {
    COMPONENTS.iter().find(|c| c.id == id)
}

/// Where to read releases from. Release builds: always GitHub.
pub fn github_api_base() -> String {
    #[cfg(debug_assertions)]
    if let Ok(v) = std::env::var("HUSK_DESKTOP_GITHUB_API") {
        if !v.trim().is_empty() {
            return v.trim_end_matches('/').to_string();
        }
    }
    GITHUB_API.to_string()
}
