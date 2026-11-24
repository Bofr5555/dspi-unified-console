// WP WB6b e2e: the EQ screen on the core's demo device (crates/dspi-wasm), in
// Chromium, Firefox and Chromium at 375 px (the `narrow` project), plus a
// 375 px block with touch in every project. Every edit is made through the
// page (mouse, keys, touch, the chip, the list) and read back twice: the page
// shows the device's value, and the core's own state (a direct call) agrees.
// A redraw probe shows the canvas and the core idle while nothing changes.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number };
type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = { numInputChannels: number; filters: Band[][]; presetNames: string[]; activePresetSlot: number };

async function coreState(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate((n) => (globalThis as unknown as Clients).dspiClients[n]!.call("state", null), n)) as { state: State };
  return snap.state;
}

/** OUT1's bands in the core. */
async function out1(page: Page, n = 0): Promise<Band[]> {
  const s = await coreState(page, n);
  return s.filters[s.numInputChannels]!;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

/** Console one's Venue A (control) or Venue B (view), on its EQ screen with OUT1 chosen. */
async function openEq(page: Page, venue = "Venue A") {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  await expect(graph).toBeVisible();
  // -- WP WB6c -- an output's settings card sits above the graph: on screen for the mouse
  await graph.evaluate((e) => e.scrollIntoView({ block: "center" }));
  await drawn(graph);
  return { session, graph };
}

const draws = (graph: Locator) => graph.locator("canvas").evaluate((c) => (c as unknown as { dspiDraws?: number }).dspiDraws ?? 0);

/** Until the canvas has been drawn and has stopped changing. */
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

/** Where the graph puts a frequency and a level (log 20 Hz..20 kHz across, +25..-25 dB down). */
async function at(graph: Locator, freq: number, db: number) {
  const b = (await graph.boundingBox())!;
  const fx = (Math.log10(freq) - Math.log10(20)) / (Math.log10(20000) - Math.log10(20));
  return { x: b.x + fx * b.width, y: b.y + ((25 - db) / 50) * b.height };
}

/** OUT1's drawn level at `freq`: its curve from the core (PEQ and crossover), shifted by its gain. */
async function levelAt(page: Page, freq: number): Promise<number> {
  return page.evaluate(async (freq) => {
    const c = (globalThis as unknown as Clients).dspiClients[0]!;
    const st = ((await c.call("state", null)) as { state: State & { xover: Band[][]; outputGainDb: number[] } }).state;
    const w = st.numInputChannels;
    const grid = (await c.call("curveGrid", null)) as { freqs: number[] };
    const r = (await c.call("curves", { chains: [[...st.filters[w]!.slice(0, 10), ...st.xover[w]!]] })) as { chains: number[][] };
    let i = 0;
    while (i < grid.freqs.length - 1 && grid.freqs[i + 1]! < freq) i++;
    return r.chains[0]![i]! + (st.outputGainDb[0] ?? 0);
  }, freq);
}

/** A synthetic touch pointer event (every browser takes PointerEvents with pointerType "touch"). */
type Touch = [type: string, id: number, x: number, y: number];

/** -- INTC -- One touch gesture: its pointer events in order, in one call into the page, each in its own task (the
 * page handles and renders it before the next, as with a real finger). The graph times its double tap (400 ms) and
 * long press (500 ms); with a round trip to the test per event, a loaded host turned the drag into a long press. */
async function gesture(graph: Locator, steps: Touch[]) {
  await graph.evaluate(async (el, steps) => {
    for (const [type, id, x, y] of steps) {
      el.dispatchEvent(
        new PointerEvent(type, { pointerId: id, pointerType: "touch", isPrimary: id === 1, clientX: x, clientY: y, bubbles: true, button: 0, buttons: type === "pointerup" ? 0 : 1 }),
      );
      await new Promise((r) => setTimeout(r, 0));
    }
  }, steps);
}

test("WB6b graph: add a band by double click, drag it, and the device keeps what the page shows", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, graph } = await openEq(page);
  const before = await out1(page);
  const slot = before.findIndex((b) => b.filterType === 0);
  expect(slot).toBeGreaterThanOrEqual(0);

  // Double click empty graph: a bell at 1 kHz, +6 dB.
  const p = await at(graph, 1000, 6);
  await page.mouse.dblclick(p.x, p.y);
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(1);
  let band = (await out1(page))[slot]!;
  expect(Math.abs(band.freq - 1000)).toBeLessThan(30);
  expect(Math.abs(band.gain - 6)).toBeLessThan(0.6);
  const chip = session.getByRole("group", { name: `Band ${slot + 1}` });
  await expect(chip).toBeVisible();
  await expect(chip.getByRole("textbox", { name: "Gain" })).toHaveValue(`+${band.gain.toFixed(2)} dB`);

  // Drag its dot an octave up and 5 dB down.
  const dot = await at(graph, band.freq, band.gain);
  const to = await at(graph, band.freq * 2, band.gain - 5);
  await page.mouse.move(dot.x, dot.y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(dot.x + ((to.x - dot.x) * i) / 8, dot.y + ((to.y - dot.y) * i) / 8);
  await page.mouse.up();
  await expect.poll(async () => (await out1(page))[slot]!.freq).toBeGreaterThan(band.freq * 1.8);
  const moved = (await out1(page))[slot]!;
  expect(Math.abs(moved.freq / band.freq - 2)).toBeLessThan(0.1);
  expect(Math.abs(moved.gain - (band.gain - 5))).toBeLessThan(0.6);
  // The page shows what the device kept, in the chip and in the list.
  await expect(chip.getByRole("textbox", { name: "Gain" })).toHaveValue(`${moved.gain >= 0 ? "+" : ""}${moved.gain.toFixed(2)} dB`);
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  // (On a phone the list shows it in short, "+1.0".)
  const short = `${moved.gain >= 0 ? "+" : ""}${moved.gain.toFixed(1)}`.replace(/[.+]/g, "\\$&");
  await expect(list.getByRole("textbox", { name: `Band ${slot + 1} gain` })).toHaveValue(new RegExp(`^${short}`));
  // Every band is labelled for a screen reader, the chosen one said.
  await expect(graph.locator(".graph-announce")).toContainText(`Band ${slot + 1}: Peaking`);
  await expect(graph.getByRole("list", { name: "Bands on the graph" })).toContainText(`Band ${slot + 1}: Peaking`);

  // Pull a band out of the curve: drag from the curve far from any dot.
  const free = (await out1(page)).findIndex((b) => b.filterType === 0);
  // Near the low end, the pulled band is a low shelf.
  const from = await at(graph, 30, await levelAt(page, 30));
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move(from.x + 3, from.y - 10);
  await page.mouse.move(from.x + 5, from.y - 30);
  await page.mouse.up();
  await expect.poll(async () => (await out1(page))[free]!.filterType).toBe(2);
  expect((await out1(page))[free]!.gain).toBeGreaterThan(2);
  expect(errors).toEqual([]);
});

