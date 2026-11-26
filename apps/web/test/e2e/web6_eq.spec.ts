// WP WEB6 e2e: the Equaliser's PEQ and Crossover pages as Qt Modern's (REVE §3.1) on the core's demo
// device, in every project: the inspector's one sentence with no band and Qt's band part, the band
// table's disc, picker and foot, the graph's chips, the page tools (Files ▾, the inspector's toggle),
// the Crossover tab in the Equaliser's frame with every output, the channel kept across the tabs;
// and REVE C1: leaving a field that was not typed in writes nothing. Every edit is read back from
// the core.

import { expect, test, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number };
type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = {
  numInputChannels: number;
  numOutputChannels: number;
  outputEnabled: boolean[];
  filters: Band[][];
  outputDelayMs: number[];
  outputGainDb: number[];
};

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const core = async (page: Page) => (await call<{ state: State }>(page, "state")).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

const wide = () => test.info().project.name !== "narrow";

/** Pictures in both themes when SHOTS_DIR names a directory (Chromium only). */
async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/web6-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** One console with Venue A's demo device on its PEQ page, OUT1 chosen; OUT1 given three bands. */
async function openEq(page: Page) {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  await expect(session.getByRole("application", { name: /^EQ graph, OUT1/ })).toBeVisible();
  const w = (await core(page)).numInputChannels;
  const bands: [number, number, number, number][] = [
    [1, 63, 3, -4],
    [1, 180, 1.4, 2.5],
    [4, 2500, 0.7, -1.5],
  ];
  for (const [b, [t, freq, q, gain]] of bands.entries()) await call(page, "filter", { channel: w, band: b, filterType: t, freq, q, gain, bypass: false, qp: 0.707 });
  await call(page, "endGroup");
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  await expect(list.getByRole("textbox", { name: "Band 3 frequency" })).toBeVisible();
  const inspector = session.getByRole("complementary", { name: "Inspector" });
  return { one, session, list, inspector, w };
}

test("WEB6: no band chosen, the inspector says Qt's one sentence; a band chosen, its shapes, rows, Band on, Invert and Reset", async ({ page }) => {
  test.skip(!wide(), "the inspector beside a wide page");
  const errors = errorsOf(page);
  const { session, list, inspector, w } = await openEq(page);
  await expect(inspector).toHaveAttribute("data-placement", "beside");
  const band = inspector.getByRole("group", { name: "Band", exact: true });
  await expect(band).toContainText("No band selected. Click a dot or a row in the table, or double-click the graph to add one.");
  // No disabled form: no shapes, no fields, no Q stepper
  await expect(inspector.getByRole("radiogroup", { name: "Shapes" })).toHaveCount(0);
  await expect(inspector.getByRole("textbox", { name: "Frequency", exact: true })).toHaveCount(0);
  await expect(session.getByRole("button", { name: "Q up" })).toHaveCount(0);
  // Qt's sections: Linkwitz and the limiter closed with "Off", Edit Routing… under them
  await expect(inspector.getByRole("group", { name: "Linkwitz transform" })).toContainText("Off");
  await expect(inspector.getByRole("group", { name: "Limiter" })).not.toHaveAttribute("open", "");
  await expect(inspector.getByRole("button", { name: "Edit Routing…" })).toBeVisible();
  await shot(page, "peq-noband");

  // Band 2 chosen from the table: the shapes once (a grid, no Shape select), the rows, Q plain
  await list.getByRole("button", { name: "Band 2", exact: true }).click();
  const two = inspector.getByRole("group", { name: "Band 2" });
  await expect(two.getByRole("radio", { name: "Bell" })).toHaveAttribute("aria-checked", "true");
  await expect(inspector.getByRole("combobox", { name: "Shape" })).toHaveCount(0);
  await expect(two.getByRole("textbox", { name: "Frequency" })).toHaveValue("180.0");
  await expect(two).toContainText("Bandwidth");
  await expect(two.getByRole("button", { name: "Q up" })).toHaveCount(0);
  await shot(page, "peq-band");
  await two.getByRole("button", { name: "Invert" }).click();
  await expect.poll(async () => (await core(page)).filters[w]![1]!.gain).toBeCloseTo(-2.5, 2);
  await two.getByRole("switch", { name: "Band on" }).click();
  await expect.poll(async () => (await core(page)).filters[w]![1]!.bypass).toBe(true);
  await two.getByRole("radio", { name: "Low Shelf" }).click();
  await expect.poll(async () => (await core(page)).filters[w]![1]!.filterType).toBe(2);
  await two.getByRole("button", { name: "Reset" }).click();
  await expect.poll(async () => (await core(page)).filters[w]![1]!.filterType).toBe(0);
  await expect(inspector.getByRole("group", { name: "Band", exact: true })).toContainText("No band selected.");
  expect(errors).toEqual([]);
});

