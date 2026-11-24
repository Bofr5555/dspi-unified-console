// WP MXW e2e: the matrix as DESREV's redesign and MX1 draw it, on the core's demo device: the
// corner, tiles that keep their size, dots, row tails, column footers, enable and mute in the
// column's head; every key (MX1's: arrows, Home/End, PgUp/PgDn, Space, Enter and digits, + and −
// with Ctrl for 3 dB, I, M, E, Delete, Shift+F10, Escape); Link L/R pairs (off by default, kept
// per device, the mirrored write as one undo step); Clear's question past four crosspoints;
// Direct 1:1; the inspector's states; dense. Every change is read back from the core.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type State = {
  numInputChannels: number;
  numOutputChannels: number;
  channelNames: string[];
  matrixRouting: boolean[][];
  matrixGain: number[][];
  matrixInvert: boolean[][];
  outputEnabled: boolean[];
  outputMuted: boolean[];
};
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function core(page: Page): Promise<State> {
  const snap = (await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("state", null))) as { state: State };
  return snap.state;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function openMatrix(page: Page) {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  await expect(matrix.getByRole("grid", { name: "Routing matrix" })).toBeVisible();
  return { session, matrix, tools: session.getByRole("toolbar", { name: "Routing tools" }), inspector: session.getByRole("complementary", { name: "Inspector" }) };
}

const cell = (matrix: Locator, i: number, o: number) => matrix.locator(`[role="gridcell"][data-cell="${i}:${o}"]`);
const route = async (page: Page, i: number, o: number) => (await core(page)).matrixRouting[i]![o]!;

test("MXW drawing: direction, tiles of one size, dots, tails, footers, enable and mute in the head", async ({ page }) => {
  const errors = errorsOf(page);
  const { matrix, inspector } = await openMatrix(page);
  const s = await core(page);
  await expect(matrix.locator(".mx-corner")).toContainText("From inputs ↓");
  await expect(matrix.locator(".mx-corner")).toContainText("To outputs →");
  // One connected and one empty cell: the same box
  const on = matrix.locator('[role="gridcell"][data-connected="true"]').first();
  const off = matrix.locator('[role="gridcell"][data-connected="false"]').first();
  const a = (await on.boundingBox())!;
  const b = (await off.boundingBox())!;
  expect([Math.round(a.width), Math.round(a.height)]).toEqual([56, 40]);
  expect([Math.round(b.width), Math.round(b.height)]).toEqual([56, 40]);
  await expect(on.locator(".mx-tile")).toBeVisible();
  await expect(off.locator(".mx-empty")).toBeVisible();
  // Each row's tail, each column's footer
  const feeds = s.matrixRouting[0]!.map((x, o) => (x ? o + 1 : 0)).filter(Boolean);
  await expect(matrix.locator("tbody tr").first().locator(".mx-tail")).toHaveText(feeds.length ? `→ Out ${feeds.join(", ")}` : "→ nothing");
  await expect(matrix.locator("tfoot")).toContainText("Gain");
  await expect(matrix.locator("tfoot")).toContainText("Delay");
  await expect(matrix.locator("tfoot")).toContainText("Fed by");
  const head = matrix.locator(".mx-colhead").first();
  await expect(head.getByRole("button", { name: /^Enable / })).toBeVisible();
  await expect(head.getByRole("button", { name: /^Mute / })).toBeVisible();
  // The inspector with nothing chosen: the routing in one line
  await expect(inspector.locator(".mx-summary")).toContainText(/\d+ crosspoints? · \d+ of \d+ outputs on/);
  expect(errors).toEqual([]);
});

