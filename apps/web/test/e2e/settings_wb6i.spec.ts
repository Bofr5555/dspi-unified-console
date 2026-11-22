// WP WB6i e2e: the Settings screen on the core's demo device, in every project (Chromium,
// Firefox, Chromium at 375 px without sideways scroll): the device name and the output latency
// through the core; Reset Channel Names as one undo step; Show Debug Information, kept; the help
// links; the pins (Advanced, not Simple): a move sent through the core, a held pin refused and a free one moved; Graphing
// applied to the EQ graph at once (its cursor readout, a redraw); and Core Voltage only in the
// Installer mode, driving the core's flow on the demo device: the word the core asks for, a
// trial started and aborted, a manual trial found and ended. No voltage write goes anywhere but
// the demo device.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = { channelNames: string[]; numInputChannels: number; outputPins: number[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}
const coreState = async (page: Page) => (await call<{ state: State; device: { name: string } }>(page, "state")).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  return session;
}

const nav = (session: Locator) => session.getByRole("navigation", { name: "Settings pages" });

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced" | "Installer") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

async function noSidewaysScroll(page: Page) {
  if (test.info().project.name !== "narrow") return;
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

test("WB6I Settings: device name and latency through the core; Reset Channel Names is one undo step; debug information kept; help links", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Settings");
  await expect(nav(session).getByRole("button", { name: "Global" })).toHaveAttribute("aria-current", "page");

  // Device name: written by the core, read back in the snapshot.
  const name = session.getByRole("region", { name: "Device Name" }).getByLabel("Device Name");
  await name.fill("Stage left");
  await session.getByRole("region", { name: "Device Name" }).getByRole("button", { name: "Rename" }).click();
  await expect.poll(async () => (await call<{ device: { name: string } }>(page, "state")).device.name).toBe("Stage left");
  await expect(name).toHaveValue("Stage left");

  // Latency: the device's options; a choice is the core's write.
  const latency = session.getByRole("region", { name: "Latency" });
  const target = latency.getByLabel("Latency target");
  await expect(target.locator("option", { hasText: "(default)" })).toHaveCount(1);
  const before = await call<{ requestedId: number; options: { id: number }[] }>(page, "outputLatency");
  const other = before.options.find((o) => o.id !== before.requestedId)!.id;
  await target.selectOption(String(other));
  await expect.poll(async () => (await call<{ requestedId: number }>(page, "outputLatency")).requestedId).toBe(other);
  await expect(latency).toContainText("no effect upon USB Audio");
  await noSidewaysScroll(page);

  // Reset Channel Names: asked first, every name back, one Undo restores both.
  await call(page, "write", { path: "ch.name", indices: [0], value: { text: "Vocal" } });
  const inputs = (await coreState(page)).numInputChannels;
  await call(page, "write", { path: "ch.name", indices: [inputs], value: { text: "Sub" } });
  await nav(session).getByRole("button", { name: "Advanced" }).click();
  await session.getByRole("region", { name: "Channel Names" }).getByRole("button", { name: "Reset" }).click();
  const dialog = page.getByRole("dialog", { name: "Reset Channel Names" });
  await dialog.getByRole("button", { name: "Reset" }).click();
  await expect.poll(async () => (await coreState(page)).channelNames[0]).toBe("USB L");
  expect((await coreState(page)).channelNames[inputs]).not.toBe("Sub");
  await session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: /Undo/ }).click();
  await expect.poll(async () => (await coreState(page)).channelNames.slice(0, 1).concat((await coreState(page)).channelNames[inputs]!)).toEqual(["Vocal", "Sub"]);

  // Show Debug Information: a section of its own, kept for the next visit.
  await expect(session.getByRole("region", { name: "Debug Information" })).toHaveCount(0);
  await session.getByRole("region", { name: "Diagnostics" }).getByLabel(/Show Debug Information/).check();
  await expect(session.getByRole("region", { name: "Debug Information" })).toContainText("demo");
  // Core Voltage is not in the Advanced mode.
  await expect(nav(session).getByRole("button", { name: "Core Voltage" })).toHaveCount(0);
  await expect(session.getByText("Core Voltage is in the Installer mode.")).toBeVisible();
  await noSidewaysScroll(page);

  // Help: the two GitHub links, opened elsewhere.
  await nav(session).getByRole("button", { name: "Help" }).click();
  const help = session.getByRole("region", { name: "Help" });
  await expect(help.getByRole("link", { name: "DSPi Firmware on GitHub" })).toHaveAttribute("href", "https://github.com/WeebLabs/DSPi");
  await expect(help.getByRole("link", { name: "DSPi Unified Console on GitHub" })).toHaveAttribute("target", "_blank");

  await page.reload();
  const again = await openDemo(page);
  await openPage(again, "Settings");
  await nav(again).getByRole("button", { name: "Advanced" }).click();
  await expect(again.getByRole("region", { name: "Diagnostics" }).getByLabel(/Show Debug Information/)).toBeChecked();
  await again.getByRole("region", { name: "Diagnostics" }).getByLabel(/Show Debug Information/).uncheck();
  expect(errors).toEqual([]);
});

