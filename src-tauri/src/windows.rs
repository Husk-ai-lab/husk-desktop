//! The three kinds of window:
//!   * `main`    — first the shell's own UI (setup / starting / errors), then Husk's page
//!   * `manager` — the shell's own UI again: components, updates, logs
//!   * `studio`  — Husk Studio's page, in its own window on its own local endpoint
//!
//! Only `main` and `manager` have IPC (see capabilities/default.json), and `main` loses it the
//! moment it navigates to the backend, because the capability only matches the bundled UI.

use std::sync::OnceLock;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{
    AppHandle, Emitter, Manager, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder, Wry,
};

use crate::app::{locale, open_studio_window, state_of};
use crate::config::{HUSK_PORT, STUDIO_PORT};
use crate::error::{Result, ShellError};
use crate::nav::{classify, is_husk_url, Nav, BACK_TO_HUSK_PATH};

static UPDATE_ITEM: OnceLock<MenuItem<Wry>> = OnceLock::new();

fn werr(e: tauri::Error) -> ShellError {
    ShellError::Other(e.to_string())
}

fn tr(en: &'static str, it: &'static str) -> &'static str {
    if locale() == "it" {
        it
    } else {
        en
    }
}

pub fn create_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let handle = app.clone();
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Husk AI")
        .inner_size(1200.0, 760.0)
        .min_inner_size(900.0, 620.0)
        .center()
        .on_navigation(move |url| match classify(url, cfg!(debug_assertions)) {
            Nav::Allow => true,
            Nav::Studio => {
                let (h, u) = (handle.clone(), url.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = open_studio_at(&h, u).await {
                        tracing::warn!(error = %e, "could not open Studio window");
                    }
                });
                false
            }
            Nav::External => {
                if let Err(e) = open::that(url.as_str()) {
                    tracing::warn!(error = %e, "could not open the system browser");
                }
                false
            }
            Nav::Block => false,
        })
        .build()
}

/// Studio links clicked inside Husk ("Visualizza in Husk Studio") land here.
async fn open_studio_at(app: &AppHandle, url: Url) -> Result<()> {
    let state = state_of(app);
    open_studio_window(app, &state).await?;
    // open_studio_window shows the home page; go to the exact run when we have a deep link
    if url.path() != "/" {
        if let Some(w) = app.get_webview_window("studio") {
            w.navigate(url).map_err(werr)?;
        }
    }
    Ok(())
}

/// Point the main window at Husk's UI (served by the FastAPI backend, same origin for REST).
pub fn show_husk(app: &AppHandle) -> Result<()> {
    let w = app
        .get_webview_window("main")
        .ok_or_else(|| ShellError::Other("main window missing".into()))?;
    let url = Url::parse(&format!("http://127.0.0.1:{HUSK_PORT}/")).expect("static url");
    w.navigate(url).map_err(werr)?;
    w.set_focus().ok();
    Ok(())
}

pub fn show_manager(app: &AppHandle) -> Result<()> {
    show_manager_tab(app, "components")
}

pub fn show_manager_tab(app: &AppHandle, tab: &str) -> Result<()> {
    if let Some(w) = app.get_webview_window("manager") {
        let _ = app.emit_to("manager", "shell://goto", tab);
        w.show().map_err(werr)?;
        w.set_focus().map_err(werr)?;
        return Ok(());
    }
    WebviewWindowBuilder::new(
        app,
        "manager",
        WebviewUrl::App(format!("index.html#{tab}").into()),
    )
    .title(tr("Husk AI — installation", "Husk AI — installazione"))
    .inner_size(1000.0, 680.0)
    .min_inner_size(820.0, 520.0)
    .center()
    .build()
    .map_err(werr)?;
    Ok(())
}

/// Bring the Husk window to the front; with a URL, show that Husk page there too.
pub fn focus_husk(app: &AppHandle, page: Option<Url>) -> Result<()> {
    let w = app
        .get_webview_window("main")
        .ok_or_else(|| ShellError::Other("main window missing".into()))?;
    w.unminimize().map_err(werr)?;
    w.show().map_err(werr)?;
    w.set_focus().map_err(werr)?;
    if let Some(u) = page {
        w.navigate(u).map_err(werr)?;
    }
    Ok(())
}

