// WP WEBM1 e2e: the Modern shell on the core's demo devices, in every project (Chromium, Firefox,
// Chromium at 375 px without sideways scroll). Two demo devices in two tabs, each with its own page
// and values; a tab's close button asks about unsaved changes; the rail and the tabs across the
// top; the dock on every page. Screenshots in both themes when SHOTS_DIR names a directory.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { appMenu, expectPage, openDevices, openPage, pageTab, pageTabs, rail, section } from "../nav";
import { presetsButton, setOutputGain } from "../presets"; // -- WP INTD --

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
    await page.screenshot({ path: `${dir}/webm1-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** The embedding page with one console: two venues, each with a demo device of its own. */
async function openTwo(page: Page, second: "Venue A" | "Venue B" = "Venue B") {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  await openDevices(one);
  await one.getByRole("region", { name: second }).getByRole("button", { name: "Connect" }).click();
  const tabs = one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  await expect(tabs).toHaveCount(2);
  await expect(session).toHaveAttribute("data-status", "open");
  return { one, session, tabs };
}

async function setVolume(session: ReturnType<Page["locator"]>, pos: string) {
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill(pos);
  await slider.dispatchEvent("change");
}

test("WEBM1: two demo devices in tabs, each with its own page and values; switching comes back to them", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session, tabs } = await openTwo(page);
  // Venue B is view only; Venue A is written to.
  await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
  await openPage(session, "Stats for nerbs");
  await tabs.nth(0).click();
  await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
  await expectPage(session, "Main");
  await setVolume(session, "128");
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");
  await openPage(session, "Matrix");
  await noSidewaysScroll(page);
  // The other tab: its own page, its own volume.
  await tabs.nth(1).click();
  await expectPage(session, "Stats for nerbs");
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");
  await expect(session.getByRole("slider", { name: "Volume" })).toBeDisabled();
  await tabs.nth(0).click();
  await expectPage(session, "Matrix");
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");
  // The keyboard: the row is one stop, the arrows choose.
  await tabs.nth(0).focus();
  await page.keyboard.press("ArrowRight");
  await expect(tabs.nth(1)).toHaveAttribute("aria-selected", "true");
  await expect(tabs.nth(1)).toBeFocused();
  // "+" shows the devices, with no tab chosen, and a tab goes back to its device.
  await openDevices(one);
  await expect(one.getByRole("tab", { selected: true, name: /Venue/ })).toHaveCount(0);
  await expect(section(session, "Devices")).toHaveAttribute("aria-current", "page");
  // A venue already open is listed, not offered.
  await expect(one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Open" })).toBeDisabled();
  await tabs.nth(1).click();
  await expectPage(session, "Stats for nerbs");
  await shot(page, "two-devices");
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("WEBM1: a tab's close button shows its device and asks about unsaved changes; Cancel keeps it, Discard closes it", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session, tabs } = await openTwo(page);
  await tabs.nth(0).click();
  // -- WP INTD -- an output's gain on Delay & gain (the Overview's level cards are gone)
  await setOutputGain(session, -1);
  // -- WP REVEFIX -- (REVE §3.1) one dirty marker: the preset button's dot, not the tab's
  await expect(session.locator(".bar-preset-dot.on")).toHaveCount(1);
  await expect(one.locator(".device-tab-unsaved")).toHaveCount(0);
  await expect(session.getByRole("group", { name: "Device actions" }).locator(".bar-count.on")).toHaveText(/^[1-9]/);
  // From the other tab, and while the devices are shown: the question comes on its own device.
  await tabs.nth(1).click();
  await openDevices(one);
  const closes = one.getByRole("button", { name: /^Close / });
  await closes.nth(0).click();
  const ask = page.getByRole("dialog", { name: "Unsaved Changes" });
  await expect(ask).toBeVisible();
  await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
  await ask.getByRole("button", { name: "Cancel" }).click();
  await expect(tabs).toHaveCount(2);
  await closes.nth(0).click();
  await ask.getByRole("button", { name: "Discard" }).click();
  await expect(tabs).toHaveCount(1);
  await expect(tabs.nth(0)).toHaveAttribute("aria-selected", "true");
  await expect(session.getByText("You can view this DSPi but not change it").first()).toBeVisible();
  // Nothing unsaved: closed at once; with none open, the devices.
  await closes.nth(0).click();
  await expect(one.getByText("No DSPi is connected. Connect one over USB, or try the demo device.")).toBeVisible();
  await expect(tabs).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WEBM1: the sections, their pages and the page last open in each; the dock on every page; the bar's actions", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/");
  await expect(section(page.locator("main"), "Devices")).toHaveAttribute("aria-current", "page");
  await shot(page, "empty");
  await noSidewaysScroll(page);
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").first();
  await expect(rail(session).getByRole("button")).toHaveText(["Overview", "Equaliser", "Routing", "Effects", "Measure", "Analyse", "Library", "System", "Devices", "Settings"]);
  await shot(page, "overview");
  for (const name of ["Main", "Overview", "EQ", "Inputs", "Matrix", "Effects", "Measure", "Analyser", "Signal Generator", "Snapshots", "Files", "System", "Events", "Connect", "Device", "Settings", "Control Surfaces"]) {
    await openPage(session, name);
    await expect(session.getByRole("slider", { name: "Volume", exact: true })).toBeVisible();
    await expect(session.getByRole("list", { name: "Levels" }).getByRole("meter")).toHaveCount(10); // -- WP INTE -- no meter for the 7 unused outputs
    await noSidewaysScroll(page);
    if (["EQ", "Analyser", "System", "Settings", "Connect"].includes(name)) await shot(page, name.toLowerCase());
  }
  // A section comes back at the page it was left at; its tabs take the arrows.
  await openPage(session, "Signal Generator");
  await openPage(session, "Matrix");
  await expect(pageTabs(session)).toHaveCount(0);
  await section(session, "Analyse").click();
  await expectPage(session, "Signal Generator");
  await pageTab(session, "Signal Generator").focus();
  await page.keyboard.press("ArrowRight");
  await expectPage(session, "Stats for nerbs");
  await expect(pageTab(session, "Stats for nerbs")).toBeFocused();
  // The bar's picker recalls, with the questions the scene bar asks; Undo takes a write back.
  await setVolume(session, "128");
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");
  await session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" }).click();
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");
  await session.getByRole("button", { name: /^Save to Device/ }).click();
  await expect(page.getByRole("dialog", { name: "Save Preset" })).toBeVisible();
  await page.getByRole("dialog", { name: "Save Preset" }).getByRole("button", { name: "Cancel" }).click();
  // The dock's source, read back.
  const source = session.getByRole("combobox", { name: "Source", exact: true });
  await expect(source).toHaveValue("0");
  await source.selectOption("1");
  await expect(source).toHaveValue("1");
  await appMenu(page, "Keyboard Shortcuts");
  await expect(page.getByRole("dialog", { name: "Keyboard Shortcuts" })).toBeVisible();
  await page.keyboard.press("Escape");
  expect(errors).toEqual([]);
});

test("WEBM1: what limits a device is above its page; the dock and the bars never cover it", async ({ page }) => {
  await page.goto("/?readonly");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").first();
  await expect(section(session, "Overview")).toBeVisible();
  const strip = (await session.locator(".read-only-reason").boundingBox())!;
  const head = (await session.locator(".page-head").boundingBox())!;
  expect(strip.y + strip.height).toBeLessThanOrEqual(head.y + 0.5);
  await expect(session.getByRole("button", { name: /^Save to Device/ })).toBeDisabled();
  await expect(session.getByRole("combobox", { name: "Source", exact: true })).toBeDisabled();
  // -- WP INTD -- the presets' popover opens (to look), its actions are off
  await presetsButton(session).click();
  for (const b of await session.getByRole("dialog", { name: "Presets" }).getByRole("button", { name: /^(Recall .*|Save|Copy Here)$/ }).all()) await expect(b).toBeDisabled();
  await page.keyboard.press("Escape");
  // Scrolled to the end, the status rail ends above the dock.
  await openPage(session, "EQ");
  await page.evaluate(() => window.scrollTo(0, document.documentElement.scrollHeight));
  const foot = (await session.getByRole("complementary", { name: "Status" }).boundingBox())!;
  const dock = (await session.locator(".dock").boundingBox())!;
  expect(foot.y + foot.height).toBeLessThanOrEqual(dock.y + 0.5);
  const view = page.viewportSize()!;
  expect(dock.y + dock.height).toBeLessThanOrEqual(view.height + 0.5);
});

test("WEBM1: arranged as tabs across the top (variant A), the same sections and pages", async ({ page }) => {
  await page.goto("/?nav=tabs");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").first();
  // -- WP INT3 -- measured once the view has settled: in Firefox the page's box
  // was null for a moment after Connect (3 runs in 10), visible or not
  const settled = async (l: Locator) => {
    let box: Awaited<ReturnType<Locator["boundingBox"]>> = null;
    await expect.poll(async () => (box = await l.boundingBox()) !== null).toBe(true);
    return box!;
  };
  const nav = await settled(rail(session));
  const body = await settled(session.locator(".shell-main"));
  expect(nav.y + nav.height).toBeLessThanOrEqual(body.y + 0.5);
  expect(nav.height).toBeLessThan(80);
  await openPage(session, "Signal Generator");
  await expect(pageTabs(session).getByRole("tab")).toHaveText(["Spectrum", "Signal Generator", "Stats for nerbs", "Interrupt Monitor"]);
  await shot(page, "nav-tabs");
  await noSidewaysScroll(page);
});

test("WEBM1: 375 px in every browser: one row of sections, the dock in two lines, no sideways scroll", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  const { one, session, tabs } = await openTwo(page);
  await noSidewaysScroll(page);
  const bar = (await rail(session).boundingBox())!;
  expect(bar.height).toBeLessThan(80);
  const dock = (await session.locator(".dock").boundingBox())!;
  expect(dock.height).toBeLessThan(112); // -- WP WEBM2 -- two lines of 44 px tap targets (RB)
  // The device's actions stay in one line.
  const actions = (await session.getByRole("group", { name: "Device actions" }).boundingBox())!;
  expect(actions.height).toBeLessThan(50); // -- WP REVEFIX -- one line of 44 px tap targets, in the top bar's second line
  expect(dock.y + dock.height).toBeLessThanOrEqual(800.5);
  // The section on show is scrolled into sight in its row.
  await openPage(session, "Settings");
  const at = (await section(session, "Settings").boundingBox())!;
  expect(at.x).toBeGreaterThanOrEqual(0);
  expect(at.x + at.width).toBeLessThanOrEqual(375.5);
  await expect(tabs).toHaveCount(2);
  await openDevices(one);
  await noSidewaysScroll(page);
});
