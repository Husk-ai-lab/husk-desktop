//! Application state and the commands the shell's own windows call. Everything here runs in the
//! Rust side; the frontend has no file, process or network capability of its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::RngCore;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use crate::bridge;
use crate::config::{
    self, ComponentSpec, COMPONENTS, HEALTH_TIMEOUT, HUSK, HUSK_PORT, STUDIO, STUDIO_MCP_PORT,
    STUDIO_PORT,
};
use crate::error::{Result, ShellError};
use crate::install::installer::{exe_path, is_intact, prune_versions, Installer, Phase};
use crate::install::verify::{Sha256Verifier, Verifier};
use crate::logs;
use crate::notify;
use crate::paths::Paths;
use crate::process::reaper;
use crate::process::supervisor::{StartRequest, Supervisor};
use crate::release::github::GitHubSource;
use crate::release::{is_newer, ReleaseInfo, ReleaseSource};
use crate::state::{Config, InstalledComponent};
use crate::windows;

pub struct AppState {
    pub paths: Paths,
    pub client: reqwest::Client,
    pub source: Arc<dyn ReleaseSource>,
    pub verifier: Arc<dyn Verifier>,
    pub config: Mutex<Config>,
    pub supervisor: Supervisor,
    /// Per-launch secret handed to the backends as `HUSK_LAUNCH_SECRET`. Memory only.
    launch_secret: String,
    /// Serialises install / update / launch so two of them never race on the same folders.
    pub op_lock: tokio::sync::Mutex<()>,
    /// Newer releases found by the update check, by component id.
    pub updates: Mutex<HashMap<String, ReleaseInfo>>,
    /// The poll loop that answers "Install Husk Studio" requests from Husk (`bridge.rs`). At most
    /// one exists; it belongs to the running Husk backend and is replaced or aborted with it.
    bridge: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
}

impl AppState {
    pub fn new(paths: Paths) -> Result<Self> {
        paths.ensure_dirs()?;
        let client = reqwest::Client::builder()
            .user_agent(config::USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .build()?;
        let source: Arc<dyn ReleaseSource> =
            Arc::new(GitHubSource::new(client.clone(), config::github_api_base()));
        let mut secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        let mut config = Config::load(&paths.config_file());
        if drop_stale_previous(&paths, &mut config) {
            tracing::info!("forgot rollback targets whose folders no longer exist");
            // best effort: in memory it is already right, and the next config write persists it
            let _ = config.save(&paths.config_file());
        }
        Ok(Self {
            paths,
            client,
            source,
            verifier: Arc::new(Sha256Verifier),
            config: Mutex::new(config),
            supervisor: Supervisor::default(),
            launch_secret: secret.iter().map(|b| format!("{b:02x}")).collect(),
            op_lock: tokio::sync::Mutex::new(()),
            updates: Mutex::new(HashMap::new()),
            bridge: Mutex::new(None),
        })
    }

    /// The per-launch secret the backends were given (`HUSK_LAUNCH_SECRET`). Callers must not log it.
    pub fn launch_secret(&self) -> &str {
        &self.launch_secret
    }

    /// Starts the one poll loop for the Husk that just became healthy, replacing any earlier one,
    /// so a new backend (whose request ids restart) always begins with a fresh cursor.
    pub fn start_bridge(&self, app: AppHandle) {
        let mut slot = self.bridge.lock().unwrap();
        if let Some(old) = slot.take() {
            old.abort();
        }
        *slot = Some(tauri::async_runtime::spawn(bridge::run(
            app,
            self.launch_secret().to_string(),
        )));
    }

    pub fn stop_bridge(&self) {
        if let Some(h) = self.bridge.lock().unwrap().take() {
            h.abort();
        }
    }

    fn cfg(&self) -> Config {
        self.config.lock().unwrap().clone()
    }

    /// Mutate the config and write it to disk (atomically) before anyone else sees the change.
    fn update_config<R>(&self, f: impl FnOnce(&mut Config) -> R) -> Result<R> {
        let mut guard = self.config.lock().unwrap();
        let mut next = guard.clone();
        let r = f(&mut next);
        next.save(&self.paths.config_file())?;
        *guard = next;
        Ok(r)
    }

    pub(crate) fn installer(&self) -> Installer<'_> {
        Installer {
            paths: &self.paths,
            client: &self.client,
            verifier: self.verifier.as_ref(),
        }
    }
}

