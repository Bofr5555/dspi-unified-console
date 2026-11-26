// WP GAP1 e2e: the web gaps on the core's demo device, every change read back from the core. The
// EQ graph's Phase chip (Qt's, between RTA and All Curves) is Settings › Graphing's switch and
// redraws the graph; Delay & Gain's Polarity (Normal / Inverted / –) writes every feeding route in
// one undo step; the matrix inspector's Polarity and the crosspoint's output line; Effects ›
// Crossfeed's Outputs pair map: K340, its question, the pair chips, the Custom editor's refusal.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, openSetting, partsOf } from "../nav";

type State = {
  numInputChannels: number;
  numOutputChannels: number;
  matrixRouting: boolean[][];
  matrixInvert: boolean[][];
  crossfeedPairMap: number[];
  crossfeedOutputPairMask: number;
};
type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const snapshot = (page: Page) => call<{ state: State; outputPolarity: string[] }>(page, "state");
const core = async (page: Page) => (await snapshot(page)).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible();
  return session;
}

const draws = (graph: Locator) => graph.locator("canvas").evaluate((c) => (c as unknown as { dspiDraws?: number }).dspiDraws ?? 0);

test("GAP1 the EQ graph's Phase chip: Qt's place, Settings › Graphing's switch, a redraw", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  await expect(graph).toBeVisible();
  const chips = session.getByRole("group", { name: "Graph options" });
  await expect(chips.getByRole("button")).toHaveText(["RTA", "Phase", "All Curves"]);
  const phase = chips.getByRole("button", { name: "Phase", exact: true });
  await expect(phase).toHaveAttribute("aria-pressed", "false");
  await expect.poll(() => draws(graph)).toBeGreaterThan(0);
  const before = await draws(graph);
  await phase.click();
  await expect(phase).toHaveAttribute("aria-pressed", "true");
  await expect.poll(() => draws(graph)).toBeGreaterThan(before);

  // The same switch in Settings › Graphing, Unwrap Phase under it
  await openPage(session, "Settings");
  await openSetting(session, "Graphing");
  const look = session.getByRole("region", { name: "Graph Appearance" });
  await expect(look.getByRole("switch", { name: "Show Phase Response" })).toHaveAttribute("aria-checked", "true");
  const unwrap = look.getByRole("switch", { name: "Unwrap Phase" });
  await expect(unwrap).toBeEnabled();
  await look.getByRole("switch", { name: "Show Phase Response" }).click();
  await expect(unwrap).toBeDisabled();
  await openPage(session, "EQ");
  await expect(session.getByRole("group", { name: "Graph options" }).getByRole("button", { name: "Phase", exact: true })).toHaveAttribute("aria-pressed", "false");
  expect(errors).toEqual([]);
});

/** Out 1 fed by USB 2 as well (at -6 dB), once the page shows it (the matrix's "Fed by"). */
async function secondRoute(page: Page, session: Locator) {
  await call(page, "crosspoint", { input: 1, output: 0, enabled: true, gain: -6, invert: false });
  await openPage(session, "Matrix");
  await expect(session.getByRole("region", { name: "Matrix Mixer" }).locator("tfoot td.mx-src").first()).toHaveAttribute("title", /USB 2/);
  await expect.poll(async () => (await core(page)).matrixRouting[1]![0]).toBe(true);
}

test("GAP1 Delay & Gain's Polarity: every feeding route; a dash where nothing feeds", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await secondRoute(page, session);
  await openPage(session, "Delay & Gain");
  const table = session.getByRole("region", { name: "Delay and gain of every output" });
  await expect(table.getByRole("columnheader")).toContainText(["Output", "On", "Gain", "Delay", "Distance", "Arrival", "Polarity", "Mute"]);
  const rows = table.locator("tbody tr");
  const out1 = rows.first().getByRole("button", { name: /^Invert / });
  await expect(out1).toHaveText("Normal");
  await expect(out1).toHaveAttribute("aria-pressed", "false");
  await out1.click();
  await expect(out1).toHaveText("Inverted");
  await expect(out1).toHaveAttribute("aria-pressed", "true");
  let s = await core(page);
  expect([s.matrixInvert[0]![0], s.matrixInvert[1]![0]]).toEqual([true, true]);
  expect((await snapshot(page)).outputPolarity[0]).toBe("inverted");
  await out1.click();
  await expect(out1).toHaveText("Normal");
  s = await core(page);
  expect([s.matrixInvert[0]![0], s.matrixInvert[1]![0]]).toEqual([false, false]);
  // The last output: no input feeds it
  const last = rows.last().getByRole("button", { name: /^Invert / });
  await expect(last).toHaveText("\u2013");
  await expect(last).toBeDisabled();
  expect(errors).toEqual([]);
});

