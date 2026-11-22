// WP WEBM3 e2e: the web Installation tab's edits that Qt's INSTQ panel has (review RB C-2): an
// S/PDIF source's role and its time-base slots, groups of units, and the source a new zone plays;
// the tab in the Advanced mode too, as Qt's. Against the real hub (`uconsole --demo hub --web
// --alink`, SW3's installation: a zone playing Music, the group Front of house), saved and read
// back from a fresh page. 375 px without sideways scroll; screenshots in both themes when
// SHOTS_DIR names a directory (chromium).
//
// The hub helpers are INSTW's (installation_instw.spec.ts), copied: that spec exports none.

import { spawn, type ChildProcess } from "node:child_process";
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
  const dir = process.env.SHOTS_DIR;
  if (!dir || test.info().project.name !== "chromium") return;
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/webm3-${name}-${scheme}.png`, fullPage: true });
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
  test("WEBM3: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("WEBM3: the Installation tab's S/PDIF source, groups and zone source", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("WEBM3: an S/PDIF source with its slots, a group's units, a new group and a zone playing a source, saved and read back", async ({ page }) => {
    test.setTimeout(120_000);
    hub = await startHub(["--alink", "--open", "--forget-clients"]);
    const inst = await advancedTab(await openOn(page, hub, "DEMO-ALINK-A"), page);

    // Terrace becomes an S/PDIF source: fed by nothing, its time base on outputs 7-8 until chosen.
    const terrace = inst.getByLabel("Unit Terrace");
    await terrace.getByRole("combobox", { name: "Role" }).selectOption("spdif_source");
    await expect(terrace.getByRole("combobox", { name: "Role" })).toHaveValue("spdif_source");
    await expect(terrace.getByRole("combobox", { name: "Fed by" })).toHaveCount(0);
    const slots = terrace.getByRole("group", { name: "Time base on outputs of Terrace" });
    const slot = (o: string) => slots.getByRole("checkbox", { name: `Terrace sends the time base on outputs ${o}` });
    await expect(slot("7-8")).toBeChecked();
    await expect(slot("1-2")).not.toBeChecked();
    await slot("1-2").check();
    await expect(slot("1-2")).toBeChecked();
    await slot("7-8").uncheck();
    await expect(slot("7-8")).not.toBeChecked();
    // The last slot stays: a source sends on one pair at least.
    await slot("1-2").click();
    await expect(inst.getByRole("alert")).toContainText("one pair of outputs at least");
    await expect(slot("1-2")).toBeChecked();

    // Front of house loses Main right; a new group of the two USB-direct units.
    const front = inst.getByLabel("Group Front of house");
    await expect(front.getByRole("checkbox", { name: "Main left" })).toBeChecked();
    await front.getByRole("checkbox", { name: "Main right" }).uncheck();
    await expect(front.getByRole("checkbox", { name: "Main right" })).not.toBeChecked();
    const newGroup = inst.getByRole("form", { name: "New group" });
    await newGroup.getByRole("textbox", { name: "New group's name" }).fill("Rears");
    await newGroup.getByRole("checkbox", { name: "Rear L/R" }).check();
    await newGroup.getByRole("checkbox", { name: "Height" }).check();
    await newGroup.getByRole("button", { name: "Add group" }).click();
    const rears = inst.getByLabel("Group Rears");
    await expect(rears.getByRole("checkbox", { name: "Rear L/R" })).toBeChecked();
    await expect(rears.getByRole("checkbox", { name: "Height" })).toBeChecked();

    // A zone of the terrace playing Music; Inside already plays it.
    await expect(inst.getByLabel("Zone Inside")).toContainText("Plays Music");
    const newZone = inst.getByRole("form", { name: "New zone" });
    await newZone.getByRole("textbox", { name: "New zone's name" }).fill("Outside");
    await newZone.getByRole("checkbox", { name: "Terrace" }).check();
    await newZone.getByRole("combobox", { name: "New zone's source" }).selectOption({ label: "Music" });
    await newZone.getByRole("button", { name: "Add zone" }).click();
    await expect(inst.getByLabel("Zone Outside")).toContainText("Plays Music");

    const unsaved = inst.getByRole("group", { name: "Unsaved installation" });
    await expect(unsaved.getByRole("button", { name: "Save" })).toBeEnabled();
    await page.setViewportSize({ width: 375, height: 900 });
    await fitsWidth(page);
    await shots(page, "installation-375");
    await save(page, inst, [/Outside/, /Rears/]);
    await expect(inst.getByRole("status", { name: "Installation saved" })).toBeVisible();

    // Read back on a fresh page.
    const again = await advancedTab(await openOn(page, hub, "DEMO-ALINK-A"), page);
    const t2 = again.getByLabel("Unit Terrace");
    await expect(t2.getByRole("combobox", { name: "Role" })).toHaveValue("spdif_source");
    const s2 = (o: string) => t2.getByRole("checkbox", { name: `Terrace sends the time base on outputs ${o}` });
    await expect(s2("1-2")).toBeChecked();
    await expect(s2("7-8")).not.toBeChecked();
    await expect(again.getByLabel("Group Front of house").getByRole("checkbox", { name: "Main right" })).not.toBeChecked();
    await expect(again.getByLabel("Group Rears").getByRole("checkbox", { name: "Height" })).toBeChecked();
    await expect(again.getByLabel("Zone Outside")).toContainText("Plays Music");
    await expect(again.getByLabel("Zone Outside").getByRole("checkbox", { name: "Terrace" })).toBeChecked();
    await fitsWidth(page);
  });
});