// ---------------------------------------------------------------- views sent to the UI

#[derive(Serialize)]
pub struct ComponentView {
    id: &'static str,
    name: &'static str,
    required: bool,
    recommended: bool,
    installed_version: Option<String>,
    intact: bool,
    previous_version: Option<String>,
    pending_confirm: bool,
    running: bool,
    update_version: Option<String>,
}

#[derive(Serialize)]
pub struct StateView {
    setup_done: bool,
    components: Vec<ComponentView>,
    locale: &'static str,
}

#[derive(Serialize)]
pub struct Availability {
    id: &'static str,
    version: Option<String>,
    size: Option<u64>,
    /// No release (or none for this platform) yet: shown as "non ancora disponibile".
    unavailable: bool,
    /// The release lookup itself failed (offline, rate limited...).
    error: Option<ShellError>,
}

#[derive(Serialize, Clone)]
struct InstallEvent {
    component: &'static str,
    phase: Phase,
    downloaded: u64,
    total: u64,
}

#[derive(Serialize, Clone)]
struct LaunchEvent {
    step: &'static str,
}

pub fn locale() -> &'static str {
    let l = sys_locale::get_locale().unwrap_or_default().to_lowercase();
    if l.starts_with("it") {
        "it"
    } else {
        "en"
    }
}

fn known(id: &str) -> Result<&'static ComponentSpec> {
    config::spec(id).ok_or_else(|| ShellError::Other(format!("unknown component {id:?}")))
}

// ---------------------------------------------------------------- commands

#[tauri::command]
pub async fn get_state(
    state: State<'_, Arc<AppState>>,
) -> std::result::Result<StateView, ShellError> {
    let cfg = state.cfg();
    let updates = state.updates.lock().unwrap().clone();
    let mut components = Vec::new();
    for spec in COMPONENTS.iter() {
        let inst = cfg.installed(spec.id);
        components.push(ComponentView {
            id: spec.id,
            name: spec.display_name,
            required: spec.required,
            recommended: spec.recommended,
            installed_version: inst.map(|i| i.version.clone()),
            intact: inst
                .map(|i| is_intact(&state.paths, spec, &i.version))
                .unwrap_or(false),
            previous_version: inst.and_then(|i| i.previous_version.clone()),
            pending_confirm: inst.map(|i| i.pending_confirm).unwrap_or(false),
            running: state.supervisor.is_running(spec.id).await,
            update_version: updates.get(spec.id).map(|r| r.version.clone()),
        });
    }
    Ok(StateView {
        setup_done: cfg.setup_done,
        components,
        locale: locale(),
    })
}

/// What could be installed right now, with download sizes, for the first-run screen.
#[tauri::command]
pub async fn check_releases(
    state: State<'_, Arc<AppState>>,
) -> std::result::Result<Vec<Availability>, ShellError> {
    let channel = state.cfg().channel;
    let lookups = COMPONENTS.iter().map(|spec| {
        let source = state.source.clone();
        async move { (spec, source.latest(spec, channel).await) }
    });
    let results = futures_util::future::join_all(lookups).await;
    Ok(results
        .into_iter()
        .map(|(spec, r)| match r {
            Ok(Some(rel)) => Availability {
                id: spec.id,
                version: Some(rel.version),
                size: Some(rel.asset.size),
                unavailable: false,
                error: None,
            },
            Ok(None) => Availability {
                id: spec.id,
                version: None,
                size: None,
                unavailable: true,
                error: None,
            },
            Err(e) => Availability {
                id: spec.id,
                version: None,
                size: None,
                unavailable: false,
                error: Some(e),
            },
        })
        .collect())
}

