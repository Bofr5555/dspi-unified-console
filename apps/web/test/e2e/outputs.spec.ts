// WP WB6c e2e: an output's page (its settings card: routing, gain, delay,
// mute, limiter; its crossover), the matrix mixer, and the EQ follow-ups
// (the Linkwitz Transform, the shape card's slopes, the data-theme redraw),
// on the core's demo device in Chromium, Firefox and Chromium at 375 px.
// Every change is made through the page and read back twice: the page shows
// the device's value, and the core's own state (a direct call) agrees. The
// delay rule (owner, SL1) is tested on the writes the page sends.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, rail } from "../nav";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number; qp: number };
type State = {
  numInputChannels: number;
  numOutputChannels: number;
  formatVersion: number;
  channelNames: string[];
  filters: Band[][];
  xover: Band[][];
  outputGainDb: number[];
  outputDelayMs: number[];
  outputMuted: boolean[];
  outputEnabled: boolean[];
  matrixRouting: boolean[][];
  matrixGain: number[][];
  matrixInvert: boolean[][];
  limiterEnabled: boolean[];
  limiterLinkGroup: number[];
  limiterThresholdDb: number[];
  limiterReleaseMs: number[];
};
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[]; dspiLog: [string, unknown][] };

// -- BUG1 -- An output's name on the page is its label ("Out 3 · R"), which follows the
// routing and the filters while the device keeps the slot's default name: matched by its number.
const OUT = (o: number) => `Out ${o + 1}(?: · [^·]+){0,2}`;
const esc = (s: string) => s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
const re = (s: string) => new RegExp(`^${s}$`);

async function core(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate((n) => (globalThis as unknown as G).dspiClients[n]!.call("state", null), n)) as { state: State };
  return snap.state;
}

/** From now on, every call console one's session makes, with its parameters. */
async function logCalls(page: Page) {
  await page.evaluate(() => {
    const g = globalThis as unknown as G;
    const c = g.dspiClients[0]!;
    g.dspiLog = [];
    const call = c.call.bind(c);
    c.call = (m: string, p: unknown) => {
      g.dspiLog.push([m, p]);
      return call(m, p);
    };
  });
}

/** The values of the writes to `path` logged so far. */
async function written(page: Page, path: string): Promise<number[]> {
  const log = await page.evaluate(() => (globalThis as unknown as G).dspiLog);
  return log
    .filter(([m, p]) => m === "write" && (p as { path: string }).path === path)
    .map(([, p]) => (p as { value: { float: number } }).value.float);
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function connect(page: Page, venue = "Venue A", url = "/test/embed.html") {
  await page.goto(url);
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(rail(session)).toBeVisible();
  return session;
}

/** Console one on OUT1's page (the EQ screen with OUT1 chosen): the session and its settings card. */
async function openOut1(page: Page, venue = "Venue A", url = "/test/embed.html") {
  const session = await connect(page, venue, url);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const card = session.getByRole("region", { name: /^Output settings, OUT1/ });
  await expect(card).toBeVisible();
  return { session, card };
}

async function openMatrix(page: Page, venue = "Venue A", url = "/test/embed.html") {
  const session = await connect(page, venue, url);
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  await expect(matrix).toBeVisible();
  return { session, matrix };
}

async function type(field: Locator, text: string) {
  await field.click();
  await field.fill(text);
  await field.press("Enter");
}

const draws = (graph: Locator) => graph.locator("canvas").evaluate((c) => (c as unknown as { dspiDraws?: number }).dspiDraws ?? 0);

async function drawn(graph: Locator) {
  let last = -1;
  await expect
    .poll(async () => {
      const n = await draws(graph);
      const settled = n > 0 && n === last;
      last = n;
      return settled;
    }, { intervals: [150] })
    .toBe(true);
}

test("WB6c card: gain typed, stepped by key and wheel, reset; mute; each read back", async ({ page }) => {
  const errors = errorsOf(page);
  const { card } = await openOut1(page);
  const field = card.getByRole("textbox", { name: "Gain" });
  const slider = card.getByRole("slider", { name: "Gain" });
  await type(field, "-6.5");
  await expect.poll(async () => (await core(page)).outputGainDb[0]).toBe(-6.5);
  await expect(field).toHaveValue("-6.5 dB");
  await expect(slider).toHaveAttribute("aria-valuetext", "-6.5 dB");
  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await core(page)).outputGainDb[0]).toBe(-6.4);
  await expect(field).toHaveValue("-6.4 dB");
  await field.hover();
  await page.mouse.wheel(0, -100);
  await expect.poll(async () => (await core(page)).outputGainDb[0]).toBe(-6.3);
  await slider.dblclick();
  await expect.poll(async () => (await core(page)).outputGainDb[0]).toBe(0);
  await expect(field).toHaveValue("0.0 dB");
  // Mute.
  const mute = card.getByRole("button", { name: "Mute" });
  await mute.click();
  await expect.poll(async () => (await core(page)).outputMuted[0]).toBe(true);
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  await mute.click();
  await expect.poll(async () => (await core(page)).outputMuted[0]).toBe(false);
  await expect(mute).toHaveAttribute("aria-pressed", "false");
  expect(errors).toEqual([]);
});

