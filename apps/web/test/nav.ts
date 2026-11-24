// -- WP WEBM1 -- Getting around the Modern shell in the e2e tests: a screen is a page of a section,
// chosen in the rail and then, where the section has several pages, by its tab. One place to follow
// when a screen moves.

import { expect, type Locator, type Page } from "@playwright/test";

/** Each screen's section and id, and its tab's label when that is not the screen's name here. */
const PLACES: Record<string, { section: string; id: string; tab?: string }> = {
  Main: { section: "Overview", id: "main", tab: "Dashboard" },
  Overview: { section: "Overview", id: "overview", tab: "All Channels" },
  Sender: { section: "Overview", id: "sender" },
  EQ: { section: "Equaliser", id: "eq", tab: "PEQ" }, // -- WP WEBM2 -- the tab is PEQ (MODLAY 2)
  Crossover: { section: "Equaliser", id: "xover" }, // -- WP WEBM2 --
  "Delay & gain": { section: "Equaliser", id: "delays" }, // -- WP WEBM2 --
  Inputs: { section: "Equaliser", id: "inputs" },
  Matrix: { section: "Routing", id: "matrix" },
  Effects: { section: "Effects", id: "effects" },
  Measure: { section: "Measure", id: "measure" },
  Analyser: { section: "Analyse", id: "analyser", tab: "Spectrum" }, // -- WP INTD -- Qt's tab name
  "Signal Generator": { section: "Analyse", id: "generator" },
  "Stats for nerbs": { section: "Analyse", id: "stats" },
  "Interrupt Monitor": { section: "Analyse", id: "monitor" },
  Snapshots: { section: "Library", id: "snapshots" },
  History: { section: "Library", id: "history" }, // -- WP WEBM3 --
  Files: { section: "Library", id: "files" },
  System: { section: "System", id: "system", tab: "Units" }, // -- WP WEB7 -- Qt's System tabs
  "Clock and Links": { section: "System", id: "system-clock" }, // -- WP WEB7 --
  Installation: { section: "System", id: "system-installation" }, // -- WP WEB7 --
  "Signal Chain": { section: "System", id: "chain" }, // -- WP WEB7 -- the web's own
  Events: { section: "System", id: "events" },
  Connect: { section: "Devices", id: "connect", tab: "Open a DSPi" },
  Hub: { section: "Devices", id: "hub" },
  "Hub Settings": { section: "Devices", id: "hub-settings" },
  Device: { section: "Devices", id: "device" },
  Settings: { section: "Settings", id: "settings" },
  "Control Surfaces": { section: "Settings", id: "surfaces" },
  Installer: { section: "Settings", id: "installer" },
};

function place(name: string): { section: string; id: string; tab: string } {
  const p = PLACES[name];
  if (!p) throw new Error(`no such screen: ${name}`);
  return { section: p.section, id: p.id, tab: p.tab ?? name };
}

export const rail = (session: Locator) => session.getByRole("navigation", { name: "Sections" });
export const section = (session: Locator, name: string) => rail(session).getByRole("button", { name, exact: true });
export const pageTabs = (session: Locator) => session.getByRole("tablist", { name: "Pages" });
export const pageTab = (session: Locator, name: string) => pageTabs(session).getByRole("tab", { name: place(name).tab, exact: true });
// -- WP WEBM3 -- a page's parts (ui/shell/Parts.tsx) are tabs in the page head, outside the page's own region
const SHELL = 'contains(concat(" ", normalize-space(@class), " "), " shell ")';
/** The tablist of a page's parts named `name`, from the session's view or anything inside it. */
export const partsOf = (inside: Locator, name: string) =>
  inside.locator(`xpath=(ancestor-or-self::*[${SHELL}] | descendant::*[${SHELL}])[last()]`).getByRole("tablist", { name });

/** Go to a screen: its section, then its tab when the section has several pages. */
export async function openPage(session: Locator, name: string): Promise<void> {
  const p = place(name);
  await section(session, p.section).click();
  await expect(section(session, p.section)).toHaveAttribute("aria-current", "page");
  // The tabs come with the section, or not at all (a section with one page).
  if (await pageTabs(session).count()) await pageTab(session, name).click();
  await expectPage(session, name);
}

/** The screen on show is `name`. */
export async function expectPage(session: Locator, name: string): Promise<void> {
  await expect(session.locator(".page[data-page]")).toHaveAttribute("data-page", place(name).id);
  await expect(section(session, place(name).section)).toHaveAttribute("aria-current", "page");
}

/** Whether the console offers the screen `name` now; goes to its section to see, when there is one. */
export async function hasPage(session: Locator, name: string): Promise<boolean> {
  const p = place(name);
  if (!(await section(session, p.section).count())) return false;
  await section(session, p.section).click();
  await expect(section(session, p.section)).toHaveAttribute("aria-current", "page");
  if (await pageTabs(session).count()) return (await pageTab(session, name).count()) > 0;
  return (await session.locator(".page[data-page]").getAttribute("data-page")) === p.id;
}

/** The page with the devices to connect to, whatever is on show. */
export async function openDevices(console_: Page | Locator): Promise<void> {
  const add = console_.getByRole("button", { name: "Open another DSPi" });
  await add.click();
  await expect(console_.getByRole("navigation", { name: "DSPi devices" })).toBeVisible();
}

/** An item of the app menu. */
export async function appMenu(console_: Page | Locator, item: string): Promise<void> {
  await console_.locator(".app-menu").getByRole("button").first().click();
  await console_.getByRole("menuitem", { name: item, exact: true }).click();
}