#[tauri::command]
pub async fn install(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    components: Vec<String>,
) -> std::result::Result<(), ShellError> {
    let state = state.inner().clone();
    let _guard = state.op_lock.lock().await;

    // Husk is mandatory whatever the UI sent. A component that is already installed and intact
    // is left alone: "add Studio" must not silently upgrade Husk (upgrades go through
    // apply_update, which keeps the previous version for rollback).
    let cfg = state.cfg();
    let mut specs: Vec<&'static ComponentSpec> = Vec::new();
    for spec in std::iter::once(&HUSK).chain(
        components
            .iter()
            .map(|id| known(id))
            .collect::<Result<Vec<_>>>()?,
    ) {
        let installed = cfg
            .installed(spec.id)
            .map(|i| is_intact(&state.paths, spec, &i.version))
            .unwrap_or(false);
        if !installed && !specs.iter().any(|x| x.id == spec.id) {
            specs.push(spec);
        }
    }

    // Resolve every release first so "nothing to install" fails before any download starts.
    let channel = state.cfg().channel;
    let mut plan = Vec::new();
    for spec in specs {
        match state.source.latest(spec, channel).await? {
            Some(rel) => plan.push((spec, rel)),
            None if spec.required => return Err(ShellError::NoRelease(spec.id.into())),
            None => tracing::info!(component = spec.id, "no release yet, skipped"),
        }
    }

    for (spec, rel) in &plan {
        install_one(&app, &state, spec, rel, false).await?;
    }
    state.update_config(|c| c.setup_done = true)?;
    Ok(())
}

async fn install_one(
    app: &AppHandle,
    state: &Arc<AppState>,
    spec: &'static ComponentSpec,
    rel: &ReleaseInfo,
    is_update: bool,
) -> Result<()> {
    let id = spec.id;
    let mut last_pct = -1i64;
    let app2 = app.clone();
    let mut progress = move |phase: Phase, d: u64, t: u64| {
        let pct = (d * 100).checked_div(t).map_or(-1, |p| p as i64);
        // one event per percent, not per network chunk
        if phase == Phase::Downloading && pct == last_pct {
            return;
        }
        last_pct = pct;
        let _ = app2.emit(
            "shell://install",
            InstallEvent {
                component: id,
                phase,
                downloaded: d,
                total: t,
            },
        );
    };
    state.installer().install(spec, rel, &mut progress).await?;

    // config.json is written last: a crash before this line leaves the component "not installed".
    state.update_config(|c| {
        let old = c.components.get(id).cloned();
        let previous = old
            .as_ref()
            .filter(|o| o.version != rel.version)
            .map(|o| o.version.clone());
        c.components.insert(
            id.to_string(),
            InstalledComponent {
                version: rel.version.clone(),
                previous_version: previous.or_else(|| old.and_then(|o| o.previous_version)),
                pending_confirm: is_update,
            },
        );
    })?;
    state.updates.lock().unwrap().remove(id);
    Ok(())
}

/// `studio_installed`: Studio's URLs are handed to Husk whenever Studio is installed, even if it is
/// still starting. Husk's Studio client probes lazily and degrades to a no-op when Studio is down.
fn husk_env(state: &AppState, studio_installed: bool) -> Vec<(String, String)> {
    let mut env = vec![
        ("HUSK_PORT".into(), HUSK_PORT.to_string()),
        (
            "HUSK_DATA_DIR".into(),
            state.paths.data_dir("husk").display().to_string(),
        ),
        ("HUSK_LAUNCH_SECRET".into(), state.launch_secret.clone()),
        ("HUSK_ENV".into(), "prod".into()),
    ];
    if studio_installed {
        env.push((
            "HUSK_STUDIO_UI_URL".into(),
            format!("http://127.0.0.1:{STUDIO_PORT}"),
        ));
        env.push((
            "HUSK_STUDIO_OTLP_URL".into(),
            format!("http://127.0.0.1:{STUDIO_PORT}"),
        ));
        env.push((
            "HUSK_STUDIO_MCP_URL".into(),
            format!("http://127.0.0.1:{STUDIO_MCP_PORT}/mcp"),
        ));
    }
    env
}

fn studio_env(state: &AppState) -> Vec<(String, String)> {
    let data = state.paths.data_dir("studio").display().to_string();
    vec![
        ("HUSK_PORT".into(), STUDIO_PORT.to_string()),
        ("HUSK_DATA_DIR".into(), data.clone()),
        ("HUSK_HOME".into(), data),
        ("HUSK_LAUNCH_SECRET".into(), state.launch_secret.clone()),
        ("HUSK_ENV".into(), "prod".into()),
        ("HUSK_NO_BROWSER".into(), "1".into()), // the shell shows Studio in its own window
        (
            "STUDIO_TOKEN_URL".into(),
            format!("http://127.0.0.1:{HUSK_PORT}/internal/studio-token"),
        ),
    ]
}