test("WB6c SL1 delay: typed fractions kept and shown exactly; a drag from a fraction sends whole ms and snaps; keys, wheel and an assistive change commit", async ({ page }) => {
  const errors = errorsOf(page);
  const { card } = await openOut1(page);
  await logCalls(page);
  const field = card.getByRole("textbox", { name: "Delay" });
  const slider = card.getByRole("slider", { name: "Delay" });
  const max = Number(await slider.getAttribute("max"));
  expect([21, 42]).toContain(max);

  // Typed: kept exactly, and shown so.
  await type(field, "7.25");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(7.25);
  await expect(field).toHaveValue("7.25 ms");
  await type(field, "3.3333");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBeCloseTo(3.3333, 4);
  await expect(field).toHaveValue("3.3333 ms");
  expect(await written(page, "out.delay")).toEqual([7.25, 3.3333]);

  // A drag from that fractional start: every live write and the release are whole ms, the thumb snaps.
  const box = (await slider.boundingBox())!;
  const pad = 8; // about half a thumb
  const xAt = (ms: number) => box.x + pad + ((box.width - 2 * pad) * ms) / max;
  const y = box.y + box.height / 2;
  await page.mouse.move(xAt(3.3333), y);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) await page.mouse.move(xAt(3.3333 + i), y);
  await page.mouse.up();
  const drag = (await written(page, "out.delay")).slice(2);
  expect(drag.length).toBeGreaterThan(0);
  for (const v of drag) {
    expect(Number.isInteger(v), `${v} is whole`).toBe(true);
    expect(Object.is(v, -0)).toBe(false);
  }
  await expect.poll(async () => Number.isInteger((await core(page)).outputDelayMs[0])).toBe(true);
  const settled = (await core(page)).outputDelayMs[0]!;
  expect(settled).toBeGreaterThan(8);
  expect(settled).toBe(drag[drag.length - 1]);
  await expect(slider).toHaveValue(String(settled));
  await expect(field).toHaveValue(`${settled}.0 ms`);

  // Keys from a fraction: to the next whole ms, committed (the field shows the device's value).
  await type(field, "7.25");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(7.25);
  await slider.focus();
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(8);
  await expect(field).toHaveValue("8.0 ms");
  await type(field, "7.25");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(7.25);
  await slider.focus();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(7);
  await page.keyboard.press("End");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(max);
  await page.keyboard.press("Home");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(0);

  // The wheel from a fraction: the next whole ms up, then down.
  await type(field, "5.5");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(5.5);
  await field.hover();
  await page.mouse.wheel(0, -100);
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(6);
  await page.mouse.wheel(0, 100);
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(5);

  // An assistive technology's change (an input event without a pointer): committed, on the grid.
  await slider.evaluate((el: HTMLInputElement) => {
    el.value = "15";
    el.dispatchEvent(new Event("input", { bubbles: true }));
  });
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(15);
  await expect(field).toHaveValue("15.0 ms");
  // A secondary click resets it.
  await slider.click({ button: "right" });
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(0);
  expect(errors).toEqual([]);
});

