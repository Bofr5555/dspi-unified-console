// WP WB8 e2e: undo and redo (buttons and keys, a drag as one step, a viewer's
// session without them, an undo another writer stepped in on), the event log
// (a clip, the hub's alarms, filter by unit, copy) and Save Diagnostics…, in
// Chromium, Firefox and Chromium at 375 px. On the core's demo device through
// the embedding page, and on `uconsole --demo hub --web --alink` for what
// needs a hub (two consoles on one DSPi, alarms).

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { hasPage, openPage, openOnHub } from "../nav"; // -- WP UB6 -- openOnHub
import { presetsButton } from "../presets"; // -- WP INTD --

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type Band = { filterType: number; freq: number; gain: number };
type State = { userVolumeDb: number; numInputChannels: number; filters: Band[][] };

function core(page: Page, method: string, params: unknown = null, n = 0) {
  return page.evaluate(
    ([n, method, params]) => (globalThis as unknown as Clients).dspiClients[n as number]!.call(method as string, params),
    [n, method, params] as const,
  );
}

async function coreState(page: Page): Promise<State> {
  return ((await core(page, "state")) as { state: State }).state;
}

/** Pictures for the report, dark and light, when WB8_SHOTS names a directory (Chromium and narrow). */
async function shot(page: Page, name: string, of?: Locator) {
  const dir = process.env.WB8_SHOTS;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    const path = `${dir}/wb8-${project}-${name}-${scheme}.png`;
    if (of) await of.screenshot({ path });
    else await page.screenshot({ path, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

/** Console one's Venue A (control) or Venue B (view) on the embedding page. */
async function openDemo(page: Page, venue = "Venue A", query = "") {
  await page.goto(`/test/embed.html${query}`);
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.locator(".volume-value")).toBeVisible();
  return session;
}

const undoButton = (s: Locator) => s.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" });
const redoButton = (s: Locator) => s.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Redo" });

async function setVolume(session: Locator, pos: string) {
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.fill(pos);
  await slider.dispatchEvent("change");
}

test("WB8 undo: the buttons and the keys step back and forward through the core's journal", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await expect(undoButton(session)).toBeDisabled();
  await expect(redoButton(session)).toBeDisabled();
  const start = (await coreState(page)).userVolumeDb;
  // -- WP WEBM1 -- Undo and Redo sit in the device's bar, in one line with the preset picker at every width
  const bar = (await session.getByRole("group", { name: "Device actions" }).boundingBox())!;
  const picker = (await presetsButton(session).boundingBox())!; // -- WP INTD -- the presets' popover button
  // (two consoles side by side at 375 px are slivers, where it wraps)
  if (test.info().project.name !== "narrow") expect(Math.abs(bar.height - picker.height)).toBeLessThan(0.5);

  await setVolume(session, "128");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).not.toBe(start);
  const set = (await coreState(page)).userVolumeDb;
  await expect(undoButton(session)).toBeEnabled();
  await undoButton(session).click();
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(start);
  await expect(session.locator(".volume-value")).toHaveText("0.0 dB");
  await expect(redoButton(session)).toBeEnabled();
  await redoButton(session).click();
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(set);

  // The keys, with the focus nowhere in particular (the page's body).
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(start);
  await page.keyboard.press("Control+Shift+z");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(set);
  await page.keyboard.press("Control+z");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(start);
  await page.keyboard.press("Control+y");
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(set);
  // Console two (another session on the same page) was not touched.
  expect(errors).toEqual([]);
});

test("WB8 undo: a slider drag is one step, however many writes it made", async ({ page }) => {
  const session = await openDemo(page);
  const start = (await coreState(page)).userVolumeDb;
  const slider = session.getByRole("slider", { name: "Volume" });
  await slider.scrollIntoViewIfNeeded();
  const box = (await slider.boundingBox())!;
  const y = box.y + box.height / 2;
  await page.mouse.move(box.x + box.width * 0.9, y);
  await page.mouse.down();
  for (let i = 1; i <= 10; i++) {
    await page.mouse.move(box.x + box.width * (0.9 - i * 0.05), y);
    await page.waitForTimeout(30);
  }
  await page.mouse.up();
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBeLessThan(-10);
  // Let the last live write go, then one undo takes the whole drag back.
  await expect.poll(async () => ((await core(page, "history", { n: 50 })) as unknown[]).length).toBeGreaterThan(0);
  await page.waitForTimeout(200);
  await undoButton(session).click();
  await expect.poll(async () => (await coreState(page)).userVolumeDb).toBe(start);
  await expect(undoButton(session)).toBeDisabled();
});