/// A "back to Husk" pill floating over Studio's page. Studio is a remote page with no IPC, so the
/// button is a plain navigation to `BACK_TO_HUSK_PATH` that the window's navigation handler
/// catches. It lives in a closed shadow root so neither side's CSS can touch the other.
fn back_button_script(label: &str) -> String {
    format!(
        r#"(function () {{
  if (window.top !== window || window.__huskBack) return;
  if (location.hostname !== '127.0.0.1' || location.port !== '{studio_port}') return;
  window.__huskBack = true;
  function add() {{
    if (document.getElementById('husk-shell-back')) return;
    var host = document.createElement('div');
    host.id = 'husk-shell-back';
    host.style.cssText = 'position:fixed;left:16px;bottom:16px;z-index:2147483647';
    var root = host.attachShadow({{ mode: 'closed' }});
    root.innerHTML = '<style>button{{all:unset;cursor:pointer;display:inline-flex;align-items:center;gap:8px;padding:10px 18px;border-radius:999px;font:600 13px/1 -apple-system,BlinkMacSystemFont,"Segoe UI",system-ui,sans-serif;color:#fff;background:#7456f6;box-shadow:0 6px 22px rgba(80,50,200,.4)}}button:hover{{filter:brightness(1.1)}}button:focus-visible{{outline:2px solid #fff;outline-offset:2px}}</style><button type="button">← {label}</button>';
    root.querySelector('button').addEventListener('click', function () {{
      location.href = 'http://127.0.0.1:{husk_port}{back}';
    }});
    document.body.appendChild(host);
  }}
  if (document.body) add(); else document.addEventListener('DOMContentLoaded', add);
}})();"#,
        studio_port = STUDIO_PORT,
        husk_port = HUSK_PORT,
        back = BACK_TO_HUSK_PATH,
        label = label,
    )
}

/// Studio gets its own window on its own endpoint, with no IPC at all.
pub fn show_studio(app: &AppHandle, url: Option<Url>) -> Result<()> {
    let url = url.unwrap_or_else(|| {
        Url::parse(&format!("http://127.0.0.1:{STUDIO_PORT}/")).expect("static url")
    });
    if let Some(w) = app.get_webview_window("studio") {
        w.show().map_err(werr)?;
        w.set_focus().map_err(werr)?;
        return Ok(());
    }
    let handle = app.clone();
    WebviewWindowBuilder::new(app, "studio", WebviewUrl::External(url))
        .title("Husk Studio")
        .inner_size(1280.0, 820.0)
        .min_inner_size(960.0, 640.0)
        .initialization_script(back_button_script(tr("Husk AI", "Husk AI")))
        .on_navigation(move |u| {
            // Husk pages never load inside the Studio window: the back button and any link
            // into Husk bring the Husk window forward (showing that page) instead.
            if is_husk_url(u) {
                let page = (u.path() != BACK_TO_HUSK_PATH).then(|| u.clone());
                let h = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = focus_husk(&h, page) {
                        tracing::warn!(error = %e, "could not bring Husk to the front");
                    }
                });
                return false;
            }
            studio_nav_allowed(u)
        })
        .build()
        .map_err(werr)?;
    Ok(())
}

fn studio_nav_allowed(u: &Url) -> bool {
    match classify(u, false) {
        // inside Studio, its own pages are fine; web links go to the system browser
        Nav::Allow | Nav::Studio => true,
        Nav::External => {
            open::that(u.as_str()).ok();
            false
        }
        Nav::Block => false,
    }
}

pub fn set_update_badge(_app: &AppHandle, count: usize) {
    if let Some(item) = UPDATE_ITEM.get() {
        let text = if count > 0 {
            tr("Update available…", "Aggiornamento disponibile…")
        } else {
            tr("Check for updates…", "Cerca aggiornamenti…")
        };
        let _ = item.set_text(text);
    }
}

pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let back_to_husk = MenuItem::with_id(
        app,
        "back_to_husk",
        tr("Go to Husk AI", "Vai a Husk AI"),
        true,
        Some("CmdOrCtrl+Shift+H"),
    )?;
    let open_studio = MenuItem::with_id(
        app,
        "open_studio",
        tr("Open Husk Studio", "Apri Husk Studio"),
        true,
        None::<&str>,
    )?;
    let manager = MenuItem::with_id(
        app,
        "manager",
        tr("Manage installation…", "Gestisci installazione…"),
        true,
        None::<&str>,
    )?;
    let update = MenuItem::with_id(
        app,
        "update",
        tr("Check for updates…", "Cerca aggiornamenti…"),
        true,
        None::<&str>,
    )?;
    let logs = MenuItem::with_id(
        app,
        "logs",
        tr("View logs…", "Visualizza log…"),
        true,
        None::<&str>,
    )?;
    let _ = UPDATE_ITEM.set(update.clone());
    let sep = PredefinedMenuItem::separator(app)?;

    let mut husk_items: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
        vec![&back_to_husk, &open_studio, &manager, &update, &logs];
    let quit = PredefinedMenuItem::quit(app, Some(tr("Quit Husk AI", "Esci da Husk AI")))?;
    if !cfg!(target_os = "macos") {
        husk_items.push(&sep);
        husk_items.push(&quit);
    }
    let husk_menu = Submenu::with_items(app, "Husk", true, &husk_items)?;

    let undo = PredefinedMenuItem::undo(app, None)?;
    let redo = PredefinedMenuItem::redo(app, None)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let cut = PredefinedMenuItem::cut(app, None)?;
    let copy = PredefinedMenuItem::copy(app, None)?;
    let paste = PredefinedMenuItem::paste(app, None)?;
    let select_all = PredefinedMenuItem::select_all(app, None)?;
    // Without an Edit menu, Cmd+C / Cmd+V do nothing inside macOS webviews.
    let edit_menu = Submenu::with_items(
        app,
        tr("Edit", "Modifica"),
        true,
        &[&undo, &redo, &sep2, &cut, &copy, &paste, &select_all],
    )?;

    #[cfg(target_os = "macos")]
    {
        let about = PredefinedMenuItem::about(app, None, None)?;
        let sep3 = PredefinedMenuItem::separator(app)?;
        let hide = PredefinedMenuItem::hide(app, None)?;
        let hide_others = PredefinedMenuItem::hide_others(app, None)?;
        let show_all = PredefinedMenuItem::show_all(app, None)?;
        let sep4 = PredefinedMenuItem::separator(app)?;
        let app_menu = Submenu::with_items(
            app,
            "Husk AI",
            true,
            &[&about, &sep3, &hide, &hide_others, &show_all, &sep4, &quit],
        )?;
        Menu::with_items(app, &[&app_menu, &husk_menu, &edit_menu])
    }
    #[cfg(not(target_os = "macos"))]
    Menu::with_items(app, &[&husk_menu, &edit_menu])
}

pub fn handle_menu_event(app: &AppHandle, id: &str) {
    match id {
        "back_to_husk" => {
            if let Err(e) = focus_husk(app, None) {
                tracing::warn!(error = %e, "could not bring Husk to the front");
            }
        }
        "open_studio" => {
            let h = app.clone();
            tauri::async_runtime::spawn(async move {
                let state = state_of(&h);
                if let Err(e) = open_studio_window(&h, &state).await {
                    tracing::warn!(error = %e, "could not open Studio");
                }
            });
        }
        "manager" => {
            let _ = show_manager(app);
        }
        "update" => {
            let _ = show_manager_tab(app, "updates");
        }
        "logs" => {
            let _ = show_manager_tab(app, "logs");
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_button_targets_the_husk_port_from_the_studio_origin_only() {
        let js = back_button_script("Husk AI");
        assert!(js.contains("location.port !== '7654'"));
        assert!(js.contains("http://127.0.0.1:8765/__shell/husk"));
        assert!(js.contains("Husk AI"));
        // the URL the button navigates to is one the handler recognises as "go back"
        let u = Url::parse("http://127.0.0.1:8765/__shell/husk").unwrap();
        assert!(is_husk_url(&u) && u.path() == BACK_TO_HUSK_PATH);
    }

    #[test]
    fn studio_window_keeps_its_own_pages_and_sends_web_links_out() {
        assert!(studio_nav_allowed(
            &Url::parse("http://127.0.0.1:7654/runs/1").unwrap()
        ));
        assert!(!studio_nav_allowed(
            &Url::parse("file:///etc/passwd").unwrap()
        ));
    }
}
