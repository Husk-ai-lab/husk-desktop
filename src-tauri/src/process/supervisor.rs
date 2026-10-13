//! Starts, watches and stops the backends. The shell only ever executes files it installed
//! under `<app-data>/bin/` (checked by the caller with `Paths::assert_executable_in_bin`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use super::{ports, reaper};
use crate::config::STOP_GRACE;
use crate::error::{Result, ShellError};
use crate::paths::Paths;

pub struct StartRequest {
    pub id: String,
    pub exe: PathBuf,
    /// Command-line arguments (no secrets: they show up in the process list).
    pub args: Vec<String>,
    pub port: u16,
    pub health_path: String,
    /// Added on top of the shell's own environment. Never logged (it holds the launch secret).
    pub env: Vec<(String, String)>,
    pub log: PathBuf,
    pub timeout: Duration,
}

#[derive(Default)]
pub struct Supervisor {
    children: Mutex<HashMap<String, Child>>,
}

impl Supervisor {
    pub async fn is_running(&self, id: &str) -> bool {
        let mut map = self.children.lock().await;
        match map.get_mut(id) {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Spawn the backend and wait until `GET /health` answers 2xx. On any failure the child is
    /// killed, so a failed start never leaves a process behind.
    pub async fn start(&self, paths: &Paths, req: StartRequest) -> Result<()> {
        if !ports::is_free(req.port) {
            return Err(ShellError::PortBusy(req.port));
        }

        let mut cmd = Command::new(&req.exe);
        cmd.args(&req.args);
        if let Some(dir) = req.exe.parent() {
            cmd.current_dir(dir);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        let log = open_log(&req.log)?;
        // the log is appended to: only what this run writes may be judged below
        let log_from = log.metadata().map(|m| m.len()).unwrap_or(0);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log))
            .kill_on_drop(true);
        harden(&mut cmd);

        let mut child = cmd
            .spawn()
            .map_err(|e| ShellError::Io(format!("cannot start {}: {e}", req.id)))?;
        let pid = child.id().unwrap_or(0);
        attach_to_job(&child);
        reaper::record(paths, &req.id, pid, &req.exe);
        tracing::info!(component = %req.id, pid, port = req.port, "backend spawned");

        match wait_healthy(&mut child, req.port, &req.health_path, req.timeout, &req.id).await {
            Ok(()) => {
                self.children.lock().await.insert(req.id.clone(), child);
                Ok(())
            }
            Err(e) => {
                terminate(&mut child).await;
                reaper::forget(paths, &req.id);
                tracing::error!(component = %req.id, error = %e, "backend failed to start");
                // PyInstaller's launcher could not find its libraries: the install is broken,
                // not the backend. Reporting it as such offers "Repair" instead of "Retry".
                if matches!(e, ShellError::StartExited(..))
                    && launcher_lost_its_libraries(&req.log, log_from)
                {
                    tracing::error!(component = %req.id, "install is incomplete (PyInstaller cannot load its Python library)");
                    return Err(ShellError::NotInstalled(req.id.clone()));
                }
                Err(e)
            }
        }
    }

    pub async fn stop(&self, paths: &Paths, id: &str) {
        let child = self.children.lock().await.remove(id);
        if let Some(mut c) = child {
            terminate(&mut c).await;
            tracing::info!(component = id, "backend stopped");
        }
        reaper::forget(paths, id);
    }

    pub async fn stop_all(&self, paths: &Paths) {
        let ids: Vec<String> = self.children.lock().await.keys().cloned().collect();
        for id in ids {
            self.stop(paths, &id).await;
        }
    }
}

/// True when the output written since `from` carries PyInstaller's "folder is incomplete" message.
fn launcher_lost_its_libraries(log: &PathBuf, from: u64) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(log) else {
        return false;
    };
    if f.seek(SeekFrom::Start(from)).is_err() {
        return false;
    }
    let mut out = String::new();
    f.take(64 * 1024).read_to_string(&mut out).ok();
    out.contains("Failed to load Python shared library")
}

fn open_log(path: &PathBuf) -> Result<std::fs::File> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?)
}

async fn wait_healthy(
    child: &mut Child,
    port: u16,
    path: &str,
    timeout: Duration,
    id: &str,
) -> Result<()> {
    // loopback traffic must never go through a user-configured proxy
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()?;
    let url = format!("http://127.0.0.1:{port}{path}");
    let deadline = Instant::now() + timeout;
    loop {
        // an early exit is reported at once instead of after the full timeout
        if let Some(status) = child.try_wait()? {
            return Err(ShellError::StartExited(id.to_string(), status.code()));
        }
        if let Ok(r) = client.get(&url).send().await {
            if r.status().is_success() {
                return Ok(());
            }
        }
        if Instant::now() >= deadline {
            return Err(ShellError::StartTimeout(id.to_string()));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

/// Ask nicely (SIGTERM), then kill after the grace period. Windows has no equivalent of
/// SIGTERM for a console-less child, so it is terminated directly.
async fn terminate(child: &mut Child) {
    if matches!(child.try_wait(), Ok(Some(_))) {
        return;
    }
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        unsafe {
            libc::kill(pid as i32, libc::SIGTERM);
        }
        if tokio::time::timeout(STOP_GRACE, child.wait()).await.is_ok() {
            return;
        }
    }
    child.kill().await.ok();
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
}

#[cfg(target_os = "linux")]
fn harden(cmd: &mut Command) {
    // the kernel kills the child if the shell dies, however it dies
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            Ok(())
        });
    }
}

