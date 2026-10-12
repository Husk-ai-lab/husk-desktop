import type { ReactNode } from "react";
import { Logo } from "./Logo";
import { Moon, Sun } from "./Icons";
import type { TFn } from "./i18n";
import type { Theme } from "./theme";

export function ThemeButton({ theme, toggle, t }: { theme: Theme; toggle: () => void; t: TFn }) {
  return (
    <button className="icon-btn" onClick={toggle} aria-label={t("theme.toggle")} title={t("theme.toggle")}>
      {theme === "dark" ? <Sun /> : <Moon />}
    </button>
  );
}

/** Full-window screen with the logo top-left, theme toggle top-right and the lavender glow. */
export function Frame(props: { theme: Theme; toggle: () => void; t: TFn; children: ReactNode; align?: "center" | "left" }) {
  return (
    <div className={`frame ${props.align === "left" ? "left" : "center"}`}>
      <header className="topbar">
        <Logo size={30} />
        <ThemeButton theme={props.theme} toggle={props.toggle} t={props.t} />
      </header>
      <main className="stage">{props.children}</main>
      <div className="glow" aria-hidden />
    </div>
  );
}
