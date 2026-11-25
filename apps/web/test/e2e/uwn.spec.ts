// WP UWN e2e (UXP A15, A16, A17) on the core's demo device: the Crossover opens on an output and says which outputs
// are off (Turn On); one PEQ vocabulary; accessible names that read right, a one-stop rail and skip links; Settings
// with no DSPi open.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, rail } from "../nav";

type State = { numInputChannels: number; outputEnabled: boolean[]; activePresetSlot: number };
type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function core(page: Page): Promise<State> {
  return ((await page.evaluate(() => (globalThis as unknown as Clients).dspiClients[0]!.call("state", null))) as { state: State }).state;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: /^Connect/ }).click();
  const session = one.getByRole("article").first();
  await expect(session).toHaveAttribute("data-status", "open");
  return session;
}

test("UWN A17: the Crossover opens on an output even after an input was chosen; an off output says Off with Turn On", async ({ page }) => {
  const session = await openDemo(page);
  await openPage(session, "EQ");
  // An input chosen on the PEQ tab (the first channel of the list is an input)
  await session.locator(".eq-pick").first().click();
  await expect(session.locator(".eq-pick").first()).toHaveAttribute("aria-pressed", "true");
  await openPage(session, "Crossover");
  const table = session.getByRole("region", { name: "Crossovers of every output" });
  const s = await core(page);
  const first = s.outputEnabled.findIndex((on) => on);
  await expect(table.locator(".xo-out[aria-pressed=true]")).toContainText(`OUT${first + 1}`);
  // The frequency beside every pass of an output that is on, "–" while the pass is off
  await expect(table.getByRole("textbox", { name: new RegExp(`^OUT${first + 1} .* low pass frequency$`) })).toBeVisible();
  // An output that is off: no pass fields, "Off" and Turn On, which turns it on (Delay & Gain's write)
  const off = s.outputEnabled.findIndex((on, i) => !on && i < s.outputEnabled.length - 1);
  test.skip(off < 0, "the demo has every output on");
  const row = table.getByRole("row").filter({ has: page.getByRole("button", { name: new RegExp(`^Turn On OUT${off + 1} `) }) });
  await expect(row).toContainText("Off");
  await expect(row.getByRole("combobox")).toHaveCount(0);
  await row.getByRole("button", { name: /^Turn On/ }).click();
  await expect.poll(async () => (await core(page)).outputEnabled[off]).toBe(true);
  await expect(table.getByRole("combobox", { name: new RegExp(`^OUT${off + 1} .* high pass$`) })).toBeVisible();
  // The legend before the table that uses its short forms
  await expect(table.locator(".xo-legend")).toHaveText("LR is Linkwitz-Riley, BW Butterworth, BS Bessel.");
});

test("UWN A17: the band table and the inspector say one vocabulary: Bell, Low Cut 12 dB/oct, Q", async ({ page }) => {
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const bands = session.getByRole("region", { name: /^Bands of / });
  await expect(bands.getByRole("columnheader", { name: "Q" })).toBeVisible();
  await expect(bands.getByRole("columnheader", { name: "Width" })).toHaveCount(0);
  const type = bands.getByRole("combobox", { name: "Band 1 type" });
  await expect(type.locator("option")).toContainText(["Off", "Bell"]);
  expect(await type.locator("option").allInnerTexts()).not.toContain("Peaking");
  expect(await type.locator("option").allInnerTexts()).toContain("Low Cut 12 dB/oct");
});

test("UWN A15: Save to Device's name has its count in words; the rail is one Tab stop with arrow keys; skip links", async ({ page }) => {
  const session = await openDemo(page);
  const save = session.getByRole("button", { name: /^Save to Device/ });
  await expect(save).toHaveAccessibleName("Save to Device");
  await openPage(session, "Delay & Gain");
  const gain = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("row", { name: /^OUT1 / }).getByRole("textbox", { name: /gain$/ });
  await gain.fill("-3");
  await gain.press("Enter");
  await expect(save).toHaveAccessibleName(/^Save to Device, \d+ unsaved changes?$/);
  // One stop: the section on show; the arrows move between sections, Enter opens one
  const sections = rail(session).locator(".rail-list > button");
  await expect(rail(session).locator('.rail-list > button[tabindex="0"]')).toHaveCount(1);
  await expect(rail(session).locator('.rail-list > button[tabindex="0"]')).toHaveAttribute("aria-current", "page");
  await rail(session).locator('.rail-list > button[tabindex="0"]').focus();
  await page.keyboard.press("ArrowDown");
  await expect(sections.nth(1)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(sections.nth(1)).toHaveAttribute("aria-current", "page");
  await page.keyboard.press("End");
  await expect(sections.last()).toBeFocused();
  // The skip links: the first Tab stops, to the page and to the volume
  await page.locator("body").focus();
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "Skip to the page" })).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(page.getByRole("link", { name: "Skip to the volume" })).toBeFocused();
  await page.keyboard.press("Enter");
  expect(await page.evaluate(() => !!document.activeElement?.closest(".dock"))).toBe(true);
});

test("UWN A15: the start page's Connect buttons say what they connect", async ({ page }) => {
  await page.goto("/");
  const nav = page.getByRole("navigation", { name: "DSPi devices" });
  await expect(nav.getByRole("button", { name: "Connect Demo DSPi" })).toBeVisible();
  await expect(nav.getByRole("button", { name: "Connect", exact: true })).toHaveCount(0);
});

test("UWN A16: Settings with no DSPi open shows the console's own parts (Application, Display)", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("navigation", { name: "Sections" }).getByRole("button", { name: "Settings", exact: true }).click();
  const parts = page.getByRole("tablist", { name: "Settings" });
  await expect(parts.getByRole("tab")).toHaveText(["Application", "Display"]);
  await expect(page.getByRole("tabpanel", { name: "Appearance" })).toBeVisible();
  await parts.getByRole("tab", { name: "Display" }).click();
  await expect(page.getByRole("navigation", { name: "Settings pages" }).getByRole("button")).toHaveText(["Graphing", "Spectrum Analyser"]);
  await page.getByRole("navigation", { name: "Settings pages" }).getByRole("button", { name: "Graphing" }).click();
  await expect(page.getByRole("tabpanel", { name: "Graphing" })).toBeVisible();
  await expect(page.getByText("No device", { exact: true })).toHaveCount(0);
});
