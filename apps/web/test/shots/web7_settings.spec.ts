// -- WP WEB7 -- Pictures of Settings and Settings › Control as Qt Modern lays them out (the pages the
// parity tour does not open: each part's other pages, a search, an open card, the save bar), dark
// and light, on the core's demo device. Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/web7_settings.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `<project>-settings-<name>-<scheme>.png` in $SHOTS_DIR.

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  await page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session.locator(".device-meta")).toBeVisible();
  return session;
}

async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/${project}-settings-${name}-${scheme}.png`, fullPage: false });
  }
  await page.emulateMedia({ colorScheme: null });
}

test("WEB7 Settings pages", async ({ page }) => {
  const narrow = test.info().project.name === "narrow";
  test.setTimeout(300_000);
  const session = await openDemo(page);
  await session.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Settings", exact: true }).click();
  const parts = session.getByRole("tablist", { name: "Settings" });
  const nav = session.getByRole("navigation", { name: "Settings pages" });
  const go = async (part: string, row: string, name: string) => {
    await parts.getByRole("tab", { name: part, exact: true }).click();
    await nav.getByRole("button", { name: row, exact: true }).click();
    await page.waitForTimeout(300);
    await shot(page, name);
  };
  if (narrow) {
    for (const part of ["Application", "Display", "Device", "Control"]) {
      await parts.getByRole("tab", { name: part, exact: true }).click();
      await page.waitForTimeout(300);
      await shot(page, part.toLowerCase());
    }
  }
  await go("Device", "Global Parameters", "global");
  await go("Device", "Speakers", "speakers");
  await go("Application", "Advanced", "advanced");
  await go("Application", "Appearance", "appearance");
  await go("Display", "Spectrum Analyser", "spectrum");
  await go("Control", "Channel Groups", "groups");
  await go("Control", "Macros", "macros");
  await go("Control", "Auxiliary Outputs", "aux");
  await go("Control", "Control Surfaces", "surfaces");
  // An open card and the save bar after an applied change
  const knob = session.getByRole("group", { name: "Volume Knob", exact: true });
  await knob.getByRole("button", { name: "Expand Volume Knob" }).click();
  await shot(page, "surfaces-open");
  const pick = knob.getByRole("combobox").first();
  const values = await pick.locator("option:not([disabled])").evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value));
  const now = await pick.inputValue();
  const other = values.find((v) => v !== now);
  if (other) {
    await pick.selectOption(other);
    await knob.getByRole("button", { name: "Apply", exact: true }).click();
    await page.waitForTimeout(400);
    await page.mouse.wheel(0, 2000);
    await shot(page, "surfaces-unsaved");
  }
  await knob.getByRole("button", { name: "Collapse Volume Knob" }).click();
  await session.getByRole("button", { name: "Add Control" }).click();
  await shot(page, "surfaces-add-menu");
  await page.keyboard.press("Escape");
  // A search over every part
  await session.getByRole("searchbox", { name: "Search settings" }).fill("grid");
  await shot(page, "search");
  await session.getByRole("searchbox", { name: "Search settings" }).fill("nothing like it");
  await shot(page, "search-none");
});