/// Command line of each backend. Husk: on a Linux session without DISPLAY/WAYLAND_DISPLAY it
/// counts as a server and then insists on an explicit `--data-dir` (the env var alone makes it
/// exit 2), so both are passed. Studio is a CLI group (`husk-studio <command>`): started with no
/// command it prints its usage and exits 2, so the shell runs `start` without opening a browser.
fn launch_args(spec: &ComponentSpec, data: &std::path::Path) -> Vec<String> {
    match spec.id {
        "husk" => vec!["--data-dir".into(), data.display().to_string()],
        "studio" => vec![
            "start".into(),
            "--port".into(),
            spec.port.to_string(),
            "--host".into(),
            "127.0.0.1".into(),
            "--no-open-browser".into(),
        ],
        _ => vec![],
    }
}

pub(crate) async fn start_component(
    state: &AppState,
    spec: &ComponentSpec,
    version: &str,
    studio_installed: bool,
) -> Result<()> {
    let exe = exe_path(&state.paths, spec, version);
    // The shell only runs what it installed under bin/.
    let exe = state
        .paths
        .assert_executable_in_bin(&exe)
        .map_err(|_| ShellError::NotInstalled(spec.id.into()))?;
    let data = state.paths.data_dir(spec.id);
    std::fs::create_dir_all(&data)?;
    let env = if spec.id == "husk" {
        husk_env(state, studio_installed)
    } else {
        studio_env(state)
    };
    let args = launch_args(spec, &data);
    state
        .supervisor
        .start(
            &state.paths,
            StartRequest {
                id: spec.id.into(),
                exe,
                args,
                port: spec.port,
                health_path: spec.health_path.into(),
                env,
                log: state.paths.log_file(spec.id),
                timeout: HEALTH_TIMEOUT,
            },
        )
        .await
}

