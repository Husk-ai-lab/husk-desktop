import "@fontsource/barlow-condensed/500.css";
import "@fontsource/barlow-condensed/600.css";
import "@fontsource/barlow-condensed/700.css";
import "@fontsource/hanken-grotesk/400.css";
import "@fontsource/hanken-grotesk/500.css";
import "@fontsource/hanken-grotesk/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "./styles.css";

import React from "react";
import ReactDOM from "react-dom/client";
import { MainFlow } from "./MainFlow";
import { Manager } from "./Manager";

async function boot() {
  // Outside Tauri (plain browser preview via `npm run dev`) the shell commands are mocked so the
  // screens can be reviewed on their own. This branch is dropped from production builds.
  if (import.meta.env.DEV && !("__TAURI_INTERNALS__" in window)) {
    const { installPreviewMocks } = await import("./preview");
    installPreviewMocks();
  }
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  // The same bundle serves both of the shell's own windows; the window label picks the view.
  const label = getCurrentWindow().label;
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>{label === "manager" ? <Manager /> : <MainFlow />}</React.StrictMode>,
  );
}
boot();
