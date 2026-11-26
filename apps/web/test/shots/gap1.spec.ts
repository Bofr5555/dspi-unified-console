// -- WP GAP1 -- Before/after pictures of the web gaps GAP1 closes: the EQ graph's Phase chip (and
// the phase drawn), Effects › Crossfeed's Outputs pair map (and its Custom editor), the matrix
// inspector's output Polarity and crosspoint line, and Delay & Gain's Polarity column. Run alone
// through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/gap1.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `<project>-gap1-<name>-<scheme>.png` in $SHOTS_DIR. Only pictures; a step that cannot
// find what it pictures is skipped (the "before" run has none of the new controls).

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-status", "open");
  return session;
}

async function shot(page: Page, name: string) {
  const only = process.env.SHOTS_ONLY;
  if (only && !new RegExp(only).test(name)) return;
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  await page.mouse.move(2, page.viewportSize()!.height - 2);
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(300);
    await page.screenshot({ path: `${dir}/${project}-gap1-${name}-${scheme}.png`, fullPage: false });
  }
  await page.emulateMedia({ colorScheme: null });
}

async function maybe(what: string, fn: () => Promise<unknown>): Promise<boolean> {
  try {
    await fn();
    return true;
  } catch (e) {
    console.log(`skipped: ${what}: ${String(e).split("\n")[0]}`);
    return false;
  }
}

const T = { timeout: 3000 };

test("EQ phase, crossfeed pairs, output polarity", async ({ page }) => {
  test.setTimeout(300_000);
  const session = await openDemo(page);

  // Equaliser › PEQ on Out 1 with a band, then the phase over it
  await openPage(session, "EQ");
  await maybe("out 1", () => session.getByRole("group", { name: "Channels" }).getByRole("button", { name: /^Out 1/ }).first().click(T));
  const graph = session.getByRole("application", { name: /^EQ graph/ });
  await maybe("add a band", async () => {
    const box = (await graph.boundingBox({ timeout: 3000 }))!;
    await page.mouse.dblclick(box.x + box.width * 0.45, box.y + box.height * 0.35);
  });
  await page.waitForTimeout(500);
  await page.keyboard.press("Escape");
  await shot(page, "eq-graph");
  if (await maybe("phase chip", () => session.getByRole("group", { name: "Graph options" }).getByRole("button", { name: "Phase", exact: true }).click(T))) {
    await page.waitForTimeout(600);
    await shot(page, "eq-graph-phase");
    await session.getByRole("group", { name: "Graph options" }).getByRole("button", { name: "Phase", exact: true }).click();
  }

  // Effects › Crossfeed, on; then its Custom pair editor
  await openPage(session, "Effects");
  await maybe("crossfeed", () => session.locator(".page-head .page-parts[role=tablist]").first().getByRole("tab", { name: /^Crossfeed/ }).click(T));
  const panel = session.getByRole("tabpanel").last();
  await maybe("crossfeed on", () => panel.locator(".fx-intro").getByRole("switch").last().click(T));
  await page.waitForTimeout(800);
  await maybe("output pairs", () => panel.getByText("Output Pairs", { exact: true }).scrollIntoViewIfNeeded(T));
  await shot(page, "crossfeed");
  if (await maybe("outputs menu", () => panel.getByRole("group", { name: "Crossfeed outputs" }).getByRole("button", { name: /^Default/ }).click(T))) {
    await page.waitForTimeout(300);
    await shot(page, "crossfeed-outputs-menu");
    if (await maybe("custom", () => page.getByRole("menuitem", { name: /^Custom/ }).click(T))) {
      await page.waitForTimeout(500);
      await maybe("editor", () => panel.getByRole("group", { name: /pair map editor/i }).scrollIntoViewIfNeeded(T));
      await shot(page, "crossfeed-custom");
      await maybe("cancel", () => panel.getByRole("button", { name: "Cancel", exact: true }).click(T));
    }
  }

  // Routing › Matrix: an output's column, then a connected crosspoint
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  await maybe("column", () => matrix.locator(".mx-colhead").first().locator(".mx-colname").click(T));
  await page.waitForTimeout(400);
  await shot(page, "matrix-output");
  await maybe("cell", () => matrix.locator('[role="gridcell"][data-connected="true"]').first().click(T));
  await page.waitForTimeout(400);
  await shot(page, "matrix-crosspoint");

  // Equaliser › Delay & Gain
  await openPage(session, "Delay & Gain");
  await page.waitForTimeout(500);
  await shot(page, "delays");
});