/// Start Studio (if installed) then Husk, and point the main window at Husk.
#[tauri::command]
pub async fn launch(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> std::result::Result<(), ShellError> {
    let state = state.inner().clone();
    let _guard = state.op_lock.lock().await;
    let _ = app.emit("shell://launch", LaunchEvent { step: "preparing" });

    let killed = reaper::reap_stale(&state.paths);
    if killed > 0 {
        tokio::time::sleep(Duration::from_millis(500)).await; // let the OS release the port
    }

    let cfg = state.cfg();
    let husk = cfg
        .installed("husk")
        .filter(|i| is_intact(&state.paths, &HUSK, &i.version))
        .cloned()
        .ok_or_else(|| ShellError::NotInstalled("husk".into()))?;

    // Studio is a development aid: it is started in the background after Husk (below), so a
    // slow or failing Studio never delays or blocks Husk.
    let studio_installed = cfg
        .installed("studio")
        .map(|i| is_intact(&state.paths, &STUDIO, &i.version))
        .unwrap_or(false);

    let _ = app.emit("shell://launch", LaunchEvent { step: "husk" });
    let mut current = husk;
    let mut result = start_component(&state, &HUSK, &current.version, studio_installed).await;
    if result.is_err() && current.pending_confirm {
        // a fresh update that does not start: go back to the version that worked
        if let Some(prev) = current.previous_version.clone() {
            if is_intact(&state.paths, &HUSK, &prev) {
                tracing::warn!(from = %current.version, to = %prev, "update failed to start, rolling back");
                state.update_config(|c| {
                    c.components.insert(
                        "husk".into(),
                        InstalledComponent {
                            version: prev.clone(),
                            previous_version: None,
                            pending_confirm: false,
                        },
                    );
                })?;
                current = InstalledComponent {
                    version: prev,
                    previous_version: None,
                    pending_confirm: false,
                };
                result = start_component(&state, &HUSK, &current.version, studio_installed).await;
            }
        }
    }
    result?;

    // Started once: the update is confirmed, so the version it replaced is deleted.
    confirm_and_prune(&state, "husk", &current.version);

    let _ = app.emit("shell://launch", LaunchEvent { step: "opening" });
    // Husk answered /health: from now on "Install Husk Studio" in its UI opens the Manager
    state.start_bridge(app.clone());
    windows::show_husk(&app)?;
    spawn_update_check(app.clone(), state.clone());
    if studio_installed {
        tauri::async_runtime::spawn(start_studio_in_background(app.clone(), state.clone()));
    }
    Ok(())
}

/// A component that has started successfully once is confirmed: the version it replaced is not
/// needed any more, so `previous_version` is forgotten and its folder deleted (together with
/// any `.tmp` leftovers). Until then the old version is kept for the automatic rollback.
fn confirm_and_prune(state: &AppState, id: &str, version: &str) {
    let saved = state.update_config(|c| {
        if let Some(i) = c.components.get_mut(id) {
            if i.version == version {
                i.pending_confirm = false;
                i.previous_version = None;
            }
        }
    });
    if let Err(e) = saved {
        tracing::warn!(component = id, error = %e, "could not record the confirmed update");
        return;
    }
    prune_versions(&state.paths, id, &[version]);
}

/// `previous_version` entries whose folder is gone (deleted by hand, an earlier prune): a
/// rollback to them cannot work and must not be offered. Returns whether anything changed.
fn drop_stale_previous(paths: &Paths, cfg: &mut Config) -> bool {
    let mut changed = false;
    for (id, inst) in cfg.components.iter_mut() {
        let Some(spec) = config::spec(id) else {
            continue;
        };
        if let Some(prev) = inst.previous_version.clone() {
            if !is_intact(paths, spec, &prev) {
                inst.previous_version = None;
                changed = true;
            }
        }
    }
    changed
}

/// Starts Studio once Husk is up, waiting for its port if the previous run is still closing it.
/// A failure is shown (it used to be only a log line) with a way to try again.
async fn start_studio_in_background(app: AppHandle, state: Arc<AppState>) {
    loop {
        let result = {
            let _guard = state.op_lock.lock().await;
            start_studio_if_needed(&state).await
        };
        let Err(e) = result else {
            return;
        };
        tracing::warn!(error = %e, "Husk Studio did not start");
        let popup = notify::studio_failed_popup(&e, locale());
        if !ask(&app, &popup).await {
            return;
        }
    }
}

/// Starts Studio unless it is running already; the caller holds `op_lock`.
async fn start_studio_if_needed(state: &AppState) -> Result<()> {
    let cfg = state.cfg();
    let Some(st) = cfg
        .installed("studio")
        .filter(|i| is_intact(&state.paths, &STUDIO, &i.version))
        .cloned()
    else {
        return Ok(());
    };
    if state.supervisor.is_running("studio").await {
        return Ok(());
    }
    start_component(state, &STUDIO, &st.version, false).await?;
    confirm_and_prune(state, "studio", &st.version);
    Ok(())
}

/// Every start: look for a newer shell and newer backends, update the menu label, and ask once.
/// Offline or rate limited it only logs: start-up never waits for it and it never shows an error.
fn spawn_update_check(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let (backend_count, shell) =
            tokio::join!(refresh_updates(&state), state.source.latest_shell());
        windows::set_update_badge(&app, backend_count);

        let shell = match shell {
            Ok(Some(r)) if is_newer(&r.version, env!("CARGO_PKG_VERSION")) => Some(r),
            Ok(_) => None,
            Err(e) => {
                tracing::info!(error = %e, "shell update check failed");
                None
            }
        };
        let cfg = state.cfg();
        let backends = notify::backends_from(
            |id| cfg.installed(id).map(|i| i.version.clone()),
            &state.updates.lock().unwrap().clone(),
        );
        let found = notify::Found { shell, backends };
        if found.is_empty() {
            return;
        }
        tracing::info!(
            shell = found.shell.as_ref().map(|s| s.version.as_str()),
            backends = found.backends.len(),
            "update available, asking"
        );
        offer_updates(&app, &state, found).await;
    });
}

/// Shows a two-button native dialog and waits for the answer (`true` = the confirm button).
async fn ask(app: &AppHandle, p: &notify::Popup) -> bool {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .message(&p.body)
        .title(&p.title)
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            p.confirm.clone(),
            p.cancel.clone(),
        ))
        .show(move |confirmed| {
            let _ = tx.send(confirmed);
        });
    rx.await.unwrap_or(false)
}