test("GAP1 Delay & Gain's Polarity is one undo step", async ({ page }) => {
  const session = await openDemo(page);
  await secondRoute(page, session);
  await openPage(session, "Delay & Gain");
  const out1 = session.getByRole("region", { name: "Delay and gain of every output" }).locator("tbody tr").first().getByRole("button", { name: /^Invert / });
  await out1.click();
  await expect(out1).toHaveText("Inverted");
  await call(page, "undo");
  const s = await core(page);
  expect([s.matrixInvert[0]![0], s.matrixInvert[1]![0]]).toEqual([false, false]);
});

test("GAP1 the matrix inspector: an output's Polarity, the crosspoint's output line", async ({ page }) => {
  test.skip(test.info().project.name === "narrow", "the inspector beside a wide page");
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  const inspector = session.getByRole("complementary", { name: "Inspector" });
  await matrix.locator(".mx-colhead").first().locator(".mx-colname").click();
  const pol = inspector.getByRole("group", { name: "Polarity" });
  await expect(pol.getByRole("button", { name: "Normal" })).toHaveAttribute("aria-pressed", "true");
  await pol.getByRole("button", { name: "Inverted" }).click();
  await expect(pol.getByRole("button", { name: "Inverted" })).toHaveAttribute("aria-pressed", "true");
  expect((await snapshot(page)).outputPolarity[0]).toBe("inverted");
  await matrix.locator('[role="gridcell"][data-cell="0:0"]').click();
  await expect(inspector).toContainText("Polarity inverted");
  // An output nothing feeds: neither lit, nothing to press
  const n = (await core(page)).numOutputChannels;
  await matrix.locator(".mx-colhead").nth(n - 1).locator(".mx-colname").click();
  await expect(inspector.getByRole("group", { name: "Polarity" }).getByRole("button", { name: "Normal" })).toBeDisabled();
  await expect(inspector.getByRole("group", { name: "Polarity" }).getByRole("button", { name: "Normal" })).toHaveAttribute("aria-pressed", "false");
  expect(errors).toEqual([]);
});

test("GAP1 Crossfeed's Outputs: K340 and its question, the pair chips, the Custom editor", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Effects");
  await partsOf(session, "Effects").getByRole("tab", { name: /^Crossfeed/ }).click();
  const panel = session.getByRole("tabpanel", { name: "Crossfeed" });
  const outputs = panel.getByRole("group", { name: "Crossfeed outputs" });
  const menu = outputs.getByRole("button", { name: /^Default \(1\/2, 3\/4, …\)/ });
  await expect(menu).toBeVisible();
  await expect(panel.getByText("Which two outputs each pair crossfeeds. An output can belong to one pair only.")).toBeVisible();
  await expect(panel.getByRole("group", { name: /Output Pairs$/ }).getByRole("button").first()).toHaveText("1: Out 1 / Out 2");

  // One DAC per ear: the core's map; the question, as the mask leaves the tweeters out
  await call(page, "write", { path: "cf.outputs", value: { mask: 1 } });
  await menu.click();
  await page.getByRole("menuitem", { name: "One DAC per ear (K340)" }).click();
  await expect.poll(async () => (await core(page)).crossfeedPairMap).toEqual([0x02, 0x13, 0, 0]);
  await expect(outputs.getByRole("button", { name: /^One DAC per ear \(K340\)/ })).toBeVisible();
  await expect(panel.getByRole("group", { name: /Output Pairs$/ }).getByRole("button").first()).toHaveText("1: Out 3 / Out 1");
  await expect(outputs).toContainText("Crossfeed the woofers and the tweeters?");
  await outputs.getByRole("button", { name: "Enable Pairs 1 and 2" }).click();
  await expect.poll(async () => (await core(page)).crossfeedOutputPairMask & 3).toBe(3);
  await expect(outputs).not.toContainText("Crossfeed the woofers and the tweeters?");

  // Custom: an output in two pairs is refused before it is sent
  await outputs.getByRole("button", { name: /^One DAC per ear/ }).click();
  await page.getByRole("menuitem", { name: "Custom…" }).click();
  const editor = outputs.getByRole("group", { name: "Crossfeed pair map editor" });
  await expect(editor).toBeVisible();
  await editor.getByRole("combobox", { name: "Pair 2, one side" }).selectOption("2");
  await expect(editor).toContainText("Each pair needs two different outputs, and no output can be in two pairs.");
  await expect(editor.getByRole("button", { name: "Apply" })).toBeDisabled();
  // Pair 2 on Out 4 and Out 2 the other way round: valid, written
  await editor.getByRole("combobox", { name: "Pair 2, one side" }).selectOption("1");
  await editor.getByRole("combobox", { name: "Pair 2, other side" }).selectOption("3");
  await expect(editor.getByRole("button", { name: "Apply" })).toBeEnabled();
  await editor.getByRole("button", { name: "Apply" }).click();
  await expect(editor).toHaveCount(0);
  await expect.poll(async () => (await core(page)).crossfeedPairMap.slice(0, 2)).toEqual([0x02, 0x31]);
  await expect(outputs.getByRole("button", { name: /^Custom/ })).toBeVisible();
  expect(errors).toEqual([]);
});
