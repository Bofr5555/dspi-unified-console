// WP UB12 e2e (UXP B12, owner D12): the demo every page opens is the core's tuned example (a stereo
// two-way on outputs 1-4, named, crossed over, its roles confirmed), and the demo strip's Reset Demo
// (also the palette's) puts a change back with nothing unsaved. The other e2e tests' pages open the
// blank demo (playwright.config.ts): this file clears that key. Pictures, light and dark, of the
// blank demo (before) and the example (after) in the test's output folder.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };
type State = { outputGainDb: number[] };

const mod = process.platform === "darwin" ? "Meta" : "Control";

function call(page: Page, method: string, params: unknown = null) {
  return page.evaluate(([m, p]) => (globalThis as unknown as G).dspiClients[0]!.call(m as string, p), [method, params] as const);
}

async function gain(page: Page, output: number): Promise<number> {
  const snap = (await call(page, "state")) as { state: State };
  return snap.state.outputGainDb[output]!;
}

/** Console one alone, on Venue A's demo, on its Overview. */
async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await openPage(session, "Overview");
  await expect(session.getByRole("region", { name: "Overview" })).toBeVisible();
  return session;
}

/** Pictures on the standalone page (the embedding page's own styles colour its buttons). */
async function shot(page: Page, name: string) {
  if (test.info().project.name === "firefox") return;
  await page.goto("/");
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article", { name: "DSPi" });
  await expect(session).toHaveAttribute("data-serial", "DEMO0000DSPI0001");
  await openPage(session, "Overview");
  await page.waitForTimeout(300);
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(200);
    await page.screenshot({ path: test.info().outputPath(`ub12-${test.info().project.name}-${name}-${scheme}.png`), fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

test.describe("the demo every page opens", () => {
  test.use({ storageState: { cookies: [], origins: [] } });

  test("UB12: the tuned example; the strip's Reset Demo and the palette's put a change back", async ({ page }) => {
    const session = await openDemo(page);
    const overview = session.getByRole("region", { name: "Overview" });
    for (const name of ["Left woofer", "Right woofer", "Left tweeter", "Right tweeter"]) await expect(overview.getByText(name).first()).toBeVisible();
    expect(await gain(page, 2)).toBe(-3);

    // The strip's button
    const reset = session.locator(".demo-strip").getByRole("button", { name: "Reset Demo" });
    await expect(reset).toBeVisible();
    await call(page, "write", { path: "out.gain", indices: [2], value: { float: -9 } });
    expect(await gain(page, 2)).toBe(-9);
    await reset.click();
    await expect.poll(() => gain(page, 2)).toBe(-3);
    await expect(overview.getByText("Left woofer").first()).toBeVisible();

    // The palette's row
    await call(page, "write", { path: "out.gain", indices: [2], value: { float: -9 } });
    await page.keyboard.press(`${mod}+k`);
    const palette = page.getByRole("dialog", { name: "Command palette" });
    await palette.getByRole("combobox", { name: "Search settings, channels, commands" }).fill("reset demo");
    await expect(palette.getByRole("option", { name: /^Reset Demo/ })).toBeVisible();
    await page.keyboard.press("Enter");
    await expect.poll(() => gain(page, 2)).toBe(-3);
    await shot(page, "after");
  });
});

test("UB12: the blank demo the other tests open has the strip's Reset Demo too", async ({ page }) => {
  const session = await openDemo(page);
  await expect(session.getByRole("region", { name: "Overview" }).getByText("Left woofer")).toHaveCount(0);
  await expect(session.locator(".demo-strip").getByRole("button", { name: "Reset Demo" })).toBeVisible();
  await shot(page, "before");
});