test("WB6c card: the routing preview connects, levels and inverts an input, read back", async ({ page }) => {
  const { card } = await openOut1(page);
  const s0 = await core(page);
  const in1 = s0.channelNames[0]!;
  const was = s0.matrixRouting[0]![0]!;
  const route = card.getByRole("button", { name: `Route ${in1}` });
  await expect(route).toHaveAttribute("aria-pressed", String(was));
  await route.click();
  await expect.poll(async () => (await core(page)).matrixRouting[0]![0]).toBe(!was);
  await expect(route).toHaveAttribute("aria-pressed", String(!was));
  if (was) await route.click();
  await expect.poll(async () => (await core(page)).matrixRouting[0]![0]).toBe(true);
  await type(card.getByRole("textbox", { name: `${in1} level` }), "-4.5");
  await expect.poll(async () => (await core(page)).matrixGain[0]![0]).toBe(-4.5);
  await expect(card.getByRole("textbox", { name: `${in1} level` })).toHaveValue("-4.5 dB");
  const inv = card.getByRole("button", { name: `${in1} inverted` });
  await inv.click();
  await expect.poll(async () => (await core(page)).matrixInvert[0]![0]).toBe(true);
  await expect(inv).toHaveAttribute("aria-pressed", "true");
});

test("WB6c limiter: toggle, threshold, release, link group and Copy to all outputs, read back", async ({ page }) => {
  const { card, session } = await openOut1(page);
  test.skip((await core(page)).formatVersion < 32, "the demo's firmware has no limiter");
  const toggle = card.getByRole("button", { name: /^Limiter OUT1/ });
  const was = (await core(page)).limiterEnabled[0]!;
  await toggle.click();
  await expect.poll(async () => (await core(page)).limiterEnabled[0]).toBe(!was);
  await expect(toggle).toHaveAttribute("aria-pressed", String(!was));
  if (was) await toggle.click();
  await expect.poll(async () => (await core(page)).limiterEnabled[0]).toBe(true);
  await card.getByRole("button", { name: /^Limiter settings/ }).click();
  const dlg = session.page().getByRole("dialog", { name: "Output Limiter" });
  await expect(dlg).toBeVisible();
  await type(dlg.getByRole("textbox", { name: "Threshold" }), "-6");
  await expect.poll(async () => (await core(page)).limiterThresholdDb[0]).toBe(-6);
  await expect(dlg.getByRole("textbox", { name: "Threshold" })).toHaveValue("-6.0 dBFS");
  await type(dlg.getByRole("textbox", { name: "Release" }), "250");
  await expect.poll(async () => (await core(page)).limiterReleaseMs[0]).toBe(250);
  await dlg.getByRole("radio", { name: "Link group 2" }).click();
  await expect.poll(async () => (await core(page)).limiterLinkGroup[0]).toBe(2);
  await expect(dlg.getByRole("radio", { name: "Link group 2" })).toHaveAttribute("aria-checked", "true");
  await expect(dlg.getByText(/^No other outputs in group 2\.|^Linked with/)).toBeVisible();
  await dlg.getByRole("button", { name: "Copy to all outputs" }).click();
  await expect.poll(async () => (await core(page)).limiterThresholdDb.every((t) => t === -6)).toBe(true);
  await dlg.getByRole("button", { name: "All outputs", exact: true }).click();
  await dlg.getByRole("menuitem", { name: "Link all stereo pairs" }).click();
  await expect.poll(async () => (await core(page)).limiterLinkGroup.slice(0, 4)).toEqual([1, 1, 2, 2]);
  await expect(dlg.getByText("Linked with")).toBeVisible();
  await dlg.getByRole("button", { name: "All outputs", exact: true }).click();
  await dlg.getByRole("menuitem", { name: "Switch every limiter off" }).click();
  await expect.poll(async () => (await core(page)).limiterEnabled.some((x) => x)).toBe(false);
  await dlg.getByRole("button", { name: "Done" }).click();
  await expect(dlg).toBeHidden();
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
});