test("WB6b chip and list: change type and Q, bypass a band (chip, Alt-click, list), all read back", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, graph } = await openEq(page);
  const slot = (await out1(page)).findIndex((b) => b.filterType === 0);
  // A band by the list: its type picker.
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  await list.getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption("1");
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(1);
  await list.getByRole("textbox", { name: `Band ${slot + 1} frequency` }).fill("2k");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await out1(page))[slot]!.freq).toBe(2000);
  await list.getByRole("button", { name: `Band ${slot + 1}`, exact: true }).click();
  const chip = session.getByRole("group", { name: `Band ${slot + 1}` });

  // The chip: Q typed, then the shape and the slope.
  await chip.getByRole("textbox", { name: "Q", exact: true }).fill("2.5");
  await page.keyboard.press("Enter");
  await expect.poll(async () => (await out1(page))[slot]!.q).toBe(2.5);
  await expect(chip.getByRole("textbox", { name: "Q", exact: true })).toHaveValue("2.500");
  await chip.getByRole("button", { name: "Q up" }).click();
  await expect.poll(async () => (await out1(page))[slot]!.q).toBeCloseTo(2.5 * 2 ** 0.1, 2);
  await chip.getByRole("combobox", { name: "Shape" }).selectOption("lowShelf");
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(2);
  await chip.getByRole("combobox", { name: "Slope" }).selectOption("1");
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(9);
  await expect(list.getByRole("combobox", { name: `Band ${slot + 1} type` })).toHaveValue("9");
  // A 1st-order shelf has no Q.
  await expect(chip.getByRole("textbox", { name: "Q", exact: true })).toBeDisabled();

  // Bypass: the chip's button, the list's box, Alt-click on the dot.
  await chip.getByRole("button", { name: "Bypass" }).click();
  await expect.poll(async () => (await out1(page))[slot]!.bypass).toBe(true);
  await expect(list.getByRole("checkbox", { name: `Band ${slot + 1} bypass` })).toBeChecked();
  await list.getByRole("checkbox", { name: `Band ${slot + 1} bypass` }).uncheck();
  await expect.poll(async () => (await out1(page))[slot]!.bypass).toBe(false);
  const b = (await out1(page))[slot]!;
  // A 1st-order shelf's dot is at about half its gain.
  const dot = await at(graph, b.freq, b.gain / 2);
  await page.keyboard.down("Alt");
  await page.mouse.click(dot.x, dot.y);
  await page.keyboard.up("Alt");
  await expect.poll(async () => (await out1(page))[slot]!.bypass).toBe(true);
  await expect(chip.getByRole("button", { name: "Bypass" })).toHaveAttribute("aria-pressed", "true");

  // The band menu: right click on the dot, Delete Band.
  await page.mouse.click(dot.x, dot.y, { button: "right" });
  await graph.getByRole("menu", { name: "Band" }).getByRole("menuitem", { name: "Delete Band" }).click();
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(0);

  // Clear All asks first, as on the Console.
  await list.getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption("1");
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(1);
  await list.getByRole("button", { name: "Clear All" }).click();
  const ask = page.getByRole("dialog", { name: "Clear All Bands?" });
  await expect(ask).toContainText("One Undo puts them all back."); // -- WP P1WB2 -- one undo group
  await ask.getByRole("button", { name: "Clear All" }).click();
  await expect.poll(async () => (await out1(page)).every((x) => x.filterType === 0)).toBe(true);
  expect(errors).toEqual([]);
});

