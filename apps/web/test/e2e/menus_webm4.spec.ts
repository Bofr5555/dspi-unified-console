// WP WEBM4 e2e: the command palette (Ctrl+K and the top bar's search), the menus from
// assets/actions.json (the app menu, a channel's, a device tab's, a preset slot's), the notification
// drawer and Settings › Appearance (the arrangement), on the core's demo devices, in every project
// (Chromium, Firefox, Chromium at 375 px without sideways scroll). Screenshots in both themes when
// SHOTS_DIR names a directory.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { expectPage, openDevices, openPage, partsOf, rail, openSetting } from "../nav";

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/webm4-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

const mod = process.platform === "darwin" ? "Meta" : "Control";

/** One console, Venue A's demo device open (written to); with `two`, Venue B's too (view only), Venue A's on show. */
async function open(page: Page, two = false): Promise<{ one: Locator; session: Locator }> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  await expect(session).toHaveAttribute("data-status", "open");
  if (two) {
    await openDevices(one);
    await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
    const tabs = one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
    await expect(tabs).toHaveCount(2);
    await expect(session).toHaveAttribute("data-status", "open");
    await tabs.first().click();
    await expect(session).toHaveAttribute("data-status", "open");
  }
  return { one, session };
}

const palette = (page: Page) => page.getByRole("dialog", { name: "Command palette" });
const field = (page: Page) => palette(page).getByRole("combobox", { name: "Search settings, channels, commands" });
const option = (page: Page, name: string | RegExp) => palette(page).getByRole("option", { name });

