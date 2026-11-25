// WP WEBM5 e2e: the Modern web console as a whole, on the core's demo devices, in every project
// (Chromium, Firefox, Chromium at 375 px). Every page at a phone's width, ~1000 px and 1280 px with
// no sideways scroll and no page errors; screenshots of each section in both themes at those widths
// when SHOTS_DIR names a directory. Review RC's web items: 44 px tap targets at 375 px (RC13),
// Appearance's Inspector choice applied at once (RC6), keys shown only where the web binds them
// (RC7), Open File… in Simple mode saying where it goes (RC16), and a device closed with no error
// from its Worker.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { expectPage, openDevices, openPage, rail } from "../nav";

/**
 * Firefox's report of a core Worker it ended itself (a reload, a page left) while a core call was
 * suspended under JSPI: worker.js's `await this.promisedCall(…)` (src/core/wasm.ts, WasmModule.call)
 * is where its uncatchable end surfaces. Not the console's doing; about 1 reload in 300 (WEBM5's probe).
 */
const FIREFOX_WORKER_END = /^\[JavaScript Error: "uncaught exception: undefined" \{file: "[^"]*\/worker\.js" line: 1\}\]$/;

/** The page's errors; with `reloads`, not Firefox's report of the Workers a reload ends. */
function errorsOf(page: Page, reloads = false) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !(reloads && FIREFOX_WORKER_END.test(m.text()))) errors.push(m.text());
  });
  return errors;
}

async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

/** Screenshots in both themes (not in Firefox), named by the viewport's width. */
async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  const w = page.viewportSize()!.width;
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/webm5-${w}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

const mod = process.platform === "darwin" ? "Meta" : "Control";

/** The standalone page with its demo device open. */
async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  await expect(session).toHaveAttribute("data-status", "open");
  return session;
}

/** One console of the embedding page, Venue A's demo device open (written to); with `two`, Venue B's too, Venue A's on show. */
async function openVenues(page: Page, two = false): Promise<{ one: Locator; session: Locator; tabs: Locator }> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  await expect(session).toHaveAttribute("data-status", "open");
  const tabs = one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  if (two) {
    await openDevices(one);
    await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
    await expect(tabs).toHaveCount(2);
    await tabs.first().click();
    await expect(session).toHaveAttribute("data-status", "open");
  }
  return { one, session, tabs };
}

async function setMode(page: Page, session: Locator, mode: "Simple" | "Advanced") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

/** Every page of the demo device, in the rail's order; the first of each section is its screenshot. */
const PAGES = [
  "Main", "Overview", "EQ", "Crossover", "Delay & Gain", "Inputs", "Matrix", "Effects", "Measure", "Analyser", "Signal Generator",
  "Stats for nerbs", "Interrupt Monitor", "Presets", "Snapshots", "History", "Files", "AutoEQ", "System", "Events", "Connect", "Device", "Settings", "Control Surfaces",
];
const SHOTS = new Set(["Main", "EQ", "Crossover", "Matrix", "Effects", "Measure", "Analyser", "Snapshots", "History", "System", "Device", "Settings"]);

/** The widths this project looks at: a phone's in the narrow project, ~1000 and 1280 px in the others. */
const widths = () => (test.info().project.name === "narrow" ? [375] : [1280, 1000]);

test("WEBM5: every page at this project's widths: the rail, the page and the dock, no sideways scroll, no page errors", async ({ page }) => {
  test.setTimeout(process.env.SHOTS_DIR ? 240_000 : 90_000);
  const errors = errorsOf(page);
  for (const width of widths()) {
    await page.setViewportSize({ width, height: width === 375 ? 800 : 800 });
    const session = await openDemo(page);
    await shot(page, "overview");
    for (const name of PAGES) {
      await openPage(session, name);
      await expect(session.getByRole("slider", { name: "Volume", exact: true })).toBeVisible();
      await noSidewaysScroll(page);
      if (SHOTS.has(name)) await shot(page, name.toLowerCase().replace(/[^a-z]+/g, "-"));
    }
  }
  expect(errors).toEqual([]);
});

