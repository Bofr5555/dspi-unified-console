// WP WB6j e2e on the core's demo device (no audio anywhere: the demo only), in Chromium, Firefox
// and Chromium at 375 px: the analyser's bar strip (M22) on the main screen and a channel's page,
// the spectrum over the EQ graph (M23), the signal generator and Identify from an output's page
// and the matrix (M46), Stats for nerbs with Reset Watermarks (M53), the Interrupt Monitor (M54)
// and dashboard cards per row (M59). What the page did is read back from the core (a direct call).

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage, rail } from "../nav";

type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as G).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}

type Status = { state: number; signalType: number };
type Config = { signalType: number; channelMask: number; levelDb: number; repeat: number };

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function connect(page: Page, venue = "Venue A"): Promise<Locator> {
  await page.goto("/test/embed.html");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(rail(session)).toBeVisible();
  return session;
}

/** Whatever a test played, stopped at once (the demo's generator). */
test.afterEach(async ({ page }) => {
  await call(page, "siggenControl", { action: "stopNow" }).catch(() => undefined);
});

test("WB6J M22: the main screen's bar strip shows the chosen tap's channels from the core's analyser", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  const strip = session.getByRole("region", { name: "Spectrum" });
  await strip.getByRole("button", { name: "Show Bars" }).click();
  const cells = strip.locator(".rta-cell");
  await expect(cells).toHaveCount(2);
  await expect(cells.first()).not.toHaveAttribute("data-bars", "0");
  // The core runs the analyser on the inputs, channels 1 and 2.
  const v = await call<{ snapshot: { tap: number; running: boolean; frames: unknown[] } }>(page, "analyserView");
  expect(v.snapshot.tap).toBe(0);
  expect(v.snapshot.frames.length).toBe(2);
  // The outputs, one channel off.
  await strip.getByRole("button", { name: "Outputs" }).click();
  await expect(strip.getByRole("group", { name: "Channels analysed" }).getByRole("button").first()).toHaveAttribute("aria-pressed", "true");
  await strip.getByRole("group", { name: "Channels analysed" }).getByRole("button").nth(1).click();
  await expect(cells).toHaveCount(1);
  await expect.poll(async () => (await call<{ snapshot: { tap: number } }>(page, "analyserView")).snapshot.tap).toBe(1);
  // Hidden: the strip draws nothing, and stays hidden when the page comes back.
  await strip.getByRole("button", { name: "Hide Bars" }).click();
  await expect(cells).toHaveCount(0);
  await page.reload();
  const again = await connect(page);
  await expect(again.getByRole("region", { name: "Spectrum" }).getByRole("button", { name: "Show Bars" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("WB6J M23 M22: a channel's page draws its spectrum over the EQ graph and in its own bars", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  await session.getByRole("button", { name: "Spectrum on Graph" }).click();
  const overlay = session.locator("canvas.rta-overlay");
  await expect(overlay).toHaveCount(1);
  await expect(overlay).not.toHaveAttribute("data-bars", "0");
  // The analyser runs on the outputs, OUT1 alone.
  await expect.poll(async () => (await call<{ snapshot: { tap: number; frames: { channel: number }[] } }>(page, "analyserView")).snapshot.frames.map((f) => f.channel)).toEqual([0]);
  // The graph under it still takes the pointer: a double click adds a band.
  const graph = session.getByRole("application", { name: /^EQ graph/ });
  await expect(graph).toBeVisible();
  // The page's own bars: one cell, the channel's.
  const strip = session.getByRole("region", { name: "Spectrum" });
  await strip.getByRole("button", { name: "Show Bars" }).click();
  await expect(strip.locator(".rta-cell")).toHaveCount(1);
  await expect(strip.locator(".rta-cell figcaption")).toContainText("OUT1");
  await session.getByRole("button", { name: "Spectrum on Graph" }).click();
  await expect(overlay).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WB6J M46: the signal generator plays only on Start, shows what it does, and stops", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Signal Generator");
  const gen = session.getByRole("region", { name: "Signal Generator" });
  await expect(gen.locator(".siggen-title")).toHaveText("Ready · Sine");
  expect((await call<Status>(page, "siggenStatus")).state).toBe(0);
  // The core's catalogue: every type the demo plays, and Pink noise chosen.
  await gen.getByRole("radio", { name: "Pink" }).click();
  // -- WP WEB -- a text field read as Qt's fields read it: U+2212 and a decimal comma
  await gen.getByRole("textbox", { name: "Level in dBFS" }).fill("−30,0");
  await gen.getByRole("textbox", { name: "Level in dBFS" }).press("Enter");
  // No outputs: Play is off and says why.
  await gen.getByRole("region", { name: "Outputs" }).getByRole("button", { name: "None", exact: true }).click(); // -- WP WEB4 -- All/None at the section's label
  await expect(gen.locator(".siggen-title")).toHaveText("Select at least one output");
  await expect(gen.getByRole("button", { name: "Start" })).toBeDisabled();
  await gen.getByRole("group", { name: "Outputs" }).getByRole("button", { name: /^Out 1\b/ }).click(); // -- WP WEB7 -- the core's label alone
  await gen.getByRole("button", { name: "Start" }).click();
  await expect(gen.locator(".siggen-title")).toHaveText(/^(Fading in|Running) · Pink noise$/);
  const cfg = await call<Config>(page, "siggenConfig");
  expect([cfg.signalType, cfg.channelMask, cfg.levelDb]).toEqual([3, 1, -30]);
  await gen.getByRole("button", { name: "Stop Now" }).click();
  await expect(gen.locator(".siggen-title")).toHaveText("Ready · Pink noise");
  expect((await call<Status>(page, "siggenStatus")).state).toBe(0);
  expect(errors).toEqual([]);
});

test("WB6J M46: Identify from an output's page and from the matrix plays channel-ID blips on that output alone", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT2" }).click();
  await session.getByRole("button", { name: "Identify" }).click();
  await expect.poll(async () => (await call<Config>(page, "siggenConfig")).channelMask).toBe(2);
  const cfg = await call<Config>(page, "siggenConfig");
  expect([cfg.signalType, cfg.levelDb, cfg.repeat]).toEqual([14, -12, 2]);
  await call(page, "siggenControl", { action: "stopNow" });
  // The matrix's column menu.
  await openPage(session, "Matrix");
  const matrix = session.getByRole("region", { name: "Matrix Mixer" });
  await matrix.getByRole("button", { name: "OUT1", exact: true }).click(); // -- WP MXW -- the column's OUTn pill
  await matrix.getByRole("menuitem", { name: "Identify" }).click();
  await expect.poll(async () => (await call<Config>(page, "siggenConfig")).channelMask).toBe(1);
  expect((await call<Status>(page, "siggenStatus")).signalType).toBe(14);
  // The generator's page shows what plays.
  await openPage(session, "Signal Generator");
  await expect(session.getByRole("region", { name: "Signal Generator" }).locator(".siggen-title")).toHaveText(/Channel ID|Ready/);
  expect(errors).toEqual([]);
});

test("WB6J M46: read only, the generator and Identify are off and nothing plays", async ({ page }) => {
  // Venue B is this console's view-only installation.
  const session = await connect(page, "Venue B");
  await openPage(session, "Signal Generator");
  const gen = session.getByRole("region", { name: "Signal Generator" });
  await expect(gen.getByRole("button", { name: "Start" })).toBeDisabled();
  await expect(gen.locator(".siggen-title")).toHaveText("You can view this DSPi but not change it");
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  await expect(session.getByRole("button", { name: "Identify" })).toBeDisabled();
  expect((await call<Status>(page, "siggenStatus")).state).toBe(0);
});

test("WB6J M53: Stats for nerbs shows the core's readings and resets the watermarks", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Stats for nerbs");
  const stats = session.getByRole("region", { name: "Stats for nerbs" });
  await expect(stats.getByRole("region", { name: "System Information" })).toBeVisible();
  await expect(stats.getByRole("region", { name: "Device Information" })).toContainText("Firmware");
  await expect(stats.getByRole("region", { name: "Buffer Fill Levels" })).toContainText("Audio Streaming");
  await stats.getByRole("button", { name: "Reset Watermarks" }).click();
  await expect(session.locator(".error")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WB6J M54: the Interrupt Monitor lists the device's notifications; Pause, Clear", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Interrupt Monitor");
  const mon = session.getByRole("region", { name: "Interrupt Monitor" });
  await expect(mon.getByRole("status")).toContainText("Listening");
  // A notification of the demo's: the generator starting and stopping.
  await call(page, "siggenSet", { config: { signalType: 0, channelMask: 1, invertMask: 0, flags: 0, levelDb: -40, durationMs: 0, repeat: 0, gapMs: 0, p: [1000, 0, 0, 0] } });
  await call(page, "siggenControl", { action: "start" });
  await call(page, "siggenControl", { action: "stopNow" });
  const lines = mon.getByRole("list", { name: "Notification lines" }).getByRole("listitem");
  await expect(lines.filter({ hasText: "SiggenState" }).first()).toBeVisible();
  await mon.getByRole("button", { name: "Pause" }).click();
  await expect(mon.getByRole("status")).toContainText("Paused");
  await mon.getByRole("button", { name: "Clear" }).click();
  await expect(mon.getByRole("status")).toContainText("0 events");
  await expect(lines).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("WB6J M22: the Analyser screen shows the strip with its chooser", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Analyser");
  const strip = session.getByRole("region", { name: "Spectrum" });
  // -- WP WEB2 -- (REVD W4) the bars at once: no Show Bars and no second heading on the Analyser page
  await expect(strip.getByRole("button", { name: "Show Bars" })).toHaveCount(0);
  await expect(strip.getByRole("heading", { name: "Spectrum Analyser" })).toHaveCount(0);
  await expect(strip.getByRole("group", { name: "Analyse", exact: true })).toBeVisible();
  await expect(strip.locator(".rta-cell")).toHaveCount(2);
  await expect(strip.locator(".rta-cell").first()).not.toHaveAttribute("data-bars", "0");
  expect(errors).toEqual([]);
});

// -- WP INTD -- M59's cards-per-row picker is gone with the Overview's old cards: the dashboard's
// channel cards wrap by width, as on Qt's Modern Overview.
test("WB6J M59 (INTD): the dashboard has no layout picker; its cards wrap by width", async ({ page }) => {
  // One console on the page (two side by side at 375 px are slivers)
  await page.goto("/test/embed.html?one");
  await page.locator("#one").getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = page.locator("#one").getByRole("article", { name: "DSPi" });
  await expect(session.locator(".main-screen")).toBeVisible();
  await expect(session.getByRole("combobox", { name: "Dashboard layout" })).toHaveCount(0);
  const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(wide).toBeLessThanOrEqual(0);
});

// -- WP WEB7 --
test("WEB7: the signal generator as Qt's: Sine has its Duration, a second click inverts an output (ø), a third leaves it out", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Signal Generator");
  const gen = session.getByRole("region", { name: "Signal Generator" });
  await expect(gen.getByRole("radio", { name: "Sine" })).toHaveAttribute("aria-checked", "true");
  // Five tiles to a row, as Qt's grid (at a desktop width; at 375 px they wrap by width).
  if (test.info().project.name !== "narrow") {
    const tiles = gen.getByRole("radiogroup", { name: "Signal" }).getByRole("radio");
    const tops = await tiles.evaluateAll((els) => els.slice(0, 6).map((e) => Math.round(e.getBoundingClientRect().top)));
    expect(new Set(tops.slice(0, 5)).size).toBe(1);
    expect(tops[5]).toBeGreaterThan(tops[0]!);
  }
  // Timing for Sine: Duration, 0 = play until stopped.
  const timing = gen.getByRole("region", { name: "Timing" });
  await expect(timing).toContainText("Duration");
  await expect(timing).toContainText("0 = play until stopped");
  await timing.getByRole("textbox", { name: "Duration in seconds" }).fill("2");
  await timing.getByRole("textbox", { name: "Duration in seconds" }).press("Enter");
  // Out 2: on, then inverted, then off; Out 1 inverted for the run.
  const outs = gen.getByRole("group", { name: "Outputs" });
  const out1 = outs.getByRole("button", { name: /^Out 1\b/ });
  const out2 = outs.getByRole("button", { name: /^Out 2\b/ });
  await expect(out1).toHaveAttribute("aria-pressed", "true");
  await out1.click();
  await expect(out1).toHaveAttribute("data-inverted", "true");
  await expect(out1).toContainText("ø");
  await out2.click();
  await expect(out2).toHaveAttribute("data-inverted", "true");
  await out2.click();
  await expect(out2).toHaveAttribute("aria-pressed", "false");
  await expect(gen.locator(".siggen-detail")).toHaveText("1 output (1 inverted) · peak -20.0 dBFS");
  await gen.getByRole("textbox", { name: "Level in dBFS" }).fill("-40");
  await gen.getByRole("textbox", { name: "Level in dBFS" }).press("Enter");
  await gen.getByRole("button", { name: "Start" }).click();
  await expect(gen.locator(".siggen-title")).toHaveText(/^(Fading in|Running) · Sine$/);
  const cfg = await call<Config & { invertMask: number; durationMs: number }>(page, "siggenConfig");
  expect([cfg.channelMask, cfg.invertMask, cfg.durationMs, cfg.levelDb]).toEqual([1, 1, 2000, -40]);
  await gen.getByRole("button", { name: "Stop Now" }).click();
  // None clears the inversions too.
  await gen.getByRole("region", { name: "Outputs" }).getByRole("button", { name: "None", exact: true }).click();
  await out1.click();
  await expect(out1).toHaveAttribute("data-inverted", "false");
  expect(errors).toEqual([]);
});

test("WEB7: Stats for nerbs as Qt's columns: the counters' sections and a fill bar per output pair", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Stats for nerbs");
  const stats = session.getByRole("region", { name: "Stats for nerbs" });
  for (const name of ["Device Information", "System Information", "Audio Output", "PDM (Subwoofer)", "SPDIF DMA Starvation", "Buffer Fill Levels"]) {
    await expect(stats.getByRole("region", { name, exact: true })).toBeVisible();
  }
  await expect(stats.getByRole("region", { name: "Audio Output" })).toContainText("ISR → Main Loop");
  const bars = stats.getByRole("region", { name: "Buffer Fill Levels" }).getByRole("img");
  await expect(bars.first()).toHaveAccessibleName(/^Out 1\/2( \([^)]+\))? \d+%, \d+–\d+% since the watermarks were reset$/);
  await expect(stats).toContainText("Updated every 2 seconds");
  expect(errors).toEqual([]);
});