test("WEBM4: Ctrl+K opens the palette; typed words find a row, Enter runs it with the focus back; a plain B types", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await open(page);
  await openPage(session, "Main");
  const before = session.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Overview", exact: true });
  await before.focus();
  await page.keyboard.press(`${mod}+k`);
  await expect(palette(page)).toBeVisible();
  await expect(field(page)).toBeFocused();
  // Everything is listed before anything is typed, the shortcut beside a row
  await expect(option(page, /^Matrix Mixer…/)).toBeVisible();
  await expect(option(page, /^Matrix Mixer…/).locator(".palette-keys")).toHaveText(process.platform === "darwin" ? "⇧⌘M" : "Ctrl+Shift+M");
  // The plain B of A/B types here (the palette is modal)
  await page.keyboard.type("b");
  await expect(field(page)).toHaveValue("b");
  await field(page).fill("cfeed");
  const first = palette(page).getByRole("option").first();
  await expect(first).toHaveAttribute("aria-selected", "true");
  await expect(first).toContainText("Headphone Crossfeed…");
  await expect(first).toContainText("Effects");
  await shot(page, "palette");
  await page.keyboard.press("Enter");
  await expect(palette(page)).toHaveCount(0);
  await expectPage(session, "Effects");
  await expect(partsOf(session, "Effects").getByRole("tab", { name: /^Crossfeed/ })).toHaveAttribute("aria-selected", "true");
  await expect(before).toBeFocused();
  // Escape closes it, nothing run
  await page.keyboard.press(`${mod}+k`);
  await field(page).fill("matrix");
  await page.keyboard.press("Escape");
  await expect(palette(page)).toHaveCount(0);
  await expectPage(session, "Effects");
  // Arrows move over the entries; a page and a channel are there too
  await page.keyboard.press(`${mod}+k`);
  await field(page).fill("crossover");
  await expect(option(page, /^Crossover/).first()).toBeVisible();
  await option(page, /^Crossover/).first().click();
  await expectPage(session, "Crossover");
  await page.keyboard.press(`${mod}+k`);
  await field(page).fill("out1");
  await expect(palette(page).getByRole("option").first()).toContainText("Channel · OUT1");
  await page.keyboard.press("Enter");
  await expectPage(session, "EQ");
  await expect(session.getByRole("application", { name: /^EQ graph, OUT1/ })).toBeVisible();
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("WEBM4: the top bar's search; greyed rows say why; other devices are a switch away", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session } = await open(page, true);
  // Venue A on show: write rows can be chosen
  // -- WP WEBM5 -- at 480 px and below the top bar leaves the search to the app menu
  const search = one.getByRole("button", { name: "Search settings, channels, commands" });
  if (await search.isVisible()) await search.click();
  else {
    await one.locator(".app-menu .menu > button").click();
    await one.getByRole("menuitem", { name: "Search…" }).click();
  }
  await field(page).fill("mute");
  await expect(option(page, /^Mute/).first()).not.toHaveAttribute("aria-disabled", "true");
  // The other device
  await field(page).fill("switch");
  const sw = option(page, /^Switch to /);
  await expect(sw).toHaveCount(1);
  await expect(sw).toContainText("Device");
  await sw.click();
  const tabs = one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
  // Venue B is view only: its writes are listed greyed, with why, and not chosen
  await expect(session.locator(".read-only-reason")).toBeVisible(); // -- WP REVEFIX -- said in the strip (the badge is gone with the session bar)
  await page.keyboard.press(`${mod}+k`);
  await field(page).fill("commit");
  const commit = option(page, /^Commit Parameters…/);
  await expect(commit).toHaveAttribute("aria-disabled", "true");
  await expect(commit).toHaveAttribute("aria-selected", "false");
  await expect(commit.locator(".palette-where")).toHaveText("You can view this DSPi but not change it");
  await commit.click({ force: true });
  await expect(palette(page)).toBeVisible();
  await page.keyboard.press("Enter");
  await expect(palette(page)).toBeVisible();
  await page.keyboard.press("Escape");
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("WEBM4: the app menu is actions.json's rows the web has; Settings… and About", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session } = await open(page);
  const button = one.locator(".app-menu").getByRole("button").first();
  await button.click();
  const menu = one.getByRole("menu").first();
  await expect(menu.getByRole("menuitem", { name: "Open a DSPi…" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Settings…", exact: true })).toHaveAttribute("aria-keyshortcuts", /\+,$/);
  await expect(menu.getByRole("menuitem", { name: "About Fleet console one" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Keyboard Shortcuts" })).toBeVisible();
  // Rows the web has no handler for are not there at all (not greyed): Quit, Classic, Minimise to Tray
  for (const name of ["Quit", "Switch to the Classic Layout", "Minimise to Tray", "Import Filters…"]) await expect(menu.getByRole("menuitem", { name })).toHaveCount(0);
  await shot(page, "app-menu");
  await menu.getByRole("menuitem", { name: "Settings…", exact: true }).click();
  await expectPage(session, "Settings");
  await expect(button).toBeFocused();
  expect(errors).toEqual([]);
});

test("WEBM4: context menus of a channel, a device tab and a preset slot, by the pointer and by the keyboard", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session } = await open(page);
  await openPage(session, "EQ");
  const out1 = session.getByRole("group", { name: "Channels" }).getByRole("button", { name: /OUT1/ }).first();
  await out1.click({ button: "right" });
  const menu = one.getByRole("menu", { name: /^OUT1/ });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Rename", exact: true })).toBeFocused(); // -- WP INTD -- actions.json's channel.rename first
  await expect(menu.getByRole("menuitem", { name: "Copy Parameters" })).toBeVisible();
  await expect(menu.getByRole("menuitem", { name: "Identify" })).toBeVisible();
  await shot(page, "channel-menu");
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(out1).toBeFocused();
  // The menu key; Edit Routing… goes to the matrix
  await page.keyboard.press("Shift+F10");
  await one.getByRole("menu", { name: /^OUT1/ }).getByRole("menuitem", { name: "Edit Routing…" }).click();
  await expectPage(session, "Matrix");
  // -- WP WEB6 -- Qt's page tools: Files ▾ and the inspector's toggle, no Channel button (the menu is the right click's)
  await openPage(session, "EQ");
  await expect(session.locator(".eq-tools").getByRole("button", { name: "Channel" })).toHaveCount(0);
  await expect(session.locator(".eq-tools").getByRole("button", { name: "Files" })).toBeVisible();

  // A preset slot (-- WP INTD -- its row in the session bar's preset popover): actions.json's `slot` menu
  await session.getByRole("button", { name: /^Presets: / }).click();
  const slot = session.getByRole("dialog", { name: "Presets" }).getByRole("listitem", { name: /^2: / });
  await slot.getByRole("button", { name: "More" }).click();
  const slotMenu = slot.getByRole("menu");
  // -- WP REVEFIX -- (REVE B1, D8) Copy to… is the active slot's only; Clear names its slot
  for (const item of ["Recall", "Save Current Settings Here", "Rename…", "Set as Startup Preset", /^Clear ".+"…$/, "Clear All Slots…"])
    await expect(slotMenu.getByRole("menuitem", { name: item, exact: true })).toBeVisible();
  await expect(slotMenu.getByRole("menuitem", { name: "Copy to…" })).toHaveCount(0);
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");

  await noSidewaysScroll(page);
  // Before the close: Firefox may report its Worker's end as an uncaught exception (the known
  // link.spec reload flake's kind), which is not this test's
  expect(errors).toEqual([]);

  // A device tab: Close (asks about unsaved changes as its ✕ does)
  const tab = one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first();
  await tab.click({ button: "right" });
  const devMenu = one.getByRole("menu", { name: "Device" });
  await expect(devMenu.getByRole("menuitem", { name: "Close" })).toBeVisible();
  await devMenu.getByRole("menuitem", { name: "Close" }).click();
  await expect(one.getByRole("tablist", { name: "Open DSPi" })).toHaveCount(0);
});

test("WEBM4: the bell opens the notification drawer; Escape closes it, the focus back on the bell", async ({ page }) => {
  const errors = errorsOf(page);
  const { one } = await open(page);
  const bell = one.getByRole("button", { name: /^Notifications/ });
  await bell.click();
  const drawer = one.getByRole("dialog", { name: "Notifications" });
  await expect(drawer).toBeVisible();
  await expect(drawer.getByText("Nothing waits for you.")).toBeVisible();
  await expect(drawer.getByRole("button", { name: "Close" })).toBeFocused();
  await noSidewaysScroll(page);
  await shot(page, "drawer");
  await page.keyboard.press("Escape");
  await expect(drawer).toHaveCount(0);
  await expect(bell).toBeFocused();
  // A press beside it closes it too
  await bell.click();
  await expect(drawer).toBeVisible();
  await one.locator(".shell-main").click({ position: { x: 5, y: 5 } });
  await expect(drawer).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WEBM4: Settings › Appearance arranges the shell at once and keeps it in this browser", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session } = await open(page, true);
  await openPage(session, "Settings");
  await openSetting(session, "Appearance"); // -- WP WEB7 -- its part, then its row
  const card = session.getByRole("region", { name: "Appearance" });
  const root = one.locator(".dspi-web");
  // The device tabs at the rail's top
  await card.getByRole("radio", { name: "At the top of the rail" }).check();
  await expect(root).toHaveAttribute("data-device-tabs", "rail");
  await expect(rail(session).getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);
  await expect(one.locator("header.top, .device-row").getByRole("tablist", { name: "Open DSPi" })).toHaveCount(0);
  await noSidewaysScroll(page);
  await shot(page, "devices-in-rail");
  // Tabs along the top: no rail, so the device tabs go back to the top bar
  await card.getByRole("radio", { name: "Tabs along the top" }).check();
  await expect(root).toHaveAttribute("data-nav", "tabs");
  await expect(root).not.toHaveAttribute("data-device-tabs", "rail");
  // -- WP REVEFIX -- two devices: their own row over the top bar (Qt's device row)
  await expect(one.locator(".device-row").getByRole("tablist", { name: "Open DSPi" })).toBeVisible();
  await expect(card.getByRole("radio", { name: "In the top bar" })).toBeDisabled();
  // The theme
  await card.getByRole("radio", { name: "Dark" }).check();
  await expect(root).toHaveAttribute("data-theme", "dark");
  await noSidewaysScroll(page);
  // Kept: a new page load has it
  await page.reload();
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(root).toHaveAttribute("data-nav", "tabs");
  await expect(root).toHaveAttribute("data-theme", "dark");
  await expect(session).toHaveAttribute("data-status", "open");
  expect(errors).toEqual([]); // before a close (see the context menus' test)
  // With no device open, Settings… in the app menu has the same
  await one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().click({ button: "right" });
  await one.getByRole("menu", { name: "Device" }).getByRole("menuitem", { name: "Close" }).click();
  await expect(one.getByRole("tablist", { name: "Open DSPi" })).toHaveCount(0);
  await one.locator(".app-menu").getByRole("button").first().click();
  await one.getByRole("menuitem", { name: "Settings…", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "Settings" });
  await dialog.getByRole("radio", { name: "System" }).check();
  await expect(root).not.toHaveAttribute("data-theme", /.+/);
  await dialog.getByRole("radio", { name: "Rail with page tabs" }).check();
  await expect(root).toHaveAttribute("data-nav", "rail");
  await dialog.getByRole("button", { name: "Done" }).click();
  await expect(dialog).toHaveCount(0);
});
