//! Husk AI desktop shell. It installs the Husk (and optionally Husk Studio) backends from GitHub
//! Releases, starts them on loopback, and shows their web UIs in native webview windows.
//! It never logs in, never talks to Neon/Stripe/the AI provider and never reads documents.

mod app;
mod config;
mod error;
mod install;
mod logs;
mod nav;
mod paths;
mod platform;
mod process;
mod release;
mod state;
mod windows;

use std::sync::Arc;

use tauri::{Manager, RunEvent, WindowEvent};

use crate::app::AppState;
use crate::paths::Paths;

/// `<app-data>` per the plan (`ai.husk.desktop`). Debug builds may point it elsewhere so tests
/// never touch a real installation.
fn app_data_root(app: &tauri::AppHandle) -> tauri::Result<std::path::PathBuf> {
    #[cfg(debug_assertions)]
    if let Ok(p) = std::env::var("HUSK_DESKTOP_APP_DATA") {
        if !p.trim().is_empty() {
            return Ok(std::path::PathBuf::from(p));
        }
    }
    app.path().app_data_dir()
}

pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // a second launch just brings the first one forward
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .setup(|app| {
            let paths = Paths::new(app_data_root(app.handle())?);
            paths.ensure_dirs()?;
            logs::init(&paths);
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Husk AI shell starting");

            let state = Arc::new(AppState::new(paths)?);
            app.manage(state);

            let handle = app.handle().clone();
            app.set_menu(windows::build_menu(&handle)?)?;
            app.on_menu_event(|app, event| windows::handle_menu_event(app, event.id().as_ref()));
            windows::create_main(&handle)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // closing the Husk window quits the app (and, through RunEvent::Exit, the backends)
            if window.label() == "main" {
                if let WindowEvent::Destroyed = event {
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            app::get_state,
            app::check_releases,
            app::install,
            app::launch,
            app::check_updates,
            app::apply_update,
            app::repair,
            app::rollback,
            app::remove_component,
            app::restart,
            app::open_studio,
            app::open_manager,
            app::read_log,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the Husk AI shell");

    app.run(|handle, event| {
        if let RunEvent::Exit = event {
            // stop the backends so none outlives the window
            if let Some(state) = handle.try_state::<Arc<AppState>>() {
                let state = state.inner().clone();
                tauri::async_runtime::block_on(async move {
                    state.supervisor.stop_all(&state.paths).await;
                });
            }
        }
    });
}
