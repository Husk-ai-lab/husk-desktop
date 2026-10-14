//! One error type for everything the shell does. `code()` is what the UI switches on
//! (it picks the localized sentence); `detail` is for the log viewer and support.

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum ShellError {
    #[error("network error: {0}")]
    Network(String),
    #[error("no release available for {0}")]
    NoRelease(String),
    #[error("invalid download: {0}")]
    DownloadInvalid(String),
    #[error("checksum mismatch: {0}")]
    ChecksumMismatch(String),
    #[error("no checksum published for {0}")]
    NoChecksum(String),
    #[error("extraction failed: {0}")]
    Extract(String),
    #[error("port {0} is already in use{}", .1.as_deref().map(|o| format!(" by {o}")).unwrap_or_default())]
    PortBusy(u16, Option<String>),
    #[error("port {0} is reserved by the system (access denied)")]
    PortReserved(u16),
    #[error("{0} did not answer /health in time")]
    StartTimeout(String),
    #[error("{0} exited during startup (code {1:?})")]
    StartExited(String, Option<i32>),
    #[error("{0} is not installed")]
    NotInstalled(String),
    #[error("i/o error: {0}")]
    Io(String),
    #[error("{0}")]
    Other(String),
}

impl ShellError {
    pub fn code(&self) -> &'static str {
        match self {
            ShellError::Network(_) => "network",
            ShellError::NoRelease(_) => "no_release",
            ShellError::DownloadInvalid(_) => "download_invalid",
            ShellError::ChecksumMismatch(_) => "checksum_mismatch",
            ShellError::NoChecksum(_) => "no_checksum",
            ShellError::Extract(_) => "extract_failed",
            ShellError::PortBusy(..) => "port_busy",
            ShellError::PortReserved(_) => "port_reserved",
            ShellError::StartTimeout(_) => "start_timeout",
            ShellError::StartExited(..) => "start_exited",
            ShellError::NotInstalled(_) => "not_installed",
            ShellError::Io(_) => "io",
            ShellError::Other(_) => "other",
        }
    }
}

impl Serialize for ShellError {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("ShellError", 2)?;
        st.serialize_field("code", self.code())?;
        st.serialize_field("detail", &self.to_string())?;
        st.end()
    }
}

impl From<std::io::Error> for ShellError {
    fn from(e: std::io::Error) -> Self {
        ShellError::Io(e.to_string())
    }
}

impl From<reqwest::Error> for ShellError {
    fn from(e: reqwest::Error) -> Self {
        ShellError::Network(e.without_url().to_string())
    }
}

impl From<serde_json::Error> for ShellError {
    fn from(e: serde_json::Error) -> Self {
        ShellError::Other(format!("json: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, ShellError>;
