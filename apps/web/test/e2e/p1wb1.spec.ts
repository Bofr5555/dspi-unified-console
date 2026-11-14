// WP P1WB1 e2e: snapshots, A/B, the import preview, safe apply and offline files on the core's
// demo device (crates/dspi-wasm), in Chromium, Firefox and Chromium at 375 px (the `narrow`
// project); and a plain recall over DSPi Link guarded by default, on `uconsole --demo hub`
// (DSPI_TERM). Every action is the page's; its effect is read back from the core.

import { spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = { outputGainDb: number[]; masterVolumeDb: number; userMute: boolean; bypass: boolean; activePresetSlot: number };

const MAC_PATH = new URL("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset", import.meta.url);
const MAC = readFileSync(MAC_PATH, "utf8");

async function call<T>(page: Page, method: string, params: unknown = null, n = 0): Promise<T> {
  return (await page.evaluate(([n, m, p]) => (globalThis as unknown as Clients).dspiClients[n as number]!.call(m as string, p), [n, method, params] as const)) as T;
}
const coreState = async (page: Page, n = 0) => (await call<{ state: State }>(page, "state", null, n)).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

/** Console one alone, Venue A (control): the demo, session 0. */
async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("region", { name: "Presets" })).toBeVisible();
  return session;
}

const tab = (session: Locator, name: string) => session.getByRole("tab", { name, exact: true });

/** -- WP P1DCFIX -- (F6) the page's own unsaved indicator (Main's Presets card) shows; then back to `back`. */
async function unsavedShown(session: Locator, back = "Snapshots") {
  await tab(session, "Main").click();
  await expect(session.getByText("Unsaved changes", { exact: true })).toBeVisible();
  await tab(session, back).click();
}

async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

test("P1WB1 snapshots: capture downloads, recall puts every value back, Open Snapshot… brings one back", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await tab(session, "Snapshots").click();
  const screen = session.getByRole("region", { name: "Snapshots" });
  await screen.getByLabel("Name").fill("Kväll ✓");
  await screen.getByLabel("Note").fill("before the gig");
  const [dl] = await Promise.all([page.waitForEvent("download"), screen.getByRole("button", { name: "Capture" }).click()]);
  expect(dl.suggestedFilename()).toBe("Kväll ✓.dspipreset");
  const text = readFileSync(await dl.path(), "utf8");
  expect(JSON.parse(text).meta.kind).toBe("snapshot");
  const list = screen.getByRole("list", { name: "Held snapshots" });
  await expect(list.getByRole("listitem")).toHaveCount(1);
  await expect(list.getByRole("listitem").first()).toContainText("before the gig");

  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -7 } });
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-7);
  // -- WP P1DCFIX -- (F6) the page asks from what it knows: wait until it shows the change as unsaved
  await unsavedShown(session);
  await list.getByRole("listitem", { name: "Kväll ✓" }).getByRole("button", { name: "Recall", exact: true }).click();
  // The change is unsaved: asked first, as the scene bar asks.
  const asked = page.getByRole("dialog", { name: "Recall Snapshot" });
  await expect(asked).toContainText("Recalling “Kväll ✓” replaces them");
  await asked.getByRole("button", { name: "Recall" }).click();
  await expect(screen.getByRole("status", { name: "Result" })).toContainText("Recalled “Kväll ✓”");
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(0);

  // The downloaded file opened again, and a file that is not one refused in the core's words.
  await screen.getByLabel("Snapshot file").setInputFiles({ name: "again.dspipreset", mimeType: "application/json", buffer: Buffer.from(text) });
  await expect(list.getByRole("listitem")).toHaveCount(2);
  await screen.getByLabel("Snapshot file").setInputFiles({ name: "broken.dspipreset", mimeType: "application/json", buffer: Buffer.from("{") });
  await expect(screen.getByRole("alert", { name: "Result" })).toContainText("Open Snapshot: not done");
  await expect(list.getByRole("listitem")).toHaveCount(2);
  await list.getByRole("button", { name: "Remove Kväll ✓ from the list" }).first().click();
  await expect(list.getByRole("listitem")).toHaveCount(1);
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("P1WB1 A/B and safe apply: the toggle in the scene bar and its key, off during a countdown; Keep, Revert", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await tab(session, "Snapshots").click();
  const screen = session.getByRole("region", { name: "Snapshots" });
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -4 } });
  await screen.getByLabel("Name").fill("Bright");
  await Promise.all([page.waitForEvent("download"), screen.getByRole("button", { name: "Capture" }).click()]);
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: 0 } });
  const row = screen.getByRole("list", { name: "Held snapshots" }).getByRole("listitem", { name: "Bright" });

  // Compare: the device now is A, Bright is B; the bar shows above the screens.
  await row.getByRole("button", { name: "Compare" }).click();
  const bar = session.getByRole("group", { name: "A/B compare" });
  await expect(bar).toContainText("Now A");
  await bar.getByRole("button", { name: /Now A · switch/ }).click();
  await expect(bar).toContainText("Now B");
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-4);
  // The key (-- WP P1DCFIX -- a plain B), with focus on no text field.
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("b");
  await expect(bar).toContainText("Now A");
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(0);
  const compare = screen.getByRole("region", { name: "A/B compare" });
  await compare.getByRole("button", { name: "Show Differences" }).click();
  await expect(compare.getByRole("table", { name: "From A to B" })).toContainText("-4");

  // A recall with rollback: the countdown bar; the toggle is off while it waits; Revert puts back and ends the compare.
  await row.getByRole("button", { name: "Recall with Rollback" }).click();
  const guard = session.getByRole("alert", { name: "Safe apply" });
  await expect(guard).toContainText("Keep these settings? They go back in"); // -- WP P1DCFIX -- the core's words
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-4);
  await expect(bar.getByRole("button", { name: /switch/ })).toBeDisabled();
  await page.keyboard.press("b");
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  await guard.getByRole("button", { name: "Revert" }).click();
  await expect(guard).toHaveCount(0);
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(0);
  await expect(bar).toHaveCount(0);
  await expect(screen.getByRole("status", { name: "Result" })).toBeVisible();

  // Keep keeps.
  await row.getByRole("button", { name: "Recall with Rollback" }).click();
  await guard.getByRole("button", { name: "Keep" }).click();
  await expect(guard).toHaveCount(0);
  await expect.poll(async () => (await coreState(page)).outputGainDb[0]).toBe(-4);
  expect(errors).toEqual([]);
});