test("WB6c crossover: family, type, slope and frequency, bypass with the Console's warning, Clear All, read back", async ({ page }) => {
  const { session } = await openOut1(page);
  const w = (await core(page)).numInputChannels;
  await openPage(session, "Crossover"); // -- WP WEB6 -- an output's XO bands are on the Crossover tab (Qt's), OUT1 still chosen
  const list = session.getByRole("region", { name: /^Crossover of OUT1/ });
  await expect(list).toBeVisible();
  const x0 = async () => (await core(page)).xover[w]![0]!;
  await list.getByRole("combobox", { name: "Crossover 1 family" }).selectOption("2"); // Butterworth
  await expect.poll(async () => (await x0()).filterType).toBe(42);
  await list.getByRole("combobox", { name: "Crossover 1 type" }).selectOption("hp");
  await expect.poll(async () => (await x0()).filterType).toBe(43);
  await list.getByRole("combobox", { name: "Crossover 1 slope" }).selectOption("4"); // 24 dB/oct
  await expect.poll(async () => (await x0()).filterType).toBe(47);
  await expect(list.getByRole("combobox", { name: "Crossover 1 slope" })).toHaveValue("4");
  // The Console's words, in short on a phone.
  const narrow = (page.viewportSize()?.width ?? 1280) <= 520;
  await expect(list.getByRole("combobox", { name: "Crossover 1 family" }).locator("option:checked")).toHaveText(narrow ? "BW" : "Butterworth");
  await expect(list.getByRole("combobox", { name: "Crossover 1 slope" }).locator("option:checked")).toHaveText(narrow ? "24 dB" : "24 dB/oct");
  await type(list.getByRole("textbox", { name: "Crossover 1 frequency" }), "80");
  await expect.poll(async () => (await x0()).freq).toBe(80);
  await expect(list.getByRole("textbox", { name: "Crossover 1 frequency" })).toHaveValue(/^80\.0+( Hz)?$/);
  // Linkwitz-Riley from Butterworth 4: LR4 high pass, order and pass kept.
  await list.getByRole("combobox", { name: "Crossover 1 family" }).selectOption("1"); // Linkwitz-Riley
  await expect.poll(async () => (await x0()).filterType).toBe(35);
  // Bypass All asks first, in the Console's words; Cancel keeps it.
  await list.getByRole("button", { name: "Bypass All" }).click();
  const ask = page.getByRole("dialog", { name: "Bypass this output's crossovers?" });
  await expect(ask).toContainText("can damage unprotected drivers such as tweeters");
  await ask.getByRole("button", { name: "Cancel" }).click();
  expect((await x0()).bypass).toBe(false);
  await list.getByRole("button", { name: "Bypass All" }).click();
  await ask.getByRole("button", { name: "Bypass All" }).click();
  await expect.poll(async () => (await x0()).bypass).toBe(true);
  await expect(list.getByRole("checkbox", { name: "Crossover 1 active" })).not.toBeChecked();
  await list.getByRole("button", { name: "Enable All" }).click();
  await expect.poll(async () => (await x0()).bypass).toBe(false);
  await list.getByRole("checkbox", { name: "Crossover 1 active" }).uncheck();
  await expect.poll(async () => (await x0()).bypass).toBe(true);
  // Clear All asks, then every band is off at 1 kHz, none bypassed.
  await list.getByRole("button", { name: "Clear All" }).click();
  await page.getByRole("dialog", { name: "Clear All Bands?" }).getByRole("button", { name: "Clear All" }).click();
  await expect.poll(async () => (await core(page)).xover[w]!.map((b) => [b.filterType, b.freq, b.bypass])).toEqual(Array(4).fill([0, 1000, false]));
  // The PEQ tab again: OUT1's band list (-- WP WEB6 -- the Equaliser's PEQ tab; OUT1 kept chosen).
  await openPage(session, "EQ");
  await expect(session.getByRole("region", { name: /^Bands of OUT1/ })).toBeVisible();
});