test("MXW keys: move, connect, type, step (Ctrl for 3 dB), invert, mute, enable, disconnect, the menu, Escape", async ({ page }) => {
  const errors = errorsOf(page);
  const { matrix, inspector } = await openMatrix(page);
  const s0 = await core(page);
  // Choose IN1 → Out 3 (empty on the demo), then the keys
  const c = cell(matrix, 0, 2);
  expect(s0.matrixRouting[0]![2]).toBe(false);
  await cell(matrix, 0, 0).click();
  await expect(cell(matrix, 0, 0)).toBeFocused();
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("ArrowRight");
  await expect(c).toBeFocused();
  await expect(c).toHaveAttribute("aria-selected", "true");
  await page.keyboard.press("Space");
  await expect.poll(() => route(page, 0, 2)).toBe(true);
  // Enter types the gain: "−2,5" as Qt's fields read it
  await page.keyboard.press("Enter");
  const editor = matrix.getByRole("textbox", { name: / level$/ });
  await expect(editor).toBeFocused();
  await editor.fill("−2,5");
  await editor.press("Enter");
  await expect.poll(async () => (await core(page)).matrixGain[0]![2]).toBe(-2.5);
  await expect(c).toBeFocused();
  await expect(c.locator(".mx-g")).toHaveText("-2.5");
  // A digit starts typing; Escape restores
  await page.keyboard.press("7");
  await expect(editor).toHaveValue("7");
  await editor.press("Escape");
  await expect(editor).toHaveCount(0);
  expect((await core(page)).matrixGain[0]![2]).toBe(-2.5);
  // + and − step 0.5 dB, with Ctrl 3 dB
  await page.keyboard.press("+");
  await expect.poll(async () => (await core(page)).matrixGain[0]![2]).toBe(-2);
  await page.keyboard.press("Control+-");
  await expect.poll(async () => (await core(page)).matrixGain[0]![2]).toBe(-5);
  // The wheel: 0.5 dB, with Shift 3
  await c.hover();
  await page.mouse.wheel(0, -100);
  await expect.poll(async () => (await core(page)).matrixGain[0]![2]).toBe(-4.5);
  await c.focus();
  // I inverts: a ø in the tile
  await page.keyboard.press("i");
  await expect.poll(async () => (await core(page)).matrixInvert[0]![2]).toBe(true);
  await expect(c.locator(".mx-inv.on")).toBeVisible();
  // M mutes the column's output, E switches it
  const muted = s0.outputMuted[2]!;
  await page.keyboard.press("m");
  await expect.poll(async () => (await core(page)).outputMuted[2]).toBe(!muted);
  await page.keyboard.press("m");
  await expect.poll(async () => (await core(page)).outputMuted[2]).toBe(muted);
  const enabled = (await core(page)).outputEnabled[2]!;
  await page.keyboard.press("e");
  const ask = page.getByRole("dialog", { name: "Warning" });
  if (await ask.isVisible().catch(() => false)) await ask.getByRole("button", { name: "Cancel" }).click();
  else {
    await expect.poll(async () => (await core(page)).outputEnabled[2]).toBe(!enabled);
    await page.keyboard.press("e");
    await expect.poll(async () => (await core(page)).outputEnabled[2]).toBe(enabled);
  }
  // The inspector follows the cell
  await expect(inspector.getByRole("switch", { name: "Connected" })).toHaveAttribute("aria-checked", "true");
  await expect(inspector.getByRole("switch", { name: "Invert phase" })).toHaveAttribute("aria-checked", "true");
  // Shift+F10: the cell's menu
  await c.focus();
  await page.keyboard.press("Shift+F10");
  const menu = page.getByRole("menu");
  await expect(menu.getByRole("menuitem", { name: "Disconnect" })).toBeVisible();
  await expect(menu.getByRole("menuitemcheckbox", { name: "Invert Phase" })).toHaveAttribute("aria-checked", "true");
  await menu.getByRole("menuitem", { name: "Gain to 0 dB" }).click();
  await expect.poll(async () => (await core(page)).matrixGain[0]![2]).toBe(0);
  // Delete disconnects
  await c.focus();
  await page.keyboard.press("Delete");
  await expect.poll(() => route(page, 0, 2)).toBe(false);
  // Home, End, PgDn, PgUp
  await page.keyboard.press("End");
  await expect(cell(matrix, 0, s0.numOutputChannels - 1)).toBeFocused();
  await page.keyboard.press("Home");
  await expect(cell(matrix, 0, 0)).toBeFocused();
  await page.keyboard.press("PageDown");
  const rows = await matrix.locator("tbody tr").count();
  await expect(cell(matrix, rows - 1, 0)).toBeFocused();
  await page.keyboard.press("PageUp");
  await expect(cell(matrix, 0, 0)).toBeFocused();
  // Escape: nothing chosen, the inspector says the routing
  await page.keyboard.press("Escape");
  await expect(cell(matrix, 0, 0)).toHaveAttribute("aria-selected", "false");
  await expect(inspector.locator(".mx-summary")).toBeVisible();
  expect(errors).toEqual([]);
});