test("WB6b keys: Tab chooses a band, arrows move frequency and gain, Alt-arrows Q, Delete removes", async ({ page }) => {
  const { session, graph } = await openEq(page);
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  const slot = (await out1(page)).findIndex((b) => b.filterType === 0);
  await list.getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption("1");
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(1);
  const start = (await out1(page))[slot]!;
  const order = (await out1(page)).map((b, i) => ({ b, i })).filter((x) => x.b.filterType !== 0).sort((a, b) => a.b.freq - b.b.freq || a.i - b.i);
  const steps = order.findIndex((x) => x.i === slot) + 1;

  await graph.focus();
  for (let i = 0; i < steps; i++) await page.keyboard.press("Tab");
  await expect(graph).toBeFocused();
  await expect(graph.locator(".graph-announce")).toContainText(`Band ${slot + 1}:`);
  await page.keyboard.press("ArrowRight");
  await expect.poll(async () => (await out1(page))[slot]!.freq).toBeCloseTo(start.freq * 2 ** (1 / 12), -1);
  await page.keyboard.press("ArrowUp");
  await expect.poll(async () => (await out1(page))[slot]!.gain).toBeCloseTo(start.gain + 0.5, 1);
  await page.keyboard.press("Shift+ArrowDown");
  await expect.poll(async () => (await out1(page))[slot]!.gain).toBeCloseTo(start.gain + 0.4, 1);
  await page.keyboard.press("Alt+ArrowUp");
  await expect.poll(async () => (await out1(page))[slot]!.q).toBeCloseTo(start.q * 2 ** 0.1, 2);
  // Enter types values: the chip's frequency takes focus.
  await page.keyboard.press("Enter");
  await expect(session.getByRole("group", { name: `Band ${slot + 1}` }).getByRole("textbox", { name: "Frequency" })).toBeFocused();
  await page.keyboard.press("Escape");
  await graph.focus();
  // The band menu from the keyboard.
  await page.keyboard.press("Shift+F10");
  await expect(graph.getByRole("menu", { name: "Band" })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(graph).toBeFocused();
  await page.keyboard.press("Delete");
  await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(0);
  // Past the last band, Tab leaves the graph.
  for (let i = 0; i < 12; i++) await page.keyboard.press("Tab");
  await expect(graph).not.toBeFocused();
});

test("WB6b marquee and wheel: a marquee chooses the bands inside it; the wheel steps their Q, Ctrl-wheel their gain", async ({ page }) => {
  const { session, graph } = await openEq(page);
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  const free = (await out1(page)).map((b, i) => (b.filterType === 0 ? i : -1)).filter((i) => i >= 0);
  const [a, b] = [free[0]!, free[1]!];
  for (const [n, f] of [
    [a, "300"],
    [b, "600"],
  ] as const) {
    await list.getByRole("combobox", { name: `Band ${n + 1} type` }).selectOption("1");
    await list.getByRole("textbox", { name: `Band ${n + 1} frequency` }).fill(f);
    await page.keyboard.press("Enter");
  }
  await expect.poll(async () => (await out1(page))[b]!.freq).toBe(600);
  // A marquee from below-left of both dots (0 dB) to above-right.
  const from = await at(graph, 250, -6);
  const to = await at(graph, 700, 6);
  await page.mouse.move(from.x, from.y);
  await page.mouse.down();
  await page.mouse.move((from.x + to.x) / 2, (from.y + to.y) / 2);
  await page.mouse.move(to.x, to.y);
  await page.mouse.up();
  for (const n of [a, b]) await expect(list.getByRole("button", { name: `Band ${n + 1}`, exact: true })).toHaveAttribute("aria-pressed", "true");
  // The wheel over one of them: both Qs; with Ctrl: both gains.
  const q0 = (await out1(page))[a]!.q;
  const dot = await at(graph, 300, 0);
  await page.mouse.move(dot.x, dot.y);
  await page.mouse.wheel(0, -100);
  await expect.poll(async () => (await out1(page))[b]!.q).toBeGreaterThan(q0 * 1.5);
  expect((await out1(page))[a]!.q).toBeGreaterThan(q0 * 1.5);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -40);
  await page.keyboard.up("Control");
  await expect.poll(async () => (await out1(page))[a]!.gain).toBeGreaterThan(1.5);
  expect((await out1(page))[b]!.gain).toBeGreaterThan(1.5);
});

