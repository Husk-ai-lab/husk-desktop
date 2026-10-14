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
    /// How long a start waits for its port (tests make this short).
    pub waits: ports::Waits,
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
    ///
    /// The port is first waited for (a backend that stopped a moment ago leaves `TIME_WAIT`
    /// sockets that make Studio's own port check fail, see `ports.rs`). If the backend still
    /// exits with "already in use", it is started once more after waiting again.
    pub async fn start(&self, paths: &Paths, req: StartRequest) -> Result<()> {
        self.ensure_port(req.port).await?;
        let (first, log_from) = self.start_once(paths, &req).await;
        match first {
            Err(ShellError::StartExited(..)) if output_says_port_in_use(&req.log, log_from) => {
                tracing::warn!(component = %req.id, "backend found its port busy, waiting and retrying once");
                self.ensure_port(req.port).await?;
                self.start_once(paths, &req).await.0
            }
            other => other,
        }
    }

    async fn ensure_port(&self, port: u16) -> Result<()> {
        match ports::wait_until_bindable(port, self.waits).await {
            // still lingering after the whole wait: try anyway, a backend that binds with
            // SO_REUSEADDR (uvicorn) starts fine and the one that does not is retried above
            ports::PortState::Free | ports::PortState::Lingering => Ok(()),
            ports::PortState::Denied => Err(ShellError::PortReserved(port)),
            ports::PortState::InUse => {
                Err(ShellError::PortBusy(port, ports::port_owner(port).await))
            }
        }
    }

    /// One spawn + health wait. Returns the result and where this run's output starts in the log.
    async fn start_once(&self, paths: &Paths, req: &StartRequest) -> (Result<()>, u64) {
        let mut cmd = Command::new(&req.exe);
        cmd.args(&req.args);
        if let Some(dir) = req.exe.parent() {
            cmd.current_dir(dir);
        }
        for (k, v) in &req.env {
            cmd.env(k, v);
        }
        let log = match open_log(&req.log) {
            Ok(l) => l,
            Err(e) => return (Err(e), 0),
        };
        // the log is appended to: only what this run writes may be judged below
        let log_from = log.metadata().map(|m| m.len()).unwrap_or(0);
        let stdio = match log.try_clone() {
            Ok(c) => c,
            Err(e) => return (Err(e.into()), log_from),
        };
        cmd.stdin(Stdio::null())
            .stdout(Stdio::from(stdio))
            .stderr(Stdio::from(log))
            .kill_on_drop(true);
        harden(&mut cmd);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return (
                    Err(ShellError::Io(format!("cannot start {}: {e}", req.id))),
                    log_from,
                )
            }
        };
        let pid = child.id().unwrap_or(0);
        attach_to_job(&child);
        reaper::record(paths, &req.id, pid, &req.exe);
        tracing::info!(component = %req.id, pid, port = req.port, "backend spawned");

        match wait_healthy(&mut child, req.port, &req.health_path, req.timeout, &req.id).await {
            Ok(()) => {
                self.children.lock().await.insert(req.id.clone(), child);
                (Ok(()), log_from)
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
                    return (Err(ShellError::NotInstalled(req.id.clone())), log_from);
                }
                (Err(e), log_from)
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

/// True when the backend wrote "... is already in use" since `from`: it could not bind its port.
fn output_says_port_in_use(log: &PathBuf, from: u64) -> bool {
    run_output(log, from)
        .to_ascii_lowercase()
        .contains("already in use")
}

fn run_output(log: &PathBuf, from: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(log) else {
        return String::new();
    };
    if f.seek(SeekFrom::Start(from)).is_err() {
        return String::new();
    }
    let mut out = String::new();
    f.take(64 * 1024).read_to_string(&mut out).ok();
    out
}

/// True when the output written since `from` carries PyInstaller's "folder is incomplete" message.
fn launcher_lost_its_libraries(log: &PathBuf, from: u64) -> bool {
    run_output(log, from).contains("Failed to load Python shared library")
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

    /// Tests run in parallel inside one process and the port is released before the backend binds
    /// it, so the same number must never be handed to two of them.
    fn free_port() -> u16 {
        use std::collections::HashSet;
        use std::sync::{Mutex, OnceLock};
        static GIVEN: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
        let given = GIVEN.get_or_init(Default::default);
        loop {
            let port = std::net::TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port();
            if given.lock().unwrap().insert(port) {
                return port;
            }
        }
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
import http.server, os, socketserver
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200); self.end_headers(); self.wfile.write(b'ok')
    def log_message(self, *a): pass
class S(http.server.HTTPServer):
    # HTTPServer.server_bind() does a reverse-DNS lookup (getfqdn) before it listens, which can
    # take longer than the test's whole timeout on a CI runner with slow DNS
    def server_bind(self):
        socketserver.TCPServer.server_bind(self)
        self.server_name, self.server_port = 'localhost', self.server_address[1]
S(('127.0.0.1', int(os.environ['HUSK_PORT'])), H).serve_forever()
""#;

    #[tokio::test]
    async fn starts_waits_for_health_and_stops_cleanly() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(t.path(), "fake-backend", SERVER);
        let port = free_port();
        let sup = Supervisor::default();

        if let Err(e) = sup
            .start(&paths, req(exe, port, paths.log_file("husk"), 15))
            .await
        {
            let log = std::fs::read_to_string(paths.log_file("husk")).unwrap_or_default();
            panic!("start failed: {e:?}; backend output: {log:?}");
        }
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

    /// Short waits: tests must not sit through the real 3 s / 60 s budgets.
    fn quick() -> Supervisor {
        Supervisor {
            waits: ports::Waits {
                lingering: Duration::from_millis(100),
                in_use: Duration::from_millis(100),
                poll: Duration::from_millis(10),
            },
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn busy_port_is_refused_before_spawning() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let exe = script(t.path(), "never-run", "touch ran.marker");
        let sup = quick();
        let err = sup
            .start(&paths, req(exe, port, paths.log_file("husk"), 2))
            .await
            .unwrap_err();
        assert!(
            matches!(err, ShellError::PortBusy(p, _) if p == port),
            "{err:?}"
        );
        assert_eq!(err.code(), "port_busy");
        assert!(!t.path().join("ran.marker").exists());
    }

    #[tokio::test]
    async fn a_port_that_frees_up_during_the_wait_is_used() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let sup = Supervisor {
            waits: ports::Waits {
                lingering: Duration::from_secs(5),
                in_use: Duration::from_secs(5),
                poll: Duration::from_millis(20),
            },
            ..Default::default()
        };
        // the previous owner goes away 300 ms in
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            drop(l);
        });
        let exe = script(t.path(), "fake-backend", SERVER);
        sup.start(&paths, req(exe, port, paths.log_file("husk"), 15))
            .await
            .unwrap();
        assert!(sup.is_running("husk").await);
        sup.stop(&paths, "husk").await;
    }

    /// First run says what Studio says when it cannot bind; the second one serves.
    const FAILS_ONCE: &str = r#"