async fn offer_updates(app: &AppHandle, state: &Arc<AppState>, found: notify::Found) {
    let lang = locale();
    let Some(popup) = notify::update_popup(&found, env!("CARGO_PKG_VERSION"), lang) else {
        return;
    };
    if !ask(app, &popup).await {
        return;
    }

    // a new shell cannot replace itself: its release page has the installers
    if let Some(shell) = &found.shell {
        if let Err(e) = open::that(&shell.url) {
            tracing::warn!(error = %e, "could not open the download page");
        }
    }
    if found.backends.is_empty() {
        return;
    }

    let guard = state.op_lock.lock().await;
    let mut done = Vec::new();
    for b in &found.backends {
        let (Some(spec), Some(rel)) = (
            config::spec(b.id),
            state.updates.lock().unwrap().get(b.id).cloned(),
        ) else {
            continue;
        };
        match install_one(app, state, spec, &rel, true).await {
            Ok(()) => done.push(b.clone()),
            Err(e) => {
                tracing::warn!(component = b.id, error = %e, "start-up update failed");
                let (title, body) = notify::failure_text(&e.to_string(), lang);
                app.dialog()
                    .message(body)
                    .title(title)
                    .kind(MessageDialogKind::Error)
                    .show(|_| {});
                break;
            }
        }
    }
    drop(guard);
    windows::set_update_badge(app, state.updates.lock().unwrap().len());
    if done.is_empty() {
        return;
    }
    if ask(app, &notify::restart_popup(&done, lang)).await {
        state.stop_bridge();
        state.supervisor.stop_all(&state.paths).await;
        app.restart();
    }
}

/// Looks for newer releases of installed components; returns how many were found.
async fn refresh_updates(state: &Arc<AppState>) -> usize {
    let cfg = state.cfg();
    let mut found = HashMap::new();
    for spec in COMPONENTS.iter() {
        let Some(inst) = cfg.installed(spec.id) else {
            continue;
        };
        match state.source.latest(spec, cfg.channel).await {
            Ok(Some(rel)) if is_newer(&rel.version, &inst.version) => {
                found.insert(spec.id.to_string(), rel);
            }
            Ok(_) => {}
            Err(e) => tracing::info!(component = spec.id, error = %e, "update check failed"),
        }
    }
    let n = found.len();
    *state.updates.lock().unwrap() = found;
    n
}

