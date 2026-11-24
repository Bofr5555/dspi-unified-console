// WP INSTW e2e: the System screen's Installation tab (Installer mode, on a hub), against the real
// hub: `uconsole --demo hub --web --alink` (the SW3 installation: a sender, a fan-out, four alink
// units and two USB-direct ones, all attached as fakes, with the inventory service on). Units are
// removed and saved, so the hub sees them but the installation does not hold them; they are joined
// again with a room, a role and an upstream, a room and a zone are made, saved, and read back. A
// removal the hub would refuse is named at the field; a save that meets another page's save says
// the installation changed elsewhere and rebuilds the draft. A Control session sees it read only.
// And a hub with no installation gets its first one.
//
// Screenshots at 375 px, dark and light, beside the Playwright output in instw-shots/ (chromium).

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Browser, type Locator, type Page } from "@playwright/test";
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

async function installerTab(session: Locator, page: Page): Promise<Locator> {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: "Installer" }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: "Mode: Installer" })).toBeVisible();
  await openPage(session, "Installation"); // -- WP WEB7 -- Qt's System tab
  const inst = session.getByRole("region", { name: "Installation", exact: true });
  await expect(inst.getByRole("heading", { name: "Units" })).toBeVisible();
  return inst;
}

/** Save, confirming in the dialog that lists the changes; `said` are sentences it must list. */
async function save(page: Page, inst: Locator, said: RegExp[] = []): Promise<void> {
  const unsaved = inst.getByRole("group", { name: "Unsaved installation" });
  await unsaved.getByRole("button", { name: "Save" }).click();
  const dialog = page.getByRole("dialog", { name: "Save the installation?" });
  for (const s of said) await expect(dialog).toContainText(s);
  await dialog.getByRole("button", { name: "Save" }).click();
}

async function shots(page: Page, name: string) {
  const out = process.env.PW_OUTPUT_DIR;
  if (!out || test.info().project.name !== "chromium") return;
  const dir = `${out}/../instw-shots`;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/${name}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** The page is no wider than its viewport (no sideways scrolling). */
async function fitsWidth(page: Page) {
  const { scroll, view } = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, view: window.innerWidth }));
  expect(scroll).toBeLessThanOrEqual(view);
}

