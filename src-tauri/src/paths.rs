//! The on-disk layout from the plan ("Where files go"). Data lives outside `bin/`, so updating
//! or reinstalling a backend never touches the user's documents.
//!
//! ```text
//! <app-data>/            macOS ~/Library/Application Support/ai.husk.desktop
//!   config.json
//!   bin/<component>/<version>/<exe>
//!   data/husk/ data/studio/
//!   downloads/  run/  logs/
//! ```

use std::path::{Component, Path, PathBuf};

use crate::error::{Result, ShellError};

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn config_file(&self) -> PathBuf {
        self.root.join("config.json")
    }
    pub fn bin_dir(&self) -> PathBuf {
        self.root.join("bin")
    }
    pub fn component_dir(&self, id: &str, version: &str) -> PathBuf {
        self.bin_dir().join(id).join(version)
    }
    pub fn component_root(&self, id: &str) -> PathBuf {
        self.bin_dir().join(id)
    }
    pub fn data_dir(&self, id: &str) -> PathBuf {
        self.root.join("data").join(id)
    }
    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }
    pub fn run_dir(&self) -> PathBuf {
        self.root.join("run")
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn log_file(&self, name: &str) -> PathBuf {
        self.logs_dir().join(format!("{name}.log"))
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        for d in [
            self.root.clone(),
            self.bin_dir(),
            self.downloads_dir(),
            self.run_dir(),
            self.logs_dir(),
        ] {
            std::fs::create_dir_all(d)?;
        }
        Ok(())
    }

    /// The shell may only execute files inside `<app-data>/bin/` (plan: Tauri permission scope).
    /// Returns the canonical path, or an error when `exe` resolves anywhere else.
    pub fn assert_executable_in_bin(&self, exe: &Path) -> Result<PathBuf> {
        let bin = self
            .bin_dir()
            .canonicalize()
            .map_err(|e| ShellError::Io(format!("bin dir: {e}")))?;
        let real = exe
            .canonicalize()
            .map_err(|e| ShellError::Io(format!("{}: {e}", exe.display())))?;
        if real.starts_with(&bin) {
            Ok(real)
        } else {
            Err(ShellError::Other(format!(
                "refusing to execute {} (outside the bin directory)",
                real.display()
            )))
        }
    }
}

/// A version string becomes a directory name, so it must not be able to climb out of `bin/`.
pub fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s != "."
        && s != ".."
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
        && Path::new(s).components().all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_segments() {
        assert!(is_safe_segment("1.4.0"));
        assert!(is_safe_segment("1.5.0-beta.1"));
        assert!(!is_safe_segment(".."));
        assert!(!is_safe_segment("../x"));
        assert!(!is_safe_segment("a/b"));
        assert!(!is_safe_segment(""));
        assert!(!is_safe_segment("a b"));
    }

    #[test]
    fn executable_must_live_in_bin() {
        let tmp = tempfile::tempdir().unwrap();
        let p = Paths::new(tmp.path().to_path_buf());
        p.ensure_dirs().unwrap();
        let inside = p.component_dir("husk", "1.0.0");
        std::fs::create_dir_all(&inside).unwrap();
        let exe = inside.join("husk-backend");
        std::fs::write(&exe, b"x").unwrap();
        assert!(p.assert_executable_in_bin(&exe).is_ok());

        let outside = tmp.path().join("evil");
        std::fs::write(&outside, b"x").unwrap();
        assert!(p.assert_executable_in_bin(&outside).is_err());
    }
}