test("P1WB1 import: the preview's rows per block before anything is written; only the chosen block changes", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await tab(session, "Files").click();
  const files = session.getByRole("region", { name: "Files" });
  const before = await coreState(page);
  await files.getByLabel("Configuration file to import").setInputFiles(MAC_PATH.pathname);
  const preview = session.getByRole("region", { name: "Import preview" });
  await expect(preview).toContainText("Import “Mac reference”");
  // Nothing written by the preview.
  expect((await coreState(page)).masterVolumeDb).toBe(before.masterVolumeDb);
  const blocks = preview.getByRole("group", { name: "What to import" });
  for (const box of await blocks.getByRole("checkbox").all()) await box.uncheck();
  await expect(preview).toContainText("Nothing is chosen.");
  await expect(preview.getByRole("button", { name: "Import", exact: true })).toBeDisabled();
  await blocks.getByRole("checkbox", { name: /Volume levels/ }).check();
  const table = preview.getByRole("table", { name: "What the import changes" });
  await expect(table.getByRole("row")).not.toHaveCount(0);
  for (const r of await table.locator("tbody tr").allTextContents()) expect(r).toMatch(/^Volume levels/);
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  await preview.getByRole("button", { name: "Import", exact: true }).click();
  await expect(files.getByRole("status", { name: "Result" })).toContainText("Imported “Mac reference”");
  const after = await coreState(page);
  expect(after.masterVolumeDb).not.toBe(before.masterVolumeDb);
  expect(after.outputGainDb).toEqual(before.outputGainDb);
  // The same file again: the volumes are the file's now.
  expect((await call<{ count: number }>(page, "configPreview", { text: MAC, options: (1 << 16) | 1 })).count).toBe(0);

  // Export downloads the device as a .dspipreset.
  await files.getByLabel("Name", { exact: true }).fill("Hall");
  const [dl] = await Promise.all([page.waitForEvent("download"), files.getByRole("button", { name: "Export Configuration" }).click()]);
  expect(dl.suggestedFilename()).toBe("Hall.dspipreset");
  expect(JSON.parse(readFileSync(await dl.path(), "utf8")).meta.name).toBe("Hall");
  expect(errors).toEqual([]);
});

