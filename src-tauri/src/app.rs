//! Application state and the commands the shell's own windows call. Everything here runs in the
//! Rust side; the frontend has no file, process or network capability of its own.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rand::RngCore;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::{
    self, ComponentSpec, COMPONENTS, HEALTH_TIMEOUT, HUSK, HUSK_PORT, STUDIO, STUDIO_MCP_PORT,
    STUDIO_PORT,
};
use crate::error::{Result, ShellError};
use crate::install::installer::{exe_path, is_intact, prune_versions, Installer, Phase};
use crate::install::verify::{Sha256Verifier, Verifier};
use crate::logs;
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
        let config = Config::load(&paths.config_file());
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
        })
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

    fn installer(&self) -> Installer<'_> {
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

fn husk_env(state: &AppState, studio_up: bool) -> Vec<(String, String)> {
    let mut env = vec![
        ("HUSK_PORT".into(), HUSK_PORT.to_string()),
        (
            "HUSK_DATA_DIR".into(),
            state.paths.data_dir("husk").display().to_string(),
        ),
        ("HUSK_LAUNCH_SECRET".into(), state.launch_secret.clone()),
        ("HUSK_ENV".into(), "prod".into()),
    ];
    if studio_up {
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

async fn start_component(
    state: &AppState,
    spec: &ComponentSpec,
    version: &str,
    studio_up: bool,
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
        husk_env(state, studio_up)
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

    // Studio is a development aid: its failure never blocks Husk.
    let mut studio_up = false;
    if let Some(st) = cfg
        .installed("studio")
        .filter(|i| is_intact(&state.paths, &STUDIO, &i.version))
    {
        let _ = app.emit("shell://launch", LaunchEvent { step: "studio" });
        match start_component(&state, &STUDIO, &st.version, false).await {
            Ok(()) => studio_up = true,
            Err(e) => {
                tracing::warn!(error = %e, "Husk Studio did not start; continuing without it")
            }
        }
    }

    let _ = app.emit("shell://launch", LaunchEvent { step: "husk" });
    let mut current = husk;
    let mut result = start_component(&state, &HUSK, &current.version, studio_up).await;
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
                result = start_component(&state, &HUSK, &current.version, studio_up).await;
            }
        }
    }
    if let Err(e) = result {
        state.supervisor.stop(&state.paths, "studio").await;
        return Err(e);
    }

    // Started once: the update is confirmed; keep this and the previous version, drop the rest.
    state.update_config(|c| {
        if let Some(i) = c.components.get_mut("husk") {
            i.pending_confirm = false;
        }
    })?;
    prune_versions(
        &state.paths,
        "husk",
        &[
            &current.version,
            current.previous_version.as_deref().unwrap_or(""),
        ],
    );

    let _ = app.emit("shell://launch", LaunchEvent { step: "opening" });
    windows::show_husk(&app)?;
    spawn_update_check(app.clone(), state.clone());
    Ok(())
}

fn spawn_update_check(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        let found = refresh_updates(&state).await;
        windows::set_update_badge(&app, found);
    });
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
    let Some(st) = cfg
        .installed("studio")
        .filter(|i| is_intact(&state.paths, &STUDIO, &i.version))
        .cloned()
    else {
        // not installed: send the user to the place where it can be added
        return windows::show_manager(app);
    };
    if !state.supervisor.is_running("studio").await {
        let _g = state.op_lock.lock().await;
        start_component(state, &STUDIO, &st.version, false).await?;
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
