// The Manager window can be opened on a tab and pointed at one component's card (Husk's
// "Install Husk Studio" button). A window that does not exist yet is told through its URL hash,
// `#components&focus=studio`, because an event sent right after it is created arrives before the
// page listens and is lost. An already open window gets the `shell://goto` / `shell://focus`
// events instead.

export type Tab = "components" | "updates" | "logs" | "settings";
export const TABS: Tab[] = ["components", "updates", "logs", "settings"];
export const FOCUSABLE = ["husk", "studio"] as const;
export type Focusable = (typeof FOCUSABLE)[number];

export const isTab = (s: string): s is Tab => (TABS as string[]).includes(s);
export const isFocusable = (s: string | null | undefined): s is Focusable =>
  !!s && (FOCUSABLE as readonly string[]).includes(s);

/** `#updates`, `#components&focus=studio`, `components` ... -> a known tab and an optional card. */
export function parseHash(hash: string): { tab: Tab; focus: Focusable | null } {
  const [head = "", ...rest] = hash.replace(/^#/, "").split("&");
  const tab: Tab = isTab(head) ? head : "components";
  let focus: Focusable | null = null;
  for (const part of rest) {
    const [k, v] = part.split("=");
    if (k === "focus" && isFocusable(v)) focus = v;
  }
  return { tab, focus };
}