#[cfg(all(unix, not(target_os = "linux")))]
fn harden(_cmd: &mut Command) {}

#[cfg(windows)]
fn harden(cmd: &mut Command) {
    // no console window for the backend
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(windows)]
fn attach_to_job(child: &Child) {
    if let Some(h) = child.raw_handle() {
        reaper::job::assign(h as _);
    }
}

#[cfg(not(windows))]
fn attach_to_job(_child: &Child) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    fn free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn req(exe: PathBuf, port: u16, log: PathBuf, secs: u64) -> StartRequest {
        StartRequest {
            id: "husk".into(),
            exe,
            args: vec![],
            port,
            health_path: "/health".into(),
            env: vec![("HUSK_PORT".into(), port.to_string())],
            log,
            timeout: Duration::from_secs(secs),
        }
    }

    const SERVER: &str = r#"exec python3 -c "
import http.server, os
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def log_message(self, *a): pass
http.server.HTTPServer(('127.0.0.1', int(os.environ['HUSK_PORT'])), H).serve_forever()
""#;

    #[tokio::test]
    async fn starts_waits_for_health_and_stops_cleanly() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(t.path(), "fake-backend", SERVER);
        let port = free_port();
        let sup = Supervisor::default();

        sup.start(&paths, req(exe, port, paths.log_file("husk"), 15))
            .await
            .unwrap();
        assert!(sup.is_running("husk").await);
        assert!(!ports::is_free(port), "backend should hold the port");

        sup.stop(&paths, "husk").await;
        assert!(!sup.is_running("husk").await);
        // port released = the process is really gone
        for _ in 0..20 {
            if ports::is_free(port) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("port still busy after stop");
    }

    #[tokio::test]
    async fn early_exit_is_reported_immediately() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(t.path(), "dies", "exit 3");
        let sup = Supervisor::default();
        let started = Instant::now();
        let err = sup
            .start(&paths, req(exe, free_port(), paths.log_file("husk"), 20))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ShellError::StartExited(_, Some(3))),
            "{err:?}"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must not wait for the timeout"
        );
    }

    #[tokio::test]
    async fn incomplete_install_is_reported_as_not_installed() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        // an older run already left a line in the shared log; only the new output counts
        std::fs::write(
            paths.log_file("husk"),
            "old run: Failed to load Python shared library\n",
        )
        .unwrap();
        let exe = script(
            t.path(),
            "no-libs",
            "echo 'Failed to load Python shared library' >&2; exit 255",
        );
        let sup = Supervisor::default();
        let err = sup
            .start(&paths, req(exe, free_port(), paths.log_file("husk"), 20))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "not_installed", "{err:?}");

        // an ordinary crash with the same old line in the log stays an ordinary crash
        let exe = script(t.path(), "crash", "echo boom >&2; exit 1");
        let err = sup
            .start(&paths, req(exe, free_port(), paths.log_file("husk"), 20))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "start_exited", "{err:?}");
    }

    #[tokio::test]
    async fn arguments_reach_the_backend() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(
            t.path(),
            "echoes",
            &format!("echo \"$1 $2\" > {}/args.txt; exit 3", t.path().display()),
        );
        let sup = Supervisor::default();
        let mut r = req(exe, free_port(), paths.log_file("husk"), 20);
        r.args = vec!["--data-dir".into(), "/some/dir".into()];
        let _ = sup.start(&paths, r).await;
        assert_eq!(
            std::fs::read_to_string(t.path().join("args.txt"))
                .unwrap()
                .trim(),
            "--data-dir /some/dir"
        );
    }

    #[tokio::test]
    async fn silent_backend_times_out_and_is_killed() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(t.path(), "silent", "exec sleep 60");
        let sup = Supervisor::default();
        let err = sup
            .start(&paths, req(exe, free_port(), paths.log_file("husk"), 1))
            .await
            .unwrap_err();
        assert_eq!(err.code(), "start_timeout");
        assert!(!sup.is_running("husk").await);
        assert!(!paths.run_dir().join("husk.pid").exists());
    }

    #[tokio::test]
    async fn busy_port_is_refused_before_spawning() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let exe = script(t.path(), "never-run", "touch ran.marker");
        let sup = Supervisor::default();
        let err = sup
            .start(&paths, req(exe, port, paths.log_file("husk"), 2))
            .await
            .unwrap_err();
        assert!(matches!(err, ShellError::PortBusy(p) if p == port));
        assert!(!t.path().join("ran.marker").exists());
    }
}
