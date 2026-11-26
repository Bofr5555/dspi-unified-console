// -- WP WEBM1 -- Getting around the Modern shell in the e2e tests: a screen is a page of a section,
// chosen in the rail and then, where the section has several pages, by its tab. One place to follow
// when a screen moves.

import { expect, type Locator, type Page } from "@playwright/test";

/** Each screen's section and id, and its tab's label when that is not the screen's name here. */
const PLACES: Record<string, { section: string; id: string; tab?: string; part?: string }> = {
  Main: { section: "Overview", id: "main", tab: "Dashboard" },
  Overview: { section: "Overview", id: "overview", tab: "All Channels" },
  Sender: { section: "Overview", id: "sender" },
  EQ: { section: "Equaliser", id: "eq", tab: "PEQ" }, // -- WP WEBM2 -- the tab is PEQ (MODLAY 2)
  Crossover: { section: "Equaliser", id: "xover" }, // -- WP WEBM2 --
  "Delay & Gain": { section: "Equaliser", id: "delays" }, // -- WP WEBM2 --
  Inputs: { section: "Equaliser", id: "inputs" },
  Matrix: { section: "Routing", id: "matrix" },
  Effects: { section: "Effects", id: "effects" },
  Measure: { section: "Measure", id: "measure" },
  Analyser: { section: "Analyse", id: "analyser", tab: "Spectrum" }, // -- WP INTD -- Qt's tab name
  "Signal Generator": { section: "Analyse", id: "generator" },
  "Stats for nerbs": { section: "Analyse", id: "stats" },
  "Interrupt Monitor": { section: "Analyse", id: "monitor" },
  Presets: { section: "Library", id: "presets" }, // -- WP WEB7 --
  Snapshots: { section: "Library", id: "snapshots" },
  History: { section: "Library", id: "history" }, // -- WP WEBM3 --
  Files: { section: "Library", id: "files" },
  AutoEQ: { section: "Library", id: "autoeq" }, // -- WP WEB7 --
  System: { section: "System", id: "system", tab: "Units" }, // -- WP WEB7 -- Qt's System tabs
  "Clock and Links": { section: "System", id: "system-clock" }, // -- WP WEB7 --
  Installation: { section: "System", id: "system-installation" }, // -- WP WEB7 --
  "Signal Chain": { section: "System", id: "chain" }, // -- WP WEB7 -- the web's own
  Events: { section: "System", id: "events" },
  Connect: { section: "Devices", id: "connect", tab: "This Computer" }, // -- WP WEB7 -- Qt's tab name
  Hub: { section: "Devices", id: "hub" },
  "Hub Settings": { section: "Devices", id: "hub-settings" },
  Device: { section: "Devices", id: "device" },
  Settings: { section: "Settings", id: "settings" },
  "Control Surfaces": { section: "Settings", id: "settings", tab: "Settings", part: "Control" }, // -- WP WEB7 -- Settings' Control part (Qt Modern)
  Installer: { section: "Settings", id: "installer" },
};

function place(name: string): { section: string; id: string; tab: string; part?: string } {
  const p = PLACES[name];
  if (!p) throw new Error(`no such screen: ${name}`);
  return { section: p.section, id: p.id, tab: p.tab ?? name, part: p.part };
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
  // -- WP WEB7 -- a place that is a part of its page (Settings › Control)
  if (p.part) await partsOf(session, p.section).getByRole("tab", { name: p.part, exact: true }).click();
  await expectPage(session, name);
}

/** The screen on show is `name`. */
export async function expectPage(session: Locator, name: string): Promise<void> {
  await expect(session.locator(".page[data-page]")).toHaveAttribute("data-page", place(name).id);
  await expect(section(session, place(name).section)).toHaveAttribute("aria-current", "page");
  const part = place(name).part; // -- WP WEB7 --
  if (part) await expect(partsOf(session, place(name).section).getByRole("tab", { name: part, exact: true })).toHaveAttribute("aria-selected", "true");
}