#[tauri::command]
pub async fn check_updates(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> std::result::Result<usize, ShellError> {
    let n = refresh_updates(state.inner()).await;
    windows::set_update_badge(&app, n);
    Ok(n)
}

/// Download a newer version next to the current one. It takes effect at the next restart.
#[tauri::command]
pub async fn apply_update(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    component: String,
) -> std::result::Result<(), ShellError> {
    let state = state.inner().clone();
    let spec = known(&component)?;
    let _guard = state.op_lock.lock().await;
    let rel = state
        .updates
        .lock()
        .unwrap()
        .get(spec.id)
        .cloned()
        .ok_or_else(|| ShellError::NoRelease(spec.id.into()))?;
    install_one(&app, &state, spec, &rel, true).await?;
    windows::set_update_badge(&app, state.updates.lock().unwrap().len());
    Ok(())
}

/// Re-download a component whose files are missing ("Ripara installazione").
#[tauri::command]
pub async fn repair(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    component: String,
) -> std::result::Result<(), ShellError> {
    let state = state.inner().clone();
    let spec = known(&component)?;
    let _guard = state.op_lock.lock().await;
    let channel = state.cfg().channel;
    let rel = state
        .source
        .latest(spec, channel)
        .await?
        .ok_or_else(|| ShellError::NoRelease(spec.id.into()))?;
    // the broken folder of the installed version is removed so the install starts clean
    if let Some(i) = state.cfg().installed(spec.id) {
        if spec.id == "husk" {
            state.stop_bridge(); // its requests belong to the Husk being replaced
        }
        state.supervisor.stop(&state.paths, spec.id).await;
        std::fs::remove_dir_all(state.paths.component_dir(spec.id, &i.version)).ok();
    }
    install_one(&app, &state, spec, &rel, false).await
}

#[tauri::command]
pub async fn rollback(
    state: State<'_, Arc<AppState>>,
    component: String,
) -> std::result::Result<(), ShellError> {
    let state = state.inner().clone();
    let spec = known(&component)?;
    let _guard = state.op_lock.lock().await;
    let cfg = state.cfg();
    let inst = cfg
        .installed(spec.id)
        .ok_or_else(|| ShellError::NotInstalled(spec.id.into()))?;
    let prev = inst
        .previous_version
        .clone()
        .filter(|p| is_intact(&state.paths, spec, p))
        .ok_or_else(|| ShellError::Other("no previous version to go back to".into()))?;
    state.update_config(|c| {
        c.components.insert(
            spec.id.into(),
            InstalledComponent {
                version: prev,
                previous_version: None,
                pending_confirm: false,
            },
        );
    })?;
    Ok(())
}

#[tauri::command]
pub async fn remove_component(
    state: State<'_, Arc<AppState>>,
    component: String,
) -> std::result::Result<(), ShellError> {
    let state = state.inner().clone();
    let spec = known(&component)?;
    if spec.required {
        return Err(ShellError::Other("Husk AI cannot be removed".into()));
    }
    let _guard = state.op_lock.lock().await;
    state.supervisor.stop(&state.paths, spec.id).await;
    // only the program is removed; the user's data folder stays
    std::fs::remove_dir_all(state.paths.component_root(spec.id)).ok();
    state.update_config(|c| {
        c.components.remove(spec.id);
    })?;
    Ok(())
}

/// Stop the backends and relaunch the shell (applies a pending update or rollback).
#[tauri::command]
pub async fn restart(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> std::result::Result<(), ShellError> {
    state.stop_bridge();
    state.supervisor.stop_all(&state.paths).await;
    app.restart()
}

#[tauri::command]
pub async fn open_studio(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> std::result::Result<(), ShellError> {
    open_studio_window(&app, state.inner()).await
}

#[tauri::command]
pub fn open_manager(app: AppHandle, tab: Option<String>) -> std::result::Result<(), ShellError> {
    match tab.as_deref() {
        Some(t @ ("components" | "updates" | "logs")) => windows::show_manager_tab(&app, t),
        _ => windows::show_manager(&app),
    }
}

#[tauri::command]
pub fn read_log(state: State<'_, Arc<AppState>>, name: String) -> String {
    logs::tail(&state.paths, &name, 64 * 1024)
}

/// Menu / command entry point for "Apri Husk Studio".
pub async fn open_studio_window(app: &AppHandle, state: &Arc<AppState>) -> Result<()> {
    let cfg = state.cfg();
    let installed = cfg
        .installed("studio")
        .map(|i| is_intact(&state.paths, &STUDIO, &i.version))
        .unwrap_or(false);
    if !installed {
        // not installed: send the user to the place where it can be added
        return windows::show_manager(app);
    }
    if !state.supervisor.is_running("studio").await {
        // checked again under the lock: the background start from `launch` may be what we waited for
        let result = {
            let _g = state.op_lock.lock().await;
            start_studio_if_needed(state).await
        };
        if let Err(e) = result {
            let (title, body) = notify::studio_failed_text(&e, locale());
            app.dialog()
                .message(body)
                .title(title)
                .kind(MessageDialogKind::Error)
                .show(|_| {});
            return Err(e);
        }
    }
    windows::show_studio(app, None)
}

pub fn state_of(app: &AppHandle) -> Arc<AppState> {
    app.state::<Arc<AppState>>().inner().clone()
}

#[cfg(test)]
mod launch_args_tests {
    use super::*;

    #[test]
    fn studio_gets_its_start_command_and_husk_its_data_dir() {
        let d = std::path::Path::new("/data/husk");
        assert_eq!(launch_args(&HUSK, d), ["--data-dir", "/data/husk"]);
        assert_eq!(
            launch_args(&STUDIO, d),
            [
                "start",
                "--port",
                "7654",
                "--host",
                "127.0.0.1",
                "--no-open-browser"
            ]
        );
    }
}

#[cfg(test)]
mod prune_tests {
    use super::*;

    /// A component folder that passes `is_intact` (executable + `_internal/`).
    fn install(paths: &Paths, spec: &ComponentSpec, version: &str) {
        let d = paths.component_dir(spec.id, version);
        std::fs::create_dir_all(d.join("_internal")).unwrap();
        std::fs::write(d.join(crate::platform::exe_file(spec.exe_name)), b"x").unwrap();
    }

    fn state_with(
        spec: &ComponentSpec,
        current: &str,
        previous: Option<&str>,
        pending: bool,
    ) -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        install(&paths, spec, current);
        if let Some(p) = previous {
            install(&paths, spec, p);
        }
        let mut cfg = Config::default();
        cfg.components.insert(
            spec.id.into(),
            InstalledComponent {
                version: current.into(),
                previous_version: previous.map(String::from),
                pending_confirm: pending,
            },
        );
        cfg.save(&paths.config_file()).unwrap();
        (dir, AppState::new(paths).unwrap())
    }

    fn dirs(state: &AppState, id: &str) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(state.paths.component_root(id))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn a_confirmed_update_deletes_the_version_it_replaced() {
        for spec in [&HUSK, &STUDIO] {
            let (_d, state) = state_with(spec, "1.1.0", Some("1.0.0"), true);
            // an interrupted download of some other version is swept too
            std::fs::create_dir_all(state.paths.component_root(spec.id).join("0.9.0.tmp")).unwrap();
            assert_eq!(dirs(&state, spec.id), ["0.9.0.tmp", "1.0.0", "1.1.0"]);

            confirm_and_prune(&state, spec.id, "1.1.0");

            assert_eq!(dirs(&state, spec.id), ["1.1.0"], "{}", spec.id);
            let c = state.cfg();
            let i = c.installed(spec.id).unwrap();
            assert_eq!(
                (
                    i.version.as_str(),
                    i.previous_version.clone(),
                    i.pending_confirm
                ),
                ("1.1.0", None, false)
            );
            // persisted, not only in memory
            let on_disk = Config::load(&state.paths.config_file());
            assert_eq!(on_disk.installed(spec.id).unwrap().previous_version, None);
        }
    }

    #[test]
    fn nothing_is_deleted_before_the_new_version_has_started() {
        let (_d, state) = state_with(&HUSK, "1.1.0", Some("1.0.0"), true);
        // an update was downloaded: no confirm yet, so the old one is still there for rollback
        assert_eq!(dirs(&state, "husk"), ["1.0.0", "1.1.0"]);
        assert_eq!(
            state
                .cfg()
                .installed("husk")
                .unwrap()
                .previous_version
                .as_deref(),
            Some("1.0.0")
        );
    }

    #[test]
    fn confirming_a_version_that_is_not_the_installed_one_changes_nothing_in_the_config() {
        let (_d, state) = state_with(&HUSK, "1.1.0", Some("1.0.0"), true);
        confirm_and_prune(&state, "husk", "9.9.9");
        assert_eq!(
            state
                .cfg()
                .installed("husk")
                .unwrap()
                .previous_version
                .as_deref(),
            Some("1.0.0")
        );
    }

    #[test]
    fn a_rollback_target_that_no_longer_exists_is_forgotten_on_start() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf());
        paths.ensure_dirs().unwrap();
        install(&paths, &HUSK, "1.1.0"); // 1.0.0 was deleted by hand
        let mut cfg = Config::default();
        cfg.components.insert(
            "husk".into(),
            InstalledComponent {
                version: "1.1.0".into(),
                previous_version: Some("1.0.0".into()),
                pending_confirm: false,
            },
        );
        cfg.save(&paths.config_file()).unwrap();

        let state = AppState::new(paths).unwrap();
        assert_eq!(
            state.cfg().installed("husk").unwrap().previous_version,
            None
        );
        assert_eq!(
            Config::load(&state.paths.config_file())
                .installed("husk")
                .unwrap()
                .previous_version,
            None
        );
    }

    #[test]
    fn a_rollback_target_that_exists_is_kept_on_start() {
        let (_d, state) = state_with(&HUSK, "1.1.0", Some("1.0.0"), true);
        drop(state);
        // a new AppState over the same folder must not drop a valid target
        let dir = _d.path().to_path_buf();
        let again = AppState::new(Paths::new(dir)).unwrap();
        assert_eq!(
            again
                .cfg()
                .installed("husk")
                .unwrap()
                .previous_version
                .as_deref(),
            Some("1.0.0")
        );
    }
}