test("WB6I Pins: every GPIO the device reports; a move goes through the core and a refusal is said; not in Simple", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "Settings");
  await nav(session).getByRole("button", { name: "Pins" }).click();
  const pins = session.getByRole("region", { name: "Pins" });
  const st = await coreState(page);
  const out1 = pins.getByRole("row", { name: /^Output 1/ });
  await expect(out1).toContainText(`GPIO ${st.outputPins[0]}`);
  // The range is the core's registry's (0..29).
  await expect(out1.getByRole("spinbutton")).toHaveAttribute("max", "29");
  // A pin out of the core's range: refused by the core before anything is sent, in its words.
  const alert = session.getByRole("alert").first();
  await out1.getByRole("spinbutton").fill("40");
  await out1.getByRole("button", { name: "Move" }).click();
  await expect(alert).toContainText("Output GPIO");
  expect((await coreState(page)).outputPins[0]).toBe(st.outputPins[0]);
  // -- WP CG-SWS-b -- the demo answers the pin requests now. A pin another output holds:
  // the core's pin map refuses it before anything is sent (PIN_IN_USE, status 2), nothing moved.
  await out1.getByRole("spinbutton").fill(String(st.outputPins[1]));
  await out1.getByRole("button", { name: "Move" }).click();
  await expect(alert).toContainText("status 2");
  expect((await coreState(page)).outputPins[0]).toBe(st.outputPins[0]);
  await expect(out1).toContainText(`GPIO ${st.outputPins[0]}`);
  // A free pin goes to the device through the core's setter (REQ_SET_OUTPUT_PIN) and moves.
  const free = [26, 27, 28].find((p) => !st.outputPins.includes(p))!;
  await out1.getByRole("spinbutton").fill(String(free));
  await out1.getByRole("button", { name: "Move" }).click();
  await expect.poll(async () => (await coreState(page)).outputPins[0]).toBe(free);
  await expect(out1).toContainText(`GPIO ${free}`);
  await noSidewaysScroll(page);

  await setMode(session, page, "Simple");
  await openPage(session, "Settings");
  await expect(nav(session).getByRole("button", { name: "Pins" })).toHaveCount(0);
  await expect(nav(session).getByRole("button", { name: "Core Voltage" })).toHaveCount(0);
  await setMode(session, page, "Advanced");
  expect(errors).toEqual([]);
});

test("WB6I Graphing: applied to the EQ graph at once (a redraw, the cursor readout); Restore Defaults", async ({ page }) => {
  test.skip(test.info().project.name === "narrow", "the cursor readout is the mouse's");
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  await expect(graph).toBeVisible();
  await graph.evaluate((e) => e.scrollIntoView({ block: "center" }));
  const box = (await graph.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height * 0.5);
  const readout = graph.locator(".eq-readout");
  await expect(readout).toContainText(/Hz/);
  await expect(readout).toContainText(/dB/);

  await openPage(session, "Settings");
  await nav(session).getByRole("button", { name: "Graphing" }).click();
  await session.getByRole("region", { name: "Scale & Grid" }).getByLabel("Show Gain Readout").uncheck();
  await session.getByRole("region", { name: "Scale & Grid" }).getByLabel("Min Frequency").selectOption("100");
  await openPage(session, "EQ");
  const graph2 = session.getByRole("application", { name: /^EQ graph/ });
  await graph2.evaluate((e) => e.scrollIntoView({ block: "center" }));
  const b2 = (await graph2.boundingBox())!;
  // The left edge is 100 Hz now; no gain in the readout.
  await page.mouse.move(b2.x + 2, b2.y + b2.height * 0.5);
  await expect(graph2.locator(".eq-readout")).toHaveText(/^10\d Hz$/);
  const drawsBefore = await graph2.locator("canvas").evaluate((c) => (c as unknown as { dspiDraws?: number }).dspiDraws ?? 0);

  await openPage(session, "Settings");
  await nav(session).getByRole("button", { name: "Graphing" }).click();
  await session.getByRole("region", { name: "Graph Appearance" }).getByLabel(/Graph Line Glow/).check();
  await session.getByRole("button", { name: "Restore Defaults" }).click();
  await expect(session.getByRole("region", { name: "Graph Appearance" }).getByLabel(/Graph Line Glow/)).not.toBeChecked();
  await expect(session.getByRole("region", { name: "Scale & Grid" }).getByLabel("Min Frequency")).toHaveValue("20");
  await openPage(session, "EQ");
  await expect.poll(() => session.getByRole("application", { name: /^EQ graph/ }).locator("canvas").evaluate((c) => (c as unknown as { dspiDraws?: number }).dspiDraws ?? 0)).toBeGreaterThan(0);
  expect(drawsBefore).toBeGreaterThan(0);
  expect(errors).toEqual([]);
});

