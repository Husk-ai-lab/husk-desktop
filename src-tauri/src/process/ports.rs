//! Is a backend's port usable, and if not, why? A plain "bind and see" is not enough, because the
//! backends do not all bind the same way:
//!
//! * Rust's `TcpListener::bind` sets `SO_REUSEADDR` on Unix, so it succeeds while sockets of a
//!   backend that stopped a moment ago are still in `TIME_WAIT` (30 s on macOS, 60 s on Linux).
//! * Husk Studio checks its port with a plain Python `bind()` (no `SO_REUSEADDR`) and, started
//!   with an explicit `--port`, exits with "port ... is already in use" in that same window.
//!
//! [`probe`] therefore binds the strict way first (what Studio does) and tells a lingering port
//! apart from one a program is really listening on, and from one the system refuses to give us
//! (Windows reserves port ranges for Hyper-V, WSL and Docker: `WSAEACCES`).

use std::io::ErrorKind;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::time::{Duration, Instant};

use socket2::{Domain, Socket, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortState {
    /// A strict bind works: any backend can take it.
    Free,
    /// No listener, only sockets in `TIME_WAIT`. A backend that binds with `SO_REUSEADDR` (uvicorn)
    /// can start, one that does not (Studio's own check) cannot until they expire.
    Lingering,
    /// Something is listening.
    InUse,
    /// The system refuses the bind (a reserved port range on Windows), not another program.
    Denied,
}

fn addr(port: u16) -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, port))
}

/// True when nothing is listening on 127.0.0.1:`port` (a bind with `SO_REUSEADDR` works, like the
/// Rust standard library's). Lingering sockets do not count as busy here; see [`probe`].
pub fn is_free(port: u16) -> bool {
    TcpListener::bind(addr(port)).is_ok()
}

pub fn probe(port: u16) -> PortState {
    // `Socket::new` does not set SO_REUSEADDR: this is the bind Python (and so Studio) does.
    let strict =
        Socket::new(Domain::IPV4, Type::STREAM, None).and_then(|s| s.bind(&addr(port).into()));
    match strict {
        Ok(()) => PortState::Free,
        Err(e) if e.kind() == ErrorKind::PermissionDenied => PortState::Denied,
        Err(_) if is_free(port) => PortState::Lingering,
        Err(_) => PortState::InUse,
    }
}

/// How long to wait for each kind of "not free yet" before giving up.
#[derive(Debug, Clone, Copy)]
pub struct Waits {
    /// Longest `TIME_WAIT` among the supported systems (Linux: 60 s).
    pub lingering: Duration,
    /// A process that was just killed gives its listener back within milliseconds.
    pub in_use: Duration,
    pub poll: Duration,
}

impl Default for Waits {
    fn default() -> Self {
        Self {
            lingering: Duration::from_secs(60),
            in_use: Duration::from_secs(3),
            poll: Duration::from_millis(500),
        }
    }
}

/// Polls `probe` until the port is `Free` or the wait for its current state is used up; returns
/// the last state seen. `Denied` returns at once: waiting does not lift a reserved range.
pub async fn wait_until_bindable_with(mut probe: impl FnMut() -> PortState, w: Waits) -> PortState {
    let start = Instant::now();
    let mut announced = false;
    loop {
        let state = probe();
        let budget = match state {
            PortState::Free | PortState::Denied => return state,
            PortState::Lingering => w.lingering,
            PortState::InUse => w.in_use,
        };
        if start.elapsed() >= budget {
            return state;
        }
        if state == PortState::Lingering && !announced {
            announced = true;
            tracing::info!("port still held by closing sockets of the previous run, waiting");
        }
        tokio::time::sleep(w.poll).await;
    }
}

pub async fn wait_until_bindable(port: u16, w: Waits) -> PortState {
    wait_until_bindable_with(|| probe(port), w).await
}

/// Best effort "name (pid N)" of the program listening on `port`, for the error message.
/// `None` when it cannot be told (no `lsof`/`netstat`, no permission, nothing listening).
pub async fn port_owner(port: u16) -> Option<String> {
    let pid = listening_pid(port).await?;
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    match sys.process(sysinfo::Pid::from_u32(pid)) {
        Some(p) => Some(format!("{} (pid {pid})", p.name().to_string_lossy())),
        None => Some(format!("pid {pid}")),
    }
}