test("WEB6: the band table's disc bypasses, its picker sets the type, its foot enables, bypasses and clears; units after the fields", async ({ page }) => {
  const errors = errorsOf(page);
  const { list, w } = await openEq(page);
  const disc = list.getByRole("checkbox", { name: "Band 1 plays" });
  await expect(disc).toBeChecked();
  await disc.click();
  await expect.poll(async () => (await core(page)).filters[w]![0]!.bypass).toBe(true);
  await expect(disc).not.toBeChecked();
  // An off band has no disc to press and no values
  await expect(list.getByRole("checkbox", { name: "Band 5 plays" })).toBeDisabled();
  await expect(list.getByRole("textbox", { name: "Band 5 frequency" })).toHaveCount(0);
  // The picker (Qt's combo, no native look) writes the type
  const type = list.getByRole("combobox", { name: "Band 3 type" });
  await expect(type).toHaveClass(/combo/);
  await type.selectOption("1");
  await expect.poll(async () => (await core(page)).filters[w]![2]!.filterType).toBe(1);
  if (wide()) {
    // The value bare in the field, its unit after it
    await expect(list.getByRole("textbox", { name: "Band 3 frequency" })).toHaveValue("2500.0");
    await expect(list.locator("tr", { has: page.getByRole("textbox", { name: "Band 3 frequency" }) }).locator(".field-unit").first()).toHaveText("Hz");
  }
  // The foot: Enable All, Bypass All, Clear PEQ
  await list.getByRole("button", { name: "Enable All" }).click();
  await expect.poll(async () => (await core(page)).filters[w]!.slice(0, 3).every((b) => !b.bypass)).toBe(true);
  await list.getByRole("button", { name: "Bypass All" }).click();
  await expect.poll(async () => (await core(page)).filters[w]!.slice(0, 3).every((b) => b.bypass)).toBe(true);
  await list.getByRole("button", { name: "Clear PEQ" }).click();
  await page.getByRole("dialog", { name: "Clear All Bands?" }).getByRole("button", { name: "Clear All" }).click();
  await expect.poll(async () => (await core(page)).filters[w]!.slice(0, 3).every((b) => b.filterType === 0)).toBe(true);
  expect(errors).toEqual([]);
});

test("WEB6: the page tools are Qt's (Files ▾, the inspector's toggle); the graph's RTA and All Curves chips", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, inspector } = await openEq(page);
  const tools = session.getByRole("toolbar", { name: "Equaliser tools" });
  await expect(tools.getByRole("button", { name: "Channel" })).toHaveCount(0);
  await expect(tools.getByRole("button", { name: "Spectrum on Graph" })).toHaveCount(0);
  // Files ▾: Import and Export Filters; Export downloads the core's filter file
  await tools.getByRole("button", { name: "Files" }).click();
  const files = page.getByRole("menu", { name: "Files" });
  await expect(files.getByRole("menuitem", { name: "Import Filters…" })).toBeEnabled();
  const download = page.waitForEvent("download");
  await files.getByRole("menuitem", { name: "Export Filters…" }).click();
  expect((await download).suggestedFilename()).toBe("DSPi Filters.txt");
  // The inspector's toggle hides and shows it
  const toggle = tools.getByRole("button", { name: /Inspector/ });
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await toggle.click();
  await expect(inspector).toHaveAttribute("data-placement", "hidden");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");
  await toggle.click();
  await expect(inspector).not.toHaveAttribute("data-placement", "hidden");
  // RTA: the spectrum over the graph; All Curves: every channel's curve, then the chosen one alone
  const chips = session.getByRole("group", { name: "Graph options" });
  await chips.getByRole("button", { name: "RTA" }).click();
  await expect(session.locator("canvas.rta-overlay")).toHaveCount(1);
  await chips.getByRole("button", { name: "RTA" }).click();
  await expect(session.locator("canvas.rta-overlay")).toHaveCount(0);
  const all = chips.getByRole("button", { name: "All Curves" });
  await expect(all).toHaveAttribute("aria-pressed", "true");
  await all.click();
  await expect(all).toHaveAttribute("aria-pressed", "false");
  for (const eye of await session.locator(".eq-eye:not([disabled])").all()) await expect(eye).toHaveAttribute("aria-pressed", "false");
  await all.click();
  for (const eye of await session.locator(".eq-eye").all()) await expect(eye).toHaveAttribute("aria-pressed", "true");
  expect(errors).toEqual([]);
});

