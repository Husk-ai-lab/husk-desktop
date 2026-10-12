//! `config.json`: which components are installed and at which version. It is written LAST
//! during an install (plan: "a half-finished install is never treated as complete") and always
//! atomically (temp file + rename).

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::release::Channel;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledComponent {
    pub version: String,
    /// Kept for one-click rollback until the new version has started successfully once.
    #[serde(default)]
    pub previous_version: Option<String>,
    /// True from an update until the new version answered `/health` once.
    #[serde(default)]
    pub pending_confirm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    pub schema: u32,
    /// The first-run screen has been completed.
    #[serde(default)]
    pub setup_done: bool,
    #[serde(default)]
    pub channel: Channel,
    #[serde(default)]
    pub components: BTreeMap<String, InstalledComponent>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            setup_done: false,
            channel: Channel::Stable,
            components: BTreeMap::new(),
        }
    }
}

impl Config {
    /// A missing or unreadable file means "nothing installed": the setup screen shows again and
    /// the existing bin/ and data/ folders are never deleted because of it.
    pub fn load(path: &Path) -> Config {
        std::fs::read(path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Config>(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn installed(&self, id: &str) -> Option<&InstalledComponent> {
        self.components.get(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_missing_file() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("config.json");
        assert_eq!(Config::load(&f), Config::default());

        let mut c = Config::default();
        c.setup_done = true;
        c.components.insert(
            "husk".into(),
            InstalledComponent {
                version: "1.4.0".into(),
                previous_version: Some("1.3.2".into()),
                pending_confirm: true,
            },
        );
        c.save(&f).unwrap();
        assert_eq!(Config::load(&f), c);
        assert!(!t.path().join("config.json.tmp").exists());
    }

    #[test]
    fn corrupt_file_is_treated_as_empty() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("config.json");
        std::fs::write(&f, b"{ not json").unwrap();
        assert_eq!(Config::load(&f), Config::default());
    }
}
