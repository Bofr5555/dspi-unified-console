// -- WP INTE -- 375 px (the narrow project): no page of a device sticks out sideways, the pages WEB6 and
// WEB7 brought included. Every page of every section of the rail on the core's demo DSPi, and each of
// its parts (the tabs in the page's head: Measure's, Settings', Library's ...); then nothing sticks out
// of the page sideways. Hub pages need a hub (hub_p2wb3 checks them at this width); the Installer page the
// Installer mode (the installer specs check its width at 375 px).

import { expect, test, type Page } from "@playwright/test";
import { openPage, rail } from "../nav";

const PAGES = ["Main", "Overview", "EQ", "Crossover", "Delay & Gain", "Inputs", "Matrix", "Effects", "Measure", "Analyser",
  "Signal Generator", "Stats for nerbs", "Interrupt Monitor", "Presets", "Snapshots", "History", "Files", "AutoEQ", "System",
  "Clock and Links", "Installation", "Signal Chain", "Events", "Connect", "Device", "Settings", "Control Surfaces"];

async function over(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
}

test("INTE: at 375 px no page and no part of one sticks out sideways", async ({ page }) => {
  test.skip(test.info().project.name !== "narrow", "the narrow project's width");
  test.setTimeout(240_000);
  await page.goto("/");
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").first();
  await expect(rail(session)).toBeVisible();
  const wide: string[] = [];
  for (const name of PAGES) {
    await openPage(session, name);
    await page.waitForTimeout(150);
    if ((await over(page)) > 0) wide.push(`${name}: ${await over(page)} px`);
    const parts = session.locator(".page-parts").getByRole("tab");
    const n = await parts.count();
    for (let i = 0; i < n; i++) {
      const tab = parts.nth(i);
      const label = (await tab.textContent())?.trim() ?? String(i);
      await tab.click();
      await page.waitForTimeout(150);
      if ((await over(page)) > 0) wide.push(`${name} › ${label}: ${await over(page)} px`);
    }
  }
  expect(wide, "pages wider than the window").toEqual([]);
});
