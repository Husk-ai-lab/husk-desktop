//! Husk -> shell requests ("Install Husk Studio" in Husk's UI opens the Manager).
//!
//! A page served by a backend gets no IPC, so Husk cannot call the shell. Instead Husk queues a
//! request and the shell **pulls**: it long-polls
//! `GET http://127.0.0.1:8765/internal/shell/requests?after=<id>&wait=25` with the launch secret
//! in `X-Husk-Launch-Secret` and acts on what it finds. Only an allow-listed action, a tab and a
//! component travel: nothing in a response is ever opened or executed, so a compromised page
//! cannot make the shell do more than show its own Manager.
//!
//! Rules this module keeps (they come from the backend, `app/core/shell_bridge.py`):
//! * the first poll of a backend only learns its newest id and acts on nothing (its backlog is
//!   not ours); ids restart from 1 on a new backend, so the cursor is re-learned on a restart;
//! * `after` is the acknowledgement, and the cursor never moves backwards;
//! * a request older than 30 s is not acted on (the user has moved on);
//! * 404 means an older Husk (no endpoint), 403 a wrong secret (a backend not started by the
//!   shell): stop after a few of those instead of spinning;
//! * the secret is never logged.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Deserialize;

use crate::config::HUSK_PORT;

pub const TABS: [&str; 3] = ["components", "updates", "logs"];
pub const COMPONENTS: [&str; 2] = ["husk", "studio"];
/// A request older than this is dropped.
pub const MAX_AGE_S: i64 = 30;

#[derive(Debug, Default, Deserialize)]
pub struct Poll {
    #[serde(default)]
    pub latest: u64,
    #[serde(default)]
    pub requests: Vec<Request>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct Request {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub tab: String,
    #[serde(default)]
    pub component: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

/// What the shell does about a request. Only values from the allow-lists can appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Open {
    pub tab: &'static str,
    pub component: Option<&'static str>,
}

/// Which requests of a poll are new to us.
#[derive(Debug, Default)]
pub struct Cursor {
    last: Option<u64>,
}

impl Cursor {
    pub fn after(&self) -> u64 {
        self.last.unwrap_or(0)
    }

    /// True until the first successful poll of this backend.
    pub fn is_learning(&self) -> bool {
        self.last.is_none()
    }

    /// The backend went away or answered something unexpected: learn it again.
    pub fn reset(&mut self) {
        self.last = None;
    }

    /// Takes a poll and returns the requests to act on.
    pub fn advance(&mut self, poll: Poll) -> Vec<Request> {
        let newest = poll
            .requests
            .iter()
            .map(|r| r.id)
            .max()
            .unwrap_or(0)
            .max(poll.latest);
        match self.last {
            // first poll: learn where the backend is, act on nothing
            None => {
                self.last = Some(newest);
                Vec::new()
            }
            // the newest id went down: a new backend (ids restart at 1); relearn, replay nothing
            Some(seen) if poll.latest < seen => {
                self.last = Some(newest);
                Vec::new()
            }
            Some(seen) => {
                self.last = Some(seen.max(newest));
                poll.requests.into_iter().filter(|r| r.id > seen).collect()
            }
        }
    }
}

/// `None` when the request must be ignored.
pub fn decide(req: &Request, now_epoch: i64) -> Option<Open> {
    if req.action != "open_manager" {
        return None;
    }
    let tab = TABS.iter().find(|t| **t == req.tab)?;
    let component = match req.component.as_deref() {
        None => None,
        Some(c) => Some(COMPONENTS.iter().find(|x| **x == c)?),
    };
    // an unreadable time is treated as stale: the user can simply click again
    let created = parse_iso_utc(&req.created_at)?;
    if now_epoch - created > MAX_AGE_S {
        return None;
    }
    Some(Open {
        tab,
        component: component.copied(),
    })
}

pub fn now_epoch() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Seconds since the Unix epoch of an ISO 8601 / RFC 3339 timestamp as Python's
/// `datetime.isoformat()` writes it: `2026-10-14T15:29:28`, optional `.ffffff`, optional `Z` or
/// `±HH:MM` (no offset means UTC).
pub fn parse_iso_utc(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, rest) = s.split_once(['T', ' '])?;
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
        d.next()?.parse().ok()?,
    );
    if d.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    // time part, then an optional zone
    let (clock, offset_s) = match rest.find(['Z', 'z', '+', '-']) {
        None => (rest, 0),
        Some(i) => {
            let (clock, zone) = rest.split_at(i);
            let off = match zone {
                "Z" | "z" => 0,
                _ => {
                    let sign = if zone.starts_with('-') { -1 } else { 1 };
                    let digits: String = zone[1..].chars().filter(|c| *c != ':').collect();
                    if digits.len() != 4 {
                        return None;
                    }
                    let h: i64 = digits[..2].parse().ok()?;
                    let mi: i64 = digits[2..].parse().ok()?;
                    sign * (h * 3600 + mi * 60)
                }
            };
            (clock, off)
        }
    };
    let clock = clock.split('.').next()?; // fractional seconds are not needed
    let mut c = clock.split(':');
    let (h, mi, sec): (i64, i64, i64) = (
        c.next()?.parse().ok()?,
        c.next()?.parse().ok()?,
        c.next().unwrap_or("0").parse().ok()?,
    );
    if c.next().is_some() || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    Some(days_from_civil(y, m, day) * 86_400 + h * 3600 + mi * 60 + sec - offset_s)
}

