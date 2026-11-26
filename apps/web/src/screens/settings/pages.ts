// -- WP WEB7 -- Settings' pages as Qt Modern lists them (SettingsController's registry, Theme's
// settingsTints): four parts, Application / Display / Device / Control, the page's tab beside the
// section's title; each page a row with its tinted badge; a search over every part's titles and
// the words on the pages (Qt's search terms). Core Voltage is no row: Advanced opens it (Qt's
// Hidden group, shown under Application). Only pages this console has: Qt's Updates, Networking,
// AI Agents, This Computer, Inputs, Outputs, I2S Configuration and Control Interfaces have no
// counterpart on the web (and the Installer is its own page of the Settings section).

/** The parts, as Qt's tabs: Application, Display, Device (Qt's System group), Control. */
export type SettingsPart = "application" | "display" | "device" | "control";

export type SettingsPageId =
  | "about"
  | "advanced"
  | "appearance"
  | "graphing"
  | "spectrum"
  | "pins"
  | "global"
  | "speakers"
  | "surfaces"
  | "groups"
  | "macros"
  | "aux"
  | "vreg";

export interface SettingsPageDef {
  id: SettingsPageId;
  title: string;
  part: SettingsPart;
  /** Qt's settingsTint for the page's badge (one value for both schemes: white text at 4.5:1). */
  tint: string;
  /** A Lucide icon's name (icons.tsx). */
  icon: string;
  /** What the search matches besides the title. */
  terms: readonly string[];
  /** Not in the list: another page opens it (Core Voltage, from Advanced). */
  hidden?: true;
}

export const PART_TITLES: Record<SettingsPart, string> = { application: "Application", display: "Display", device: "Device", control: "Control" };
export const PARTS: readonly SettingsPart[] = ["application", "display", "device", "control"];

const DEFAULT_TINT = "#646971";

/** In Qt's order within each part. */
export const SETTINGS_PAGE_DEFS: readonly SettingsPageDef[] = [
  { id: "about", title: "About", part: "application", tint: "#646971", icon: "info", terms: ["Version", "Credits", "Links & Support", "Made By", "Licence", "Help", "GitHub", "Weeb Labs", "Third-Party Notices"] },
  { id: "advanced", title: "Advanced", part: "application", tint: "#727880", icon: "settings-2", terms: ["Channel Names", "Reset all channel names to factory defaults.", "Diagnostics", "Show Debug Information", "Platform", "Firmware", "Serial Number", "Core Voltage"] },
  { id: "appearance", title: "Appearance", part: "application", tint: DEFAULT_TINT, icon: "layout-grid", terms: ["Layout", "Navigation", "Rail", "Tabs", "Theme"] },
  { id: "graphing", title: "Graphing", part: "display", tint: "#587a89", icon: "chart-line", terms: ["Graph Line Glow", "Line Width", "Scale & Grid", "Frequency Grid", "dB Grid", "Readout", "Grid Opacity", "Vertical Range", "Min Frequency", "Max Frequency"] },
  { id: "spectrum", title: "Spectrum Analyser", part: "display", tint: "#4c808a", icon: "audio-waveform", terms: ["Spectrum Strength", "Peak Hold", "Smoothing", "Floor", "Ceiling", "Transform Size", "Averaging", "Peak Decay", "RTA", "FFT"] },
  { id: "pins", title: "Pins", part: "device", tint: "#627080", icon: "cpu", terms: ["GPIO", "Pin", "Output pins", "Move"] },
  { id: "global", title: "Global Parameters", part: "device", tint: "#807701", icon: "hard-drive", terms: ["Device Name", "Latency", "Output latency", "Serial"] },
  { id: "speakers", title: "Speakers", part: "device", tint: DEFAULT_TINT, icon: "volume-2", terms: ["Speaker layout", "Roles", "Tweeter", "Woofer", "Subwoofer", "Way"] },
  { id: "surfaces", title: "Control Surfaces", part: "control", tint: "#8354a0", icon: "sliders-horizontal", terms: ["Push Button", "Toggle Switch", "Potentiometer", "Fader", "Rotary Encoder", "Indicator LED", "IR Remote", "Display", "GPIO"] },
  { id: "groups", title: "Channel Groups", part: "control", tint: "#6c64b7", icon: "group", terms: ["Members", "Channel Type", "zone", "stereo pair"] },
  { id: "macros", title: "Macros", part: "control", tint: "#9e528d", icon: "list-ordered", terms: ["Step", "Wait Before", "sequence"] },
  { id: "aux", title: "Auxiliary Outputs", part: "control", tint: "#7a5aac", icon: "power", terms: ["On/Off Output", "Dimmable Output", "relay", "trigger"] },
  { id: "vreg", title: "Core Voltage", part: "application", tint: DEFAULT_TINT, icon: "gauge", terms: ["Calibrate", "Soak", "Trial", "Hangs"], hidden: true },
];

export function pageDef(id: SettingsPageId): SettingsPageDef {
  return SETTINGS_PAGE_DEFS.find((p) => p.id === id)!;
}

/** A page id from outside (the command palette, an older id): Help is About now; unknown → null. */
export function pageIdOf(v: unknown): SettingsPageId | null {
  if (v === "help") return "about";
  return SETTINGS_PAGE_DEFS.some((p) => p.id === v) ? (v as SettingsPageId) : null;
}

/** Whether `query` finds the page: its title or one of its terms, case ignored (Qt's `matches`). */
export function pageMatches(def: SettingsPageDef, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return def.title.toLowerCase().includes(q) || def.terms.some((t) => t.toLowerCase().includes(q));
}

export interface ListHeading {
  heading: string;
}
export type ListRow = ListHeading | SettingsPageDef;

/**
 * The list beside the page (Qt's `sidebarRows` filtered as SettingsPanel does): with no query the
 * shown part's pages alone; with one, every part's matching pages under its part's heading.
 */
export function listRows(shown: readonly SettingsPageId[], part: SettingsPart, query: string): ListRow[] {
  const visible = SETTINGS_PAGE_DEFS.filter((p) => !p.hidden && shown.includes(p.id));
  if (!query.trim()) return visible.filter((p) => p.part === part);
  const out: ListRow[] = [];
  for (const g of PARTS) {
    const mine = visible.filter((p) => p.part === g && pageMatches(p, query));
    if (mine.length) out.push({ heading: PART_TITLES[g] }, ...mine);
  }
  return out;
}

export function isHeading(r: ListRow): r is ListHeading {
  return (r as ListHeading).heading !== undefined;
}

/** The parts with a page shown, in Qt's order. */
export function partsShown(shown: readonly SettingsPageId[]): SettingsPart[] {
  return PARTS.filter((g) => SETTINGS_PAGE_DEFS.some((p) => p.part === g && !p.hidden && shown.includes(p.id)));
}

/** The page a part opens: the one last shown there when it still is, else its first. */
export function pageOfPart(part: SettingsPart, shown: readonly SettingsPageId[], last: SettingsPageId | undefined): SettingsPageId | null {
  if (last && shown.includes(last) && pageDef(last).part === part) return last;
  return SETTINGS_PAGE_DEFS.find((p) => p.part === part && !p.hidden && shown.includes(p.id))?.id ?? null;
}