async fn listening_pid(port: u16) -> Option<u32> {
    #[cfg(windows)]
    let (program, args) = (
        "netstat",
        vec!["-ano".to_string(), "-p".into(), "tcp".into()],
    );
    #[cfg(not(windows))]
    let (program, args) = (
        "lsof",
        vec![
            "-nP".to_string(),
            format!("-iTCP:{port}"),
            "-sTCP:LISTEN".into(),
            "-Fp".into(),
        ],
    );

    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let out = tokio::time::timeout(Duration::from_secs(2), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    #[cfg(windows)]
    return parse_netstat(&text, port);
    #[cfg(not(windows))]
    return parse_lsof(&text, port);
}

/// `lsof -Fp` prints one `p<pid>` line per process.
#[cfg_attr(windows, allow(dead_code))]
fn parse_lsof(out: &str, _port: u16) -> Option<u32> {
    out.lines()
        .find_map(|l| l.strip_prefix('p')?.trim().parse().ok())
}

/// `netstat -ano -p tcp` rows: `TCP  127.0.0.1:7654  0.0.0.0:0  LISTENING  4321`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_netstat(out: &str, port: u16) -> Option<u32> {
    let suffix = format!(":{port}");
    out.lines().find_map(|l| {
        let cols: Vec<&str> = l.split_whitespace().collect();
        let listening = cols.len() >= 5
            && cols[0].eq_ignore_ascii_case("TCP")
            && cols[3].eq_ignore_ascii_case("LISTENING");
        (listening && cols[1].ends_with(&suffix))
            .then(|| cols[4].parse().ok())
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn detects_a_busy_port() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(!is_free(port));
        assert_eq!(probe(port), PortState::InUse);
        drop(l);
        assert!(is_free(port));
    }

    #[test]
    fn an_unused_port_is_free_for_a_strict_bind() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        // a listener that never accepted a connection leaves nothing behind
        assert_eq!(probe(port), PortState::Free);
    }

    /// The case behind "port 7654 is already in use": the server side closes first, so the port
    /// has `TIME_WAIT` sockets but no listener.
    #[cfg(unix)]
    #[test]
    fn a_port_with_time_wait_sockets_is_lingering_not_in_use() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        let (served, _) = listener.accept().unwrap();
        drop(served); // server closes first -> TIME_WAIT on the server's port
        std::thread::sleep(Duration::from_millis(100));
        drop(client);
        drop(listener);
        assert!(is_free(port), "the reuse bind (Rust std) still succeeds");
        assert_eq!(
            probe(port),
            PortState::Lingering,
            "the strict bind (Python) does not"
        );
    }

    #[tokio::test]
    async fn a_lingering_port_is_waited_for_until_it_is_free() {
        let calls = Cell::new(0);
        let state = wait_until_bindable_with(
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 4 {
                    PortState::Lingering
                } else {
                    PortState::Free
                }
            },
            Waits {
                lingering: Duration::from_secs(5),
                in_use: Duration::ZERO,
                poll: Duration::from_millis(5),
            },
        )
        .await;
        assert_eq!(state, PortState::Free);
        assert_eq!(calls.get(), 4);
    }

    #[tokio::test]
    async fn each_state_has_its_own_wait() {
        let w = Waits {
            lingering: Duration::from_millis(60),
            in_use: Duration::from_millis(20),
            poll: Duration::from_millis(5),
        };
        // reserved range: no waiting at all
        let t = Instant::now();
        assert_eq!(
            wait_until_bindable_with(|| PortState::Denied, w).await,
            PortState::Denied
        );
        assert!(t.elapsed() < Duration::from_millis(20));
        // a real listener: short grace, then the answer
        let t = Instant::now();
        assert_eq!(
            wait_until_bindable_with(|| PortState::InUse, w).await,
            PortState::InUse
        );
        assert!(
            t.elapsed() >= Duration::from_millis(20) && t.elapsed() < Duration::from_millis(500)
        );
        // closing sockets that never expire: gives up after the long wait, still "lingering"
        let t = Instant::now();
        assert_eq!(
            wait_until_bindable_with(|| PortState::Lingering, w).await,
            PortState::Lingering
        );
        assert!(t.elapsed() >= Duration::from_millis(60));
    }

    #[test]
    fn owner_output_is_parsed() {
        assert_eq!(parse_lsof("p4321\n", 7654), Some(4321));
        assert_eq!(parse_lsof("", 7654), None);
        let netstat = "Active Connections\n\n  Proto  Local Address          Foreign Address        State           PID\n  \
                       TCP    0.0.0.0:135            0.0.0.0:0              LISTENING       1012\n  \
                       TCP    127.0.0.1:7654         0.0.0.0:0              LISTENING       4321\n  \
                       TCP    127.0.0.1:7654         127.0.0.1:50000        ESTABLISHED     4321\n";
        assert_eq!(parse_netstat(netstat, 7654), Some(4321));
        assert_eq!(
            parse_netstat(netstat, 765),
            None,
            "a longer port number must not match"
        );
    }
}
