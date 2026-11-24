// WP WEB7 e2e: Library's Presets tab as Qt Modern's, on the core's demo device
// (crates/dspi-wasm), in Chromium, Firefox and Chromium at 375 px (the `narrow` project). Every
// action is the page's; its effect is read back from the core. The rule REVE B1 found broken in
// the popover is pinned here for the tab: Copy to… is offered on the active row only and copies
// the active slot (the core's copy saves the LIVE settings into both slots), so a stored slot that
// is not active is never overwritten by it.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type Band = { gain: number };
type State = { outputGainDb: number[]; activePresetSlot: number; presetOccupied: number; presetNames: string[]; filters: Band[][] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const coreState = async (page: Page) => (await call<{ state: State }>(page, "state")).state;
const gain0 = async (page: Page) => (await coreState(page)).outputGainDb[0];

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function openDemo(page: Page, path = "/test/embed.html?one"): Promise<Locator> {
  await page.goto(path);
  if (path.startsWith("/test/embed.html")) {
    const one = page.locator("#one");
    await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
    const session = one.getByRole("article").first();
    await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible();
    return session;
  }
  await page.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article", { name: "DSPi" });
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible();
  return session;
}

async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

const slotRow = (screen: Locator, label: RegExp | string) => screen.getByRole("list", { name: "Preset slots" }).getByRole("listitem", { name: label });

/** The row's menu, opened; its rows' names. */
async function menuOf(session: Locator, row: Locator, label: string): Promise<Locator> {
  await row.getByRole("button", { name: `More, ${label}` }).click();
  return session.getByRole("menu", { name: `More, ${label}` });
}

test("WEB7 Presets: Qt's rows; Copy to… only on the active row, copying the active slot; a stored slot keeps what it holds", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Presets");
  const screen = session.getByRole("region", { name: "Presets" });
  await expect(screen).toContainText("At power-on the device loads preset 1.");
  await expect(screen.getByRole("list", { name: "Preset slots" }).getByRole("listitem")).toHaveCount(10);
  const one = slotRow(screen, /^1: Demo, Active/);
  await expect(one).toHaveAttribute("aria-label", "1: Demo, Active · Startup");
  await expect(slotRow(screen, "2: Empty")).toBeVisible();

  // Slot 3 gets the current settings (gain 0) by its menu's Copy "1: Demo" Here; slot 1 stays active.
  let menu = await menuOf(session, slotRow(screen, "3: Empty"), "3: Empty");
  await expect(menu.getByRole("menuitem", { name: "Copy to…" })).toHaveCount(0);
  await menu.getByRole("menuitem", { name: "Copy “1: Demo” Here" }).click();
  await expect.poll(async () => ((await coreState(page)).presetOccupied & 0b100) !== 0).toBe(true);
  expect((await coreState(page)).activePresetSlot).toBe(0);
  const three = slotRow(screen, "3: Preset 3");
  await expect(three).toBeVisible();

  // The live settings change (unsaved), and a stored slot that is not active offers no Copy to… (REVE B1).
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -7 } });
  await expect.poll(() => gain0(page)).toBe(-7);
  await expect(session.getByRole("button", { name: /^Presets: .*, unsaved changes$/ })).toBeVisible();
  menu = await menuOf(session, three, "3: Preset 3");
  await expect(menu.getByRole("menuitem", { name: "Copy to…" })).toHaveCount(0);
  await expect(menu.getByRole("menuitem", { name: "Recall", exact: true })).toBeEnabled();
  await page.keyboard.press("Escape");

  // Copy to… on the active row: the active slot's settings (saved first, as asked) go to slot 5.
  menu = await menuOf(session, slotRow(screen, /^1: Demo/), "1: Demo");
  await menu.getByRole("menuitem", { name: "Copy to…" }).click();
  const to = page.getByRole("dialog", { name: "Copy to…" });
  await expect(to).toContainText("Copy the current settings of “1: Demo” to");
  await expect(to.getByRole("combobox").getByRole("option")).toHaveCount(9);
  await to.getByRole("combobox").selectOption({ label: "5: Empty" });
  await to.getByRole("button", { name: "Copy" }).click();
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await prompt.getByRole("button", { name: "Save" }).click();
  await expect.poll(async () => ((await coreState(page)).presetOccupied & 0b10000) !== 0).toBe(true);
  expect((await coreState(page)).activePresetSlot).toBe(0);

  // Slot 3 still holds gain 0 (not the live -7), slot 5 holds -7.
  await three.getByRole("button", { name: "Recall 3: Preset 3" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(2);
  await expect.poll(() => gain0(page)).toBe(0);
  await slotRow(screen, "5: Preset 5").getByRole("button", { name: "Recall 5: Preset 5" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(4);
  await expect.poll(() => gain0(page)).toBe(-7);
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("WEB7 Presets: Save to Preset N in the page's tools saves the active slot; Clear… asks first", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Presets");
  const screen = session.getByRole("region", { name: "Presets" });
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -3 } });
  await expect(session.getByRole("button", { name: /^Presets: .*, unsaved changes$/ })).toBeVisible();
  await expect(slotRow(screen, /^1: Demo/)).toHaveAttribute("aria-label", "1: Demo, Active, with unsaved changes · Startup");
  await session.getByRole("toolbar", { name: "Presets tools" }).getByRole("button", { name: "Save to Preset 1" }).click();
  await expect(session.getByRole("button", { name: /^Presets: 1: Demo$/ })).toBeVisible();
  await expect(slotRow(screen, /^1: Demo/)).toHaveAttribute("aria-label", "1: Demo, Active · Startup");

  // Clear… names the slot and asks; Cancel keeps it.
  const menu = await menuOf(session, slotRow(screen, /^1: Demo/), "1: Demo");
  await menu.getByRole("menuitem", { name: "Clear “1: Demo”…" }).click();
  const ask = page.getByRole("dialog", { name: "Clear Preset?" });
  await expect(ask).toContainText("This cannot be undone.");
  await ask.getByRole("button", { name: "Cancel" }).click();
  expect((await coreState(page)).presetOccupied & 1).toBe(1);
  expect(errors).toEqual([]);
});

test("WEB7 Presets read only: the slots can be looked at, not used", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page, "/?readonly");
  await openPage(session, "Presets");
  const screen = session.getByRole("region", { name: "Presets" });
  await expect(screen.getByRole("list", { name: "Preset slots" }).getByRole("listitem")).toHaveCount(10);
  await expect(session.getByRole("toolbar", { name: "Presets tools" }).getByRole("button", { name: "Save to Preset 1" })).toBeDisabled();
  for (const b of await screen.getByRole("button", { name: /^(Recall .*|Save)$/ }).all()) await expect(b).toBeDisabled();
  const menu = await menuOf(session, slotRow(screen, /^1: Demo/), "1: Demo");
  for (const item of await menu.getByRole("menuitem").all()) await expect(item).toBeDisabled();
  expect(errors).toEqual([]);
});
