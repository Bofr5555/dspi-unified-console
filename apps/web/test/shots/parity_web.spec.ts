// -- WP WEB -- Before/after pictures for the web's design pass (DESREV's web parity: the outline
// and accent rules, the matrix, the empty states): every page and every part of a page on the
// core's demo device, the matrix's states, and the pages without a device, dark and light.
// Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/parity_web.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `<project>-<name>-<scheme>.png` in $SHOTS_DIR. Only pictures; a step that cannot find
// what it pictures is skipped (the "before" run has no new matrix).

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";

const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
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
    await page.screenshot({ path: `${dir}/${project}-${name}-${scheme}.png`, fullPage: false });
  }
  await page.emulateMedia({ colorScheme: null });
}

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

test("every page and part on the demo", async ({ page }) => {
  test.setTimeout(600_000);
  const session = await openDemo(page);
  const rail = session.getByRole("navigation", { name: "Sections" });
  const sections = await rail.getByRole("button").allInnerTexts();
  for (const name of sections.map((s) => s.trim()).filter(Boolean)) {
    const button = rail.getByRole("button", { name, exact: true });
    if (!(await button.count())) continue;
    await button.click();
    await page.waitForTimeout(300);
    const tabs = session.getByRole("tablist", { name: "Pages" }).getByRole("tab");
    const pages = (await tabs.count()) ? await tabs.allInnerTexts() : [""];
    for (const tab of pages) {
      if (tab) await session.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: tab.trim(), exact: true }).click();
      await page.waitForTimeout(400);
      const base = `${slug(name)}${tab ? `-${slug(tab)}` : ""}`;
      const parts = session.locator(".page-head .page-parts[role=tablist]").first();
      const partTabs = (await parts.count()) ? await parts.getByRole("tab").allInnerTexts() : [];
      if (!partTabs.length) {
        await shot(page, base);
        continue;
      }
      for (let i = 0; i < partTabs.length; i++) {
        await parts.getByRole("tab").nth(i).click();
        await page.waitForTimeout(400);
        await shot(page, `${base}-${slug(partTabs[i]!)}`);
      }
    }
  }
});

test("the matrix's states", async ({ page }) => {
  test.setTimeout(120_000);
  const session = await openDemo(page);
  await session.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Routing", exact: true }).click();
  await page.waitForTimeout(400);
  await shot(page, "matrix");
  const grid = session.getByRole("grid", { name: "Routing matrix" });
  if (!(await grid.count())) return;
  // A crosspoint, an output's column, an input's row in the inspector; Link on; dense
  await grid.getByRole("gridcell").first().click();
  await shot(page, "matrix-crosspoint");
  await grid.getByRole("gridcell").first().press("ArrowRight");
  await grid.getByRole("gridcell").first().press("Space").catch(() => undefined);
  await session.locator(".mx-colhead").nth(2).locator(".mx-colname").click();
  await shot(page, "matrix-output");
  await session.locator(".mx-rowhead").first().locator(".mx-rowname").click();
  await shot(page, "matrix-input");
  await session.getByRole("button", { name: "Link L/R pairs" }).click();
  await shot(page, "matrix-link");
  await session.getByRole("button", { name: "Link L/R pairs" }).click();
  await session.getByRole("button", { name: "Dense" }).click();
  await shot(page, "matrix-dense");
  await session.getByRole("button", { name: "Dense" }).click();
  await page.setViewportSize({ width: 1100, height: 720 });
  await shot(page, "matrix-1100");
});

test("no device", async ({ page }) => {
  await page.goto("/");
  await expect(picker(page)).toBeVisible();
  await shot(page, "nodevice-devices");
  // A DSPi that went away: its tab stays, with nothing to show but the way back
  const session = await openDemo(page);
  await session.getByRole("button", { name: "Disconnect" }).click();
  await page.waitForTimeout(400);
  await shot(page, "nodevice-closed");
  // A hub that does not answer: the session's tab with no device behind it
  await page.getByLabel("Add a hub").fill("127.0.0.1:9");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await picker(page).getByRole("listitem").filter({ hasText: "Hub 127.0.0.1:9" }).getByRole("button", { name: "Connect" }).click();
  await page.waitForTimeout(3000);
  await shot(page, "nodevice-hub-failed");
});
