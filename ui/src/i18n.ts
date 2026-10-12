export type Lang = "it" | "en";

const dict = {
  it: {
    "boot": "Un attimo…",
    // first run
    "setup.eyebrow": "Benvenuto",
    "setup.title": "Cosa vuoi installare?",
    "setup.lead": "Husk gira sul tuo computer: i documenti restano qui. Puoi aggiungere o togliere componenti in qualsiasi momento dalle impostazioni.",
    "setup.husk.desc": "L'app principale: trova le attività ripetitive nei tuoi documenti e crea le automazioni.",
    "setup.studio.desc": "Il debugger per chi costruisce agenti: auto-riparazione per Husk e interfaccia manuale.",
    "tag.required": "Obbligatorio",
    "tag.recommended": "Consigliato",
    "avail.unavailable": "Non ancora disponibile",
    "avail.checking": "Controllo le versioni…",
    "avail.download": "Download",
    "setup.total": "Download totale",
    "setup.install": "Installa",
    "setup.retry": "Riprova",
    "setup.privacy": "Elaborazione locale · conservazione dati zero",
    // progress
    "install.eyebrow": "Installazione · passo {n} di {m}",
    "install.title": "Stiamo preparando Husk.",
    "install.lead": "Non chiudere la finestra. Se la connessione cade, riprendo da dove ero.",
    "step.download": "Download di {name}",
    "step.install": "Installazione di {name}",
    "launch.eyebrow": "Avvio · passo {n} di {m}",
    "launch.title": "Stiamo avviando Husk.",
    "launch.lead": "Tutto parte sul tuo computer, solo su questa macchina.",
    "launch.preparing": "Preparazione",
    "launch.studio": "Avvio di Husk Studio",
    "launch.husk": "Avvio di Husk",
    "launch.opening": "Apertura dell'area di lavoro",
    // errors
    "err.eyebrow": "Errore",
    "err.title": "Qualcosa non va.",
    "err.network": "Impossibile raggiungere GitHub. Controlla la connessione e riprova.",
    "err.no_release": "Non c'è ancora una versione scaricabile per questo computer.",
    "err.download_invalid": "Download non valido. Riprova: il file scaricato è stato scartato.",
    "err.checksum_mismatch": "Il file scaricato non corrisponde al checksum pubblicato ed è stato scartato. Riprova.",
    "err.no_checksum": "Questa versione non ha un checksum, quindi non è stata installata.",
    "err.extract_failed": "Non riesco a estrarre i file scaricati.",
    "err.port_busy": "La porta locale è già occupata da un altro programma. Chiudilo e riprova.",
    "err.start_timeout": "Husk non ha risposto in tempo. Guarda i log per i dettagli.",
    "err.start_exited": "Husk si è chiuso durante l'avvio. Guarda i log per i dettagli.",
    "err.not_installed": "Husk non è installato correttamente.",
    "err.io": "Errore di lettura o scrittura sul disco.",
    "err.other": "Errore imprevisto.",
    "act.retry": "Riprova",
    "act.repair": "Ripara installazione",
    "act.logs": "Visualizza log",
    // manager
    "mgr.breadcrumb": "installazione/",
    "mgr.eyebrow": "Installazione",
    "mgr.nav.components": "Componenti",
    "mgr.nav.updates": "Aggiornamenti",
    "mgr.nav.logs": "Log",
    "mgr.nav.settings": "Impostazioni",
    "mgr.new": "Nuovo",
    "mgr.title.components": "I tuoi componenti.",
    "mgr.title.updates": "Sempre aggiornato.",
    "mgr.title.logs": "Cosa sta succedendo.",
    "mgr.title.settings": "Il tuo spazio, le tue regole.",
    "mgr.lead.components": "Husk gira sul tuo computer. Qui vedi cosa è installato e cosa sta girando.",
    "mgr.lead.updates": "Gli aggiornamenti si scaricano in background e si applicano al riavvio. La versione precedente resta per tornare indietro.",
    "mgr.lead.logs": "I log restano sul tuo computer e non contengono documenti.",
    "mgr.notInstalled": "Non installato",
    "mgr.running": "In esecuzione",
    "mgr.stopped": "Fermo",
    "mgr.broken": "File mancanti",
    "mgr.add": "Aggiungi",
    "mgr.remove": "Rimuovi",
    "mgr.openStudio": "Apri Husk Studio",
    "mgr.removeNote": "Rimuove solo il programma. I dati restano sul computer.",
    "mgr.husk.desc": "L'app principale.",
    "mgr.studio.desc": "Il debugger per sviluppatori.",
    "upd.check": "Cerca aggiornamenti",
    "upd.none": "Tutto aggiornato.",
    "upd.available": "Aggiornamento disponibile",
    "upd.download": "Scarica",
    "upd.restart": "Riavvia per applicare",
    "upd.pending": "Pronto: si applica al riavvio.",
    "upd.rollback": "Torna indietro",
    "upd.previous": "Versione precedente",
    "log.refresh": "Aggiorna",
    "log.empty": "Nessun log ancora.",
    "set.appearance": "Aspetto",
    "set.appearance.desc": "Tema chiaro o scuro per le finestre di installazione.",
    "set.light": "Chiaro",
    "set.dark": "Scuro",
    "set.language": "Lingua",
    "set.language.desc": "Lingua di queste finestre. Husk ha la sua lingua nelle sue impostazioni.",
    "set.privacy": "Privacy e dati",
    "set.privacy.desc": "Documenti e tracce restano sul dispositivo. Questa app non li legge mai.",
    "set.privacy.tag": "Documenti in locale",
    "busy": "Operazione in corso…",
    "theme.toggle": "Cambia tema",
  },
  en: {
    "boot": "One moment…",
    "setup.eyebrow": "Welcome",
    "setup.title": "What do you want to install?",
    "setup.lead": "Husk runs on your computer: your documents stay here. You can add or remove components at any time from settings.",
    "setup.husk.desc": "The main app: finds the repetitive work in your documents and builds the automations.",
    "setup.studio.desc": "The debugger for people building agents: self-healing for Husk and a manual interface.",
    "tag.required": "Required",
    "tag.recommended": "Recommended",
    "avail.unavailable": "Not available yet",
    "avail.checking": "Checking versions…",
    "avail.download": "Download",
    "setup.total": "Total download",
    "setup.install": "Install",
    "setup.retry": "Try again",
    "setup.privacy": "Local processing · zero data retention",
    "install.eyebrow": "Installation · step {n} of {m}",
    "install.title": "We're getting Husk ready.",
    "install.lead": "Keep this window open. If the connection drops, I pick up where I left off.",
    "step.download": "Downloading {name}",
    "step.install": "Installing {name}",
    "launch.eyebrow": "Starting · step {n} of {m}",
    "launch.title": "We're starting Husk.",
    "launch.lead": "Everything starts on your computer, on this machine only.",
    "launch.preparing": "Preparing",
    "launch.studio": "Starting Husk Studio",
    "launch.husk": "Starting Husk",
    "launch.opening": "Opening your workspace",
    "err.eyebrow": "Error",
    "err.title": "Something went wrong.",
    "err.network": "Cannot reach GitHub. Check your connection and try again.",
    "err.no_release": "There is no downloadable version for this computer yet.",
    "err.download_invalid": "Invalid download. Try again: the downloaded file was discarded.",
    "err.checksum_mismatch": "The downloaded file does not match its published checksum and was discarded. Try again.",
    "err.no_checksum": "This release has no checksum, so it was not installed.",
    "err.extract_failed": "Cannot extract the downloaded files.",
    "err.port_busy": "The local port is already used by another program. Close it and try again.",
    "err.start_timeout": "Husk did not answer in time. See the logs for details.",
    "err.start_exited": "Husk closed during startup. See the logs for details.",
    "err.not_installed": "Husk is not installed correctly.",
    "err.io": "Disk read or write error.",
    "err.other": "Unexpected error.",
    "act.retry": "Try again",
    "act.repair": "Repair installation",
    "act.logs": "View logs",
    "mgr.breadcrumb": "installation/",
    "mgr.eyebrow": "Installation",
    "mgr.nav.components": "Components",
    "mgr.nav.updates": "Updates",
    "mgr.nav.logs": "Logs",
    "mgr.nav.settings": "Settings",
    "mgr.new": "New",
    "mgr.title.components": "Your components.",
    "mgr.title.updates": "Always up to date.",
    "mgr.title.logs": "What's going on.",
    "mgr.title.settings": "Your space, your rules.",
    "mgr.lead.components": "Husk runs on your computer. Here you see what is installed and what is running.",
    "mgr.lead.updates": "Updates download in the background and apply on restart. The previous version stays so you can go back.",
    "mgr.lead.logs": "Logs stay on your computer and never contain documents.",
    "mgr.notInstalled": "Not installed",
    "mgr.running": "Running",
    "mgr.stopped": "Stopped",
    "mgr.broken": "Files missing",
    "mgr.add": "Add",
    "mgr.remove": "Remove",
    "mgr.openStudio": "Open Husk Studio",
    "mgr.removeNote": "Removes the program only. Your data stays on this computer.",
    "mgr.husk.desc": "The main app.",
    "mgr.studio.desc": "The developer debugger.",
    "upd.check": "Check for updates",
    "upd.none": "Everything is up to date.",
    "upd.available": "Update available",
    "upd.download": "Download",
    "upd.restart": "Restart to apply",
    "upd.pending": "Ready: applies on restart.",
    "upd.rollback": "Go back",
    "upd.previous": "Previous version",
    "log.refresh": "Refresh",
    "log.empty": "No log yet.",
    "set.appearance": "Appearance",
    "set.appearance.desc": "Light or dark theme for the installation windows.",
    "set.light": "Light",
    "set.dark": "Dark",
    "set.language": "Language",
    "set.language.desc": "Language of these windows. Husk has its own language in its settings.",
    "set.privacy": "Privacy and data",
    "set.privacy.desc": "Documents and traces stay on the device. This app never reads them.",
    "set.privacy.tag": "Documents stay local",
    "busy": "Working…",
    "theme.toggle": "Toggle theme",
  },
} as const;

export type Key = keyof (typeof dict)["en"];
export type TFn = (key: Key, vars?: Record<string, string | number>) => string;

const LANG_KEY = "husk-desktop-lang";

export function detectLang(): Lang {
  try {
    const saved = localStorage.getItem(LANG_KEY);
    if (saved === "it" || saved === "en") return saved;
  } catch {
    /* storage can be unavailable */
  }
  return navigator.language?.toLowerCase().startsWith("it") ? "it" : "en";
}

export function saveLang(lang: Lang) {
  try {
    localStorage.setItem(LANG_KEY, lang);
  } catch {
    /* ignore */
  }
}

export function makeT(lang: Lang): TFn {
  return (key, vars) => {
    let s: string = dict[lang][key] ?? dict.en[key] ?? key;
    for (const [k, v] of Object.entries(vars ?? {})) s = s.replace(`{${k}}`, String(v));
    return s;
  };
}

export function errorKey(code: string): Key {
  const k = `err.${code}` as Key;
  return k in dict.en ? k : "err.other";
}

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}
