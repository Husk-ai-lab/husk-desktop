import { useCallback, useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { api, onGoto, toShellError, type ComponentView, type ShellError, type StateView } from "./api";
import { errorKey, type TFn } from "./i18n";
import { useLang } from "./hooks";
import { useTheme } from "./theme";
import { Logo } from "./Logo";
import { ThemeButton } from "./Layout";
import { FileText, Layers, Refresh, Shield, Sliders } from "./Icons";

type Tab = "components" | "updates" | "logs" | "settings";
const TABS: Tab[] = ["components", "updates", "logs", "settings"];
const isTab = (s: string): s is Tab => (TABS as string[]).includes(s);
const ICONS = { components: Layers, updates: Refresh, logs: FileText, settings: Sliders };

type Run = (key: string, fn: () => Promise<unknown>) => Promise<void>;

export function Manager() {
  const { t, lang, setLang } = useLang();
  const { theme, setTheme, toggle } = useTheme();
  const [tab, setTab] = useState<Tab>(() => {
    const h = location.hash.replace("#", "");
    return isTab(h) ? h : "components";
  });
  const [state, setState] = useState<StateView | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<ShellError | null>(null);
  const [version, setVersion] = useState("");

  const reload = useCallback(async () => setState(await api.getState()), []);

  useEffect(() => {
    reload().catch((e) => setError(toShellError(e)));
    getVersion().then(setVersion).catch(() => {});
    const un = onGoto((tb) => isTab(tb) && setTab(tb));
    return () => void un.then((u) => u());
  }, [reload]);

  // one action at a time; show its error; refresh afterwards
  const run: Run = async (key, fn) => {
    setBusy(key);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(toShellError(e));
    } finally {
      setBusy(null);
      reload().catch(() => {});
    }
  };

  const hasUpdate = !!state?.components.some((c) => c.update_version || c.pending_confirm);

  return (
    <div className="app">
      <aside className="side">
        <Logo size={30} />
        <nav className="nav" role="tablist">
          {TABS.map((x) => {
            const Icon = ICONS[x];
            return (
              <button key={x} role="tab" aria-selected={tab === x} className={tab === x ? "on" : ""} onClick={() => setTab(x)}>
                <Icon />
                {t(`mgr.nav.${x}` as "mgr.nav.logs")}
                {x === "updates" && hasUpdate && <span className="chip">{t("mgr.new")}</span>}
              </button>
            );
          })}
        </nav>
        <div className="grow" />
        <div className="user">
          <div className="avatar">H</div>
          <div><b>Husk Desktop</b><span>{version && `v${version}`}</span></div>
        </div>
      </aside>

      <section className="main">
        <div className="crumb">
          <span>{t("mgr.breadcrumb")}{t(`mgr.nav.${tab}` as "mgr.nav.logs").toLowerCase()}</span>
          <ThemeButton theme={theme} toggle={toggle} t={t} />
        </div>
        <div className="content">
          <div className="inner">
            <p className="eyebrow">{t("mgr.eyebrow")}</p>
            <h1 className="display">{t(`mgr.title.${tab}` as "mgr.title.logs")}</h1>
            {tab !== "settings" && <p className="lead">{t(`mgr.lead.${tab}` as "mgr.lead.logs")}</p>}

            {error && (
              <div className="card alert" role="alert">
                <b>{t(errorKey(error.code))}</b>
                <pre className="selectable">{error.detail}</pre>
              </div>
            )}

            {tab === "components" && state && <Components state={state} busy={busy} run={run} t={t} />}
            {tab === "updates" && state && <Updates state={state} busy={busy} run={run} t={t} />}
            {tab === "logs" && <Logs t={t} />}
            {tab === "settings" && (
              <div className="card">
                <div className="setrow">
                  <div><b>{t("set.appearance")}</b><span className="d">{t("set.appearance.desc")}</span></div>
                  <div className="seg" role="group" aria-label={t("set.appearance")}>
                    <button className={theme === "light" ? "on" : ""} onClick={() => setTheme("light")}>{t("set.light")}</button>
                    <button className={theme === "dark" ? "on" : ""} onClick={() => setTheme("dark")}>{t("set.dark")}</button>
                  </div>
                </div>
                <div className="setrow">
                  <div><b>{t("set.language")}</b><span className="d">{t("set.language.desc")}</span></div>
                  <div className="seg" role="group" aria-label={t("set.language")}>
                    <button className={lang === "it" ? "on" : ""} onClick={() => setLang("it")}>Italiano</button>
                    <button className={lang === "en" ? "on" : ""} onClick={() => setLang("en")}>English</button>
                  </div>
                </div>
                <div className="setrow">
                  <div><b>{t("set.privacy")}</b><span className="d">{t("set.privacy.desc")}</span></div>
                  <span className="chip ok" style={{ display: "inline-flex", gap: 6, alignItems: "center" }}>
                    <Shield width={14} height={14} />{t("set.privacy.tag")}
                  </span>
                </div>
              </div>
            )}
          </div>
        </div>
      </section>
    </div>
  );
}

