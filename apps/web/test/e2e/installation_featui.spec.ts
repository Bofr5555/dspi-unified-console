// WP FEATUI e2e: FEAT1 and FEAT2 on the web, as Qt's (featui-installation smoke). Against the real
// hub (`uconsole --demo hub --web --alink`, SW3's installation: Inside playing Music, units with
// their own slots): a source on the sender's channels 8-9; a unit's own slots; Inside switched to
// it, the units it routes named and the notice that their hand-made slots are replaced; the
// terrace moved into the zone; a programme source; a scene that switches Inside back to Music.
// Saved (read back from a fresh page), "N units play the new channels after Align", Align there;
// the scene recalled says the zone's source; a zone's volume planned and set. 1280 and 375 px
// without sideways scroll; screenshots in both themes beside the Playwright output in featui-shots/ (chromium).
//
// The hub helpers are WEBM3's (installation_webm3.spec.ts), copied: that spec exports none.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type OwnHub = { port: number; pin: string; child: ChildProcess };

async function startHub(extra: string[], pairing = false): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/instw-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
      if (e.event === "status") {
        port = e.port ?? 0;
        if (!pairing) ok("");
      }
      if (e.event === "pairing" && e.pin) ok(e.pin);
    });
  });
  return { port, pin, child };
}

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** Connect the serving hub (pairing with its PIN when it has one) and open unit `serial`, or the hub's one device. */
async function openOn(page: Page, hub: OwnHub, serial: string): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (hub.pin) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  const meta = session.locator(".device-meta");
  // A hub with one DSPi opens it at once; one with several lists them.
  await expect(entry.or(meta.filter({ hasText: serial }))).toBeVisible();
  if (await entry.isVisible()) await entry.getByRole("button", { name: "Open" }).click();
  await expect(meta).toContainText(serial);
  const rail = session.getByRole("complementary", { name: "Status" });
  await expect(rail.getByLabel("Connection")).toHaveText(/^Connected/);
  await expect(session.locator(".banner")).toHaveCount(0);
  return session;
}

/** Save, confirming in the dialog that lists the changes; `said` are sentences it must list. */
async function save(page: Page, inst: Locator, said: RegExp[] = []): Promise<void> {
  const unsaved = inst.getByRole("group", { name: "Unsaved installation" });
  await unsaved.getByRole("button", { name: "Save" }).click();
  const dialog = page.getByRole("dialog", { name: "Save the installation?" });
  for (const s of said) await expect(dialog).toContainText(s);
  await dialog.getByRole("button", { name: "Save" }).click();
}

/** The page is no wider than its viewport (no sideways scrolling). */
async function fitsWidth(page: Page) {
  const { scroll, view } = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, view: window.innerWidth }));
  expect(scroll).toBeLessThanOrEqual(view);
}

