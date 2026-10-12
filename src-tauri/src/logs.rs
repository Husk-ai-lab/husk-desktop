//! `shell.log` (rolling, daily) plus a reader for the log viewer. Backend output goes to
//! `husk.log` / `studio.log` through the supervisor. Environment variables and the launch secret
//! are never logged.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;

use crate::paths::Paths;

static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

pub fn init(paths: &Paths) {
    let appender = tracing_appender::rolling::daily(paths.logs_dir(), "shell.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = GUARD.set(guard);
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .try_init()
        .ok();
}

/// Names the log viewer may open (never a path from the UI).
pub const LOG_NAMES: [&str; 3] = ["shell", "husk", "studio"];

/// Last `max_bytes` of a log, cut at a line boundary. Daily-rolled shell logs are found by prefix.
pub fn tail(paths: &Paths, name: &str, max_bytes: u64) -> String {
    if !LOG_NAMES.contains(&name) {
        return String::new();
    }
    let file = if name == "shell" {
        newest_with_prefix(&paths.logs_dir(), "shell.log")
    } else {
        Some(paths.log_file(name))
    };
    let Some(file) = file else { return String::new() };
    read_tail(&file, max_bytes)
}

fn newest_with_prefix(dir: &Path, prefix: &str) -> Option<std::path::PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(prefix))
        .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
        .map(|e| e.path())
}

fn read_tail(path: &Path, max_bytes: u64) -> String {
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut buf = Vec::new();
    f.take(max_bytes).read_to_end(&mut buf).ok();
    let text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_cuts_at_a_line_boundary_and_rejects_unknown_names() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        std::fs::write(paths.log_file("husk"), "line one\nline two\nline three\n").unwrap();
        assert_eq!(tail(&paths, "husk", 1000), "line one\nline two\nline three\n");
        assert_eq!(tail(&paths, "husk", 16), "line three\n");
        assert_eq!(tail(&paths, "../../etc/passwd", 100), "");
        assert_eq!(tail(&paths, "studio", 100), "");
    }
}