test("MXW mouse: a click connects an empty cell, a second click on the chosen one disconnects, a double click types", async ({ page }) => {
  const { matrix } = await openMatrix(page);
  const c = cell(matrix, 1, 2);
  expect(await route(page, 1, 2)).toBe(false);
  await c.click();
  await expect.poll(() => route(page, 1, 2)).toBe(true);
  await page.waitForTimeout(400);
  await c.click();
  await expect.poll(() => route(page, 1, 2)).toBe(false);
  await c.dblclick();
  const editor = matrix.getByRole("textbox", { name: / level$/ });
  await expect(editor).toBeFocused();
  await editor.fill("+1.5 dB");
  await editor.press("Enter");
  await expect.poll(async () => (await core(page)).matrixGain[1]![2]).toBe(1.5);
  expect(await route(page, 1, 2)).toBe(true);
});

test("MXW Link L/R pairs: off by default; L→Out 3 mirrors to R→Out 4 in one write, one undo; kept for the device", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, matrix, tools, inspector } = await openMatrix(page);
  const link = tools.getByRole("button", { name: "Link L/R pairs" });
  await expect(link).toHaveAttribute("aria-pressed", "false");
  // Off: one crosspoint alone
  await cell(matrix, 0, 4).click();
  await expect.poll(() => route(page, 0, 4)).toBe(true);
  expect(await route(page, 1, 5)).toBe(false);
  await link.click();
  await expect(link).toHaveAttribute("aria-pressed", "true");
  await expect(matrix.locator(".mx-pairglyph")).toHaveCount(1);
  // On: the partner, one column over, is a copy
  await cell(matrix, 0, 2).click();
  await expect.poll(() => route(page, 1, 3)).toBe(true);
  expect(await route(page, 0, 2)).toBe(true);
  await expect(inspector).toContainText("changes here are made on");
  await cell(matrix, 0, 2).dblclick();
  const editor = matrix.getByRole("textbox", { name: / level$/ });
  await editor.fill("-6");
  await editor.press("Enter");
  await expect.poll(async () => (await core(page)).matrixGain[1]![3]).toBe(-6);
  expect((await core(page)).matrixGain[0]![2]).toBe(-6);
  await cell(matrix, 0, 2).focus();
  await page.keyboard.press("i");
  await expect.poll(async () => (await core(page)).matrixInvert[1]![3]).toBe(true);
  // One undo takes back both crosspoints of the last change
  const undo = session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" });
  await undo.click();
  await expect.poll(async () => (await core(page)).matrixInvert[1]![3]).toBe(false);
  expect((await core(page)).matrixInvert[0]![2]).toBe(false);
  // Kept for this device: on again after a reload
  const again = await openMatrix(page);
  await expect(again.tools.getByRole("button", { name: "Link L/R pairs" })).toHaveAttribute("aria-pressed", "true");
  await again.tools.getByRole("button", { name: "Link L/R pairs" }).click();
  expect(errors).toEqual([]);
});