async function shots(page: Page, name: string) {
  const out = process.env.SHOTS_DIR || (process.env.PW_OUTPUT_DIR ? `${process.env.PW_OUTPUT_DIR}/../featui-shots` : "");
  if (!out || test.info().project.name !== "chromium") return;
  mkdirSync(out, { recursive: true });
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${out}/featui-${name}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** The Installation tab in the Advanced mode. */
async function advancedTab(session: Locator, page: Page): Promise<Locator> {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Advanced" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: "Mode: Advanced" })).toBeVisible();
  await openPage(session, "System");
  await session.getByRole("tablist", { name: "System" }).getByRole("tab", { name: "Installation" }).click();
  const inst = session.getByRole("region", { name: "Installation", exact: true });
  await expect(inst.getByRole("heading", { name: "Units" })).toBeVisible();
  return inst;
}

if (!process.env.DSPI_TERM) {
  test("FEATUI: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("FEATUI: sources, slots, programme sources and scenes; a zone's volume", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("FEATUI: a zone's source replaces hand-made slots with a notice, saved, aligned; a scene switches it back; a zone's volume", async ({ page }) => {
    test.setTimeout(150_000);
    await page.setViewportSize({ width: 1280, height: 900 });
    hub = await startHub(["--alink", "--open", "--forget-clients"]);
    const session = await openOn(page, hub, "DEMO-ALINK-A");
    const inst = await advancedTab(session, page);

    // What the demo has: Bar's slots belong to Inside (Music); Terrace's are its own.
    const bar = inst.getByLabel("Unit Bar");
    const terrace = inst.getByLabel("Unit Terrace");
    await expect(bar.getByRole("group", { name: "Audio link slots of Bar" })).toContainText("Set by Inside, which plays Music");
    await expect(bar.getByLabel("Channels on the inputs of Bar")).toBeDisabled();
    const terraceSlots = terrace.getByLabel("Channels on the inputs of Terrace");
    await expect(terraceSlots).toHaveValue("2 3 - - - - - -");
    await expect(inst.getByLabel("Zone Inside")).toContainText("Routes Main left, Main right, Bar to its channels.");

    // A source on channels Music has is refused; on 8-9 it is made.
    const newSource = inst.getByRole("form", { name: "New source" });
    await newSource.getByRole("textbox", { name: "New source's name" }).fill("Booth");
    await newSource.getByRole("combobox", { name: "New source's sender" }).selectOption({ label: "Sender" });
    await newSource.getByRole("textbox", { name: "New source's channels" }).fill("0");
    await newSource.getByRole("button", { name: "Add source" }).click();
    await expect(inst.getByRole("alert").first()).toContainText(/clash/i);
    await newSource.getByRole("textbox", { name: "New source's channels" }).fill("8-9");
    await newSource.getByRole("button", { name: "Add source" }).click();
    await expect(inst.getByLabel("Source Booth").getByLabel("Channels of Booth")).toHaveValue("8 9");

    // Terrace's own slots by hand; text that is no slots is put back.
    await terraceSlots.fill("4 x");
    await terraceSlots.press("Enter");
    await terraceSlots.blur();
    await expect(terraceSlots).toHaveValue("2 3 - - - - - -");
    await terraceSlots.fill("4 5");
    await terraceSlots.blur();
    await expect(terraceSlots).toHaveValue("4 5 - - - - - -");
    await expect(inst.getByRole("alert", { name: "Slots replaced" })).toHaveCount(0);

    // Inside plays Booth: the notice names the units whose own slots are replaced.
    await inst.getByLabel("Zone Inside").getByRole("combobox", { name: "Source of Inside" }).selectOption({ label: "Booth" });
    const notice = inst.getByRole("alert", { name: "Slots replaced" });
    await expect(notice).toContainText("Main left's own slots will be replaced: Inside plays Booth.");
    await expect(notice).toContainText("Bar's own slots will be replaced");
    await expect(bar.getByLabel("Channels on the inputs of Bar")).toHaveValue("8 9 - - - - - -");
    // The terrace joins Inside: Terrace is routed too, and its slots are no longer its own.
    await inst.getByLabel("Zone Inside").getByRole("checkbox", { name: "Terrace" }).check();
    await expect(notice).toContainText("Terrace's own slots will be replaced");
    await expect(terraceSlots).toHaveValue("8 9 - - - - - -");
    await expect(terraceSlots).toBeDisabled();

    // A programme source, and a scene that switches Inside back to Music.
    const newProg = inst.getByRole("form", { name: "New programme source" });
    await newProg.getByRole("textbox", { name: "New programme source's name" }).fill("Desk computer");
    await newProg.getByRole("combobox", { name: "What it feeds" }).selectOption({ label: "Main left" });
    await newProg.getByRole("textbox", { name: "New programme source's inputs" }).fill("0 1");
    await newProg.getByRole("button", { name: "Add programme source" }).click();
    await expect(inst.getByLabel("Programme source Desk computer")).toContainText("Computer over USB into Main left");
    const newScene = inst.getByRole("form", { name: "New scene" });
    await newScene.getByRole("textbox", { name: "New scene's name" }).fill("DJ");
    await newScene.getByRole("button", { name: "Add scene" }).click();
    const dj = inst.getByLabel("Scene DJ");
    const action = dj.getByRole("group", { name: "New action of DJ" });
    await action.getByRole("combobox", { name: "Where" }).selectOption({ label: "Inside" });
    await action.getByRole("combobox", { name: "Switch to source" }).selectOption({ label: "Music" });
    await action.getByRole("button", { name: "Add action" }).click();
    await expect(dj.getByRole("list", { name: "Actions of DJ" })).toContainText("Switch Inside to source Music");

    await fitsWidth(page);
    await shots(page, "installation-1280");
    await page.setViewportSize({ width: 375, height: 900 });
    await fitsWidth(page);
    await shots(page, "installation-375");
    await page.setViewportSize({ width: 1280, height: 900 });
    await save(page, inst, [/Booth/, /Desk computer/, /DJ/]);
    await expect(inst.getByRole("status", { name: "Installation saved" })).toBeVisible();
    const due = inst.getByRole("status", { name: "Alignment needed" });
    await expect(due).toHaveText(/4 units play the new channels after Align\./);
    await due.getByRole("button", { name: "Align" }).click();
    const ask = page.getByRole("dialog", { name: "Align audio link delays?" });
    await ask.getByRole("button", { name: "Align" }).click();
    await expect(inst.getByRole("status", { name: "Alignment" })).toContainText("written");
    await expect(due).toHaveCount(0);

    // Read back on a fresh page.
    const again = await advancedTab(await openOn(page, hub, "DEMO-ALINK-A"), page);
    await expect(again.getByLabel("Zone Inside").getByRole("combobox", { name: "Source of Inside" })).toHaveValue("Booth");
    await expect(again.getByLabel("Unit Terrace").getByLabel("Channels on the inputs of Terrace")).toHaveValue("8 9 - - - - - -");
    await expect(again.getByLabel("Programme source Desk computer").getByLabel("Inputs of Desk computer")).toHaveValue("0 1");
    await expect(again.getByLabel("Scene DJ")).toContainText("Switch Inside to source Music");

    // The scene recalled says the zone's source; a zone's volume.
    const session2 = page.getByRole("article").last();
    await session2.getByRole("tablist", { name: "System" }).getByRole("tab", { name: "Status" }).click();
    const scenes = session2.getByLabel("Scenes", { exact: true });
    await scenes.getByRole("button", { name: "Recall DJ" }).click();
    await expect(session2.getByLabel("Scene result")).toContainText("Zone Inside plays Music");
    const vol = session2.getByRole("region", { name: "Room and zone volume" });
    await vol.getByRole("combobox", { name: "Room or zone" }).selectOption({ label: "Zone Inside" });
    await vol.getByRole("spinbutton", { name: "Volume in dB" }).fill("-30");
    await vol.getByRole("button", { name: "Show changes" }).click();
    const rows = vol.getByRole("list", { name: "Volume per unit" });
    await expect(rows).toContainText("Main left:");
    await expect(rows).toContainText("→ -30.0 dB");
    await shots(page, "zone-volume-1280");
    await vol.getByRole("button", { name: "Set" }).click();
    const louder = page.getByRole("dialog", { name: "Make it louder?" });
    if (await louder.isVisible()) await louder.getByRole("button", { name: "Set" }).click();
    await expect(vol.getByRole("status")).toContainText(/Inside: \d+ set, 0 refused\./);
    await expect(rows).toContainText("(set)");
    await page.setViewportSize({ width: 375, height: 900 });
    await fitsWidth(page);
    await shots(page, "zone-volume-375");
  });
});
