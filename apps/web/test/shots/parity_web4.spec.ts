// -- WP WEB4 -- Before/after pictures for the web's Measure, Analyse and Library pages (web
// versions of DL2 and DL3a): every part of those pages on the core's demo device, empty and with
// content (a measurement on the demo's simulated microphone, the live view, the history, a
// snapshot), dark and light. Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/parity_web4.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `<project>-web4-<name>-<scheme>.png` in $SHOTS_DIR. Only pictures; a step that cannot
// find what it pictures is skipped (the "before" run has other words on some buttons).

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-status", "open");
  return session;
}

async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  // The pointer away from the tabs and rows, so nothing shows its hover
  await page.mouse.move(2, page.viewportSize()!.height - 2);
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/${project}-web4-${name}-${scheme}.png`, fullPage: false });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** `fn`, or nothing when what it needs is not there (a few seconds at most). */
async function maybe(what: string, fn: () => Promise<unknown>): Promise<boolean> {
  try {
    await fn();
    return true;
  } catch (e) {
    console.log(`skipped: ${what}: ${String(e).split("\n")[0]}`);
    return false;
  }
}

async function go(session: Locator, section: string, tab?: string, part?: string) {
  await session.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: section, exact: true }).click({ timeout: 5000 });
  if (tab) await session.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: tab, exact: true }).click({ timeout: 5000 });
  if (part) await session.locator(".page-head .page-parts[role=tablist]").first().getByRole("tab", { name: part, exact: true }).click({ timeout: 5000 });
  await session.page().waitForTimeout(500);
}

const T = { timeout: 4000 };

test("Measure, Analyse and Library, empty and with content", async ({ page }) => {
  test.setTimeout(480_000);
  const session = await openDemo(page);

  // Empty: every part as it opens
  await go(session, "Measure");
  const parts = await session.locator(".page-head .page-parts[role=tablist]").first().getByRole("tab").allInnerTexts();
  for (const p of parts.map((s) => s.trim()).filter(Boolean)) {
    await go(session, "Measure", undefined, p);
    await shot(page, `measure-${p.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`);
  }
  for (const t of ["Analyser", "Spectrum", "Signal Generator", "Stats for nerbs", "Interrupt Monitor"]) {
    if (!(await maybe(`Analyse › ${t}`, () => go(session, "Analyse", t)))) continue;
    await shot(page, `analyse-${t.toLowerCase().replace(/[^a-z0-9]+/g, "-")}`);
  }
  for (const t of ["Snapshots", "History", "Files"]) {
    await go(session, "Library", t);
    await shot(page, `library-${t.toLowerCase()}`);
  }

  // The demo's volume to −20 dB (the preflight's limit), on the dock's slider
  await maybe("volume", () => session.getByRole("slider", { name: "Volume", exact: true }).fill("89", T));

  // Measure: the placement, a short sweep, Check, the run, its results
  await go(session, "Measure", undefined, "Measure");
  await maybe("placement", () => session.getByRole("radio", { name: "Listening position" }).first().click(T));
  await maybe("sweep", () => session.getByLabel("Sweep length").fill("1", T));
  await maybe("check", () => session.getByRole("button", { name: "Check", exact: true }).click(T));
  await page.waitForTimeout(800);
  await shot(page, "measure-checked");
  const start = session.getByRole("region", { name: "Measurement setup" }).getByRole("button", { name: /^(Start measuring|Measure)$/ });
  if (await maybe("start", () => start.first().click(T))) {
    await page.waitForTimeout(2500);
    await shot(page, "measure-running");
    await maybe("run end", () => expect(session.getByRole("status", { name: "Measuring" })).toHaveCount(0, { timeout: 150_000 }));
    await page.waitForTimeout(800);
    await shot(page, "measure-done");
    await go(session, "Measure", undefined, "Results");
    await page.waitForTimeout(1200);
    await shot(page, "measure-results-curves");
    await go(session, "Measure", undefined, "Auto-tune");
    await page.waitForTimeout(1200);
    await shot(page, "measure-auto-tune-measured");
    if (await maybe("propose", () => session.getByRole("button", { name: /^Propose a [Tt]une$/ }).click(T))) {
      await page.waitForTimeout(4000);
      await shot(page, "measure-auto-tune-proposal");
    }
    await go(session, "Measure", undefined, "Documents");
    await page.waitForTimeout(1000);
    await shot(page, "measure-documents-list");
  }

  // Live: the placement, then a few sweeps
  await go(session, "Measure", undefined, "Live");
  await maybe("live placement", () => session.getByLabel("Live placement", { exact: true }).selectOption("listeningPosition", T));
  if (await maybe("start live", () => session.getByRole("button", { name: /^Start [Ll]ive$/ }).click(T))) {
    await page.waitForTimeout(6000);
    await shot(page, "measure-live-running");
    await maybe("stop live", () => session.getByRole("button", { name: /^Stop [Ll]ive$/ }).click(T));
    await page.waitForTimeout(800);
  }

  // The analyser with its bars, the generator with a sweep chosen
  if (await maybe("analyser", () => go(session, "Analyse", "Analyser"))) {
    await maybe("show bars", () => session.getByRole("button", { name: "Show Bars" }).click({ timeout: 1500 }));
    await page.waitForTimeout(1500);
    await shot(page, "analyse-analyser-running");
  }

  // Library with content: the history of the above, a snapshot captured
  await go(session, "Library", "History");
  await page.waitForTimeout(600);
  await shot(page, "library-history-list");
  await go(session, "Library", "Snapshots");
  if (await maybe("capture", () => session.getByRole("button", { name: /^Capture/ }).first().click(T))) {
    await page.waitForTimeout(400);
    // The capture popover (after) asks for a name first
    await maybe("capture name", () => session.getByRole("dialog", { name: /Capture/ }).getByRole("button", { name: "Capture" }).click({ timeout: 1500 }));
    await page.waitForTimeout(1000);
    await shot(page, "library-snapshots-list");
    await maybe("hover snapshot", () => session.getByRole("list", { name: "Held snapshots" }).getByRole("listitem").first().hover({ timeout: 1500 }));
    await page.emulateMedia({ colorScheme: "dark" });
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${process.env.SHOTS_DIR ?? "test-results/shots"}/${test.info().project.name}-web4-library-snapshots-hover-dark.png` });
  }
});
