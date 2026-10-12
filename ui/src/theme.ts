import { useEffect, useState } from "react";

export type Theme = "light" | "dark";
const KEY = "husk-desktop-theme";

function system(): Theme {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function stored(): Theme | null {
  try {
    const v = localStorage.getItem(KEY);
    return v === "light" || v === "dark" ? v : null;
  } catch {
    return null;
  }
}

/** Explicit choice wins; otherwise follow the system (the prototype ships both themes). */
export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() => stored() ?? system());
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);
  const set = (t: Theme) => {
    try {
      localStorage.setItem(KEY, t);
    } catch {
      /* ignore */
    }
    setTheme(t);
  };
  return { theme, setTheme: set, toggle: () => set(theme === "dark" ? "light" : "dark") };
}
