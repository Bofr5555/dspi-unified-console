// -- WP WEB6 -- Before/after pictures of the pages WEB6 ports to Qt Modern: Analyse › Spectrum,
// every Effects page (off, then switched on so its graph draws), Equaliser › PEQ (no band, a band
// chosen) and Crossover. Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/web6.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `<project>-web6-<name>-<scheme>.png` in $SHOTS_DIR. Only pictures; a step that cannot
// find what it pictures is skipped (the "before" run has other controls).

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session.locator(".device-meta")).toBeVisible();
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
    await page.screenshot({ path: `${dir}/${project}-web6-${name}-${scheme}.png`, fullPage: false });
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

async function go(session: Locator, section: string, tab?: string) {
  await session.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: section, exact: true }).click({ timeout: 5000 });
  if (tab) await session.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: tab, exact: true }).click({ timeout: 5000 });
  await session.page().waitForTimeout(500);
}

const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");
const T = { timeout: 3000 };

test("Spectrum, Effects, Equaliser PEQ and Crossover", async ({ page }) => {
  test.setTimeout(480_000);
  const session = await openDemo(page);

  // Analyse › Spectrum, running on the demo's analyser
  await go(session, "Analyse", "Spectrum");
  await page.waitForTimeout(2500);
  await shot(page, "analyse-spectrum");
  const view = session.getByRole("group", { name: "View" });
  if (await maybe("bars", () => view.getByRole("button", { name: "Bars", exact: true }).click(T))) {
    await page.waitForTimeout(1500);
    await shot(page, "analyse-spectrum-bars");
    await maybe("both", () => view.getByRole("button", { name: "Both", exact: true }).click(T));
    await page.waitForTimeout(1500);
    await shot(page, "analyse-spectrum-both");
    await maybe("curves", () => view.getByRole("button", { name: "Curves", exact: true }).click(T));
  }

  // Effects: every part, off and on
  await go(session, "Effects");
  const parts = session.locator(".page-head .page-parts[role=tablist]").first();
  // A tab's text may carry its state dot's words ("Crossfeed off"): the name is the rest
  const names = (await parts.getByRole("tab").allInnerTexts()).map((t) => t.replace(/\b(on|off)\s*$/i, "").trim());
  for (const [i, name] of names.entries()) {
    await parts.getByRole("tab").nth(i).click();
    await page.waitForTimeout(500);
    await shot(page, `effects-${slug(name)}-off`);
    const panel = session.getByRole("tabpanel").last();
    const sw = panel.locator(".fx-intro").getByRole("switch").last();
    if (await maybe(`switch ${name}`, () => sw.click(T))) {
      await page.waitForTimeout(800);
      await shot(page, `effects-${slug(name)}-on`);
      await maybe(`switch ${name} back`, () => sw.click(T));
      await page.waitForTimeout(300);
    }
  }

  // Equaliser › PEQ with no band chosen, then band 1; Crossover
  await go(session, "Equaliser", "PEQ");
  await page.waitForTimeout(600);
  await shot(page, "equaliser-peq");
  // Out 1, then a band: the demo's Out 1 has none, so a double click on the graph adds one (it is chosen)
  await maybe("out 1", () => session.getByRole("group", { name: "Channels" }).getByRole("button", { name: /^Out 1/ }).first().click(T));
  await page.waitForTimeout(400);
  await shot(page, "equaliser-peq-out1");
  const graph = session.getByRole("application", { name: /^EQ graph/ });
  const added = await maybe("add a band", async () => {
    const box = (await graph.boundingBox({ timeout: 3000 }))!;
    await page.mouse.dblclick(box.x + box.width * 0.45, box.y + box.height * 0.35);
  });
  if (added || (await maybe("band 1", () => session.getByRole("button", { name: "Band 1", exact: true }).first().click(T)))) {
    await page.waitForTimeout(500);
    await shot(page, "equaliser-peq-band");
    await page.keyboard.press("Escape");
  }
  await go(session, "Equaliser", "Crossover");
  await page.waitForTimeout(600);
  await shot(page, "equaliser-crossover");
});