test("WB6c M49: a Linkwitz Transform is offered on an output, edited in its panel with Apply, read back", async ({ page }) => {
  const { session } = await openOut1(page);
  const w = (await core(page)).numInputChannels;
  const slot = (await core(page)).filters[w]!.findIndex((b) => b.filterType === 0);
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  await list.getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption({ label: "Linkwitz Transform" });
  await expect.poll(async () => (await core(page)).filters[w]![slot]!.filterType).toBe(11);
  await list.getByRole("button", { name: `Band ${slot + 1} Linkwitz Transform` }).click();
  const dlg = page.getByRole("dialog", { name: "Linkwitz Transform" });
  await type(dlg.getByRole("textbox", { name: "f0", exact: true }), "40");
  await type(dlg.getByRole("textbox", { name: "fp", exact: true }), "25");
  await type(dlg.getByRole("textbox", { name: "Qp", exact: true }), "0.6");
  await expect(dlg.getByText("Not applied yet")).toBeVisible();
  await expect(dlg.getByText("DC boost +8.2 dB")).toBeVisible();
  // Nothing sent before Apply.
  expect((await core(page)).filters[w]![slot]!.gain).not.toBe(25);
  await dlg.getByRole("button", { name: "Apply" }).click();
  await expect.poll(async () => {
    const b = (await core(page)).filters[w]![slot]!;
    return [b.filterType, b.freq, Math.round(b.gain * 100) / 100, Math.round(b.qp * 100) / 100];
  }).toEqual([11, 40, 25, 0.6]);
  await expect(dlg.getByText("Applied", { exact: true })).toBeVisible();
  // Revert drops a draft.
  await type(dlg.getByRole("textbox", { name: "fp", exact: true }), "30");
  await dlg.getByRole("button", { name: "Revert" }).click();
  await expect(dlg.getByRole("textbox", { name: "fp", exact: true })).toHaveValue("25.00 Hz");
  await dlg.getByRole("button", { name: "Close" }).click();
  await expect(list.getByRole("combobox", { name: `Band ${slot + 1} type` })).toHaveValue("11");
});

test("WB6c WB6b-4 and WB6b-2: the shape card asks a shape's slope; a host's data-theme flip redraws the graph", async ({ page }) => {
  const { session } = await openOut1(page);
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  await drawn(graph);
  const w = (await core(page)).numInputChannels;
  const before = (await core(page)).filters[w]!.map((b) => b.filterType);
  // Below the output card: on screen first, a mouse click needs it.
  await graph.evaluate((e) => e.scrollIntoView({ block: "center" }));
  const box = (await graph.boundingBox())!;
  await page.keyboard.down("Control");
  await page.mouse.click(box.x + box.width * 0.3, box.y + box.height * 0.92);
  await page.keyboard.up("Control");
  const card = graph.getByRole("menu", { name: "Add Band" });
  await card.getByRole("menuitem", { name: "Low Shelf…" }).click();
  await expect(card.getByRole("menuitem", { name: "12 dB" })).toBeVisible();
  await expect(card.getByRole("menuitem", { name: "6 dB" })).toBeFocused();
  await card.getByRole("menuitem", { name: "6 dB" }).click();
  await expect.poll(async () => (await core(page)).filters[w]!.filter((t, i) => t.filterType === 9 && before[i] === 0).length).toBe(1);
  // A shape with one slope adds at once (-- WP WEB6 -- below the graph's toggle chips in its top right corner).
  await page.keyboard.down("Control");
  await page.mouse.click(box.x + box.width * 0.6, box.y + box.height * 0.2);
  await page.keyboard.up("Control");
  await graph.getByRole("menu", { name: "Add Band" }).getByRole("menuitem", { name: "Bell" }).click();
  await expect.poll(async () => (await core(page)).filters[w]!.filter((t, i) => t.filterType === 1 && before[i] === 0).length).toBe(1);

  // The host flips data-theme on the console: the canvas draws again.
  await drawn(graph);
  const n0 = await draws(graph);
  await page.evaluate(() => document.querySelector("#one .dspi-web, #one.dspi-web")?.setAttribute("data-theme", "dark"));
  await expect.poll(() => draws(graph)).toBeGreaterThan(n0);
  const n1 = await draws(graph);
  await page.evaluate(() => document.querySelector("#one .dspi-web, #one.dspi-web")?.setAttribute("data-theme", "light"));
  await expect.poll(() => draws(graph)).toBeGreaterThan(n1);
});

