// -- WP WEBM4 -- The command palette's entries, made as it opens: the other open devices, the rows
// of actions.json the console has a handler for now (the device's or the page's), the device's
// sections, pages, channels and Settings pages, and the page's own (`extra`). Nothing is listed here
// by hand but what the web has besides actions.json (the shortcut list).

// -- WP UFW -- (UXP A8) And found by the words users type for an entry that are not on it: other
// names for a page or a row (`SYNONYMS`, Qt's CommandPalette `synonyms`), each Settings page by the
// words on it (Settings' own search terms), and "Help: …" entries that open the manual where it
// answers (`HELP`, the same as Qt's). With no DSPi open, what gets one (`noDeviceEntries`).

import { SETTINGS_PAGE_DEFS } from "../../screens/settings/pages";
import { ACTIONS, actionDef, type Handlers } from "../menus/actions";
import type { PaletteEntry, SessionActions } from "../menus/board";
import { actionEntry } from "../menus/session-actions";
import type { Session } from "../session";
import { SECTIONS } from "../shell/sections";

/** The project's pages, where the manual is. */
export const DOCS_URL = "https://github.com/Bofr5555/dspi-unified-console/blob/main/docs/";

/** The words users type for an entry, by its id, besides its own. */
export const SYNONYMS: Record<string, readonly string[]> = {
  "page:xover": ["high-pass", "low-pass", "HPF", "LPF", "tweeter", "woofer", "xover", "slope", "Linkwitz-Riley", "Butterworth"],
  "page:eq": ["EQ", "equalizer", "bell", "peaking", "shelf", "filter", "band"],
  "page:delays": ["time alignment", "distance", "level", "trim", "polarity"],
  "page:installer": ["protection", "tweeter floor", "limits", "lock", "PIN"],
  "settings:speakers": ["roles", "tweeter", "woofer", "mid driver", "subwoofer"],
  "settings:global": ["rename", "name", "device name", "latency"],
  "page:history": ["undo", "changes", "history"],
  "device.commit": ["Commit Parameters", "store", "write", "keep"],
  "web.shortcuts": ["keys", "keyboard", "hotkeys", "shortcuts"],
  "section:measure": ["microphone", "mic", "sweep", "measurement", "auto-tune", "trial"],
};

/** The manual's answers: the page and anchor under docs/, and the words that find them. */
export const HELP: readonly { id: string; title: string; path: string; keywords: readonly string[] }[] = [
  { id: "connect", title: "Help: Connect an AI Agent", path: "manual/ai-agents.md", keywords: ["connect", "Claude", "MCP", "agent", "pairing"] },
  {
    id: "questions",
    title: "Help: The Questions an Agent Asks",
    path: "manual/ai-agents.md#4-questions-what-is-asked-before-a-change",
    keywords: ["question", "ask", "allow", "deny", "confirm"],
  },
  { id: "undo", title: "Help: Watching and Undoing an Agent", path: "manual/ai-agents.md#5-watching-and-undoing", keywords: ["undo", "stop", "pause", "revert"] },
  {
    id: "refused",
    title: "Help: Why Was My Measurement Refused?",
    path: "manual/measurement.md#4-the-safety-rules-and-what-they-mean-for-you",
    keywords: ["refused", "blocked", "safety", "measure", "tweeter"],
  },
  { id: "keep", title: "Help: Auto-tune's Trial, Keep or Revert", path: "manual/measurement.md#72-apply-verify-and-keep", keywords: ["keep", "revert", "trial", "countdown", "auto-tune"] },
  { id: "usb", title: "Help: USB Access on Linux", path: "INSTALL.md#usb-access-on-linux", keywords: ["USB", "rule", "udev", "permission", "not found"] },
  { id: "words", title: "Help: The Words the App Uses", path: "manual/words.md", keywords: ["words", "glossary", "meaning", "what is"] },
];

/** The Help entries, each opening its page in a new tab. */
export function helpEntries(open: (url: string) => void): PaletteEntry[] {
  return HELP.map((h) => ({ id: `help:${h.id}`, kind: "action", title: h.title, where: "Help", keywords: h.keywords, keys: "", enabled: true, run: () => open(DOCS_URL + h.path) }));
}

/** With no DSPi open: the ways to get one, each where the page has it (`null`: not on this page). */
export function noDeviceEntries(ways: { openDemo: (() => void) | null; devices: () => void; addHub: (() => void) | null }): PaletteEntry[] {
  const out: PaletteEntry[] = [];
  const entry = (id: string, title: string, keywords: string[], run: () => void): PaletteEntry => ({ id, kind: "action", title, where: "Devices", keywords, keys: "", enabled: true, run });
  if (ways.openDemo) out.push(entry("web.demo", "Open Demo", ["demo", "try", "example"], ways.openDemo));
  out.push(entry("web.usb", "Connect on USB", ["USB", "connect", "plug", "device"], ways.devices));
  if (ways.addHub) out.push(entry("web.addHub", "Add a Hub…", ["hub", "network", "LAN", "remote"], ways.addHub));
  return out;
}

/** `e` with the words users type for it: its synonyms, and a Settings page's terms. */
function withWords(e: PaletteEntry): PaletteEntry {
  const page = e.id.startsWith("settings:") ? SETTINGS_PAGE_DEFS.find((p) => `settings:${p.id}` === e.id) : undefined;
  const more = [...(page?.terms ?? []), ...(SYNONYMS[e.id] ?? [])];
  return more.length ? { ...e, keywords: [...(e.keywords ?? []), ...more] } : e;
}

/** Where a row is: the section of the page it opens (actions.json's `page.<section>/…`), else its Classic menu. */
export function rowWhere(id: string): string {
  const d = actionDef(id);
  const page = d?.places.find((p) => p.startsWith("page."));
  const section = page ? SECTIONS.find((s) => s.id === page.slice(5).split("/")[0]) : undefined;
  return section?.title ?? d?.menu ?? "";
}

export function paletteEntries(args: {
  /** The device on show, and what it can do. */
  current: Session | null;
  session: SessionActions | null;
  /** The device's handlers over the page's. */
  handlers: Handlers;
  /** The other open devices, each with how to show it. */
  others: readonly { session: Session; name: string; where: string; show: () => void }[];
  extra: readonly PaletteEntry[];
}): PaletteEntry[] {
  const out: PaletteEntry[] = [];
  for (const o of args.others)
    out.push({ id: `device:${o.session.id}`, kind: "device", title: `Switch to ${o.name}`, where: o.where, keys: "", enabled: true, run: o.show });
  for (const e of args.session?.entries ?? []) out.push(e);
  for (const d of ACTIONS) {
    const e = actionEntry(d.id, args.handlers[d.id], args.current, rowWhere);
    if (e) out.push(e);
  }
  out.push(...args.extra);
  return out.map(withWords); // -- WP UFW --
}