test("WB6b redraws: the canvas and the core stay idle while nothing changes; a change draws again", async ({ page }) => {
  const { graph } = await openEq(page);
  // Count what the page asks of the core from now on.
  await page.evaluate(() => {
    const c = (globalThis as unknown as Clients & { dspiCalls: string[] }).dspiClients[0]!;
    const calls: string[] = [];
    (globalThis as unknown as { dspiCalls: string[] }).dspiCalls = calls;
    const call = c.call.bind(c);
    c.call = (m: string, p: unknown) => {
      calls.push(m);
      return call(m, p);
    };
  });
  const n0 = await draws(graph);
  await page.waitForTimeout(3000);
  expect(await draws(graph)).toBe(n0);
  const calls = await page.evaluate(() => (globalThis as unknown as { dspiCalls: string[] }).dspiCalls);
  expect(calls.filter((m) => m === "curves" || m === "curveGrid")).toEqual([]);
  // A hover over empty graph changes nothing drawn.
  const p = await at(graph, 3000, -20);
  await page.mouse.move(p.x, p.y);
  await page.mouse.move(p.x + 10, p.y);
  await page.waitForTimeout(300);
  expect(await draws(graph)).toBe(n0);
  // A change: drawn again, a few frames at most.
  await page.mouse.dblclick(p.x, p.y);
  await drawn(graph);
  const n1 = await draws(graph);
  expect(n1).toBeGreaterThan(n0);
  expect(n1 - n0).toBeLessThan(8);
  const after = await page.evaluate(() => (globalThis as unknown as { dspiCalls: string[] }).dspiCalls);
  expect(after.filter((m) => m === "curves").length).toBeGreaterThan(0);
  expect(after.filter((m) => m === "curves").length).toBeLessThan(5);
});