test("WEBM5: two devices, the menus, the palette and the drawer at ~1000 px and 375 px: screenshots, no sideways scroll", async ({ page }) => {
  test.setTimeout(process.env.SHOTS_DIR ? 120_000 : 60_000);
  const errors = errorsOf(page);
  for (const width of test.info().project.name === "narrow" ? [375] : [1000]) {
    await page.setViewportSize({ width, height: 800 });
    const { one, session } = await openVenues(page, true);
    await openPage(session, "EQ");
    await shot(page, "two-devices");
    await one.locator(".app-menu").getByRole("button").first().click();
    await expect(one.getByRole("menuitem", { name: "Search…" })).toBeVisible();
    await noSidewaysScroll(page);
    await shot(page, "app-menu");
    await page.keyboard.press("Escape");
    await page.keyboard.press(`${mod}+k`);
    await page.getByRole("dialog", { name: "Command palette" }).getByRole("combobox").fill("gain");
    await shot(page, "palette");
    await page.keyboard.press("Escape");
    await openPage(session, "Settings");
    await session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: "Appearance" }).click();
    await noSidewaysScroll(page);
    await shot(page, "appearance");
  }
  expect(errors).toEqual([]);
});

test("WEBM5 (review RC13): at 375 px the shell's controls are 44 px tap targets, in every browser", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  const { one, session } = await openVenues(page, true);
  await openPage(session, "EQ");
  // Every one measured; the ones too small are named at the end
  const small: string[] = [];
  const tall = async (l: Locator, what: string, square = false) => {
    const all = await l.all();
    expect(all.length, what).toBeGreaterThan(0);
    for (const el of all) {
      const box = (await el.boundingBox())!;
      if (box.height < 43.5 || (square && box.width < 43.5)) small.push(`${what}: ${Math.round(box.width)}×${Math.round(box.height)}`);
    }
  };
  const top = one.locator("header.top");
  await tall(top.locator(".app-menu .menu > button"), "the app menu");
  await tall(top.locator(".device-tab > [role=tab]"), "a device tab");
  await tall(top.locator(".device-tab-close"), "a tab's close", true);
  await tall(top.locator(".device-add"), "Open another DSPi", true);
  // (the search is the app menu's here)
  await expect(top.locator(".top-search")).toBeHidden();
  await tall(top.locator(".bell"), "the bell", true);
  await tall(session.locator(".dock button:visible, .dock select:visible"), "the dock");
  await tall(session.locator(".inspector-toggle:visible"), "the inspector's toggle");
  await tall(session.getByRole("tablist", { name: "Pages" }).getByRole("tab"), "the page tabs");
  await tall(rail(session).locator(".rail-list button"), "the rail's sections");
  // A menu's rows: the app menu's, and a context menu's (portalled out of the console)
  await one.locator(".app-menu .menu > button").click();
  await tall(one.locator(".action-row:visible"), "the app menu's rows");
  await page.keyboard.press("Escape");
  await one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().click({ button: "right" });
  await tall(one.getByRole("menu", { name: "Device" }).locator(".action-row"), "a device tab menu's rows");
  await page.keyboard.press("Escape");
  // The drawer's close
  await one.getByRole("button", { name: /^Notifications/ }).click();
  await tall(one.getByRole("dialog", { name: "Notifications" }).getByRole("button", { name: "Close" }), "the drawer's close", true);
  await page.keyboard.press("Escape");
  // Appearance's choices, and the device tabs at the rail's top
  await openPage(session, "Settings");
  await session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: "Appearance" }).click();
  const card = session.getByRole("region", { name: "Appearance" });
  await tall(card.locator("label"), "Appearance's choices");
  await card.getByRole("radio", { name: "At the top of the rail" }).check();
  await tall(rail(session).locator(".device-tab > [role=tab]"), "a device tab in the rail");
  await tall(rail(session).locator(".device-tab-close"), "a close in the rail", true);
  await noSidewaysScroll(page);
  await shot(page, "devices-in-rail");
  await card.getByRole("radio", { name: "In the top bar" }).check();
  expect(small).toEqual([]);
});