if [ ! -f "$0.ran" ]; then
  touch "$0.ran"
  echo "error: port $HUSK_PORT is already in use; --port was given explicitly, so it will not pick another" >&2
  exit 1
fi
"#;

    #[tokio::test]
    async fn a_backend_that_finds_its_port_busy_is_started_once_more() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(t.path(), "studio-like", &format!("{FAILS_ONCE}\n{SERVER}"));
        let sup = quick();
        sup.start(&paths, req(exe, free_port(), paths.log_file("studio"), 15))
            .await
            .unwrap();
        assert!(sup.is_running("husk").await);
        sup.stop(&paths, "husk").await;
    }

    #[tokio::test]
    async fn a_backend_that_always_finds_its_port_busy_fails_after_one_retry() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let body = r#"
echo x >> "$0.count"
echo "error: port $HUSK_PORT is already in use" >&2
exit 1
"#;
        let exe = script(t.path(), "always-busy", body);
        let sup = quick();
        let err = sup
            .start(
                &paths,
                req(exe.clone(), free_port(), paths.log_file("husk"), 15),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, ShellError::StartExited(_, Some(1))),
            "{err:?}"
        );
        let runs = std::fs::read_to_string(format!("{}.count", exe.display())).unwrap();
        assert_eq!(runs.lines().count(), 2, "one start and one retry, no more");
        assert!(!sup.is_running("husk").await);
        assert!(!paths.run_dir().join("husk.pid").exists());
    }

    #[tokio::test]
    async fn another_exit_code_is_not_retried() {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        let exe = script(
            t.path(),
            "crashes",
            "echo x >> \"$0.count\"\necho boom >&2\nexit 3",
        );
        let sup = quick();
        let err = sup
            .start(
                &paths,
                req(exe.clone(), free_port(), paths.log_file("husk"), 15),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ShellError::StartExited(_, Some(3))));
        let runs = std::fs::read_to_string(format!("{}.count", exe.display())).unwrap();
        assert_eq!(runs.lines().count(), 1);
    }
}