test("WB6b read only: a view-only session shows the EQ with editing off, and a double click adds nothing", async ({ page }) => {
  const { session, graph } = await openEq(page, "Venue B");
  const before = await out1(page);
  const p = await at(graph, 1000, 6);
  await page.mouse.dblclick(p.x, p.y);
  await page.waitForTimeout(300);
  expect(await out1(page)).toEqual(before);
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  for (const c of await list.getByRole("combobox").all()) await expect(c).toBeDisabled();
  for (const t of await list.getByRole("textbox").all()) await expect(t).toBeDisabled();
  await expect(list.getByRole("button", { name: "Clear All" })).toBeDisabled();
  for (const name of ["Shape", "Slope"]) await expect(session.getByRole("combobox", { name, exact: true })).toBeDisabled();
  await expect(session.getByRole("textbox", { name: "Frequency", exact: true })).toBeDisabled();
  // Keys change nothing either.
  await graph.focus();
  await page.keyboard.press("Tab");
  await page.keyboard.press("ArrowUp");
  await page.keyboard.press("Delete");
  await page.waitForTimeout(200);
  expect(await out1(page)).toEqual(before);
});

test("WB6b unsaved: the session's state survives a screen switch; the host page learns it (leavePrompt off)", async ({ page }) => {
  await page.goto("/test/embed.html");
  const two = page.locator("#two");
  await two.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = two.getByRole("article", { name: "DSPi" });
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const list = session.getByRole("region", { name: /^Bands of OUT1/ });
  const slot = (await out1(page)).findIndex((b) => b.filterType === 0);
  await list.getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption("1");
  const unsaved = () => page.evaluate(() => (globalThis as unknown as { dspiConsoles: { unsaved(): boolean }[] }).dspiConsoles[1]!.unsaved());
  await expect.poll(unsaved).toBe(true);
  expect(await page.evaluate(() => (globalThis as unknown as { dspiUnsavedChanges: boolean[] }).dspiUnsavedChanges)).toEqual([true]);
  // The Main screen shows it too; Disconnect asks from the EQ screen as well.
  await openPage(session, "Main");
  // -- WP WB6F -- exact: the status rail says "· unsaved changes" too
  await expect(session.getByText("Unsaved changes", { exact: true })).toBeVisible();
  await openPage(session, "EQ");
  await session.getByRole("button", { name: "Disconnect" }).click();
  const prompt = page.getByRole("dialog", { name: "Unsaved Changes" });
  await expect(prompt).toBeVisible();
  await prompt.getByRole("button", { name: "Save" }).click();
  await expect(two.getByText("Choose a DSPi to connect to.")).toBeVisible();
  await expect.poll(unsaved).toBe(false);
  // No leave prompt for this console: the page closes without asking.
  let asked = false;
  page.on("dialog", (d) => {
    asked = true;
    void d.dismiss();
  });
  await page.close({ runBeforeUnload: true });
  expect(asked).toBe(false);
});

test("WB6b leave prompt: on by default, while something is unsaved the browser asks before leaving", async ({ page }) => {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const slot = (await out1(page)).findIndex((b) => b.filterType === 0);
  await session.getByRole("region", { name: /^Bands of OUT1/ }).getByRole("combobox", { name: `Band ${slot + 1} type` }).selectOption("1");
  await expect.poll(() => page.evaluate(() => (globalThis as unknown as { dspiConsoles: { unsaved(): boolean }[] }).dspiConsoles[0]!.unsaved())).toBe(true);
  const dialog = page.waitForEvent("dialog");
  await page.close({ runBeforeUnload: true });
  const d = await dialog;
  expect(d.type()).toBe("beforeunload");
  await d.dismiss();
});