/** Where the EQ graph puts a frequency and a level (log 20 Hz..20 kHz across, +25..-25 dB down). */
async function at(graph: Locator, freq: number, db: number) {
  const b = (await graph.boundingBox())!;
  const fx = (Math.log10(freq) - Math.log10(20)) / (Math.log10(20000) - Math.log10(20));
  return { x: b.x + fx * b.width, y: b.y + ((25 - db) / 50) * b.height };
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

test("WB8 undo: a graph drag is one step; a text field keeps Ctrl+Z for itself", async ({ page }) => {
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  await graph.evaluate((e) => e.scrollIntoView({ block: "center" }));
  await drawn(graph);
  const out1 = async () => {
    const s = await coreState(page);
    return s.filters[s.numInputChannels]!;
  };
  const slot = (await out1()).findIndex((b) => b.filterType === 0);
  const p = await at(graph, 1000, 6);
  await page.mouse.dblclick(p.x, p.y);
  await expect.poll(async () => (await out1())[slot]!.filterType).toBe(1);
  const band = (await out1())[slot]!;

  const dot = await at(graph, band.freq, band.gain);
  const to = await at(graph, band.freq * 2, band.gain - 5);
  await page.mouse.move(dot.x, dot.y);
  await page.mouse.down();
  for (let i = 1; i <= 8; i++) await page.mouse.move(dot.x + ((to.x - dot.x) * i) / 8, dot.y + ((to.y - dot.y) * i) / 8);
  await page.mouse.up();
  await expect.poll(async () => (await out1())[slot]!.freq).toBeGreaterThan(band.freq * 1.8);
  await page.waitForTimeout(200);

  // Ctrl+Z in the band's Gain field is the field's own: the device keeps the drag.
  const gain = session.getByRole("group", { name: `Band ${slot + 1}` }).getByRole("textbox", { name: "Gain" });
  await gain.focus();
  await page.keyboard.press("Control+z");
  await page.waitForTimeout(300);
  expect((await out1())[slot]!.freq).toBeGreaterThan(band.freq * 1.8);
  // Out of the field (Escape: nothing typed is written), one undo takes the whole drag back.
  await page.keyboard.press("Escape");
  await graph.focus();
  await page.keyboard.press("Control+z");
  await expect.poll(async () => Math.round((await out1())[slot]!.freq)).toBe(Math.round(band.freq));
  expect((await out1())[slot]!.gain).toBeCloseTo(band.gain, 1);
});

test("WB8 undo: a viewer's session has Undo and Redo off, and the keys do nothing", async ({ page }) => {
  const session = await openDemo(page, "Venue B");
  await expect(session.locator(".read-only-reason")).toBeVisible(); // -- WP REVEFIX -- said in the strip (the badge went with the session bar)
  await expect(undoButton(session)).toBeDisabled();
  await expect(redoButton(session)).toBeDisabled();
  await page.keyboard.press("Control+z");
  await expect(session.locator(".step-notice")).toHaveCount(0);
});

// -- The event log and Save Diagnostics… on the demo --

test("WB8 events: a clip is listed with its time, unit and text, and copies as text", async ({ page, browserName, context }) => {
  const errors = errorsOf(page);
  if (browserName === "chromium") await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const session = await openDemo(page, "Venue A", "?one");
  await openPage(session, "Events");
  const log = session.getByRole("region", { name: "Event log" });
  await expect(log.getByRole("heading", { name: "Event log" })).toBeVisible();
  // The connection is the first record.
  await expect(log.getByRole("row").filter({ hasText: "Connected" })).toHaveCount(1);
  await core(page, "demo", { action: "clip", mask: 1 });
  const clip = log.getByRole("row").filter({ hasText: "Clip" }).first();
  await expect(clip).toBeVisible({ timeout: 10_000 });
  await expect(clip.locator(".event-time")).toHaveText(/^\d\d:\d\d:\d\d$/);
  // The unit as the System view names it.
  const unitName = (await clip.locator(".event-unit").textContent())!;
  expect(unitName).toMatch(/^DSPi/);
  // Newest first: the clip above the connection.
  await expect(log.locator("tbody tr").first()).toContainText("Clip");
  // Every unit, and the one: the demo is the one unit here.
  const unit = log.getByLabel("Unit");
  await expect(unit.locator("option")).toHaveText(["Every unit", unitName]);
  await unit.selectOption({ label: unitName });
  await expect(log.locator("tbody tr").first()).toContainText("Clip");

  await log.getByRole("button", { name: "Copy as Text" }).click();
  await expect(log.getByRole("status").filter({ hasText: /events? copied\./ })).toBeVisible();
  if (browserName === "chromium") {
    const text = await page.evaluate(() => navigator.clipboard.readText());
    const lines = text.split("\n");
    expect(lines[0]).toMatch(new RegExp(`^\\d{4}-\\d\\d-\\d\\d \\d\\d:\\d\\d:\\d\\d\\t${unitName}\\tClip\\t.+`));
    expect(lines.some((l) => /\tConnected\t/.test(l))).toBe(true);
  }
  await shot(page, "events-clip");
  // At 375 px the page never scrolls sideways.
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
  expect(errors).toEqual([]);
});

// -- fixup wb8 -- a hub's page over plain http has no Clipboard API
test("WB8 events: without the Clipboard API Copy as Text copies the older way, and failing that selects the text", async ({ page, browserName, context }) => {
  if (browserName === "chromium") await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.addInitScript(() => {
    Object.defineProperty(Navigator.prototype, "clipboard", { get: () => undefined, configurable: true });
  });
  const session = await openDemo(page, "Venue A", "?one");
  await openPage(session, "Events");
  const log = session.getByRole("region", { name: "Event log" });
  await expect(log.getByRole("row").filter({ hasText: "Connected" })).toHaveCount(1);
  await log.getByRole("button", { name: "Copy as Text" }).click();
  // execCommand("copy") from the click: copied, nothing shown to select
  await expect(log.getByRole("status").filter({ hasText: /events? copied\./ })).toBeVisible();
  await expect(log.getByRole("textbox", { name: "Events as text" })).toHaveCount(0);
  // And when the browser refuses that too: the text, selected, to copy by hand.
  await page.evaluate(() => {
    document.execCommand = () => false;
  });
  await log.getByRole("button", { name: "Copy as Text" }).click();
  const box = log.getByRole("textbox", { name: "Events as text" });
  await expect(box).toBeVisible();
  await expect(box).toBeFocused();
  await expect(box).toHaveValue(/\tConnected\t/);
  const selected = await box.evaluate((el) => {
    const t = el as HTMLTextAreaElement;
    return t.selectionEnd - t.selectionStart === t.value.length;
  });
  expect(selected).toBe(true);
  await expect(log.getByRole("status").filter({ hasText: "the text is selected below" })).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

test("WB8 diagnostics: Save Diagnostics… hands over the core's zip; Simple mode has no Events screen", async ({ page }) => {
  const session = await openDemo(page, "Venue A", "?one");
  await openPage(session, "Events");
  const log = session.getByRole("region", { name: "Event log" });
  const [download] = await Promise.all([page.waitForEvent("download"), log.getByRole("button", { name: "Save Diagnostics…" }).click()]);
  expect(download.suggestedFilename()).toMatch(/^dspi-diagnostics-\d{8}-\d{6}Z\.zip$/);
  const path = await download.path();
  const bytes = readFileSync(path!);
  expect(bytes.subarray(0, 4).toString("latin1")).toBe("PK\x03\x04");
  const names = bytes.toString("latin1");
  for (const f of ["README.txt", "versions.json", "device/dump.json", "events/", "client/browser.txt", "notes.txt"]) expect(names).toContain(f);
  await expect(log.getByRole("status").filter({ hasText: "Nothing was sent anywhere" })).toContainText(download.suggestedFilename());
  await shot(page, "diagnostics-saved", log);

  // Simple: today's screens only (UI-MODES), so neither the log nor the bundle.
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Simple" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  expect(await hasPage(session, "Events")).toBe(false);
  await expect(session.getByRole("button", { name: "Save Diagnostics…" })).toHaveCount(0);
  // Undo stays: it is not a mode's.
  await expect(undoButton(session)).toBeVisible();
});

// -- On a hub: two consoles on one DSPi, and the installation's alarms --

type OwnHub = { port: number; child: ChildProcess };

async function linkHub(): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/wb8-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--alink", "--console", "dist", "--open", "--forget-clients", "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  const port = await new Promise<number>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number };
      if (e.event === "status") ok(e.port ?? 0);
    });
  });
  return { port, child };
}

