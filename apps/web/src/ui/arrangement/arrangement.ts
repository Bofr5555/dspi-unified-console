// -- WP WEBM4 -- How this viewer has the Modern shell arranged (review RB: the arrangement settings,
// as the desktop app's Settings › Appearance, ui/arrangement/*): where the sections are (a rail
// with the pages' tabs, or tabs along the top), where the open devices' tabs are (the top bar, or
// the rail's top), the colours (the system's, light or dark) and whether the inspector shows.
// Kept in this browser only, under WEBM1's key, which held `navigation` alone before; the
// inspector's choice is WEBM2's own key.
// -- WP UB13 -- (UXP B13, D11) And two for those who need them, both off on a mouse desktop as the
// desktop app's Settings › Appearance › Accessibility: "Outline controls" (a line round every
// field, menu and button) and the control size (Automatic: Comfortable on a touch screen, which
// the page knows by `pointer: coarse`). The console carries them as data-outline and
// data-density (access.css).

import type { KeyValueStore } from "../../fleet/local";
import { isNavigation, type Navigation } from "../shell/sections";
import { loadCollapsed, saveCollapsed } from "../inspector/model";

export type DeviceTabsAt = "topBar" | "rail";
export type Theme = "system" | "light" | "dark";
export type Outline = "off" | "on"; // -- WP UB13 --
export type Density = "automatic" | "standard" | "comfortable"; // -- WP UB13 --

export interface Arrangement {
  navigation: Navigation;
  deviceTabs: DeviceTabsAt;
  theme: Theme;
  outline: Outline; // -- WP UB13 --
  density: Density; // -- WP UB13 --
}

export const DEFAULT_ARRANGEMENT: Arrangement = {
  navigation: "rail",
  deviceTabs: "topBar",
  theme: "system",
  outline: "off", // -- WP UB13 --
  density: "automatic", // -- WP UB13 --
};

const KEY = "dspi-web.arrangement";

const isDeviceTabsAt = (v: unknown): v is DeviceTabsAt => v === "topBar" || v === "rail";
const isTheme = (v: unknown): v is Theme => v === "system" || v === "light" || v === "dark";
// -- WP UB13 --
const isOutline = (v: unknown): v is Outline => v === "off" || v === "on";
const isDensity = (v: unknown): v is Density => v === "automatic" || v === "standard" || v === "comfortable";

/** -- WP UB13 -- The Comfortable density: chosen, or Automatic on a touch screen (a coarse pointer). */
export function comfortableFor(density: Density, coarsePointer: boolean): boolean {
  return density === "comfortable" || (density === "automatic" && coarsePointer);
}

/** As kept; the default for what is not (or not readable). */
export function loadArrangement(store: KeyValueStore | null): Arrangement {
  let v: Record<string, unknown> = {};
  try {
    const got: unknown = JSON.parse(store?.getItem(KEY) ?? "null");
    if (got && typeof got === "object") v = got as Record<string, unknown>;
  } catch {
    // nothing readable: the defaults
  }
  return {
    navigation: isNavigation(v.navigation) ? v.navigation : DEFAULT_ARRANGEMENT.navigation,
    deviceTabs: isDeviceTabsAt(v.deviceTabs) ? v.deviceTabs : DEFAULT_ARRANGEMENT.deviceTabs,
    theme: isTheme(v.theme) ? v.theme : DEFAULT_ARRANGEMENT.theme,
    outline: isOutline(v.outline) ? v.outline : DEFAULT_ARRANGEMENT.outline, // -- WP UB13 --
    density: isDensity(v.density) ? v.density : DEFAULT_ARRANGEMENT.density, // -- WP UB13 --
  };
}

export function saveArrangement(store: KeyValueStore | null, a: Arrangement): void {
  try {
    store?.setItem(KEY, JSON.stringify(a));
  } catch {
    // A private window or blocked storage: the choice lasts as long as the page.
  }
}

/** The device tabs are at the rail's top only where there is a rail. */
export function deviceTabsInRail(a: Pick<Arrangement, "navigation" | "deviceTabs">): boolean {
  return a.navigation === "rail" && a.deviceTabs === "rail";
}

/** One of the settings: its words, its choices. The palette finds them by these words too. */
export interface Choice<K extends keyof Arrangement | "inspector"> {
  key: K;
  label: string;
  values: readonly { value: string; text: string }[];
}

export const CHOICES = [
  {
    key: "navigation",
    label: "Sections",
    values: [
      { value: "rail", text: "Rail with page tabs" },
      { value: "tabs", text: "Tabs along the top" },
    ],
  },
  {
    key: "deviceTabs",
    label: "Device tabs",
    values: [
      { value: "topBar", text: "In the top bar" },
      { value: "rail", text: "At the top of the rail" },
    ],
  },
  {
    key: "theme",
    label: "Theme",
    values: [
      { value: "system", text: "System" },
      { value: "light", text: "Light" },
      { value: "dark", text: "Dark" },
    ],
  },
  // -- WP UB13 --
  {
    key: "outline",
    label: "Outline controls",
    values: [
      { value: "off", text: "Off" },
      { value: "on", text: "On" },
    ],
  },
  {
    key: "density",
    label: "Control size",
    values: [
      { value: "automatic", text: "Automatic" },
      { value: "standard", text: "Standard" },
      { value: "comfortable", text: "Comfortable" },
    ],
  },
  {
    key: "inspector",
    label: "Inspector",
    values: [
      { value: "shown", text: "Beside the page" },
      { value: "hidden", text: "Hidden" },
    ],
  },
] as const satisfies readonly Choice<keyof Arrangement | "inspector">[];

/** The inspector's choice, WEBM2's: "hidden" when the viewer hid it. */
export function inspectorShown(store: KeyValueStore | null): "shown" | "hidden" {
  return loadCollapsed(store) ? "hidden" : "shown";
}

export function setInspectorShown(store: KeyValueStore | null, v: "shown" | "hidden"): void {
  saveCollapsed(store, v === "hidden");
}

/** The arrangement with one choice made; null for a value that is not one of its choices. */
export function withChoice(a: Arrangement, key: keyof Arrangement, value: string): Arrangement | null {
  if (key === "navigation" && isNavigation(value)) return { ...a, navigation: value };
  if (key === "deviceTabs" && isDeviceTabsAt(value)) return { ...a, deviceTabs: value };
  if (key === "theme" && isTheme(value)) return { ...a, theme: value };
  if (key === "outline" && isOutline(value)) return { ...a, outline: value }; // -- WP UB13 --
  if (key === "density" && isDensity(value)) return { ...a, density: value }; // -- WP UB13 --
  return null;
}