test("WEB6: Crossover sits in the Equaliser's frame, lists every output, keeps the chosen channel; a row chooses its output", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, inspector } = await openEq(page);
  // OUT2 chosen on PEQ: still chosen on Crossover (Qt's one selection), and back
  await session.locator(".eq-pick", { hasText: "OUT2" }).click();
  await openPage(session, "Crossover");
  await expect(session.locator(".eq-pick", { hasText: "OUT2" })).toHaveAttribute("aria-pressed", "true");
  await expect(session.getByRole("group", { name: "Channels" })).toBeVisible();
  await expect(session.getByRole("region", { name: /^Output settings, OUT2/ })).toBeVisible();
  await expect(inspector.getByRole("group", { name: "Crossover" })).toBeVisible();
  const s = await core(page);
  const table = session.getByRole("region", { name: "Crossovers of every output" });
  await expect(table.getByRole("row")).toHaveCount(1 + s.numOutputChannels);
  expect(s.outputEnabled.filter((e) => e === false).length).toBeGreaterThan(0);
  // A disabled output is listed, dimmed
  const off = s.outputEnabled.findIndex((e) => e === false);
  await expect(table.getByRole("row", { name: new RegExp(`^OUT${off + 1} `) })).toHaveClass(/xo-disabled/);
  // Qt's short words; the frequency field only with a pass
  const hp = table.getByRole("combobox", { name: /^OUT2 .* high pass$/ });
  await hp.selectOption({ label: "LR 24 dB" });
  await expect.poll(async () => (await call<{ state: { xover: Band[][] } }>(page, "state")).state.xover[s.numInputChannels + 1]!.some((b) => b.filterType === 35)).toBe(true);
  await expect(table.getByRole("textbox", { name: /^OUT2 .* high pass frequency$/ })).toBeVisible();
  await expect(session.getByRole("region", { name: /^Crossover bands of OUT2 / })).toBeVisible();
  await shot(page, "crossover");
  // A row chooses its output for the whole frame
  await table.getByRole("button", { name: /^OUT1 / }).click();
  await expect(session.locator(".eq-pick", { hasText: "OUT1" })).toHaveAttribute("aria-pressed", "true");
  await openPage(session, "EQ");
  await expect(session.locator(".eq-pick", { hasText: "OUT1" })).toHaveAttribute("aria-pressed", "true");
  expect(errors).toEqual([]);
});

test("WEB6 (REVE C1): tabbing through Delay & Gain's fields writes nothing; typing writes", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openEq(page);
  await call(page, "write", { path: "out.delay", indices: [0], value: { float: 4.1234 } });
  await call(page, "endGroup");
  const before = (await core(page)).outputDelayMs[0]!;
  expect(before).toBeCloseTo(4.1234, 3);
  await openPage(session, "Delay & Gain");
  const row = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("row", { name: /^OUT1 / });
  await expect(row.getByRole("textbox", { name: /delay$/ })).toHaveValue(/^4\.1/);
  // Through gain, delay and distance (the distance's rounded "1.41 m" once wrote 4.1108 ms back)
  await row.getByRole("textbox", { name: /gain$/ }).focus();
  for (let i = 0; i < 3; i++) await page.keyboard.press("Tab");
  await page.waitForTimeout(300);
  expect((await core(page)).outputDelayMs[0]).toBe(before);
  // Typed text is written
  await row.getByRole("textbox", { name: /gain$/ }).fill("-3");
  await page.keyboard.press("Tab");
  await expect.poll(async () => (await core(page)).outputGainDb[0]).toBe(-3);
  expect((await core(page)).outputDelayMs[0]).toBe(before);
  expect(errors).toEqual([]);
});