if (!process.env.DSPI_TERM) {
  test("INSTW: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("INSTW: the Installation tab on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("INSTW: join units with a room, a role and an upstream, make a room and a zone, save, and read it back", async ({ page }) => {
    test.setTimeout(120_000);
    hub = await startHub(["--alink", "--open", "--forget-clients"]);
    const session = await openOn(page, hub, "DEMO-ALINK-A");
    const inst = await installerTab(session, page);
    const seen = inst.getByRole("region", { name: "Units not in the installation" });
    await expect(seen).toContainText("Every unit the hub sees is in the installation.");

    // Removing the fan-out leaves Main right fed by nothing: the hub's validator says so at the field, and Save is not offered.
    await inst.getByLabel("Fan-out Fan-out").getByRole("button", { name: "Remove Fan-out" }).click();
    const right = inst.getByLabel("Unit Main right");
    await expect(right.locator(".sys-issue")).toContainText('upstream "fan" is not a sender, fan-out or repeater');
    await expect(inst.getByRole("group", { name: "Unsaved installation" }).getByRole("button", { name: "Save" })).toBeDisabled();
    await inst.getByRole("button", { name: "Revert" }).click();
    await expect(inst.getByLabel("Fan-out Fan-out")).toBeVisible();
    await expect(right.locator(".sys-issue")).toHaveCount(0);

    // Take two units out and save: the hub then sees them, outside the installation.
    await inst.getByLabel("Unit Terrace").getByRole("button", { name: "Remove Terrace" }).click();
    await inst.getByLabel("Unit Bar").getByRole("button", { name: "Remove Bar" }).click();
    await save(page, inst, [/Removes unit/]);
    await expect(inst.getByRole("status", { name: "Installation saved" })).toBeVisible();
    await expect(inst.getByLabel("Unit Terrace")).toHaveCount(0);
    const cardC = seen.getByLabel(/^Seen /).filter({ hasText: "DEMO-ALINK-C" });
    const cardD = seen.getByLabel(/^Seen /).filter({ hasText: "DEMO-ALINK-D" });
    await expect(cardC).toBeVisible();
    await expect(cardD).toBeVisible();

    // A new room, then the two units joined: one in it behind the fan-out, one a repeater in the bar.
    await inst.getByRole("textbox", { name: "New room's name" }).fill("Patio");
    await inst.getByRole("button", { name: "Add room" }).click();
    await expect(inst.getByLabel("Room Patio")).toBeVisible();

    await cardC.getByRole("button", { name: "Join" }).click();
    const joinC = cardC.getByRole("group", { name: /^Join / });
    await joinC.getByRole("textbox", { name: "Name" }).fill("Patio speaker");
    await joinC.getByRole("combobox", { name: "As" }).selectOption("receiver");
    await joinC.getByRole("combobox", { name: "Room" }).selectOption({ label: "Patio" });
    await joinC.getByRole("combobox", { name: "Fed by" }).selectOption({ label: "Fan-out" });
    await joinC.getByRole("button", { name: "Add to the installation" }).click();
    await expect(inst.getByLabel("Unit Patio speaker")).toBeVisible();
    await expect(cardC).toHaveCount(0);

    await cardD.getByRole("button", { name: "Join" }).click();
    const joinD = cardD.getByRole("group", { name: /^Join / });
    await joinD.getByRole("textbox", { name: "Name" }).fill("Bar repeater");
    await joinD.getByRole("combobox", { name: "As" }).selectOption("repeater");
    await joinD.getByRole("combobox", { name: "Room" }).selectOption({ label: "Bar" });
    await joinD.getByRole("combobox", { name: "Fed by" }).selectOption({ label: "Sender" });
    await joinD.getByRole("button", { name: "Add to the installation" }).click();
    const barRep = inst.getByLabel("Unit Bar repeater");
    await expect(barRep.getByRole("combobox", { name: "Role" })).toHaveValue("repeater");

    // The new unit may now be fed by the repeater; and a zone of Terrace and Patio.
    const patio = inst.getByLabel("Unit Patio speaker");
    await patio.getByRole("combobox", { name: "Fed by" }).selectOption({ label: "Bar repeater" });
    const newZone = inst.getByRole("form", { name: "New zone" });
    await newZone.getByRole("textbox", { name: "New zone's name" }).fill("Outside");
    await newZone.getByRole("checkbox", { name: "Terrace" }).check();
    await newZone.getByRole("checkbox", { name: "Patio" }).check();
    await newZone.getByRole("button", { name: "Add zone" }).click();
    await expect(inst.getByLabel("Zone Outside")).toBeVisible();

    const changes = inst.getByRole("list", { name: "Changes" });
    await expect(changes).toContainText("Adds room Patio");
    await expect(changes).toContainText("Adds zone Outside");
    await expect(changes).toContainText("Adds unit Patio speaker");
    await expect(changes).toContainText("Adds unit Bar repeater");

    // 375 px, both themes: everything wraps, nothing scrolls sideways.
    await page.setViewportSize({ width: 375, height: 900 });
    await fitsWidth(page);
    await shots(page, "installation-375-draft");

    await save(page, inst, [/Adds zone Outside/, /Adds unit Patio speaker/]);
    await expect(inst.getByRole("status", { name: "Installation saved" })).toBeVisible();
    await expect(inst.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    // Read back from the hub: the units with their room, role and upstream, the zone's rooms.
    await expect(patio.getByRole("combobox", { name: "Room" })).toHaveValue("Patio");
    await expect(patio.getByRole("combobox", { name: "Role" })).toHaveValue("receiver");
    await expect(patio.getByRole("combobox", { name: "Fed by" }).locator("option:checked")).toHaveText("Bar repeater");
    await expect(barRep.getByRole("combobox", { name: "Room" })).toHaveValue("bar");
    const zone = inst.getByLabel("Zone Outside");
    await expect(zone.getByRole("checkbox", { name: "Terrace" })).toBeChecked();
    await expect(zone.getByRole("checkbox", { name: "Patio" })).toBeChecked();
    await expect(zone.getByRole("checkbox", { name: "Main room" })).not.toBeChecked();
    await expect(seen).toContainText("Every unit the hub sees is in the installation.");
    await fitsWidth(page);
    await shots(page, "installation-375-saved");

    // The System screen's status shows the new room and the zone.
    await openPage(session, "System"); // -- WP WEB7 -- Qt\'s Units tab
    const room = session.getByRole("region", { name: "Room Patio" });
    await expect(room.getByLabel("Unit Patio speaker")).toBeVisible();
    await expect(session.getByRole("region", { name: "Room Terrace" }).locator(".sys-chip", { hasText: "Outside" })).toBeVisible();
    await fitsWidth(page);
  });

  test("INSTW: a save after someone else's says the installation changed elsewhere and rebuilds the draft", async ({ page, browser }) => {
    test.setTimeout(120_000);
    hub = await startHub(["--alink", "--open", "--forget-clients"]);
    const inst = await installerTab(await openOn(page, hub, "DEMO-ALINK-A"), page);
    await inst.getByRole("textbox", { name: "New room's name" }).fill("Cellar");
    await inst.getByRole("button", { name: "Add room" }).click();
    await expect(inst.getByLabel("Room Cellar")).toBeVisible();

    // Another page saves first.
    const other = await otherPage(browser);
    const inst2 = await installerTab(await openOn(other, hub, "DEMO-ALINK-B"), other);
    await inst2.getByRole("textbox", { name: "New room's name" }).fill("Attic");
    await inst2.getByRole("button", { name: "Add room" }).click();
    await save(other, inst2, [/Adds room Attic/]);
    await expect(inst2.getByRole("status", { name: "Installation saved" })).toBeVisible();
    await other.context().close();

    await save(page, inst, [/Adds room Cellar/]);
    const stale = inst.getByRole("alert", { name: "Installation changed elsewhere" });
    await expect(stale).toContainText("changed elsewhere");
    // The draft is the hub's now: the other save is there, this page's room is not (nothing was overwritten).
    await expect(inst.getByLabel("Room Attic")).toBeVisible();
    await expect(inst.getByLabel("Room Cellar")).toHaveCount(0);
    await expect(inst.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    // Made again on it, then saved: both rooms.
    await stale.getByRole("button", { name: "Make them again" }).click();
    await expect(inst.getByLabel("Room Cellar")).toBeVisible();
    await save(page, inst, [/Adds room Cellar/]);
    await expect(inst.getByRole("status", { name: "Installation saved" })).toBeVisible();
    await expect(inst.getByLabel("Room Attic")).toBeVisible();
    await expect(inst.getByLabel("Room Cellar")).toBeVisible();
  });

  test("INSTW: a Control session sees the installation read only", async ({ page }) => {
    test.setTimeout(90_000);
    hub = await startHub(["--alink", "--pair"], true);
    const inst = await installerTab(await openOn(page, hub, "DEMO-ALINK-A"), page);
    await expect(inst).toContainText("Changing the installation needs the hub's Admin role: the installation is shown, not changed.");
    await expect(inst.getByLabel("Unit Main left").getByRole("combobox", { name: "Room" })).toBeDisabled();
    await expect(inst.getByRole("button", { name: /^Remove/ })).toHaveCount(0);
    await expect(inst.getByRole("button", { name: "Add room" })).toHaveCount(0);
    await expect(inst.getByRole("button", { name: "Join" })).toHaveCount(0);
  });

  test("INSTW: a hub with no installation gets its first one", async ({ page }) => {
    test.setTimeout(90_000);
    hub = await startHub(["--open", "--forget-clients"]);
    const demo = "DEMO0000DSPI0001";
    const session = await openOn(page, hub, demo);
    const inst = await installerTab(session, page);
    await expect(inst).toContainText("No units yet");
    await inst.getByRole("textbox", { name: "New room's name" }).fill("Studio");
    await inst.getByRole("button", { name: "Add room" }).click();
    const card = inst.getByRole("region", { name: "Units not in the installation" }).getByLabel(/^Seen /).filter({ hasText: demo });
    await card.getByRole("button", { name: "Join" }).click();
    const join = card.getByRole("group", { name: /^Join / });
    await join.getByRole("textbox", { name: "Name" }).fill("Desk");
    await join.getByRole("combobox", { name: "As" }).selectOption("usb_direct");
    await expect(join.getByRole("combobox", { name: "Fed by" })).toHaveCount(0);
    await join.getByRole("combobox", { name: "Room" }).selectOption({ label: "Studio" });
    await join.getByRole("button", { name: "Add to the installation" }).click();
    await save(page, inst, [/Adds unit Desk/]);
    await expect(inst.getByRole("status", { name: "Installation saved" })).toBeVisible();
    await expect(inst.getByLabel("Unit Desk").getByRole("combobox", { name: "Role" })).toHaveValue("usb_direct");
    // The System screen shows the installation now, not the device alone.
    await openPage(session, "System"); // -- WP WEB7 -- Qt\'s Units tab
    await expect(session.getByRole("region", { name: "Room Studio" }).getByLabel("Unit Desk")).toBeVisible();
  });
});

async function otherPage(browser: Browser): Promise<Page> {
  const ctx = await browser.newContext();
  return ctx.newPage();
}
