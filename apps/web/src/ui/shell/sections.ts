// -- WP WEBM1 -- The Modern shell's sections (MODLAY 2), the same ids and names as the desktop
// app's (Qt's SectionTable), and the screens that are their pages. A section is a place in the
// rail; its pages are its tabs. A screen of `ui/screens.tsx` is listed here once; one that a mode
// does not show has no tab, and a section with no page shown is not in the rail (so Simple hides
// the sections it has nothing in).
//
// How the shell is arranged (`Arrangement`) is data, not layout code: the sections in a rail with
// the pages' tabs beside the title (MODLAY's variant B), or as tabs across the top (A).

import type { KeyValueStore } from "../../fleet/local";
import type { ScreenDef } from "../screens";

export type SectionId =
  | "overview"
  | "equaliser"
  | "routing"
  | "effects"
  | "measure"
  | "analyse"
  | "library"
  | "system"
  | "devices"
  | "settings";

export interface SectionDef {
  id: SectionId;
  title: string;
  /** At the rail's foot (Devices, Settings). */
  foot?: true;
  /** Its screens' ids, in their tabs' order. */
  pages: readonly string[];
}

/** The page with the devices to connect to (the page's own, not a session's screen). */
export const CONNECT = "connect";
export const CONNECT_PAGE: Page = { id: CONNECT, label: "Open a DSPi" };

export const SECTIONS: readonly SectionDef[] = [
  { id: "overview", title: "Overview", pages: ["main", "overview", "sender"] },
  { id: "equaliser", title: "Equaliser", pages: ["eq", "xover", "delays", "inputs"] }, // -- WP WEBM2 -- PEQ | Crossover | Delay & gain
  { id: "routing", title: "Routing", pages: ["matrix"] },
  { id: "effects", title: "Effects", pages: ["effects"] },
  { id: "measure", title: "Measure", pages: ["measure"] },
  { id: "analyse", title: "Analyse", pages: ["analyser", "generator", "stats", "monitor"] },
  { id: "library", title: "Library", pages: ["snapshots", "history", "files"] }, // -- WP WEBM3 -- history
  { id: "system", title: "System", pages: ["system", "events"] },
  { id: "devices", title: "Devices", foot: true, pages: [CONNECT, "hub", "hub-settings", "device"] },
  { id: "settings", title: "Settings", foot: true, pages: ["settings", "surfaces", "installer"] },
];

/** The section a screen is a page of; Settings for one no section lists. */
export function sectionOf(screen: string): SectionId {
  return SECTIONS.find((s) => s.pages.includes(screen))?.id ?? "settings";
}

export interface Page {
  id: string;
  label: string;
}

export interface Section {
  id: SectionId;
  title: string;
  foot: boolean;
  pages: Page[];
}

/** The sections `shown` has pages for, in the rail's order, with the devices' own page in Devices. */
export function sectionsFor(shown: readonly (Page | ScreenDef)[]): Section[] {
  const all: Page[] = [CONNECT_PAGE, ...shown.filter((x) => x.id !== CONNECT)];
  const out: Section[] = [];
  for (const def of SECTIONS) {
    const mine = all.filter((x) => sectionOf(x.id) === def.id);
    if (!mine.length) continue;
    const rank = (x: Page) => {
      const at = def.pages.indexOf(x.id);
      return at < 0 ? def.pages.length : at;
    };
    const pages = mine.map((x) => ({ id: x.id, label: x.label })).sort((a, b) => rank(a) - rank(b));
    out.push({ id: def.id, title: def.title, foot: def.foot === true, pages });
  }
  return out;
}

/** The page a click on `section` opens: the one last open there when it is still shown, else its first. */
export function pageToOpen(section: Section, last: ReadonlyMap<SectionId, string>): string {
  const kept = last.get(section.id);
  return section.pages.some((p) => p.id === kept) ? kept! : section.pages[0]!.id;
}

// ---------------------------------------------------------------- the arrangement

/** The sections in a rail (variant B) or as tabs across the top (variant A); the desktop app's `ui/arrangement/navigation`. */
export type Navigation = "rail" | "tabs";

const ARRANGEMENT_KEY = "dspi-web.arrangement";

export function isNavigation(v: unknown): v is Navigation {
  return v === "rail" || v === "tabs";
}

/** How this viewer has the shell arranged; a rail when nothing (or nothing readable) is kept. */
export function loadNavigation(store: KeyValueStore | null): Navigation {
  try {
    const v: unknown = JSON.parse(store?.getItem(ARRANGEMENT_KEY) ?? "null");
    const n = v && typeof v === "object" ? (v as { navigation?: unknown }).navigation : null;
    return isNavigation(n) ? n : "rail";
  } catch {
    return "rail";
  }
}
