// -- WP REVEFIX -- Pictures of the shell's states the page tours do not reach: the preset popover (clean, with unsaved
// changes, a slot's menu), Save ▾, the bar while something is unsaved, the no-device frame on two of its pages, and
// two devices (the device row over the top bar). Dark and light. Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/revefix.spec.ts" tools/shots/web.sh <out dir>
//
// Files: `<project>-<name>-<scheme>.png` in $SHOTS_DIR, with REVE des-A's names where it had the same state.

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPresets, setOutputGain, slotRow } from "../presets";

const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-status", "open");
  return session;
}

async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

test("the shell's states", async ({ page }) => {
  test.setTimeout(240_000);
  // No device: the whole rail, the strip, the devices' page; then the Overview's empty state
  await page.goto("/");
  await expect(page.getByText("No DSPi is connected.")).toBeVisible();
  await shot(page, "nodevice-start");
  await page.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Equaliser", exact: true }).click();
  await shot(page, "nodevice-equaliser");

  const session = await openDemo(page);
  await page.mouse.move(700, 500);
  await shot(page, "overview-clean");
  const pop = await openPresets(session);
  await shot(page, "preset-open");
  await slotRow(pop, 1).getByRole("button", { name: /^More/ }).click();
  await shot(page, "preset-slotmenu");
  await page.keyboard.press("Escape");
  await page.keyboard.press("Escape");
  await session.getByRole("button", { name: "Save options" }).click();
  await shot(page, "save-menu");
  await page.keyboard.press("Escape");
  await setOutputGain(session, -3);
  await page.mouse.move(700, 500);
  await shot(page, "dirty-bar");
  await openPresets(session);
  await shot(page, "preset-open-dirty");
  await page.keyboard.press("Escape");

  // Two devices (the embedding page's two venues): their tabs are a row over the top bar
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("article").first()).toHaveAttribute("data-status", "open");
  await one.getByRole("button", { name: "Open another DSPi" }).click();
  await one.getByRole("region", { name: "Venue B" }).getByRole("button", { name: "Connect" }).click();
  await expect(one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);
  await page.waitForTimeout(500);
  await shot(page, "two-devices");
});