// -- WP WEB7 -- Settings as Qt Modern: its parts (Application, Display, Device, Control) are the
// page's tabs; each lists its pages beside the page ("Settings pages")
const SETTINGS_PART: Record<string, string> = {
  About: "Application",
  Advanced: "Application",
  Appearance: "Application",
  Graphing: "Display",
  "Spectrum Analyser": "Display",
  Pins: "Device",
  "Global Parameters": "Device",
  Speakers: "Device",
  "Control Surfaces": "Control",
  "Channel Groups": "Control",
  Macros: "Control",
  "Auxiliary Outputs": "Control",
};

/** The Settings tab of `title`'s part, then its row in the list ("Settings pages"). */
export async function openSetting(session: Locator, title: string): Promise<void> {
  const part = SETTINGS_PART[title];
  if (!part) throw new Error(`no such Settings page: ${title}`);
  await partsOf(session, "Settings").getByRole("tab", { name: part, exact: true }).click();
  await session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: title, exact: true }).click();
  await expect(session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: title, exact: true })).toHaveAttribute("aria-current", "page");
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

// -- WP UB6 -- (UXP B6, D13)
/** The Fleet of a hub-served console (its first page while no DSPi is open). */
export const fleetOf = (page: Page | Locator) => page.getByRole("region", { name: "Fleet" });

/**
 * Open DSPi `serial` of the hub that served the page (`host`: its "127.0.0.1:port"), whichever way the page offers it: a
 * hub that lists its DSPis to this page starts the console on its Fleet (Open in the DSPi's row); one that wants its PIN
 * first has no Fleet (its entry on This Computer, the PIN when `pin`, then the DSPi in the hub's list, or the hub's one
 * DSPi opened at once). The session, once it shows `serial`.
 */
export async function openOnHub(page: Page, o: { host: string; serial: string; pin?: string }): Promise<Locator> {
  const fleet = fleetOf(page).getByRole("table");
  const entry = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub ${o.host}` });
  await expect(fleet.or(entry).first()).toBeVisible({ timeout: 20_000 });
  const session = page.getByRole("article").last();
  if (await fleet.isVisible()) {
    await fleet.getByRole("row").filter({ hasText: o.serial }).getByRole("button", { name: /^Open\b/ }).click();
  } else {
    await entry.getByRole("button", { name: "Connect" }).click();
    if (o.pin) {
      await session.getByLabel("PIN").fill(o.pin);
      await session.getByRole("button", { name: "Pair" }).click();
    }
    // The list shows a serial's last 8 characters; a hub with one DSPi opens it at once.
    const choice = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: o.serial.slice(-8) });
    const opened = page.locator(`article[data-serial*="${o.serial}"]`);
    await expect(choice.or(opened).first()).toBeVisible({ timeout: 20_000 });
    if (await choice.isVisible()) await choice.getByRole("button", { name: "Open" }).click();
  }
  await expect(session).toHaveAttribute("data-serial", new RegExp(o.serial));
  return session;
}

/** -- WP UB6 -- The hub page's part `name` (Devices, Alarms, Updates, Groups, Hub): its tab in the page head. */
export async function openHubPart(session: Locator, name: string): Promise<void> {
  const tab = partsOf(session, "Hub").getByRole("tab", { name, exact: true });
  await tab.click();
  await expect(tab).toHaveAttribute("aria-selected", "true");
}

/** -- WP UB6 -- A DSPi's body in the hub page's device table, its details shown: its row (`name`) and the details under it. */
export async function hubDetails(devices: Locator, name: string): Promise<Locator> {
  const row = devices.getByRole("row", { name, exact: true });
  const more = row.getByRole("button", { name: `Details of ${name}` });
  if ((await more.getAttribute("aria-expanded")) !== "true") await more.click();
  await expect(more).toHaveAttribute("aria-expanded", "true");
  return devices.locator("tbody").filter({ has: devices.page().getByRole("row", { name, exact: true }) });
}

/** -- WP UB6 -- This Computer (the device list) on a console with no DSPi open, from the Fleet a hub's page starts on. */
export async function showThisComputer(page: Page): Promise<void> {
  const list = page.getByRole("navigation", { name: "DSPi devices" });
  const tab = page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "This Computer" });
  await expect(list.or(fleetOf(page)).first()).toBeVisible({ timeout: 20_000 });
  if (!(await list.isVisible())) await tab.click();
  await expect(list).toBeVisible();
}
