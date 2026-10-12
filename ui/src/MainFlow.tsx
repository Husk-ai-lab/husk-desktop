import { useCallback, useEffect, useRef, useState } from "react";
import {
  api, onInstall, onLaunch, toShellError,
  type Availability, type InstallEvent, type LaunchStep, type ShellError,
} from "./api";
import { errorKey, formatBytes } from "./i18n";
import { useLang } from "./hooks";
import { useTheme } from "./theme";
import { Frame } from "./Layout";
import { ProgressScreen, type StepState } from "./ProgressScreen";
import { Check, GearsOutline, Shield } from "./Icons";
import { Illustration } from "./Illustration";

type View =
  | { name: "boot" }
  | { name: "setup" }
  | { name: "install" }
  | { name: "launch" }
  | { name: "error"; error: ShellError; repairable: boolean };

type Id = "husk" | "studio";
const IDS: Id[] = ["husk", "studio"];
const NAMES: Record<Id, string> = { husk: "Husk AI", studio: "Husk Studio" };

function stepStates(n: number, current: number): StepState[] {
  return Array.from({ length: n }, (_, i) => (i < current ? "done" : i === current ? "now" : "todo"));
}

export function MainFlow() {
  const { t } = useLang();
  const { theme, toggle } = useTheme();
  const [view, setView] = useState<View>({ name: "boot" });
  const [avail, setAvail] = useState<Availability[] | null>(null);
  const [picked, setPicked] = useState<Record<Id, boolean>>({ husk: true, studio: true });
  const [last, setLast] = useState<InstallEvent | null>(null);
  const [plan, setPlan] = useState<Id[]>(["husk"]);
  const [launchStep, setLaunchStep] = useState<LaunchStep>("preparing");
  const [withStudio, setWithStudio] = useState(false);
  const started = useRef(false);

  const fail = useCallback((e: unknown, repairable = false) => {
    setView({ name: "error", error: toShellError(e), repairable });
  }, []);

  const launch = useCallback(async () => {
    setLaunchStep("preparing");
    try {
      const s = await api.getState();
      const st = s.components.find((c) => c.id === "studio");
      setWithStudio(!!st?.installed_version && st.intact);
    } catch {
      /* the launch itself reports real problems */
    }
    setView({ name: "launch" });
    try {
      await api.launch(); // on success this window navigates to Husk and this page goes away
    } catch (e) {
      const err = toShellError(e);
      fail(err, ["not_installed", "start_exited", "start_timeout"].includes(err.code));
    }
  }, [fail]);

  const loadAvailability = useCallback(async () => {
    setAvail(null);
    try {
      const list = await api.checkReleases();
      setAvail(list);
      // Studio is recommended (pre-checked) but only when a release exists
      setPicked({ husk: true, studio: !!list.find((a) => a.id === "studio" && !a.unavailable && !a.error) });
    } catch (e) {
      fail(e);
    }
  }, [fail]);

  // first screen: straight to launch when installed, otherwise "what to install"
  useEffect(() => {
    if (started.current) return;
    started.current = true;
    (async () => {
      try {
        const s = await api.getState();
        const husk = s.components.find((c) => c.id === "husk");
        if (s.setup_done && husk?.installed_version && husk.intact) return launch();
        if (s.setup_done && husk?.installed_version && !husk.intact) {
          return fail({ code: "not_installed", detail: "files missing" }, true);
        }
        setView({ name: "setup" });
        loadAvailability();
      } catch (e) {
        fail(e);
      }
    })();
  }, [launch, loadAvailability, fail]);

  useEffect(() => {
    const a = onInstall(setLast);
    const b = onLaunch(setLaunchStep);
    return () => {
      a.then((u) => u());
      b.then((u) => u());
    };
  }, []);

  const doInstall = async () => {
    const chosen = IDS.filter((id) => picked[id]);
    setPlan(chosen);
    setLast(null);
    setView({ name: "install" });
    try {
      await api.install(chosen);
      await launch();
    } catch (e) {
      fail(e);
    }
  };

  const doRepair = async () => {
    setPlan(["husk"]);
    setLast(null);
    setView({ name: "install" });
    try {
      await api.repair("husk");
      await launch();
    } catch (e) {
      fail(e);
    }
  };

  const frame = (children: React.ReactNode, align: "center" | "left" = "center") => (
    <Frame theme={theme} toggle={toggle} t={t} align={align}>{children}</Frame>
  );

  if (view.name === "boot") return frame(<p className="lead">{t("boot")}</p>);

  if (view.name === "error") {
    const { error, repairable } = view;
    return frame(
      <div className="split">
        <div className="copy">
          <p className="eyebrow">{t("err.eyebrow")}</p>
          <h1 className="display">{t("err.title")}</h1>
          <div className="problem" role="alert">
            <h3>{t(errorKey(error.code))}</h3>
            <pre className="selectable">{error.detail}</pre>
          </div>
          <div className="actions">
            <button className="btn primary" onClick={() => (repairable ? launch() : location.reload())}>{t("act.retry")}</button>
            {repairable && <button className="btn" onClick={doRepair}>{t("act.repair")}</button>}
            <button className="btn ghost" onClick={() => api.openManager("logs")}>{t("act.logs")}</button>
          </div>
        </div>
        <div className="art"><Illustration /></div>
      </div>,
      "left",
    );
  }

  if (view.name === "install") {
    // two steps per component: download, then install (extract + place)
    const labels = plan.flatMap((id) => [
      t("step.download", { name: NAMES[id] }),
      t("step.install", { name: NAMES[id] }),
    ]);
    const idx = last ? plan.indexOf(last.component) * 2 + (last.phase === "downloading" ? 0 : 1) : 0;
    const states = stepStates(labels.length, Math.max(idx, 0));
    const pct = last?.phase === "downloading" && last.total > 0 ? Math.round((last.downloaded / last.total) * 100) : null;
    return frame(
      <ProgressScreen
        eyebrow={t("install.eyebrow", { n: Math.min(idx + 1, labels.length), m: labels.length })}
        title={t("install.title")}
        lead={t("install.lead")}
        headLabel={labels[Math.min(idx, labels.length - 1)]}
        percent={pct}
        steps={labels.map((label, i) => ({ label, state: states[i] }))}
      />,
      "left",
    );
  }

  if (view.name === "launch") {
    const order: LaunchStep[] = withStudio ? ["preparing", "studio", "husk", "opening"] : ["preparing", "husk", "opening"];
    const idx = Math.max(order.indexOf(launchStep), 0);
    const states = stepStates(order.length, idx);
    return frame(
      <ProgressScreen
        eyebrow={t("launch.eyebrow", { n: idx + 1, m: order.length })}
        title={t("launch.title")}
        lead={t("launch.lead")}
        headLabel={t(`launch.${order[idx]}` as "launch.husk")}
        percent={null}
        steps={order.map((s, i) => ({ label: t(`launch.${s}` as "launch.husk"), state: states[i] }))}
      />,
      "left",
    );
  }

  // setup: "Cosa vuoi installare?"
  const card = (id: Id) => {
    const a = avail?.find((x) => x.id === id);
    const unavailable = !!a && (a.unavailable || !!a.error);
    const required = id === "husk";
    const on = required || (!!picked[id] && !unavailable);
    const meta = !a
      ? t("avail.checking")
      : a.unavailable
        ? t("avail.unavailable")
        : a.error
          ? t(errorKey(a.error.code))
          : `v${a.version} · ${t("avail.download")} ${formatBytes(a.size ?? 0)}`;
    return (
      <label key={id} className={`option${on ? " on" : ""}${required ? " locked" : ""}${unavailable && !required ? " off" : ""}`}>
        <input
          type="checkbox"
          checked={on}
          disabled={required || unavailable}
          onChange={(e) => setPicked({ ...picked, [id]: e.target.checked })}
          aria-label={NAMES[id]}
        />
        <span className="tick"><Check width={15} height={15} /></span>
        <div className="art">{id === "husk" ? <img src="/brand/husk-logo-128.png" alt="" /> : <GearsOutline />}</div>
        <h2>{NAMES[id]}</h2>
        <span className={`chip${required ? "" : " ok"}`}>{required ? t("tag.required") : t("tag.recommended")}</span>
        <p>{t(id === "husk" ? "setup.husk.desc" : "setup.studio.desc")}</p>
        <span className="mono meta">{meta}</span>
      </label>
    );
  };

  const huskReady = !!avail?.find((a) => a.id === "husk" && !a.unavailable && !a.error);
  const huskFailed = !!avail?.find((a) => a.id === "husk" && (a.unavailable || a.error));
  const total = (avail ?? []).filter((a) => picked[a.id] && a.size).reduce((n, a) => n + (a.size ?? 0), 0);

  return frame(
    <div className="welcome">
      <p className="eyebrow">{t("setup.eyebrow")}</p>
      <h1 className="display">{t("setup.title")}</h1>
      <p className="lead">{t("setup.lead")}</p>
      <div className="options">{IDS.map(card)}</div>
      <div className="cta">
        <div className="actions" style={{ marginTop: 0 }}>
          <button className="btn primary" disabled={!huskReady} onClick={doInstall}>{t("setup.install")}</button>
          {huskFailed && <button className="btn" onClick={loadAvailability}>{t("setup.retry")}</button>}
        </div>
        {total > 0 && <span className="mono">{t("setup.total")} · {formatBytes(total)}</span>}
      </div>
      <span className="foot-note"><Shield width={15} height={15} />{t("setup.privacy")}</span>
    </div>,
  );
}