test.describe("375 px, in every browser", () => {
  test.use({ viewport: { width: 375, height: 800 } });

  test("WB6b narrow: no sideways scroll, nothing grows; touch adds, drags and pinches a band", async ({ page }, info) => {
    // The embedding page with one console, full width.
    await page.goto("/test/embed.html?one");
    await page.locator("#one").getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
    const session = page.locator("#one").getByRole("article", { name: "DSPi" });
    await openPage(session, "EQ");
    await session.locator(".eq-pick", { hasText: "OUT1" }).click();
    const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
    await drawn(graph);
    const screen = session.locator(".eq-screen");
    const overflow = () =>
      page.evaluate(() => {
        const w = document.documentElement.clientWidth;
        const wide = Array.from(document.querySelectorAll<HTMLElement>(".dspi-web *"))
          .filter((e) => e.getBoundingClientRect().right > w + 0.5 && !e.closest(".eq-channels, .rail, .page-tabs, .device-tab-row") /* -- WP WEBM1 -- rows that scroll on their own */)
          .map((e) => `${e.tagName.toLowerCase()}.${e.className} ${Math.round(e.getBoundingClientRect().right)}`);
        return wide.slice(0, 8);
      });
    expect(await overflow()).toEqual([]);
    const height = (await screen.boundingBox())!.height;

    // Touch: a double tap adds a bell.
    const slot = (await out1(page)).findIndex((b) => b.filterType === 0);
    const p = await at(graph, 1000, 6);
    await gesture(graph, ["pointerdown", "pointerup", "pointerdown", "pointerup"].map((t): Touch => [t, 1, p.x, p.y]));
    await expect.poll(async () => (await out1(page))[slot]!.filterType).toBe(1);
    const band = (await out1(page))[slot]!;
    // A finger drags its dot up 4 dB.
    const dot = await at(graph, band.freq, band.gain);
    const up = await at(graph, band.freq, band.gain + 4);
    const drag: Touch[] = [["pointerdown", 1, dot.x, dot.y]];
    for (let i = 1; i <= 6; i++) drag.push(["pointermove", 1, dot.x, dot.y + ((up.y - dot.y) * i) / 6]);
    drag.push(["pointerup", 1, up.x, up.y]);
    await gesture(graph, drag);
    await expect.poll(async () => (await out1(page))[slot]!.gain).toBeGreaterThan(band.gain + 3);
    // Two fingers pinch its Q: closer together, narrower.
    const q0 = (await out1(page))[slot]!.q;
    const pinch: Touch[] = [
      ["pointerdown", 1, dot.x - 60, dot.y],
      ["pointerdown", 2, dot.x + 60, dot.y],
    ];
    for (let i = 1; i <= 4; i++) {
      pinch.push(["pointermove", 1, dot.x - 60 + i * 7, dot.y]);
      pinch.push(["pointermove", 2, dot.x + 60 - i * 7, dot.y]);
    }
    pinch.push(["pointerup", 2, dot.x + 32, dot.y], ["pointerup", 1, dot.x - 32, dot.y]);
    await gesture(graph, pinch);
    await expect.poll(async () => (await out1(page))[slot]!.q).toBeGreaterThan(q0 * 1.5);

    // With a band chosen, its chip filled, and a menu open: the same height, still no sideways scroll.
    await expect(session.getByRole("group", { name: `Band ${slot + 1}` })).toBeVisible();
    expect(Math.abs((await screen.boundingBox())!.height - height)).toBeLessThan(0.5);
    expect(await overflow()).toEqual([]);
    await session.getByRole("group", { name: `Band ${slot + 1}` }).getByRole("button", { name: "Bypass" }).click();
    await expect.poll(async () => (await out1(page))[slot]!.bypass).toBe(true);
    expect(Math.abs((await screen.boundingBox())!.height - height)).toBeLessThan(0.5);
    await page.screenshot({ path: info.outputPath("eq-375.png"), fullPage: true });
  });
});
