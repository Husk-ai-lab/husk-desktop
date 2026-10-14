//! The start-up update popup: what to say, and what a click means. The wording and the decision
//! of what was found are plain functions (unit-tested); `app.rs` shows the dialogs and runs the
//! update.
//!
//! One popup per start, no "don't ask again": every launch looks again and asks again. A new
//! backend is downloaded in place (applied at the next restart, the previous version is kept for
//! rollback). A new shell cannot replace itself, so its release page is opened instead.

use std::collections::HashMap;

use crate::config::{COMPONENTS, STUDIO_PORT};
use crate::error::ShellError;
use crate::release::{ReleaseInfo, ShellRelease};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendUpdate {
    pub id: &'static str,
    pub name: &'static str,
    pub installed: String,
    pub version: String,
}

/// Everything newer than what is installed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    pub shell: Option<ShellRelease>,
    pub backends: Vec<BackendUpdate>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.shell.is_none() && self.backends.is_empty()
    }
}

/// Installed components that have a newer release, in the shell's own order (Husk, then Studio).
pub fn backends_from(
    installed: impl Fn(&str) -> Option<String>,
    updates: &HashMap<String, ReleaseInfo>,
) -> Vec<BackendUpdate> {
    COMPONENTS
        .iter()
        .filter_map(|spec| {
            let inst = installed(spec.id)?;
            let rel = updates.get(spec.id)?;
            Some(BackendUpdate {
                id: spec.id,
                name: spec.display_name,
                installed: inst,
                version: rel.version.clone(),
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    pub title: String,
    pub body: String,
    pub confirm: String,
    pub cancel: String,
}

fn pick(lang: &str, en: &str, it: &str) -> String {
    if lang == "it" { it } else { en }.to_string()
}

/// The question shown at start-up. `None` when there is nothing to say.
pub fn update_popup(found: &Found, shell_installed: &str, lang: &str) -> Option<Popup> {
    if found.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = Vec::new();
    if let Some(s) = &found.shell {
        lines.push(match lang {
            "it" => format!(
                "Husk AI (app) {} è disponibile (hai la {shell_installed}).",
                s.version
            ),
            _ => format!(
                "Husk AI app {} is available (you have {shell_installed}).",
                s.version
            ),
        });
    }
    for b in &found.backends {
        lines.push(match lang {
            "it" => format!(
                "{} {} è disponibile (hai la {}).",
                b.name, b.version, b.installed
            ),
            _ => format!(
                "{} {} is available (you have {}).",
                b.name, b.version, b.installed
            ),
        });
    }
    let mut body = lines.join("\n");
    body.push_str("\n\n");
    if !found.backends.is_empty() {
        body.push_str(&pick(
            lang,
            "Update now downloads and verifies the new version; it is used the next time \
             Husk AI starts.",
            "Aggiorna ora scarica e verifica la nuova versione; viene usata al prossimo \
             avvio di Husk AI.",
        ));
        if found.shell.is_some() {
            body.push(' ');
            body.push_str(&pick(
                lang,
                "The new app is installed from its download page, which will open.",
                "La nuova app si installa dalla sua pagina di download, che verrà aperta.",
            ));
        }
    } else {
        body.push_str(&pick(
            lang,
            "The new app is installed from its download page.",
            "La nuova app si installa dalla sua pagina di download.",
        ));
    }
    Some(Popup {
        title: pick(lang, "Husk AI update", "Aggiornamento di Husk AI"),
        body,
        confirm: if found.backends.is_empty() {
            pick(lang, "Open download page", "Apri pagina di download")
        } else {
            pick(lang, "Update now", "Aggiorna ora")
        },
        cancel: pick(lang, "Later", "Più tardi"),
    })
}

/// Shown once the backends are downloaded: they take effect after a restart.
pub fn restart_popup(updated: &[BackendUpdate], lang: &str) -> Popup {
    let list = updated
        .iter()
        .map(|b| format!("{} {}", b.name, b.version))
        .collect::<Vec<_>>()
        .join(", ");
    Popup {
        title: pick(lang, "Update ready", "Aggiornamento pronto"),
        body: match lang {
            "it" => format!("Scaricato: {list}.\n\nRiavvia Husk AI per usare la nuova versione."),
            _ => format!("Downloaded: {list}.\n\nRestart Husk AI to use the new version."),
        },
        confirm: pick(lang, "Restart now", "Riavvia ora"),
        cancel: pick(lang, "Later", "Più tardi"),
    }
}

/// An update that could not be downloaded or verified. Nothing was changed.
pub fn failure_text(detail: &str, lang: &str) -> (String, String) {
    (
        pick(lang, "Update failed", "Aggiornamento non riuscito"),
        match lang {
            "it" => format!(
                "Non è stato possibile scaricare l'aggiornamento; la versione attuale resta \
                 invariata.\n\n{detail}"
            ),
            _ => format!(
                "The update could not be downloaded; the current version is unchanged.\n\n{detail}"
            ),
        },
    )
}

/// Why Husk Studio did not start, in words a user can act on, plus the technical detail.
pub fn studio_failed_text(err: &ShellError, lang: &str) -> (String, String) {
    let reason = match (err.code(), lang) {
        ("port_busy", "it") => format!(
            "La porta locale {STUDIO_PORT} è già occupata da un altro programma. Chiudilo e riprova."
        ),
        ("port_busy", _) => format!(
            "The local port {STUDIO_PORT} is already used by another program. Close it and try again."
        ),
        ("port_reserved", "it") => format!(
            "Il sistema riserva la porta {STUDIO_PORT} (spesso Hyper-V, WSL o Docker su Windows). \
             Riavvia il PC o libera l'intervallo, poi riprova."
        ),
        ("port_reserved", _) => format!(
            "The system reserves port {STUDIO_PORT} (often Hyper-V, WSL or Docker on Windows). \
             Restart the PC or free the range, then try again."
        ),
        ("start_timeout", "it") => "Husk Studio non ha risposto in tempo.".to_string(),
        ("start_timeout", _) => "Husk Studio did not answer in time.".to_string(),
        ("start_exited", "it") => "Husk Studio si è chiuso durante l'avvio.".to_string(),
        ("start_exited", _) => "Husk Studio stopped while starting.".to_string(),
        ("not_installed", "it") => {
            "L'installazione di Husk Studio è incompleta: ripristinala dal Gestore.".to_string()
        }
        ("not_installed", _) => {
            "The Husk Studio installation is incomplete: repair it from the Manager.".to_string()
        }
        (_, "it") => "Husk Studio non è stato avviato.".to_string(),
        _ => "Husk Studio could not be started.".to_string(),
    };
    let label = pick(lang, "Details", "Dettagli");
    (
        pick(
            lang,
            "Husk Studio did not start",
            "Husk Studio non è partito",
        ),
        format!("{reason}\n\n{label}: {err}"),
    )
}

/// The same text as a two-button question: try again, or leave Studio off.
pub fn studio_failed_popup(err: &ShellError, lang: &str) -> Popup {
    let (title, body) = studio_failed_text(err, lang);
    Popup {
        title,
        body,
        confirm: pick(lang, "Try again", "Riprova"),
        cancel: pick(lang, "Close", "Chiudi"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::Asset;

    fn shell(v: &str) -> ShellRelease {
        ShellRelease {
            version: v.into(),
            url: format!("https://x/{v}"),
        }
    }

    fn backend(name: &'static str, from: &str, to: &str) -> BackendUpdate {
        BackendUpdate {
            id: "husk",
            name,
            installed: from.into(),
            version: to.into(),
        }
    }

    fn rel(component: &str, version: &str) -> ReleaseInfo {
        ReleaseInfo {
            component: component.into(),
            version: version.into(),
            prerelease: false,
            asset: Asset {
                name: "a".into(),
                size: 1,
                url: "https://x".into(),
                digest: None,
            },
        }
    }

    #[test]
    fn nothing_found_means_no_popup() {
        assert!(update_popup(&Found::default(), "0.1.1", "en").is_none());
    }

    #[test]
    fn backend_update_offers_update_now_and_names_both_versions() {
        let f = Found {
            shell: None,
            backends: vec![backend("Husk AI", "0.1.0", "0.1.2")],
        };
        let p = update_popup(&f, "0.1.1", "en").unwrap();
        assert_eq!(p.confirm, "Update now");
        assert_eq!(p.cancel, "Later");
        assert!(
            p.body
                .contains("Husk AI 0.1.2 is available (you have 0.1.0)."),
            "{}",
            p.body
        );
        assert!(!p.body.contains("download page"), "{}", p.body);
    }

    #[test]
    fn shell_only_offers_the_download_page() {
        let f = Found {
            shell: Some(shell("0.2.0")),
            backends: vec![],
        };
        let p = update_popup(&f, "0.1.1", "en").unwrap();
        assert_eq!(p.confirm, "Open download page");
        assert!(p
            .body
            .contains("Husk AI app 0.2.0 is available (you have 0.1.1)."));
    }

    #[test]
    fn both_are_listed_in_one_popup_and_say_the_page_will_open() {
        let f = Found {
            shell: Some(shell("0.2.0")),
            backends: vec![
                backend("Husk AI", "0.1.0", "0.1.2"),
                backend("Husk Studio", "0.9", "0.10"),
            ],
        };
        let p = update_popup(&f, "0.1.1", "en").unwrap();
        assert_eq!(p.confirm, "Update now");
        for s in [
            "app 0.2.0",
            "Husk AI 0.1.2",
            "Husk Studio 0.10",
            "will open",
        ] {
            assert!(p.body.contains(s), "{s}: {}", p.body);
        }
    }

    #[test]
    fn italian_is_used_for_it_and_english_for_anything_else() {
        let f = Found {
            shell: None,
            backends: vec![backend("Husk AI", "0.1.0", "0.1.2")],
        };
        let it = update_popup(&f, "0.1.1", "it").unwrap();
        assert_eq!(
            (it.confirm.as_str(), it.cancel.as_str()),
            ("Aggiorna ora", "Più tardi")
        );
        assert!(it.body.contains("è disponibile (hai la 0.1.0)"));
        assert_eq!(
            update_popup(&f, "0.1.1", "fr").unwrap().confirm,
            "Update now"
        );
    }

    #[test]
    fn restart_and_failure_texts() {
        let r = restart_popup(&[backend("Husk AI", "0.1.0", "0.1.2")], "en");
        assert_eq!(r.confirm, "Restart now");
        assert!(r.body.contains("Husk AI 0.1.2"));
        let (title, body) = failure_text("checksum mismatch: x", "en");
        assert_eq!(title, "Update failed");
        assert!(body.contains("unchanged") && body.contains("checksum mismatch: x"));
    }

    #[test]
    fn backends_come_from_installed_components_with_a_newer_release_only() {
        let mut updates = HashMap::new();
        updates.insert("husk".to_string(), rel("husk", "0.1.2"));
        updates.insert("studio".to_string(), rel("studio", "0.10"));
        // Studio is not installed: its release is not offered
        let only_husk = backends_from(|id| (id == "husk").then(|| "0.1.0".to_string()), &updates);
        assert_eq!(only_husk.len(), 1);
        assert_eq!(
            (only_husk[0].id, only_husk[0].installed.as_str()),
            ("husk", "0.1.0")
        );
        // both installed: the shell's own order, Husk first
        let both = backends_from(|_| Some("0.0.1".to_string()), &updates);
        assert_eq!(
            both.iter().map(|b| b.id).collect::<Vec<_>>(),
            ["husk", "studio"]
        );
        assert!(backends_from(|_| Some("1".into()), &HashMap::new()).is_empty());
    }
    #[test]
    fn studio_failure_text_names_the_port_the_owner_and_the_way_out() {
        let busy = ShellError::PortBusy(7654, Some("python3 (pid 4321)".into()));
        let p = studio_failed_popup(&busy, "en");
        assert_eq!(
            (p.confirm.as_str(), p.cancel.as_str()),
            ("Try again", "Close")
        );
        assert!(
            p.body
                .contains("port 7654 is already used by another program"),
            "{}",
            p.body
        );
        assert!(p.body.contains("python3 (pid 4321)"), "{}", p.body);

        let reserved = studio_failed_text(&ShellError::PortReserved(7654), "it");
        assert!(
            reserved.1.contains("riserva la porta 7654"),
            "{}",
            reserved.1
        );

        let other = studio_failed_text(&ShellError::Other("x".into()), "en");
        assert!(other.1.starts_with("Husk Studio could not be started."));
    }
}