test("WB6c matrix: crosspoints connect, level and invert; outputs enable, gain, delay (SL1) and mute; a column renames; read back", async ({ page }) => {
  const errors = errorsOf(page);
  const { matrix } = await openMatrix(page);
  const s0 = await core(page);
  const inName = s0.channelNames[1]!;
  const where = `${esc(inName)} to ${OUT(2)}`;
  // -- WP MXW -- a crosspoint is a grid cell: a click on an empty one connects it, a second
  // click on the chosen one disconnects it; its gain is typed in its tile, I inverts it
  const point = matrix.getByRole("gridcell", { name: re(`${where}, .*`) });
  const was = s0.matrixRouting[1]![2]!;
  await point.click();
  if (was) {
    await point.click();
    await expect.poll(async () => (await core(page)).matrixRouting[1]![2]).toBe(false);
    await point.click();
  }
  await expect.poll(async () => (await core(page)).matrixRouting[1]![2]).toBe(true);
  await expect(point).toHaveAttribute("data-connected", "true");
  await point.dblclick();
  await type(matrix.getByRole("textbox", { name: re(`${where} level`) }), "+3,5");
  await expect.poll(async () => (await core(page)).matrixGain[1]![2]).toBe(3.5);
  await expect(point).toContainText("+3.5");
  await expect(point).toHaveAttribute("aria-label", /connected, \+3\.5 dB$/);
  await point.press("i");
  await expect.poll(async () => (await core(page)).matrixInvert[1]![2]).toBe(true);
  await expect(point).toHaveAttribute("aria-label", /phase inverted$/);

  // Output rows.
  const o = 1;
  const oName = OUT(o);
  await type(matrix.getByRole("textbox", { name: re(`${oName} gain`) }), "−2,5");
  await expect.poll(async () => (await core(page)).outputGainDb[o]).toBe(-2.5);
  await expect(matrix.getByRole("textbox", { name: re(`${oName} gain`) })).toHaveValue("-2.5");
  const delay = matrix.getByRole("textbox", { name: re(`${oName} delay`) });
  await type(delay, "4.125 ms");
  await expect.poll(async () => (await core(page)).outputDelayMs[o]).toBe(4.125);
  await expect(delay).toHaveValue("4.125");
  await delay.hover();
  await page.mouse.wheel(0, -100);
  await expect.poll(async () => (await core(page)).outputDelayMs[o]).toBe(5);
  await page.mouse.wheel(0, 100);
  await expect.poll(async () => (await core(page)).outputDelayMs[o]).toBe(4);
  await expect(delay).toHaveValue(/^4(\.0)?$/); // -- WP WEB3 -- WEB2 shows a whole delay bare ("4", REVD W11)
  const mute = matrix.getByRole("button", { name: re(`Mute ${oName}`) });
  await mute.click();
  await expect.poll(async () => (await core(page)).outputMuted[o]).toBe(true);
  await expect(mute).toHaveAttribute("aria-pressed", "true");
  const enable = matrix.getByRole("button", { name: re(`Enable ${oName}`) });
  await enable.click();
  await expect.poll(async () => (await core(page)).outputEnabled[o]).toBe(false);
  await expect(enable).toHaveAttribute("aria-pressed", "false");
  await enable.click();
  await expect.poll(async () => (await core(page)).outputEnabled[o]).toBe(true);

  // Enabling PDM while EQ-worker outputs are on asks first; Cancel changes nothing.
  const s1 = await core(page);
  const pdm = s1.numOutputChannels - 1;
  if (!s1.outputEnabled[pdm] && s1.outputEnabled.slice(2, pdm).some((x) => x)) {
    await matrix.getByRole("button", { name: re(`Enable ${OUT(pdm)}`) }).click();
    const ask = page.getByRole("dialog", { name: "Warning" });
    await expect(ask).toContainText("will be disabled. Are you sure?");
    await ask.getByRole("button", { name: "Cancel" }).click();
    expect((await core(page)).outputEnabled[pdm]).toBe(false);
  }

  // Rename from the column's menu (-- WP MXW -- its OUTn pill).
  await matrix.getByRole("button", { name: `OUT${o + 1}`, exact: true }).click();
  await matrix.getByRole("menuitem", { name: "Rename…" }).click();
  const dlg = page.getByRole("dialog", { name: re(`Rename ${oName}`) });
  await dlg.getByRole("textbox").fill("Sub left");
  await dlg.getByRole("button", { name: "Rename" }).click();
  await expect.poll(async () => (await core(page)).channelNames[s0.numInputChannels + o]).toBe("Sub left");
  await expect(matrix.getByRole("button", { name: "Sub left", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("WB6c redraws: the output page and the matrix change nothing while idle; a change touches only its own part", async ({ page }) => {
  const { session, card } = await openOut1(page);
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  // -- fixup b4fix F1 -- in view first: a graph below the fold defers what changed until it is
  // seen (EqGraph's IntersectionObserver), so counting from there would count that owed draw
  // when the Delay field's click scrolls it in; the scene bar above it put it below the fold.
  await graph.scrollIntoViewIfNeeded();
  await drawn(graph);
  const watch = (sel: string) =>
    page.evaluate((sel) => {
      const g = globalThis as unknown as { dspiMut: number };
      g.dspiMut = 0;
      new MutationObserver((m) => {
        g.dspiMut += m.length;
      }).observe(document.querySelector(sel)!, { subtree: true, childList: true, attributes: true, characterData: true });
    }, sel);
  const mutations = () => page.evaluate(() => (globalThis as unknown as { dspiMut: number }).dspiMut);
  await logCalls(page);
  await watch("#one .eq-screen");
  const n0 = await draws(graph);
  await page.waitForTimeout(3000);
  expect(await mutations()).toBe(0);
  expect(await draws(graph)).toBe(n0);
  const calls = await page.evaluate(() => (globalThis as unknown as G).dspiLog.map(([m]) => m));
  expect(calls.filter((m) => m !== "meters")).toEqual([]);
  // A delay change: the canvas is not drawn again (the curve did not change).
  await type(card.getByRole("textbox", { name: "Delay" }), "2");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBe(2);
  await page.waitForTimeout(300);
  expect(await draws(graph)).toBe(n0);

  await openPage(session, "Matrix");
  await expect(session.getByRole("region", { name: "Matrix Mixer" })).toBeVisible();
  await page.waitForTimeout(300);
  await watch("#one .matrix-screen");
  await page.waitForTimeout(3000);
  expect(await mutations()).toBe(0);
});

test("WB6c read only: a view-only session shows the output page and the matrix with every control off", async ({ page }) => {
  const { session, card } = await openOut1(page, "Venue B");
  await expect(card.getByRole("textbox", { name: "Delay" })).toBeDisabled();
  await expect(card.getByRole("slider", { name: "Delay" })).toBeDisabled();
  await expect(card.getByRole("button", { name: "Mute" })).toBeDisabled();
  await openPage(session, "Crossover"); // -- WP WEB6 -- an output's XO bands are on the Crossover tab (Qt's), OUT1 still chosen
  await expect(session.getByRole("combobox", { name: "Crossover 1 family" })).toBeDisabled();
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  // -- WP MXW -- every control but the names that choose a row or column for the inspector
  const buttons = matrix.locator("button:not(.mx-pick)");
  const n = await buttons.count();
  expect(n).toBeGreaterThan(0);
  for (let i = 0; i < n; i++) await expect(buttons.nth(i)).toBeDisabled();
  for (const tool of ["Direct 1:1", "Clear"]) await expect(session.getByRole("toolbar", { name: "Routing tools" }).getByRole("button", { name: tool })).toBeDisabled();
  // Nothing in the grid changes: a click on an empty cell only chooses it, keys do nothing
  const s0 = await core(page);
  const empty = matrix.locator('[role="gridcell"][data-connected="false"]').first();
  await empty.click();
  await empty.press("Space");
  await empty.press("Enter");
  await expect(matrix.getByRole("textbox", { name: / level$/ })).toHaveCount(0);
  expect((await core(page)).matrixRouting).toEqual(s0.matrixRouting);
});

test.describe("375 px, in every browser", () => {
  test.use({ viewport: { width: 375, height: 800 }, hasTouch: true });

  const overflow = (page: Page, inside: string) =>
    page.evaluate((inside) => {
      const w = document.documentElement.clientWidth;
      return Array.from(document.querySelectorAll<HTMLElement>(".dspi-web *"))
        .filter((e) => e.getBoundingClientRect().right > w + 0.5 && !e.closest(".eq-channels, .rail, .page-tabs, .device-tab-row") && !e.closest(inside))
        .map((e) => `${e.tagName.toLowerCase()}.${e.className} ${Math.round(e.getBoundingClientRect().right)}`)
        .slice(0, 8);
    }, inside);

  test("WB6c narrow: the output page and the matrix fit; the matrix scrolls in its card; touch works; nothing grows", async ({ page }) => {
    const { session, card } = await openOut1(page, "Venue A", "/test/embed.html?one");
    expect(await overflow(page, ".none")).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
    const screen = session.locator(".eq-screen");
    const h0 = (await screen.boundingBox())!.height;
    // A tap mutes, and the card keeps its height.
    const s0 = await core(page);
    await card.getByRole("button", { name: "Mute" }).tap();
    await expect.poll(async () => (await core(page)).outputMuted[0]).toBe(true);
    await card.getByRole("button", { name: `Route ${s0.channelNames[0]}` }).tap();
    await expect.poll(async () => (await core(page)).matrixRouting[0]![0]).toBe(!s0.matrixRouting[0]![0]);
    expect((await screen.boundingBox())!.height).toBe(h0);
    await page.screenshot({ path: test.info().outputPath("output-375.png"), fullPage: true });

    await openPage(session, "Matrix");
    const scroll = session.getByRole("region", { name: "Matrix", exact: true });
    await expect(scroll).toBeVisible();
    expect(await overflow(page, ".mx-scroll")).toEqual([]);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
    expect(await scroll.evaluate((e) => e.scrollWidth > e.clientWidth)).toBe(true);
    const ms = (await session.locator(".matrix-screen").boundingBox())!.height;
    const inName = s0.channelNames[0]!;
    // -- WP MXW -- a tap on an empty cell connects it; on a connected one it chooses it, and a second tap disconnects it
    const point = scroll.getByRole("gridcell", { name: re(`${esc(inName)} to ${OUT(1)}, .*`) });
    const was = (await core(page)).matrixRouting[0]![1]!;
    await point.tap();
    if (was) {
      await page.waitForTimeout(500);
      await point.tap();
    }
    await expect.poll(async () => (await core(page)).matrixRouting[0]![1]).toBe(!was);
    // A cell's level and polarity are inside its tile: the matrix does not grow.
    // -- WP P5WMS -- within a thousandth of a pixel: Firefox rounds a box at a fractional top (the
    // tab row's wrap moved with the Measure tab) differently from one layout to the next
    expect((await session.locator(".matrix-screen").boundingBox())!.height).toBeCloseTo(ms, 3);
    await page.screenshot({ path: test.info().outputPath("matrix-375.png"), fullPage: true });
  });
});