test("MXW Clear asks past four crosspoints; Cancel keeps them; Direct 1:1 routes each input to its output", async ({ page }) => {
  const { matrix, tools } = await openMatrix(page);
  for (const [i, o] of [
    [0, 2],
    [0, 3],
    [1, 4],
    [1, 5],
  ] as const) {
    if (!(await route(page, i, o))) await cell(matrix, i, o).click();
    await expect.poll(() => route(page, i, o)).toBe(true);
  }
  const n = (await core(page)).matrixRouting.flat().filter(Boolean).length;
  expect(n).toBeGreaterThan(4);
  await tools.getByRole("button", { name: "Clear" }).click();
  const ask = page.getByRole("dialog", { name: "Clear the routing?" });
  await expect(ask).toContainText(/will be disconnected/);
  await ask.getByRole("button", { name: "Cancel" }).click();
  expect((await core(page)).matrixRouting.flat().filter(Boolean).length).toBe(n);
  await tools.getByRole("button", { name: "Clear" }).click();
  await page.getByRole("dialog", { name: "Clear the routing?" }).getByRole("button", { name: "Clear" }).click();
  await expect.poll(async () => (await core(page)).matrixRouting.flat().filter(Boolean).length).toBe(0);
  await expect(matrix.locator("tbody tr").first().locator(".mx-tail")).toHaveText("→ nothing");
  await tools.getByRole("button", { name: "Direct 1:1" }).click();
  await expect.poll(async () => (await core(page)).matrixRouting[0]![0]).toBe(true);
  expect((await core(page)).matrixRouting[1]![1]).toBe(true);
  // Two crosspoints now: Clear goes at once
  await tools.getByRole("button", { name: "Clear" }).click();
  await expect.poll(async () => (await core(page)).matrixRouting.flat().filter(Boolean).length).toBe(0);
  await expect(page.getByRole("dialog", { name: "Clear the routing?" })).toHaveCount(0);
});

test("MXW inspector: an output's column, an input's row; their switches and chips write; dense cells", async ({ page }) => {
  const { matrix, tools, inspector } = await openMatrix(page);
  const s0 = await core(page);
  // An output's column
  await matrix.locator(".mx-colhead").nth(2).locator(".mx-colname").click();
  await expect(inspector.getByRole("group", { name: "Output" })).toBeVisible();
  await inspector.getByRole("switch", { name: "Mute" }).click();
  await expect.poll(async () => (await core(page)).outputMuted[2]).toBe(!s0.outputMuted[2]);
  await inspector.getByRole("switch", { name: "Mute" }).click();
  const fed = inspector.getByRole("group", { name: / fed by$/ });
  await fed.getByRole("button").first().click();
  await expect.poll(() => route(page, 0, 2)).toBe(!s0.matrixRouting[0]![2]);
  await inspector.getByRole("textbox", { name: "Output gain" }).fill("−1,5 dB");
  await inspector.getByRole("textbox", { name: "Output gain" }).press("Enter");
  await expect.poll(async () => (await core(page)).outputGainDb[2]).toBe(-1.5);
  // An input's row
  await matrix.locator(".mx-rowhead").first().locator(".mx-rowname").click();
  await expect(inspector.getByRole("group", { name: "Input" })).toBeVisible();
  const feeds = inspector.getByRole("group", { name: / feeds$/ });
  await expect(feeds.getByRole("button")).toHaveCount(s0.numOutputChannels);
  // Dense: smaller cells, no footer
  await tools.getByRole("button", { name: "Dense" }).click();
  const box = (await cell(matrix, 0, 0).boundingBox())!;
  expect([Math.round(box.width), Math.round(box.height)]).toEqual([44, 32]);
  await expect(matrix.locator("tfoot")).toHaveCount(0);
  await tools.getByRole("button", { name: "Dense" }).click();
  await expect(matrix.locator("tfoot")).toHaveCount(1);
});
