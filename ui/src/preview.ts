// Dev-only: fake Tauri backend so the screens render in a normal browser.
//   /?view=setup | install | launch | error | manager   (+ &tab=updates|logs|settings, &lang=en, &theme=dark)
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

export function installPreviewMocks() {
  const q = new URLSearchParams(location.search);
  const view = q.get("view") ?? "setup";
  const label = view === "manager" ? "manager" : "main";
  mockWindows(label);
  if (q.get("theme")) try { localStorage.setItem("husk-desktop-theme", q.get("theme")!); } catch { /* ignore */ }
  if (q.get("lang")) try { localStorage.setItem("husk-desktop-lang", q.get("lang")!); } catch { /* ignore */ }
  if (view === "manager" && q.get("tab")) location.hash = q.get("tab")!;

  const installed = view === "manager" || view === "launch" || view === "error";
  const comp = (id: string, name: string, v: string | null, extra = {}) => ({
    id, name, required: id === "husk", recommended: true, installed_version: v, intact: true,
    previous_version: null, pending_confirm: false, running: !!v, update_version: null, ...extra,
  });

  if (view === "install") {
    // click "Installa" and feed the progress events a real install would send
    setTimeout(() => {
      (document.querySelector(".btn.primary") as HTMLButtonElement | null)?.click();
      setTimeout(() => emit("shell://install", { component: "husk", phase: "downloading", downloaded: 54, total: 100 }), 300);
    }, 800);
  }
  if (view === "launch") setTimeout(() => emit("shell://launch", { step: "husk" }), 400);

  mockIPC((cmd) => {
    switch (cmd) {
      case "get_state":
        return {
          setup_done: installed,
          locale: "it",
          components: installed
            ? [comp("husk", "Husk AI", "1.4.0", view === "manager" ? { update_version: "1.5.0", previous_version: "1.3.2" } : {}), comp("studio", "Husk Studio", null)]
            : [comp("husk", "Husk AI", null, { running: false }), comp("studio", "Husk Studio", null, { running: false })],
        };
      case "check_releases":
        return [
          { id: "husk", version: "1.4.0", size: 98213345, unavailable: false, error: null },
          { id: "studio", version: "2.1.0", size: 64120000, unavailable: false, error: null },
        ];
      case "launch":
        if (view === "error") return new Promise((_, rej) => setTimeout(() => rej({ code: "port_busy", detail: "port 8765 is already in use" }), 200));
        return new Promise(() => {});
      case "install":
        return new Promise(() => {});
      case "read_log":
        return "2026-10-11T22:20:44Z  INFO husk_desktop_lib: Husk AI shell starting version=\"0.1.0\"\n2026-10-11T22:20:45Z  INFO backend spawned component=husk pid=4121 port=8765";
      default:
        return null;
    }
  }, { shouldMockEvents: true });
}
