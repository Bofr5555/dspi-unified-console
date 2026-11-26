// WP UB34 e2e (UXP B3, owner D7): the top bar's A | B on the core's demo device. Hidden with nothing
// unsaved; after an edit it shows with B (now) pressed and says what A and B are; A recalls the
// preset as saved, the compare's bar names the sides, the plain B goes back to the edit. With
// Chromium, pictures of each step in the dark and the light scheme go to the output folder's ub34-shots.

import { expect, test, type Locator, type Page } from "@playwright/test";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const gain0 = async (page: Page) => (await call<{ state: { outputGainDb: number[] } }>(page, "state")).state.outputGainDb[0]!;

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible();
  return session;
}

async function shots(page: Page, name: string) {
  if (test.info().project.name !== "chromium") return;
  const dir = `${process.env.PW_OUTPUT_DIR ?? "test-results"}/ub34-shots`;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/web-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

test("UB34: A | B in the top bar is as saved against now from the first edit", async ({ page }) => {
  const session = await openDemo(page);
  const ab = page.getByRole("group", { name: "A/B toggle" });
  await expect(ab).toHaveCount(0);
  await shots(page, "before-edit");

  const saved = await gain0(page);
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: saved - 5 } });
  await page.waitForTimeout(500);
  await shots(page, "edited"); // before UB34: no A | B here
  await expect(ab).toBeVisible();
  await expect(ab.getByRole("button", { name: "B, now" })).toHaveAttribute("aria-pressed", "true");
  await expect(ab).toHaveAttribute("title", /A is the preset as saved on the device, B is now/);
  await expect(ab).toHaveAttribute("aria-keyshortcuts", "B");
  await shots(page, "offered");

  await ab.getByRole("button", { name: "A, as saved" }).click();
  await expect(ab.getByRole("button", { name: "A, as saved" })).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => gain0(page)).toBe(saved);
  const bar = session.getByRole("group", { name: "A/B compare" });
  await expect(bar).toContainText("Now A");
  await expect(bar).toContainText("As saved");
  await expect(ab).toHaveAttribute("title", /you hear A/);
  await shots(page, "on-a");

  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("b");
  await expect(bar).toContainText("Now B");
  await expect.poll(() => gain0(page)).toBe(saved - 5);
  await bar.getByRole("button", { name: "Keep B" }).click();
  await expect(bar).toHaveCount(0);
  await expect(ab.getByRole("button", { name: "B, now" })).toHaveAttribute("aria-pressed", "true");
});