async function openUnit(page: Page, hub: OwnHub, serial: string): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  return openOnHub(page, { host: `127.0.0.1:${hub.port}`, serial }); // -- WP UB6 -- the Fleet's Open
}

test.describe("WB8 on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(() => {
    hub?.child.kill();
    hub = undefined;
  });

  test("WB8 undo: a value another console changed since is left as they set it, and they are named", async ({ page, browser }) => {
    test.setTimeout(60_000);
    hub = await linkHub();
    const mine = await openUnit(page, hub, "DEMO-ALINK-A");
    await openPage(mine, "Main");
    const before = await mine.locator(".volume-value").textContent();
    await setVolume(mine, "128");
    await expect(mine.locator(".volume-value")).not.toHaveText(before!);
    const mineShown = (await mine.locator(".volume-value").textContent())!;
    await expect(undoButton(mine)).toBeEnabled();
    const other = await browser.newPage();
    try {
      const theirs = await openUnit(other, hub, "DEMO-ALINK-A");
      await openPage(theirs, "Main");
      await setVolume(theirs, "100");
      await expect(theirs.locator(".volume-value")).not.toHaveText(mineShown);
      // Their change reaches this console as a notification.
      await expect(mine.locator(".volume-value")).not.toHaveText(mineShown, { timeout: 10_000 });
      const theirsShown = await mine.locator(".volume-value").textContent();
      await undoButton(mine).click();
      const notice = mine.locator(".step-notice");
      await expect(notice).toHaveText(/^Volume was changed by .+ since; it was left as they set it/);
      await expect(notice).toHaveClass(/warn/);
      await expect(mine.locator(".volume-value")).toHaveText(theirsShown!);
      await shot(page, "undo-other-writer");
      await notice.getByRole("button", { name: "Dismiss" }).click();
      await expect(notice).toHaveCount(0);
    } finally {
      await other.close();
    }
  });

  test("WB8 events: the installation's alarms by unit, beside the device's own records", async ({ page }) => {
    hub = await linkHub();
    const session = await openUnit(page, hub, "DEMO-ALINK-B");
    const rail = session.getByRole("complementary", { name: "Status" });
    const mute = rail.getByRole("button", { name: "System mute" });
    await mute.click();
    await expect(mute).toContainText(": on", { timeout: 10_000 });
    await expect(rail.getByLabel("Alarms")).toHaveText("1 alarm", { timeout: 10_000 });
    await openPage(session, "Events");
    const log = session.getByRole("region", { name: "Event log" });
    const alarm = log.getByRole("row").filter({ hasText: "Alarm" });
    await expect(alarm.first()).toBeVisible();
    await expect(alarm.first().locator(".event-unit")).toHaveText("Sender");
    await expect(log.getByRole("row").filter({ hasText: "Connected" })).toHaveCount(1);
    const unit = log.getByLabel("Unit");
    await unit.selectOption({ label: "Sender" });
    await expect(log.locator("tbody tr")).toHaveCount(await alarm.count());
    await shot(page, "events-hub-sender", log);
    await expect(log.getByRole("row").filter({ hasText: "Connected" })).toHaveCount(0);
    await unit.selectOption({ label: "Every unit" });
    await expect(log.getByRole("row").filter({ hasText: "Connected" })).toHaveCount(1);
    await mute.click();
    await expect(mute).toContainText(": off", { timeout: 10_000 });
  });
});
