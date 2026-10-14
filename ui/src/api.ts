// Thin typed wrappers over the shell's Rust commands. The UI has no other way to touch the
// machine: no files, no processes, no network of its own.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type ShellError = { code: string; detail: string };

export type ComponentView = {
  id: "husk" | "studio";
  name: string;
  required: boolean;
  recommended: boolean;
  installed_version: string | null;
  intact: boolean;
  previous_version: string | null;
  pending_confirm: boolean;
  running: boolean;
  update_version: string | null;
};

export type StateView = { setup_done: boolean; components: ComponentView[]; locale: "it" | "en" };

export type Availability = {
  id: "husk" | "studio";
  version: string | null;
  size: number | null;
  unavailable: boolean;
  error: ShellError | null;
};

export type InstallEvent = {
  component: "husk" | "studio";
  phase: "downloading" | "extracting" | "finalizing";
  downloaded: number;
  total: number;
};

export type LaunchStep = "preparing" | "studio" | "husk" | "opening";

export const api = {
  getState: () => invoke<StateView>("get_state"),
  checkReleases: () => invoke<Availability[]>("check_releases"),
  install: (components: string[]) => invoke<void>("install", { components }),
  launch: () => invoke<void>("launch"),
  checkUpdates: () => invoke<number>("check_updates"),
  applyUpdate: (component: string) => invoke<void>("apply_update", { component }),
  repair: (component: string) => invoke<void>("repair", { component }),
  rollback: (component: string) => invoke<void>("rollback", { component }),
  removeComponent: (component: string) => invoke<void>("remove_component", { component }),
  restart: () => invoke<void>("restart"),
  openStudio: () => invoke<void>("open_studio"),
  openManager: (tab?: string) => invoke<void>("open_manager", { tab }),
  readLog: (name: string) => invoke<string>("read_log", { name }),
};

export const onInstall = (cb: (e: InstallEvent) => void): Promise<UnlistenFn> =>
  listen<InstallEvent>("shell://install", (e) => cb(e.payload));
export const onLaunch = (cb: (s: LaunchStep) => void): Promise<UnlistenFn> =>
  listen<{ step: LaunchStep }>("shell://launch", (e) => cb(e.payload.step));
export const onGoto = (cb: (tab: string) => void): Promise<UnlistenFn> =>
  listen<string>("shell://goto", (e) => cb(e.payload));
/** The shell asks the Manager to point at one component's card ("Install Husk Studio" in Husk). */
export const onFocus = (cb: (component: string | null) => void): Promise<UnlistenFn> =>
  listen<string | null>("shell://focus", (e) => cb(e.payload));

/** A rejected invoke carries our serialized ShellError; anything else becomes `other`. */
export function toShellError(e: unknown): ShellError {
  if (e && typeof e === "object" && "code" in e) return e as ShellError;
  return { code: "other", detail: String(e) };
}