test("P1WB1 offline: Open File… and New from Template… open sessions with no device, File: in the strip", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  const filesSide = one.getByRole("region", { name: "Files" }).first();
  await filesSide.getByLabel("File to open").setInputFiles(MAC_PATH.pathname);
  const doc = one.getByRole("article").first();
  await expect(doc.locator(".device-meta")).toContainText("File: mac-swift-v32.dspipreset");
  await expect(doc.getByRole("region", { name: "Presets" })).toBeVisible();
  await doc.getByRole("complementary", { name: "Status" }).getByText("a file").waitFor();

  await filesSide.getByRole("button", { name: "New from Template…" }).click();
  await filesSide.getByRole("list", { name: "Templates" }).getByRole("button", { name: /Open template/ }).first().click();
  await expect(one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab")).toHaveCount(2);
  const tpl = one.getByRole("article").first();
  await expect(tpl.locator(".device-meta")).toContainText(/File: .*\.dspipreset/);
  // An edit, then Save File: the document as it stands.
  await tab(tpl, "Files").click();
  const [dl] = await Promise.all([page.waitForEvent("download"), tpl.getByRole("button", { name: "Save File" }).click()]);
  expect(JSON.parse(readFileSync(await dl.path(), "utf8")).meta.kind).toBe("template");
  if (test.info().project.name === "narrow") await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("P1WB1 keys: Qt's table, never while a text field has focus", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  const body = page.locator("body");
  await body.click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("Control+m");
  await expect.poll(async () => (await coreState(page)).userMute).toBe(true);
  await page.keyboard.press("Control+b");
  await expect.poll(async () => (await coreState(page)).bypass).toBe(true);
  // A text field keeps its keys.
  await tab(session, "Snapshots").click();
  await session.getByRole("region", { name: "Snapshots" }).getByLabel("Name").focus();
  await page.keyboard.press("Control+m");
  await page.waitForTimeout(300);
  expect((await coreState(page)).userMute).toBe(true);
  await body.click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("Control+m");
  await expect.poll(async () => (await coreState(page)).userMute).toBe(false);
  // Commit asks first.
  await page.keyboard.press("Control+s");
  const dialog = page.getByRole("dialog", { name: "Save Preset" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  // Preset 2 (stored first), with an unsaved change: asked, as the picker asks.
  await call(page, "presetCopy", { source: 0, dest: 1 });
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -3 } });
  // -- WP P1DCFIX -- (F6) once the page shows it as unsaved; the focus on Main's tab, in this console
  await unsavedShown(session, "Main");
  await page.keyboard.press("Control+2");
  const ask = page.getByRole("dialog", { name: "Unsaved Changes" });
  await expect(ask).toBeVisible();
  await ask.getByRole("button", { name: "Discard" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(1);
  expect(errors).toEqual([]);
});

// -- Over DSPi Link: a plain recall is guarded by default (OD-P1g, p1cfix) --

type OwnHub = { port: number; child: ChildProcess };

async function ownHub(): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/p1wb1-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", "--open", "--forget-clients", "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  const port = await new Promise<number>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number };
      if (e.event === "status" && e.port) ok(e.port);
    });
  });
  return { port, child };
}

test.describe("over DSPi Link", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (!hub) return;
    const exited = new Promise<void>((r) => hub!.child.once("exit", () => r()));
    hub.child.kill();
    await exited;
    hub = undefined;
  });

  test("P1WB1: a recall through a hub with the guard is a safe apply: countdown, Keep or Revert", async ({ page }) => {
    hub = await ownHub();
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);
    const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
    await row.getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").first().getByRole("button", { name: "Open" }).click();
    await expect(session.locator(".device-meta")).toContainText("DEMO");
    await tab(session, "Snapshots").click();
    const screen = session.getByRole("region", { name: "Snapshots" });
    await expect(screen).toContainText("Every recall here can be taken back");
    await screen.getByLabel("Name").fill("Site");
    await Promise.all([page.waitForEvent("download"), screen.getByRole("button", { name: "Capture" }).click()]);
    const item = screen.getByRole("list", { name: "Held snapshots" }).getByRole("listitem", { name: "Site" });
    await expect(item.getByRole("button", { name: "Recall with Rollback" })).toHaveCount(0);
    await item.getByRole("button", { name: "Recall", exact: true }).click();
    const guard = session.getByRole("alert", { name: "Safe apply" });
    await expect(guard).toContainText("The hub puts them back too if this page goes away");
    await guard.getByRole("button", { name: "Keep" }).click();
    await expect(guard).toHaveCount(0);
    await expect(screen.getByRole("status", { name: "Result" })).toBeVisible();
    // And an import there is guarded too: no plain Import button.
    await tab(session, "Files").click();
    await session.getByRole("region", { name: "Files" }).getByLabel("Configuration file to import").setInputFiles(MAC_PATH.pathname);
    const preview = session.getByRole("region", { name: "Import preview" });
    await expect(preview.getByRole("button", { name: "Import with Rollback" })).toBeVisible();
    await expect(preview.getByRole("button", { name: "Import", exact: true })).toHaveCount(0);
    await preview.getByRole("button", { name: "Import with Rollback" }).click();
    await expect(guard).toBeVisible();
    await guard.getByRole("button", { name: "Revert" }).click();
    await expect(guard).toHaveCount(0);
  });
});
