// WP WEBM3 e2e: the section pages' parts as tabs in the page head (MODLAY 2) on the core's demo
// devices, in every project (Chromium, Firefox, Chromium at 375 px without sideways scroll):
// Effects (one tab each with its on/off dot), Measure, Library › History, Settings' groups; the
// part on show kept per device tab. Screenshots in both themes when SHOTS_DIR names a directory.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openDevices, openPage, partsOf, rail } from "../nav";

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
    await page.screenshot({ path: `${dir}/webm3-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** One console, Venue A's demo device open (written to). */
async function openOne(page: Page): Promise<{ one: Locator; session: Locator }> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  await expect(session.locator(".device-meta")).toBeVisible();
  return { one, session };
}

test("WEBM3: Effects has a tab per effect in the page head, each with its dot; the tab on show is kept per device", async ({ page }) => {
  const errors = errorsOf(page);
  const { one, session } = await openOne(page);
  await openPage(session, "Effects");
  const head = session.locator(".page-head");
  const parts = partsOf(session, "Effects");
  await expect(head.getByRole("tablist", { name: "Effects" })).toBeVisible();
  await expect(head.getByRole("tablist", { name: "Pages" })).toHaveCount(0);
  const loud = parts.getByRole("tab", { name: /^Loudness (On|Off)$/ });
  await loud.click();
  await expect(loud).toHaveAttribute("aria-selected", "true");
  const panel = session.getByRole("tabpanel", { name: "Loudness" });
  const sw = panel.getByRole("switch", { name: "Loudness", exact: true });
  const was = (await sw.getAttribute("aria-checked")) === "true";
  await sw.click();
  // The dot follows the effect's switch.
  await expect(parts.getByRole("tab", { name: was ? "Loudness Off" : "Loudness On" })).toBeVisible();
  await expect(loud.locator(".part-dot")).toHaveClass(was ? /^part-dot$/ : /part-dot on/);
  await noSidewaysScroll(page);
  await shot(page, "effects");
  // The keys of a tab row: the arrows choose.
  await loud.focus();
  await page.keyboard.press("ArrowRight");
  const next = parts.getByRole("tab").nth(((await parts.getByRole("tab").allTextContents()).findIndex((t) => t.startsWith("Loudness")) + 1) % (await parts.getByRole("tab").count()));
  await expect(next).toHaveAttribute("aria-selected", "true");
  await loud.click();
  // Another section and back: Loudness again. Another device and back: Loudness again.
  await openPage(session, "Matrix");
  await openPage(session, "Effects");
  await expect(parts.getByRole("tab", { name: /^Loudness / })).toHaveAttribute("aria-selected", "true");
  await openDevices(one);
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  const tabs = one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab");
  await expect(tabs).toHaveCount(2);
  await openPage(session, "Effects");
  await expect(partsOf(session, "Effects").getByRole("tab").first()).toHaveAttribute("aria-selected", "true");
  await tabs.nth(0).click();
  await expect(partsOf(session, "Effects").getByRole("tab", { name: /^Loudness / })).toHaveAttribute("aria-selected", "true");
  expect(errors).toEqual([]);
});

test("WEBM3: Measure's tabs are in the page head and its tab is kept over a section change", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openOne(page);
  await openPage(session, "Measure");
  const parts = partsOf(session, "Measurement");
  await expect(session.locator(".page-head").getByRole("tablist", { name: "Measurement" })).toBeVisible();
  await parts.getByRole("tab", { name: "Results" }).click();
  await expect(parts.getByRole("tab", { name: "Results" })).toHaveAttribute("aria-selected", "true");
  await expect(session.getByRole("tabpanel")).toBeVisible();
  await noSidewaysScroll(page);
  await shot(page, "measure");
  await openPage(session, "Effects");
  await openPage(session, "Measure");
  await expect(parts.getByRole("tab", { name: "Results" })).toHaveAttribute("aria-selected", "true");
  expect(errors).toEqual([]);
});

test("WEBM3: Library › History lists this connection's changes, newest first, and Undo steps back", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openOne(page);
  await openPage(session, "History");
  const screen = session.getByRole("region", { name: "History" });
  await expect(screen.getByText("No changes since this DSPi was opened.")).toBeVisible();
  // A change from the dock: the volume.
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill("128");
  await slider.dispatchEvent("change");
  await expect(session.locator(".volume-value")).toHaveText("-12.0 dB");
  const list = screen.getByRole("list", { name: "Changes, newest first" });
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await expect(list.getByRole("listitem").first()).toContainText("−12");
  await noSidewaysScroll(page);
  await shot(page, "history");
  await screen.getByRole("button", { name: "Undo" }).click();
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");
  await expect(list.getByRole("listitem").first()).toContainText("Undo");
  expect(errors).toEqual([]);
});

test("WEBM3: Settings' pages are listed by group beside the page", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openOne(page);
  await openPage(session, "Settings");
  const nav = session.getByRole("navigation", { name: "Settings pages" });
  for (const g of ["Device", "Display", "Application"]) await expect(nav.getByRole("group", { name: g })).toBeVisible();
  await nav.getByRole("group", { name: "Display" }).getByRole("button", { name: "Graphing" }).click();
  await expect(nav.getByRole("button", { name: "Graphing" })).toHaveAttribute("aria-current", "page");
  await noSidewaysScroll(page);
  if (test.info().project.name !== "narrow") {
    // Beside the page: the list's right edge is left of the page's content.
    const n = await nav.boundingBox();
    const p = await session.locator(".settings-page").boundingBox();
    expect(n!.x + n!.width).toBeLessThanOrEqual(p!.x);
  }
  await shot(page, "settings");
  expect(errors).toEqual([]);
});
