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
  // -- WP WEB6 -- Qt's RTA chip in the graph's corner (was the page tool "Spectrum on Graph")
  const rta = session.getByRole("group", { name: "Graph options" }).getByRole("button", { name: "RTA" });
  await rta.click();
  await expect(rta).toHaveAttribute("aria-pressed", "true");
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
  await rta.click();
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
  // -- WP WEB6 -- (REVE D2) the chip names the output once, by the core's label
  await expect(gen.getByRole("group", { name: "Outputs" }).getByRole("button", { name: /^OUT/ })).toHaveCount(0);
  await gen.getByRole("group", { name: "Outputs" }).getByRole("button", { name: /^Out 1\b/ }).click();
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
  // -- WP WEB6 -- the channel's menu (its right click), as Qt's: no Identify among the page tools
  await session.locator(".eq-pick", { hasText: "OUT2" }).click({ button: "right" });
  await page.getByRole("menu", { name: /^OUT2/ }).getByRole("menuitem", { name: "Identify" }).click();
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
  await session.locator(".eq-pick", { hasText: "OUT1" }).click({ button: "right" }); // -- WP WEB6 -- the channel's menu
  await expect(page.getByRole("menu", { name: /^OUT1/ }).getByRole("menuitem", { name: "Identify" })).toBeDisabled();
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
  // -- WP WEB6 -- Qt's Spectrum page: the chosen channels' curves over one plot at once, no Show Bars,
  // no second heading; the tap and the channels in the head row
  const strip = session.getByRole("region", { name: "Spectrum" });
  await expect(strip.getByRole("button", { name: "Show Bars" })).toHaveCount(0);
  await expect(strip.getByRole("heading", { name: "Spectrum Analyser" })).toHaveCount(0);
  await expect(strip.getByRole("group", { name: "Analyse", exact: true })).toBeVisible();
  await expect(strip.getByRole("group", { name: "Channels analysed" }).getByRole("button", { pressed: true })).toHaveCount(2);
  await expect(strip.locator(".sp-curves")).toHaveAttribute("data-curves", "2");
  expect(errors).toEqual([]);
});

// -- WP WEB6 -- the Spectrum page as Qt's: Curves / Bars / Both, the legend choosing the channels,
// the status line, "On the Equaliser's Graph"
test("WEB6: the Spectrum page draws curves, bars or both, says how the analyser runs, and puts the spectrum on the EQ graph", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await connect(page);
  await openPage(session, "Analyser");
  const sp = session.getByRole("region", { name: "Spectrum" });
  const view = sp.getByRole("group", { name: "View" });
  await expect(view.getByRole("button", { name: "Curves" })).toHaveAttribute("aria-pressed", "true");
  await expect(sp.locator(".sp-curves")).toHaveAttribute("data-curves", "2");
  await expect(sp.locator(".sp-cell")).toHaveCount(0);
  // Bars: one cell per channel, each with its bands; no curves
  await view.getByRole("button", { name: "Bars" }).click();
  await expect(sp.locator(".sp-cell")).toHaveCount(2);
  await expect(sp.locator(".sp-cell").first()).not.toHaveAttribute("data-bars", "0");
  await expect(sp.locator(".sp-curves")).toHaveCount(0);
  // Both: the curves and the bars
  await view.getByRole("button", { name: "Both" }).click();
  await expect(sp.locator(".sp-curves")).toHaveAttribute("data-curves", "2");
  await expect(sp.locator(".sp-cell")).toHaveCount(2);
  // The status line: running, the frame rate as the core reports it
  const status = sp.getByRole("status", { name: "Analyser status" });
  await expect(status).toContainText("Running");
  await expect(status).toContainText(/\d+ frames\/s/);
  // The legend: the second input off, the analyser runs on the first alone
  const legend = sp.getByRole("group", { name: "Channels analysed" });
  await legend.getByRole("button").nth(1).click();
  await expect(legend.getByRole("button").nth(1)).toHaveAttribute("aria-pressed", "false");
  await expect(sp.locator(".sp-curves")).toHaveAttribute("data-curves", "1");
  await expect.poll(async () => (await call<{ snapshot: { frames: { channel: number }[] } }>(page, "analyserView")).snapshot.frames.map((f) => f.channel)).toEqual([0]);
  // The outputs: the first output alone, named as the page names it
  await sp.getByRole("group", { name: "Analyse", exact: true }).getByRole("button", { name: "Outputs" }).click();
  await expect.poll(async () => (await call<{ snapshot: { tap: number } }>(page, "analyserView")).snapshot.tap).toBe(1);
  await expect(legend.getByRole("button", { pressed: true })).toHaveCount(1);
  await expect(legend.getByRole("button", { pressed: true })).toContainText("Out 1");
  // The choice is kept: the page comes back as it was left
  await page.reload();
  const again = await connect(page);
  await openPage(again, "Analyser");
  const sp2 = again.getByRole("region", { name: "Spectrum" });
  await expect(sp2.getByRole("group", { name: "View" }).getByRole("button", { name: "Both" })).toHaveAttribute("aria-pressed", "true");
  // On the Equaliser's Graph: the EQ graph shows the spectrum
  const tool = again.getByRole("toolbar", { name: "Spectrum tools" }).getByRole("checkbox", { name: "On the Equaliser's Graph" });
  await expect(tool).toHaveAttribute("aria-checked", "false");
  await tool.click();
  await expect(tool).toHaveAttribute("aria-checked", "true");
  await openPage(again, "EQ");
  await expect(again.locator("canvas.rta-overlay")).toHaveCount(1);
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
