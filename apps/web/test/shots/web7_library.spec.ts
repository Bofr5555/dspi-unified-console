// -- WP WEB7 -- Pictures of Library's pages in states the page-by-page tour does not reach: Presets
// with unsaved changes and a slot's menu open, Snapshots with a list (a row hovered) and a compare
// running, AutoEQ with a search and a headphone chosen. Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/web7_library.spec.ts --project desktop" tools/shots/web.sh <out dir>

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";


async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/${project}-${name}-${scheme}.png`, fullPage: false });
  }
  await page.emulateMedia({ colorScheme: null });
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  await page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-status", "open"); // -- WP UWN -- (UXP A17) the tab's second line is hidden at 375 px
  return session;
}

test("WEB7 Library's states", async ({ page }) => {
  test.setTimeout(180_000);
  const session = await openDemo(page);
  const narrow = test.info().project.name === "narrow";

  // Presets: a second stored slot, unsaved changes, the active row's menu open.
  await openPage(session, "Presets");
  const presets = session.getByRole("region", { name: "Presets" });
  await presets.getByRole("listitem", { name: "3: Empty" }).getByRole("button", { name: "More, 3: Empty" }).click();
  await session.getByRole("menuitem", { name: "Copy “1: Demo” Here" }).click();
  await expect(presets.getByRole("listitem", { name: "3: Preset 3" })).toBeVisible();
  // An unsaved change: Bypass EQ by its key (the page itself, not the test page's core handle)
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("Control+b");
  await expect(session.getByRole("button", { name: /unsaved changes$/ })).toBeVisible();
  await shot(page, "library-presets-dirty");
  await presets.getByRole("button", { name: /^More, 1: Demo/ }).click();
  await shot(page, "library-presets-menu");
  await page.keyboard.press("Escape");

  // Snapshots: two captured, the second row hovered; then a compare running.
  await openPage(session, "Snapshots");
  const snaps = session.getByRole("region", { name: "Snapshots" });
  for (const [name, note] of [["Evening", "before the gig"], ["Bright", ""]] as const) {
    await session.getByRole("button", { name: "Capture…" }).first().click();
    const pop = session.getByRole("dialog", { name: "Capture Snapshot" });
    await pop.getByLabel("Name").fill(name);
    if (note) await pop.getByLabel("Note").fill(note);
    await Promise.all([page.waitForEvent("download"), pop.getByRole("button", { name: "Capture", exact: true }).click()]);
  }
  await snaps.getByRole("list", { name: "Held snapshots" }).getByRole("listitem", { name: "Bright" }).hover();
  await shot(page, "library-snapshots-list");
  await snaps.getByRole("region", { name: "A/B compare" }).getByRole("button", { name: "Start" }).click();
  await expect(snaps.getByRole("button", { name: "Toggle A/B" })).toBeVisible();
  await page.mouse.move(1, 1);
  await shot(page, "library-snapshots-compare");
  await snaps.getByRole("button", { name: "Keep A" }).click();

  // AutoEQ: a search, a headphone chosen (its preview), a favourite.
  await openPage(session, "AutoEQ");
  const tools = session.getByRole("toolbar", { name: "AutoEQ tools" });
  await tools.getByLabel("Search headphones").fill("sennheiser hd 6");
  const aeq = session.getByRole("region", { name: "AutoEQ" });
  const hit = aeq.getByRole("button", { name: /^Sennheiser HD 600 / }).first();
  await expect(hit).toBeVisible({ timeout: 15_000 });
  await aeq.getByRole("button", { name: "Favorite Sennheiser HD 600", exact: true }).first().click();
  await hit.click();
  await expect(aeq.getByRole("img", { name: "The response of Sennheiser HD 600" })).toBeVisible();
  await page.mouse.move(1, 1);
  await shot(page, "library-autoeq-chosen");
  if (narrow) return;
  await tools.getByRole("button", { name: "More" }).click();
  await session.getByRole("menuitem", { name: "Update Database…" }).click();
  await expect(page.getByRole("dialog", { name: "Update Database" })).toBeVisible();
  await shot(page, "library-autoeq-database");
});
