// WP UB2 e2e (UXP B2, D4): Home, the remote, on the core's demo device in every project (Chromium,
// Firefox, Chromium at 375 px without sideways scroll). Simple opens on it: one big volume, Mute,
// the source as segments, the presets as a Recall list; the dock has no Master and no Bypass EQ.
// Advanced has no Home. The page names a web app manifest. The locked user's Home is
// p1wb2.spec.ts's lock test (through `uconsole --demo hub` with a PIN in its store).

import { mkdirSync } from "node:fs";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { expectPage, rail } from "../nav";
import { presetsReady } from "../presets";

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await presetsReady(session);
  return session;
}

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

/** Pictures for the report (Chromium and narrow), when UB2_SHOTS names a directory. */
async function shot(page: Page, name: string) {
  const dir = process.env.UB2_SHOTS;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/ub2-web-${project}-${name}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

test("UB2 B2: Simple is a remote: Home first with volume, Mute, source segments and a Recall list; no Master, no Bypass EQ", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // Advanced (the web's default): no Home, the dock as before
  await expect(session.getByRole("tab", { name: "Home", exact: true })).toHaveCount(0);
  await expect(session.getByRole("button", { name: "Bypass EQ", exact: true })).toBeVisible();

  await setMode(session, page, "Simple");
  // The session opens on Home in Simple: a fresh view, as a phone opens it
  await page.reload();
  const again = page.locator("#one");
  await again.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const s = again.getByRole("article").first();
  await presetsReady(s);
  await expect(s.getByRole("button", { name: "Mode: Simple" })).toBeVisible();
  // The picture first: on a tree without Home it is the "before" (Simple opening on the Dashboard)
  await s.locator(".page[data-page]").waitFor();
  await shot(page, "simple-open");
  await expectPage(s, "Home");
  await expect(rail(s).getByRole("button", { name: "Overview", exact: true })).toHaveAttribute("aria-current", "page");

  const home = s.locator(".home-screen");
  await expect(home.getByRole("heading", { level: 2 })).not.toHaveText("");
  await expect(home.getByRole("status").first()).toContainText("Connected");
  // One big User volume; a move is written and read back
  const volume = home.getByRole("slider", { name: "Volume" });
  const before = await home.locator(".home-volume-value").textContent();
  await volume.fill("150");
  await volume.dispatchEvent("change");
  await expect(home.locator(".home-volume-value")).not.toHaveText(before!);
  // Mute flips and shows it
  const mute = home.getByRole("button", { name: "Mute", exact: true });
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  await mute.click();
  await expect(mute).toHaveAttribute("aria-pressed", "false");
  // The source as segments, the chosen one pressed
  const source = home.getByRole("group", { name: "Source" });
  await expect(source.locator('button[aria-pressed="true"]')).toHaveCount(1);
  // The presets as a Recall list: the one that plays pressed, no Save, no Copy Here
  const presets = home.getByRole("region", { name: "Presets" });
  await expect(presets.locator('button[aria-pressed="true"]')).toHaveCount(1);
  await expect(presets.getByRole("button", { name: /^Save|Copy Here/ })).toHaveCount(0);
  // One health line
  await expect(home.locator(".home-health")).toHaveText(/^Health: /);
  // The dock: the User volume and Mute, no Master, no Bypass EQ
  const dock = s.locator("footer.dock");
  await expect(dock.getByRole("button", { name: "Master", exact: true })).toHaveCount(0);
  await expect(dock.getByRole("button", { name: "Bypass EQ", exact: true })).toHaveCount(0);
  await expect(dock.getByRole("button", { name: "Mute", exact: true })).toBeVisible();

  // Recall: another stored preset, when the demo has one, becomes the one that plays
  const others = presets.locator('button[aria-pressed="false"]');
  if (await others.count()) {
    const next = others.first();
    const name = (await next.getAttribute("aria-label"))!;
    await next.click();
    const dialog = page.getByRole("dialog", { name: "Unsaved Changes" });
    if (await dialog.count()) await dialog.getByRole("button", { name: "Discard" }).click();
    await expect(presets.getByRole("button", { name })).toHaveAttribute("aria-pressed", "true");
  }

  // Simple hides nothing permanently: the Dashboard is a tab away
  await expect(s.getByRole("tab", { name: "Dashboard", exact: true })).toBeVisible();
  if (test.info().project.name === "narrow") {
    const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(over).toBeLessThanOrEqual(0);
  }
  await setMode(s, page, "Advanced");
  await expect(s.getByRole("tab", { name: "Home", exact: true })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("UB2 B2: Add to Home Screen gives an app: the page names its manifest and theme colours", async ({ page, request }) => {
  await page.goto("/");
  await expect(page.locator('link[rel="manifest"]')).toHaveAttribute("href", "manifest.json");
  await expect(page.locator('meta[name="theme-color"]')).toHaveCount(2);
  const m = (await (await request.get("/manifest.json")).json()) as { display: string; icons: { src: string }[] };
  expect(m.display).toBe("standalone");
  for (const i of m.icons) expect((await request.get(`/${i.src}`)).ok()).toBe(true);
});