/// Days since 1970-01-01 of a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12; // March = 0
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// The `wait` sent with every poll after the first.
    pub wait_s: u32,
    /// Must be above `wait_s`, or a quiet poll turns into an error loop.
    pub request_timeout: Duration,
    pub backoff_start: Duration,
    pub backoff_max: Duration,
    /// An older Husk without the endpoint: look again rarely.
    pub not_found_sleep: Duration,
    /// Consecutive 403 answers after which the loop gives up.
    pub max_forbidden: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            wait_s: 25,
            request_timeout: Duration::from_secs(30),
            backoff_start: Duration::from_secs(2),
            backoff_max: Duration::from_secs(10),
            not_found_sleep: Duration::from_secs(60),
            max_forbidden: 3,
        }
    }
}

/// The poll loop. Returns only when the secret is rejected repeatedly (otherwise it runs until
/// the task is aborted when Husk stops). `on_open` is called once per accepted request.
pub async fn run_loop(
    client: reqwest::Client,
    base_url: String,
    secret: String,
    timing: Timing,
    mut on_open: impl FnMut(Open) + Send,
) {
    let url = format!("{base_url}/internal/shell/requests");
    let mut cursor = Cursor::default();
    let mut backoff = timing.backoff_start;
    let mut forbidden = 0u32;
    loop {
        // the first poll answers at once, so Husk sees a shell attached without waiting 25 s
        let wait = if cursor.is_learning() {
            0
        } else {
            timing.wait_s
        };
        let sent = client
            .get(&url)
            .header("X-Husk-Launch-Secret", &secret)
            .query(&[("after", cursor.after()), ("wait", u64::from(wait))])
            .timeout(timing.request_timeout)
            .send()
            .await;
        let pause = match sent {
            Ok(resp) if resp.status().is_success() => match resp.json::<Poll>().await {
                Ok(poll) => {
                    backoff = timing.backoff_start;
                    forbidden = 0;
                    for req in cursor.advance(poll) {
                        match decide(&req, now_epoch()) {
                            Some(open) => on_open(open),
                            None => tracing::debug!(id = req.id, "shell request ignored"),
                        }
                    }
                    None
                }
                Err(e) => {
                    tracing::warn!(error = %e, "shell requests: unreadable answer");
                    cursor.reset();
                    Some(backoff)
                }
            },
            Ok(resp) if resp.status() == reqwest::StatusCode::NOT_FOUND => {
                // an older Husk: no such endpoint, the UI falls back to the GitHub page itself
                cursor.reset();
                Some(timing.not_found_sleep)
            }
            Ok(resp) if resp.status() == reqwest::StatusCode::FORBIDDEN => {
                forbidden += 1;
                if forbidden >= timing.max_forbidden {
                    tracing::warn!(
                        "shell requests: Husk rejected the launch secret {forbidden} times; \
                         it was probably not started by this shell. Not polling any more."
                    );
                    return;
                }
                Some(backoff)
            }
            Ok(resp) => {
                tracing::debug!(status = %resp.status(), "shell requests: unexpected status");
                cursor.reset();
                Some(backoff)
            }
            Err(e) => {
                tracing::debug!(error = %e.without_url(), "shell requests: Husk not reachable");
                cursor.reset();
                Some(backoff)
            }
        };
        if let Some(d) = pause {
            tokio::time::sleep(d).await;
            if d == backoff {
                backoff = (backoff * 2).min(timing.backoff_max);
            }
        }
    }
}

