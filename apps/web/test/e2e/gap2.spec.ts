// WP GAP2 e2e: "Solo this output" on the core's demo device. The Routing inspector's Solo switch solos the
// column's output (the core says so, the state keeps the user's mutes), the strip names it and its End Solo
// ends it; the EQ's channel menu has Solo / End Solo. Pictures (chromium, dark and light) go to gap2-shots/.

import { mkdirSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

const call = (page: Page, m: string) => page.evaluate((m) => (globalThis as unknown as G).dspiClients[0]!.call(m, null), m);
const soloed = async (page: Page) => ((await call(page, "soloState")) as { output: number | null }).output;
const mutes = async (page: Page) => ((await call(page, "state")) as { state: { outputMuted: boolean[] } }).state.outputMuted.filter(Boolean).length;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function shot(page: Page, name: string) {
  const out = process.env.PW_OUTPUT_DIR;
  if (!out || test.info().project.name !== "chromium") return;
  const dir = `${out}/../gap2-shots`;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

test("GAP2: Solo in the Routing inspector and the channel menu; the strip says it and ends it", async ({ page }) => {
  const errors = errorsOf(page);
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session).toHaveAttribute("data-status", "open");
  const before = await mutes(page);
  const strip = session.getByRole("status", { name: "Solo" });

  // The Routing inspector: an output's column, its Solo switch
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  const inspector = session.getByRole("complementary", { name: "Inspector" });
  await matrix.locator(".mx-colhead").nth(1).locator(".mx-colname").click();
  const sw = inspector.getByRole("switch", { name: "Solo" });
  await expect(sw).toHaveAttribute("aria-checked", "false");
  await shot(page, "routing-no-solo");
  await sw.click();
  await expect(sw).toHaveAttribute("aria-checked", "true");
  await expect.poll(() => soloed(page)).toBe(1);
  expect(await mutes(page)).toBe(before);
  await expect(strip).toContainText("Solo: only ");
  await expect(strip).toContainText("nothing is saved");
  await shot(page, "routing-solo");
  // Another output's switch moves it
  await matrix.locator(".mx-colhead").nth(0).locator(".mx-colname").click();
  await expect(sw).toHaveAttribute("aria-checked", "false");
  await sw.click();
  await expect.poll(() => soloed(page)).toBe(0);
  // The strip's End Solo
  await strip.getByRole("button", { name: "End Solo" }).click();
  await expect.poll(() => soloed(page)).toBe(null);
  await expect(strip).toHaveCount(0);
  await expect(sw).toHaveAttribute("aria-checked", "false");

  // The EQ's channel menu
  await openPage(session, "EQ");
  const out1 = session.getByRole("group", { name: "Channels" }).getByRole("button", { name: /OUT1/ }).first();
  await out1.click({ button: "right" });
  let menu = one.getByRole("menu", { name: /^OUT1/ });
  await expect(menu.getByRole("menuitem", { name: "Solo", exact: true })).toBeVisible();
  await shot(page, "menu-solo");
  await menu.getByRole("menuitem", { name: "Solo", exact: true }).click();
  await expect.poll(() => soloed(page)).toBe(0);
  await expect(strip).toBeVisible();
  await out1.click({ button: "right" });
  menu = one.getByRole("menu", { name: /^OUT1/ });
  await expect(menu.getByRole("menuitem", { name: "End Solo" })).toBeVisible();
  await shot(page, "menu-end-solo");
  await menu.getByRole("menuitem", { name: "End Solo" }).click();
  await expect.poll(() => soloed(page)).toBe(null);
  await expect(strip).toHaveCount(0);
  expect(await mutes(page)).toBe(before);
  expect(errors).toEqual([]);
});