function Tile({ id }: { id: ComponentView["id"] }) {
  return <div className="tile">{id === "husk" ? <img src="/brand/husk-logo-64.png" alt="" /> : <Sliders />}</div>;
}

function statusOf(c: ComponentView, t: TFn) {
  if (!c.installed_version) return { dot: "", text: t("mgr.notInstalled") };
  if (!c.intact) return { dot: "bad", text: t("mgr.broken") };
  return c.running ? { dot: "ok", text: t("mgr.running") } : { dot: "", text: t("mgr.stopped") };
}

function Components({ state, busy, run, t }: { state: StateView; busy: string | null; run: Run; t: TFn }) {
  return (
    <>
      {state.components.map((c) => {
        const s = statusOf(c, t);
        return (
          <div className="card row" key={c.id}>
            <Tile id={c.id} />
            <div className="grow">
              <div className="name">
                {c.name}
                {c.installed_version && <span className="chip">v{c.installed_version}</span>}
              </div>
              <div className="desc">{t(c.id === "husk" ? "mgr.husk.desc" : "mgr.studio.desc")}</div>
              <div className="metaline"><span className={`dot ${s.dot}`} />{s.text}{!c.required && c.installed_version && <> · {t("mgr.removeNote")}</>}</div>
            </div>
            <div className="btns">
              {c.installed_version && !c.intact && (
                <button className="btn small" disabled={!!busy} onClick={() => run(`repair-${c.id}`, () => api.repair(c.id))}>
                  {busy === `repair-${c.id}` ? t("busy") : t("act.repair")}
                </button>
              )}
              {c.id === "studio" && c.installed_version && c.intact && (
                <button className="btn small" onClick={() => run("open-studio", api.openStudio)}>{t("mgr.openStudio")}</button>
              )}
              {c.id === "studio" && !c.installed_version && (
                <button className="btn small primary" disabled={!!busy} onClick={() => run("add-studio", () => api.install(["studio"]))}>
                  {busy === "add-studio" ? t("busy") : t("mgr.add")}
                </button>
              )}
              {!c.required && c.installed_version && (
                <button className="btn small ghost" disabled={!!busy} onClick={() => run(`rm-${c.id}`, () => api.removeComponent(c.id))}>
                  {t("mgr.remove")}
                </button>
              )}
            </div>
          </div>
        );
      })}
    </>
  );
}

function Updates({ state, busy, run, t }: { state: StateView; busy: string | null; run: Run; t: TFn }) {
  const installed = state.components.filter((c) => c.installed_version);
  const anyUpdate = installed.some((c) => c.update_version);
  const anyPending = installed.some((c) => c.pending_confirm);
  return (
    <>
      <div className="toolbar">
        <button className="btn small" disabled={!!busy} onClick={() => run("check", api.checkUpdates)}>
          {busy === "check" ? t("busy") : t("upd.check")}
        </button>
        {anyPending && (
          <button className="btn small primary" disabled={!!busy} onClick={() => run("restart", api.restart)}>{t("upd.restart")}</button>
        )}
        {!anyUpdate && !anyPending && <span className="mono">{t("upd.none")}</span>}
      </div>
      {installed.map((c) => (
        <div className="card row" key={c.id}>
          <Tile id={c.id} />
          <div className="grow">
            <div className="name">{c.name}<span className="chip">v{c.installed_version}</span></div>
            {c.update_version && <div className="metaline">{t("upd.available")} · v{c.update_version}</div>}
            {c.pending_confirm && <div className="metaline">{t("upd.pending")}</div>}
            {c.previous_version && <div className="metaline">{t("upd.previous")} · v{c.previous_version}</div>}
          </div>
          <div className="btns">
            {c.update_version && !c.pending_confirm && (
              <button className="btn small primary" disabled={!!busy} onClick={() => run(`upd-${c.id}`, () => api.applyUpdate(c.id))}>
                {busy === `upd-${c.id}` ? t("busy") : t("upd.download")}
              </button>
            )}
            {c.previous_version && (
              <button className="btn small" disabled={!!busy} onClick={() => run(`rb-${c.id}`, () => api.rollback(c.id))}>
                {t("upd.rollback")}
              </button>
            )}
          </div>
        </div>
      ))}
    </>
  );
}

function Logs({ t }: { t: TFn }) {
  const [name, setName] = useState("husk");
  const [text, setText] = useState("");
  const load = useCallback(() => api.readLog(name).then(setText).catch(() => setText("")), [name]);
  useEffect(() => {
    load();
  }, [load]);
  return (
    <>
      <div className="toolbar">
        <div className="seg" role="group" aria-label="log">
          {["husk", "studio", "shell"].map((n) => (
            <button key={n} className={name === n ? "on" : ""} onClick={() => setName(n)}>{n}.log</button>
          ))}
        </div>
        <button className="btn small" onClick={load}>{t("log.refresh")}</button>
      </div>
      <pre className="log selectable">{text || t("log.empty")}</pre>
    </>
  );
}