/// The loop wired to the app: requests open the Manager on the main thread.
pub async fn run(app: tauri::AppHandle, secret: String) {
    // loopback traffic must never go through a user-configured proxy
    let Ok(client) = reqwest::Client::builder().no_proxy().build() else {
        tracing::warn!("shell requests: could not build an HTTP client");
        return;
    };
    run_loop(
        client,
        format!("http://127.0.0.1:{HUSK_PORT}"),
        secret,
        Timing::default(),
        move |open| {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                if let Err(e) =
                    crate::windows::show_manager_focus(&handle, open.tab, open.component)
                {
                    tracing::warn!(error = %e, "could not open the Manager");
                }
            });
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const NOW: i64 = 1_791_991_768; // 2026-10-14T15:29:28Z

    fn req(id: u64) -> Request {
        Request {
            id,
            action: "open_manager".into(),
            tab: "components".into(),
            component: Some("studio".into()),
            created_at: "2026-10-14T15:29:28+00:00".into(),
        }
    }

    fn poll(latest: u64, ids: &[u64]) -> Poll {
        Poll {
            latest,
            requests: ids.iter().map(|i| req(*i)).collect(),
        }
    }

    // ---- timestamp parsing

    #[test]
    fn iso_timestamps_in_the_forms_python_writes() {
        assert_eq!(parse_iso_utc("1970-01-01T00:00:00+00:00"), Some(0));
        assert_eq!(parse_iso_utc("2026-10-14T15:29:28+00:00"), Some(NOW));
        assert_eq!(parse_iso_utc("2026-10-14T15:29:28Z"), Some(NOW));
        assert_eq!(parse_iso_utc("2026-10-14T15:29:28"), Some(NOW));
        assert_eq!(parse_iso_utc("2026-10-14T15:29:28.123456"), Some(NOW));
        assert_eq!(parse_iso_utc("2026-10-14T15:29:28.123456+00:00"), Some(NOW));
        assert_eq!(parse_iso_utc("2026-10-14 15:29:28"), Some(NOW));
        // an offset is subtracted: 17:29:28 at +02:00 is 15:29:28 UTC
        assert_eq!(parse_iso_utc("2026-10-14T17:29:28+02:00"), Some(NOW));
        assert_eq!(parse_iso_utc("2026-10-14T10:29:28-0500"), Some(NOW));
        assert_eq!(parse_iso_utc("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_iso_utc("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        assert_eq!(parse_iso_utc("1999-12-31T23:59:59Z"), Some(946_684_799));
    }

    #[test]
    fn garbage_is_not_a_timestamp() {
        for bad in [
            "",
            "yesterday",
            "2026-13-01T00:00:00Z",
            "2026-10-14",
            "2026-10-14T25:00:00Z",
            "2026-10-14T15:29:28+2",
        ] {
            assert_eq!(parse_iso_utc(bad), None, "{bad:?}");
        }
    }

    // ---- cursor

    #[test]
    fn the_first_poll_learns_and_acts_on_nothing() {
        let mut c = Cursor::default();
        assert!(c.is_learning());
        assert!(c.advance(poll(7, &[5, 6, 7])).is_empty());
        assert!(!c.is_learning());
        assert_eq!(c.after(), 7);
    }

    #[test]
    fn later_polls_return_only_new_requests_and_never_move_back() {
        let mut c = Cursor::default();
        c.advance(poll(3, &[]));
        let got = c.advance(poll(5, &[4, 5]));
        assert_eq!(got.iter().map(|r| r.id).collect::<Vec<_>>(), [4, 5]);
        assert_eq!(c.after(), 5);
        // a stale answer with a lower newest id changes nothing
        assert!(c.advance(poll(5, &[])).is_empty());
        assert_eq!(c.after(), 5);
        // a request the server repeats is not acted on twice
        assert!(c.advance(poll(5, &[5])).is_empty());
    }

    #[test]
    fn a_restarted_backend_is_learned_again_not_replayed() {
        let mut c = Cursor::default();
        c.advance(poll(40, &[]));
        // new backend: ids start from 1 again, and request 2 is already queued
        assert!(c.advance(poll(2, &[1, 2])).is_empty());
        assert_eq!(c.after(), 2);
        assert_eq!(c.advance(poll(3, &[3])).len(), 1);
    }

    // ---- decide

    #[test]
    fn open_manager_maps_to_a_tab_and_component() {
        let open = decide(&req(1), NOW + 1).unwrap();
        assert_eq!((open.tab, open.component), ("components", Some("studio")));
        let mut r = req(2);
        r.tab = "updates".into();
        r.component = None;
        assert_eq!(
            decide(&r, NOW).unwrap(),
            Open {
                tab: "updates",
                component: None
            }
        );
    }

    #[test]
    fn anything_outside_the_allow_lists_is_ignored() {
        let mut r = req(1);
        r.action = "open_url".into();
        assert!(decide(&r, NOW).is_none());
        let mut r = req(1);
        r.tab = "settings".into();
        assert!(decide(&r, NOW).is_none());
        let mut r = req(1);
        r.component = Some("../../etc".into());
        assert!(decide(&r, NOW).is_none());
        let mut r = req(1);
        r.created_at = "not a time".into();
        assert!(decide(&r, NOW).is_none());
    }

    #[test]
    fn a_stale_request_is_skipped_but_a_30_second_old_one_is_not() {
        assert!(decide(&req(1), NOW + MAX_AGE_S).is_some());
        assert!(decide(&req(1), NOW + MAX_AGE_S + 1).is_none());
    }

    #[test]
    fn a_response_with_extra_fields_still_parses() {
        let body = r#"{"latest": 3, "requests": [
            {"id": 3, "action": "open_manager", "tab": "components", "component": "studio",
             "created_at": "2026-10-14T15:29:28.5+00:00", "future_field": {"a": 1}}], "other": true}"#;
        let p: Poll = serde_json::from_str(body).unwrap();
        assert_eq!((p.latest, p.requests.len()), (3, 1));
        assert!(serde_json::from_str::<Poll>("{}").is_ok());
    }

    // ---- the loop against a fake endpoint

    /// A scripted `/internal/shell/requests`: answers come from `script`, in order, and the last
    /// one repeats. Every request's secret header and query are recorded.
    struct Fake {
        url: String,
        seen: Arc<Mutex<Vec<(String, String)>>>, // (query, secret)
        _t: std::thread::JoinHandle<()>,
    }

    fn fake(script: Vec<(u16, String)>) -> Fake {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr().to_ip().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let t = std::thread::spawn(move || {
            for (n, rq) in server.incoming_requests().enumerate() {
                let secret = rq
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("X-Husk-Launch-Secret"))
                    .map(|h| h.value.to_string())
                    .unwrap_or_default();
                let query = rq
                    .url()
                    .split_once('?')
                    .map(|x| x.1.to_string())
                    .unwrap_or_default();
                seen2.lock().unwrap().push((query, secret));
                let (code, body) = script[n.min(script.len() - 1)].clone();
                let resp = tiny_http::Response::from_string(body)
                    .with_status_code(code)
                    .with_header(
                        "Content-Type: application/json"
                            .parse::<tiny_http::Header>()
                            .unwrap(),
                    );
                let _ = rq.respond(resp);
            }
        });
        Fake { url, seen, _t: t }
    }

    fn fast() -> Timing {
        Timing {
            wait_s: 0,
            request_timeout: Duration::from_secs(2),
            backoff_start: Duration::from_millis(5),
            backoff_max: Duration::from_millis(20),
            not_found_sleep: Duration::from_millis(20),
            max_forbidden: 3,
        }
    }

    fn now_iso() -> String {
        // `created_at` of "now", in the form the backend writes
        let t = now_epoch();
        let (days, rem) = (t.div_euclid(86_400), t.rem_euclid(86_400));
        // civil date from days (inverse of days_from_civil)
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}+00:00",
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    }

    fn body(latest: u64, ids: &[u64]) -> String {
        let at = now_iso();
        let items: Vec<String> = ids
            .iter()
            .map(|i| {
                format!(r#"{{"id":{i},"action":"open_manager","tab":"components","component":"studio","created_at":"{at}"}}"#)
            })
            .collect();
        format!(r#"{{"latest":{latest},"requests":[{}]}}"#, items.join(","))
    }

    async fn run_for(f: &Fake, secs_ms: u64) -> Vec<Open> {
        let opened = Arc::new(Mutex::new(Vec::new()));
        let o = opened.clone();
        let h = tokio::spawn(run_loop(
            reqwest::Client::new(),
            f.url.clone(),
            "s3cret".into(),
            fast(),
            move |open| o.lock().unwrap().push(open),
        ));
        tokio::time::sleep(Duration::from_millis(secs_ms)).await;
        h.abort();
        let v = opened.lock().unwrap().clone();
        v
    }

    #[tokio::test]
    async fn learns_first_then_opens_the_manager_once_and_never_again() {
        // poll 1: backend is at 3, a stale backlog item 3 is in it (must be ignored);
        // poll 2: request 4 arrives; every later poll repeats it (must not fire again)
        let f = fake(vec![(200, body(3, &[3])), (200, body(4, &[4]))]);
        let opened = run_for(&f, 400).await;
        assert_eq!(
            opened,
            vec![Open {
                tab: "components",
                component: Some("studio")
            }]
        );

        let seen = f.seen.lock().unwrap().clone();
        assert!(seen.len() >= 3, "kept polling: {}", seen.len());
        assert!(
            seen.iter().all(|(_, s)| s == "s3cret"),
            "secret sent on every poll"
        );
        assert!(
            seen[0].0.contains("after=0") && seen[0].0.contains("wait=0"),
            "{}",
            seen[0].0
        );
        assert!(seen[1].0.contains("after=3"), "{}", seen[1].0);
        assert!(seen[2].0.contains("after=4"), "{}", seen[2].0);
    }

    #[tokio::test]
    async fn a_missing_endpoint_is_polled_rarely_and_opens_nothing() {
        let f = fake(vec![(404, r#"{"detail":"Not Found"}"#.into())]);
        let opened = run_for(&f, 200).await;
        assert!(opened.is_empty());
        // 20 ms sleep between polls in `fast()`: a handful, not a tight loop
        assert!(f.seen.lock().unwrap().len() < 30);
    }

    #[tokio::test]
    async fn a_rejected_secret_stops_the_loop_after_three_tries() {
        let f = fake(vec![(403, r#"{"detail":"forbidden"}"#.into())]);
        let finished = tokio::time::timeout(
            Duration::from_secs(3),
            run_loop(
                reqwest::Client::new(),
                f.url.clone(),
                "wrong".into(),
                fast(),
                |_| {},
            ),
        )
        .await;
        assert!(finished.is_ok(), "the loop must end by itself");
        assert_eq!(f.seen.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_unreachable_backend_is_retried_with_backoff() {
        // nothing listens here
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let h = tokio::spawn(run_loop(
            reqwest::Client::new(),
            format!("http://127.0.0.1:{port}"),
            "x".into(),
            fast(),
            |_| {},
        ));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(!h.is_finished(), "it keeps trying");
        h.abort();
    }
}