test("WEBM5 (review RC6): Appearance's Inspector choice applies at once, and the inspector's own Hide is the same choice", async ({ page }) => {
  const errors = errorsOf(page, true);
  const { session } = await openVenues(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const inspector = session.getByRole("complementary", { name: "Inspector" });
  const toggle = inspector.locator(".inspector-toggle");
  await expect(inspector).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  // Hidden in Settings › Appearance: the EQ page has it hidden at once, with no reload or tab switch
  await openPage(session, "Settings");
  await session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: "Appearance" }).click();
  const card = session.getByRole("region", { name: "Appearance" });
  await card.getByRole("radio", { name: "Hidden" }).check();
  await openPage(session, "EQ");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await expect(inspector.locator(".inspector-body")).toBeHidden();
  // Shown by its own button: Appearance says so
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await openPage(session, "Settings");
  await session.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: "Appearance" }).click();
  await expect(card.getByRole("radio", { name: "Beside the page" })).toBeChecked();
  // And kept: a new page load has the choice
  await card.getByRole("radio", { name: "Hidden" }).check();
  await page.reload();
  const again = await openVenues(page);
  await openPage(again.session, "EQ");
  await again.session.locator(".eq-pick", { hasText: "OUT1" }).click();
  await expect(again.session.getByRole("complementary", { name: "Inspector" }).locator(".inspector-toggle")).toHaveAttribute("aria-expanded", "false");
  expect(errors).toEqual([]);
});

test("WEBM5 (review RC7, RC16): the palette shows only keys the web binds; Open File… in Simple mode says it goes to the Devices page", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session } = await openVenues(page);
  const palette = page.getByRole("dialog", { name: "Command palette" });
  const field = palette.getByRole("combobox");
  await page.keyboard.press(`${mod}+k`);
  await field.fill("history");
  const history = palette.getByRole("option", { name: /^History…/ });
  await expect(history).toBeVisible();
  await expect(history.locator(".palette-keys")).toHaveCount(0);
  await field.fill("redo");
  await expect(palette.getByRole("option", { name: /^Redo/ }).first().locator(".palette-keys")).toHaveCount(1);
  await page.keyboard.press("Escape");
  // Advanced: Open File… goes to Library › Files
  await page.keyboard.press(`${mod}+k`);
  await field.fill("open file");
  await expect(palette.getByRole("option", { name: /^Open File…/ })).toBeVisible();
  await page.keyboard.press("Enter");
  await expectPage(session, "Files");
  // Simple has no Files page: the row says where it goes, and goes there
  await setMode(page, session, "Simple");
  await page.keyboard.press(`${mod}+k`);
  await field.fill("open file");
  const row = palette.getByRole("option", { name: /^Open File on the Devices Page…/ });
  await expect(row).toBeVisible();
  await row.click();
  await expect(one.getByRole("navigation", { name: "DSPi devices" })).toBeVisible();
  await expect(one.getByRole("region", { name: "Files" })).toBeVisible();
  await one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().click();
  await one.locator(".app-menu .menu > button").click();
  await expect(one.getByRole("menuitem", { name: "Open File on the Devices Page…" })).toBeVisible();
  await page.keyboard.press("Escape");
  await setMode(page, session, "Advanced");
  expect(errors).toEqual([]);
});

test("WEBM5: devices opened and closed in turn, from the tab's ✕ and its menu: no page errors, after the close too", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session, tabs } = await openVenues(page, true);
  for (let i = 0; i < 3; i++) {
    // Venue A's tab: the first at the start, the last once it is opened again
    const a = i === 0 ? one.locator(".device-tab").first() : one.locator(".device-tab").last();
    await a.getByRole("tab").click();
    await openPage(session, i === 1 ? "Analyser" : "EQ");
    if (i % 2) {
      await a.getByRole("tab").click({ button: "right" });
      await one.getByRole("menu", { name: "Device" }).getByRole("menuitem", { name: "Close" }).click();
    } else await a.locator(".device-tab-close").click();
    await expect(tabs).toHaveCount(1);
    await openDevices(one);
    await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
    await expect(tabs).toHaveCount(2);
    await expect(session).toHaveAttribute("data-status", "open");
  }
  for (let n = 2; n > 0; n--) {
    await one.getByRole("button", { name: /^Close / }).first().click();
    await expect(tabs).toHaveCount(n - 1);
  }
  await expect(one.getByText("No DSPi is connected. Connect one over USB, or try the demo device.")).toBeVisible();
  // A Worker's end is reported a moment after the close, if at all
  await page.waitForTimeout(500);
  expect(errors).toEqual([]);
});