test("WB6I Core Voltage: the Installer mode's; the core's word asked; a trial started and aborted; a manual trial found and ended (demo device)", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  const demo = await call<{ device: { demo: boolean } }>(page, "state");
  expect(demo.device.demo).toBe(true);
  await setMode(session, page, "Installer");
  await openPage(session, "Settings");
  await nav(session).getByRole("button", { name: "Advanced" }).click();
  await expect(session.getByRole("region", { name: "Core Voltage" })).toContainText("Running at 1.20 V");
  await session.getByRole("region", { name: "Core Voltage" }).getByRole("button", { name: "Open…" }).click();
  const vreg = session.getByRole("region", { name: "Core Voltage" });
  const device = vreg.getByRole("group", { name: "On the Device" });
  await expect(device).toContainText("Running at");
  await expect(device).toContainText("1.20 V");
  await noSidewaysScroll(page);

  // Calibrate: the core's warning; 1.25 V asks for the word (the core's rule, CG14).
  const cal = vreg.getByRole("group", { name: "Calibrate" });
  await cal.getByRole("button", { name: "Calibrate…" }).click();
  await expect(cal.locator(".vreg-warning")).toBeVisible();
  await cal.getByLabel("Trial voltage").selectOption({ label: "1.25 V" });
  await cal.getByRole("button", { name: "Start…" }).click();
  let dialog = page.getByRole("dialog", { name: "Start Trial" });
  await expect(dialog).toContainText("type OVERVOLT, exactly");
  await dialog.getByLabel("Confirmation word").fill("overvolt");
  await dialog.getByRole("button", { name: "Start" }).click();
  await expect(vreg.getByRole("alert")).toBeVisible();
  expect((await call<{ phase: string }>(page, "vregView")).phase).toBe("review");

  // The lower one needs no word: started, soaking, aborted.
  await cal.getByLabel("Trial voltage").selectOption({ label: "1.20 V" });
  await cal.getByRole("button", { name: "Start…" }).click();
  dialog = page.getByRole("dialog", { name: "Start Trial" });
  await expect(dialog.getByLabel("Confirmation word")).toHaveCount(0);
  await dialog.getByRole("button", { name: "Start" }).click();
  await expect(cal.getByRole("button", { name: "Abort" })).toBeVisible();
  await expect.poll(async () => (await call<{ phase: string }>(page, "vregView")).phase, { timeout: 10_000 }).toMatch(/soaking|starting_trial|clearing/);
  await expect(vreg.getByRole("group", { name: "Manual" })).toContainText("Not available while a calibration runs.");
  await cal.getByRole("button", { name: "Abort" }).click();
  await expect.poll(async () => (await call<{ phase: string }>(page, "vregView")).phase, { timeout: 10_000 }).toMatch(/ended|ending/);

  // Manual trial: found on the next poll, then ended.
  const manual = vreg.getByRole("group", { name: "Manual" });
  await manual.getByLabel("Voltage").selectOption({ label: "1.20 V" });
  await manual.getByRole("button", { name: "Trial…" }).click();
  await page.getByRole("dialog", { name: "Trial Voltage" }).getByRole("button", { name: "Try" }).click();
  await expect(cal.getByRole("button", { name: "End Trial" })).toBeVisible({ timeout: 10_000 });
  await expect(device).toContainText("(trial)");
  await cal.getByRole("button", { name: "End Trial" }).click();
  await expect(device).not.toContainText("(trial)", { timeout: 10_000 });
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});
