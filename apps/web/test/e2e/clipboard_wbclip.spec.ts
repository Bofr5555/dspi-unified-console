// WP WBCLIP e2e: the channel clipboard (M45) on the core's demo device, in
// Chromium, Firefox and Chromium at 375 px. The EQ screen's Channel menu and
// the matrix's column menu copy a channel's parameters and paste them onto
// another, one undo step; Ctrl/Cmd+C and +V do the same on the EQ screen's
// chosen channel (a linked input's partner too), never inside a text field;
// a view-only session can copy but never paste. Every result is read back
// from the core's own state (a direct call).

import { expect, test, type Locator, type Page } from "@playwright/test";
import { appMenu, openPage } from "../nav";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number; qp: number };
type State = { numInputChannels: number; filters: Band[][]; outputGainDb: number[] };
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function core(page: Page): Promise<State> {
  const snap = (await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("state", null))) as { state: State };
  return snap.state;
}

/** A band and a gain on OUT1, made in the core directly (copy reads the core's mirror). */
async function shapeOut1(page: Page) {
  await page.evaluate(async () => {
    const c = (globalThis as unknown as G).dspiClients[0]!;
    const s = ((await c.call("state", null)) as { state: State }).state;
    await c.call("filter", { channel: s.numInputChannels, band: 2, filterType: 1, bypass: false, freq: 315, q: 1.4, gain: 5, qp: 0.707 });
    await c.call("write", { path: "out.gain", indices: [0], value: { float: -7 } });
  });
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function openEq(page: Page, venue = "Venue A") {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await openPage(session, "EQ");
  await expect(session.locator(".eq-pick").first()).toBeVisible();
  return session;
}

/** Choose a channel on the EQ screen, and wait until it shows chosen (the keys act on it from then). */
async function pick(session: Locator, desc: string) {
  const b = session.locator(".eq-pick", { hasText: desc });
  await b.click();
  await expect(b).toHaveAttribute("aria-pressed", "true");
}
const channelMenu = (session: Locator) => session.locator(".eq-tools").getByRole("button", { name: "Channel" });
const item = (session: Locator, name: string) => session.getByRole("menuitem", { name });
const undo = (s: Locator) => s.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" });

test("WBCLIP M45: the EQ screen's Channel menu copies OUT1 and pastes onto OUT2, one undo step", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openEq(page);
  await shapeOut1(page);
  const s0 = await core(page);
  const out2 = s0.numInputChannels + 1;
  const before = { gain: s0.outputGainDb[1], bands: s0.filters[out2] };

  await pick(session, "OUT2");
  await channelMenu(session).click();
  await expect(item(session, "Paste Parameters")).toBeDisabled(); // nothing copied yet
  await expect(item(session, "Copy Parameters")).toBeEnabled();
  // The menu stays on the page at every width.
  const box = (await session.getByRole("menu", { name: "Channel" }).boundingBox())!;
  const width = await page.evaluate(() => document.documentElement.clientWidth);
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(width);
  await page.keyboard.press("Escape");

  await pick(session, "OUT1");
  await channelMenu(session).click();
  await item(session, "Copy Parameters").click();
  await pick(session, "OUT2");
  await channelMenu(session).click();
  await item(session, "Paste Parameters").click();
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBe(-7);
  expect((await core(page)).filters[out2]![2]!.freq).toBe(315);

  await undo(session).click();
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBe(before.gain);
  expect((await core(page)).filters[out2]).toEqual(before.bands);
  expect(errors).toEqual([]);
});

test("WBCLIP M45: Ctrl/Cmd+C and +V on the chosen channel; a linked input pastes onto its partner; a text field keeps its keys", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openEq(page);
  await shapeOut1(page);
  const s0 = await core(page);

  await pick(session, "OUT1");
  await page.keyboard.press("ControlOrMeta+c");
  await pick(session, "IN1");
  await page.keyboard.press("ControlOrMeta+v");
  // IN1/IN2 are linked by default: both get OUT1's bands.
  await expect.poll(async () => (await core(page)).filters[0]![2]!.freq).toBe(315);
  expect((await core(page)).filters[1]![2]!.freq).toBe(315);

  await undo(session).click();
  await expect.poll(async () => (await core(page)).filters.slice(0, 2)).toEqual(s0.filters.slice(0, 2));

  // Inside a text field the keys are the field's.
  const field = session.getByRole("textbox", { name: "IN1 trim" });
  await field.focus();
  await expect(field).toBeFocused();
  await page.keyboard.press("ControlOrMeta+v");
  await page.waitForTimeout(200);
  expect((await core(page)).filters.slice(0, 2)).toEqual(s0.filters.slice(0, 2));

  // The shortcut list names them.
  await appMenu(page.locator("#one"), "Keyboard Shortcuts"); // -- WP WEBM1 --
  const list = page.getByRole("dialog", { name: "Keyboard Shortcuts" });
  await expect(list.getByText("Copy Parameters")).toBeVisible();
  await expect(list.getByText("Paste Parameters")).toBeVisible();
  // While a dialog is open the keys do nothing.
  await page.keyboard.press("ControlOrMeta+v");
  await page.waitForTimeout(200);
  expect((await core(page)).filters.slice(0, 2)).toEqual(s0.filters.slice(0, 2));
  expect(errors).toEqual([]);
});

test("WBCLIP M45: the matrix column menu copies and pastes an output", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openEq(page);
  await shapeOut1(page);
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  // -- WP MXW -- a column's menu is its OUTn pill
  const column = (n: number) => matrix.getByRole("button", { name: `OUT${n + 1}`, exact: true });
  await column(1).click();
  await expect(item(matrix, "Paste Parameters")).toBeDisabled();
  await page.keyboard.press("Escape");
  await column(0).click();
  await item(matrix, "Copy Parameters").click();
  await column(1).click();
  await item(matrix, "Paste Parameters").click();
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBe(-7);
  // Off the EQ screen no channel is chosen: the keys do nothing.
  const g = (await core(page)).outputGainDb;
  await session.locator(".page-title").click(); // -- WP MXW -- the matrix has no heading of its own
  await page.keyboard.press("ControlOrMeta+v");
  await page.waitForTimeout(200);
  expect((await core(page)).outputGainDb).toEqual(g);
  expect(errors).toEqual([]);
});

test("WBCLIP read only: a view-only session copies but never pastes, by menu or key", async ({ page }) => {
  const session = await openEq(page, "Venue B");
  const s0 = await core(page);
  await pick(session, "OUT1");
  await channelMenu(session).click();
  await item(session, "Copy Parameters").click();
  await pick(session, "OUT2");
  await channelMenu(session).click();
  await expect(item(session, "Paste Parameters")).toBeDisabled();
  await page.keyboard.press("Escape");
  await page.keyboard.press("ControlOrMeta+v");
  await page.waitForTimeout(200);
  const s1 = await core(page);
  expect([s1.filters, s1.outputGainDb]).toEqual([s0.filters, s0.outputGainDb]);
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  await expect(matrix.getByRole("button", { name: "OUT1", exact: true })).toBeDisabled(); // -- WP MXW --
});
