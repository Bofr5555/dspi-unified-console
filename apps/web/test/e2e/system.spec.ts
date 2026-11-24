// WP SW3 e2e: the System view (WB6d), the Installer mode's "Clock and audio links" panel, the
// scene bar (WB6e) and the status rail (WB6f), against the real hub: `uconsole --demo hub
// --web --alink`, an installation of the fake device's audio-link roles (a sender, a
// fan-out re-emitting 3 frames late, four units) with its system model. Everything goes
// through the hub's `system.*` on the session's own Link connection. An open hub makes the
// page Admin; a hub with a PIN pairs it as Control, and the hub's roles decide.
//
// And one device on its own (the core's demo, as for USB) is a system of one unit.
//
// Screenshots for the report, dark and light, when SW3_SHOTS names a directory.

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { expectPage, hasPage, openPage } from "../nav";

type OwnHub = { port: number; pin: string; child: ChildProcess; store: string };

/**
 * `uconsole --demo hub --web --alink ...` on a free port (or `port`), once it is up (with
 * `pairing`, its PIN). -- fixup sw3x -- `store` and `port` to start the same hub again.
 */
async function linkHub(extra: string[], pairing = false, store?: string, port0 = 0): Promise<OwnHub> {
  store ??= `${process.env.TMPDIR ?? "test-results"}/sw3-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--alink", "--console", "dist", ...extra, "--port", String(port0), "--json", "--no-keep-awake", "--store", store],
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
  return { port, pin, child, store };
}

/** -- fixup sw3x -- stop a hub and wait until it has gone. */
async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

// -- WP FLAKE1 --
/**
 * The same hub's installation with an apply lead of `frames` (at 48 kHz) and without its
 * USB-direct units (a bus frame is at most 1.92 s ahead, so they would not be on time with a
 * longer lead): the hub is stopped, the model its store kept is changed, and the hub started
 * again on that store. A scene through the sender is then held for that long, however slowly a
 * loaded host runs the page.
 */
async function withApplyLead(h: OwnHub, frames: number, extra: string[]): Promise<OwnHub> {
  await stopHub(h);
  const file = `${h.store}/hub-store.json`;
  const kept = JSON.parse(readFileSync(file, "utf8")) as { system: { applyLeadFrames?: number; units: { transport?: string }[] } };
  kept.system.applyLeadFrames = frames;
  kept.system.units = kept.system.units.filter((u) => u.transport !== "usb_direct");
  writeFileSync(file, JSON.stringify(kept));
  return linkHub(extra, false, h.store);
}

const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

/** Connect the serving hub, pair when asked, open unit `serial`'s session. */
async function openUnit(page: Page, hub: OwnHub, serial: string, pair = !!hub.pin): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = picker(page).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (pair) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(serial);
  // -- WP FLAKE1 -- the page settles before a test clicks: Link's "bound" phase arrives after the
  // open (its banner above the screens goes then), and the installation's first status after it.
  // A click while either still moves the page lands where the button was (the rail's Mode and
  // System mute buttons, on a loaded host).
  const rail = session.getByRole("complementary", { name: "Status" });
  await expect(rail.getByLabel("Connection")).toHaveText(/^Connected/);
  await expect(session.locator(".banner")).toHaveCount(0);
  await expect(rail.getByRole("button", { name: "System mute" })).toHaveText(/^System mute: (on|off)$/);
  return session;
}

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced" | "Installer") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

/** -- WP AUSB1 -- the report's pictures: beside the Playwright output, in ausb1-shots/ (chromium only). */
async function ausb1Shot(page: Page, name: string) {
  const out = process.env.PW_OUTPUT_DIR;
  if (!out || test.info().project.name !== "chromium") return;
  const dir = `${out}/../ausb1-shots`;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/${name}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

async function shot(page: Page, name: string) {
  const dir = process.env.SW3_SHOTS;
  if (!dir || test.info().project.name !== "chromium") return;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/${name}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

test.describe("SW3: an audio link installation on the hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  // Each test has its hub: scenes, the mute and the alignment change the units.
  test.afterEach(() => {
    hub?.child.kill();
    hub = undefined;
  });

  test("SW3: the System view shows rooms, zones, units and the topology", async ({ page }) => {
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    // On an installation the System view is the first screen (Advanced).
    await expectPage(session, "System");
    const system = session.getByRole("region", { name: "System" });
    const main = system.getByRole("region", { name: "Room Main room" });
    await expect(main.getByText("Inside")).toBeVisible();
    for (const name of ["Main left", "Main right"]) {
      const unit = main.getByLabel(`Unit ${name}`);
      await expect(unit).toContainText("Connected");
      await expect(unit).toContainText("Audio link: Locked");
    }
    await expect(system.getByRole("region", { name: "Room Terrace" }).getByLabel("Unit Terrace")).toContainText("Ch 2, Ch 3");
    // The topology: the sender, its fan-out (3 frames) and the units behind each.
    const topo = system.getByRole("list", { name: "Topology" });
    await expect(topo.getByLabel("Sender Sender")).toContainText("Clock");
    await expect(topo.getByLabel("Fan-out Fan-out")).toContainText("+3 frames");
    // The innermost item holding the fan-out: the sender's holds it too.
    const fanItem = topo.getByRole("listitem").filter({ has: page.getByLabel("Fan-out Fan-out") }).last();
    await expect(fanItem.getByLabel("Unit Main right")).toBeVisible();
    await expect(fanItem.getByLabel("Unit Terrace")).toBeVisible();
    await expect(fanItem.getByLabel("Unit Main left")).toHaveCount(0);
    await expect(topo.getByLabel("Audio link slots of Bar")).toBeVisible();
    // The rail: connected, no alarms, the system mute off, Advanced.
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByLabel("Connection")).toContainText("Connected");
    await expect(rail.getByLabel("Alarms")).toHaveText("No alarms");
    await expect(rail.getByRole("button", { name: "System mute" })).toContainText(": off");
    await expect(rail.getByRole("button", { name: "Mode: Advanced" })).toBeVisible();
    await shot(page, "system-view");
  });

  test("SW3: a scene is recalled at one frame and each unit's result shown", async ({ page }) => {
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const bar = session.getByLabel("Scenes", { exact: true });
    // One op for every unit: through the sender's superframe.
    await bar.getByRole("button", { name: "Recall Evening" }).click();
    const result = session.getByLabel("Scene result");
    await expect(result).toContainText("through the sender");
    await expect(result).toContainText("on time on every unit");
    const eveningLead = Number(/\((\d+) frames ahead/.exec((await result.textContent()) ?? "")?.[1]);
    expect(eveningLead).toBeGreaterThan(0);
    await expect(session.getByRole("complementary", { name: "Status" }).getByLabel("Scene")).toHaveText("Scene: Evening");
    // A zone and a room: to each unit, each unit's own answer.
    await bar.getByRole("button", { name: "Recall Quiet inside" }).click();
    await expect(result).toContainText("to each unit");
    const perUnit = result.getByRole("list", { name: "Per unit" });
    await expect(perUnit.getByLabel("Result for Main left")).toContainText("Master volume -30.0 dB: Queued");
    await expect(perUnit.getByLabel("Result for Bar")).toContainText("Queued");
    await expect(perUnit.getByLabel("Result for Terrace")).toContainText("Mute: Queued");
    // The system mute with a scene, through the sender again. A sender holds one superframe
    // apply at a time and refuses a second until its frame has passed (the result says so):
    // wait out Evening's lead, which the result showed, at 48 frames a millisecond.
    await page.waitForTimeout(Math.ceil(eveningLead / 48) + 300);
    await bar.getByRole("button", { name: "Recall Closed" }).click();
    await expect(result).toContainText("system mute on");
    await expect(result).toContainText("on time on every unit");
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByRole("button", { name: "System mute" })).toContainText(": on", { timeout: 10_000 });
    await shot(page, "scene-bar");
  });

  test("SW3: the system mute and an alarm acknowledged from the rail", async ({ page }) => {
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-B");
    const rail = session.getByRole("complementary", { name: "Status" });
    const mute = rail.getByRole("button", { name: "System mute" });
    await expect(mute).toContainText(": off");
    await mute.click();
    await expect(mute).toContainText(": on", { timeout: 10_000 });
    const alarm = rail.getByRole("list", { name: "Alarm list" }).getByLabel("Alarm system_muted on Sender");
    await expect(alarm).toHaveAttribute("data-state", "raised", { timeout: 10_000 });
    await expect(rail.getByLabel("Alarms")).toHaveText("1 alarm");
    await alarm.getByRole("button", { name: "Acknowledge" }).click();
    await expect(alarm).toHaveAttribute("data-state", "acknowledged");
    await shot(page, "status-rail");
    await mute.click();
    await expect(mute).toContainText(": off", { timeout: 10_000 });
    await expect(alarm).toHaveAttribute("data-state", "cleared", { timeout: 10_000 });
    await expect(rail.getByLabel("Alarms")).toHaveText("No alarms");
  });

  test("SW3: Clock and audio links: delays, slots, the alignment, link stats and the trim scan (Admin)", async ({ page }) => {
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const system = session.getByRole("region", { name: "System" });
    // -- WP WEB7 -- Qt's tabs: the clock is a page of its own (Clock and Links), not part of Units
    await expect(system.getByRole("region", { name: "Clock and audio links" })).toHaveCount(0);
    await setMode(session, page, "Installer");
    await openPage(session, "Clock and Links");
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    await expect(panel).toBeVisible();
    await expect(panel.getByText("this panel is read only")).toHaveCount(0);
    // Per unit: the range the unit reports, its audio link delay and hop offset as the hub computes them,
    // in the glossary's words with the firmware's names in a tooltip.
    await expect(panel.getByRole("columnheader", { name: "Audio link delay", exact: true }).locator("abbr")).toHaveAttribute("title", /^d_local /);
    await expect(panel.getByRole("columnheader", { name: "Hop offset" }).locator("abbr")).toHaveAttribute("title", /^hops_offset /);
    await expect(panel).not.toContainText(/d_local|hops_offset/);
    const units = panel.getByRole("table", { name: "Units on the audio link" });
    const right = units.getByRole("row", { name: "Clock of Main right" });
    await expect(right).toContainText("257..");
    await expect(right).toContainText("3 frames"); // behind the fan-out
    // Link stats and the trim scan of a receiver.
    const stats = panel.getByRole("table", { name: "Audio link stats" });
    await expect(stats.getByRole("row", { name: "Audio link stats of Main left" })).toContainText("locked");
    await expect(stats.getByLabel("Trim scan: clean -3 to 4, using 0").first()).toBeVisible();
    // Compute, then align: each unit's result.
    const align = panel.getByRole("region", { name: "Alignment" });
    await align.getByRole("button", { name: "Compute" }).click();
    await expect(align.getByLabel("Alignment result")).toContainText("Computed, not written");
    await expect(align.getByLabel("Plan for Terrace")).toContainText("Writable");
    await align.getByRole("button", { name: "Align audio link delays…", exact: true }).click();
    await page.getByRole("dialog", { name: "Align audio link delays?" }).getByRole("button", { name: "Align" }).click();
    // -- WP HWLK1F -- the core's sentence for what was written
    await expect(align.getByLabel("Alignment result")).toContainText("The audio link delays were written.");
    const writes = align.getByRole("list", { name: "Alignment writes" });
    await expect(writes.getByLabel("Result for Main right")).toContainText("Audio link delay: Done");
    await expect(writes.getByLabel("Result for Main left")).toContainText("Delay OUT1");
    await shot(page, "alink-clock");
    // The slot matrix: Main right's input 3 from audio link channel 6, a measured latency; saved, then aligned.
    const matrix = panel.getByRole("table", { name: "Audio link slots" });
    const cell = matrix.getByRole("button", { name: "Main right IN3 from Ch 6" });
    await expect(cell).toHaveAttribute("aria-pressed", "false");
    await cell.click();
    await expect(cell).toHaveAttribute("aria-pressed", "true");
    const measured = right.getByLabel("Measured latency of Main right");
    await measured.fill("250");
    await measured.press("Enter");
    await panel.getByRole("button", { name: "Save" }).click();
    await expect(panel.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    await expect(cell).toHaveAttribute("aria-pressed", "true");
    await expect(measured).toHaveValue("250");
    await openPage(session, "System"); // -- WP WEB7 -- the units are on their own tab
    await expect(system.getByRole("region", { name: "Room Main room" }).getByLabel("Unit Main right")).toContainText("Ch 6");
    await openPage(session, "Clock and Links");
    await align.getByRole("button", { name: "Align audio link delays…", exact: true }).click();
    await page.getByRole("dialog", { name: "Align audio link delays?" }).getByRole("button", { name: "Align" }).click();
    await expect(writes.getByLabel("Result for Main right")).toContainText("Audio link slots: Done");
  });

  test("SW3: a Control role sees the installer panel read only and a refused op named", async ({ page }) => {
    hub = await linkHub(["--pair"], true);
    const session = await openUnit(page, hub, "DEMO-ALINK-C");
    await setMode(session, page, "Installer");
    await openPage(session, "Clock and Links"); // -- WP WEB7 -- Qt's tab
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    await expect(panel.getByText("Changing the installation needs the hub's Admin role: this panel is read only.")).toBeVisible();
    await expect(panel.getByRole("button", { name: "Align audio link delays…", exact: true })).toBeDisabled();
    await expect(panel.getByRole("table", { name: "Audio link slots" }).getByRole("button").first()).toBeDisabled();
    await expect(panel.getByLabel(/^Measured latency of/)).toHaveCount(0);
    // The hub takes system.align from Admin only; the status's computation is shown all the same.
    await expect(panel.getByRole("button", { name: "Compute" })).toBeDisabled();
    await expect(panel.getByRole("row", { name: "Clock of Terrace" })).toContainText("3 frames");
    // A scene with an admin-only op is refused whole, and the op named.
    const bar = session.getByLabel("Scenes", { exact: true });
    await bar.getByRole("button", { name: "Recall Test click" }).click();
    await expect(session.getByLabel("Scene result").getByRole("alert")).toContainText("Test click: refused. Op click on unit:A");
    // A control op runs.
    await bar.getByRole("button", { name: "Recall Evening" }).click();
    await expect(session.getByLabel("Scene result")).toContainText("on time on every unit");
    await shot(page, "control-role");
  });

  // -- fixup sw3x --
  test("SW3: a second scene through the sender while one is scheduled is refused, said plainly and not marked", async ({ page }) => {
    // -- WP FLAKE1 -- Evening is held by the sender for ten seconds, not the default 50 ms: on a
    // loaded host the second recall came after Evening's frame, and the sender took it.
    hub = await withApplyLead(await linkHub(["--open", "--forget-clients"]), 480_000, ["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const bar = session.getByLabel("Scenes", { exact: true });
    const result = session.getByLabel("Scene result");
    const rail = session.getByRole("complementary", { name: "Status" });
    await bar.getByRole("button", { name: "Recall Evening" }).click();
    await expect(result).toContainText("on time on every unit");
    // At once, before Evening's frame: the sender holds Evening and refuses Closed.
    await bar.getByRole("button", { name: "Recall Closed" }).click();
    const alert = result.getByRole("alert");
    // -- WP WB8 -- the core's sentence (`sceneOutcome`), the one every client shows
    await expect(alert).toHaveText(/^Closed was refused: a scene is already scheduled on the sender for audio link frame \d+, \d+ ms after this recall\. The sender takes one scene at a time; recall Closed again after that\. System mute unchanged\.$/);
    await expect(result).not.toContainText("system mute on");
    // -- fixup lk1 -- the sender's write is the scene's, never read as a refused mute
    await expect(result.getByRole("list", { name: "Per unit" })).toContainText("Scene Closed: Refused");
    await expect(result).not.toContainText("Mute: Refused");
    await expect(bar.getByRole("button", { name: "Recall Evening" })).toHaveAttribute("aria-current", "true");
    await expect(bar.getByRole("button", { name: "Recall Closed" })).not.toHaveAttribute("aria-current", "true");
    await expect(bar.getByRole("button", { name: "Recall Closed" })).not.toHaveClass(/active/);
    await expect(rail.getByLabel("Scene")).toHaveText("Scene: Evening · Closed refused");
    await expect(rail.getByRole("button", { name: "System mute" })).toContainText(": off");
    await shot(page, "scene-refused");
  });

  // -- WP HWLK1F -- hw-lk1's K340: a unit whose firmware has no audio link (`--with-plain-unit`)
  test("HWLK1F: a unit without the audio link is said so, takes a scene at once, is skipped by the alignment and warned about", async ({ page }) => {
    test.setTimeout(60_000);
    hub = await linkHub(["--open", "--forget-clients", "--with-plain-unit"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const system = session.getByRole("region", { name: "System" });
    const plain = system.getByRole("region", { name: "Room Bar" }).getByLabel("Unit Plain unit");
    await expect(plain).toContainText("Connected");
    await expect(plain).toContainText("No audio link (firmware)");
    await expect(plain).not.toContainText("Unknown");
    await expect(system.getByRole("list", { name: "Topology" }).getByLabel("Unit Plain unit")).toContainText("No audio link (firmware)");
    // A scene for every unit: at once on P, through the sender on the others; never "on time on every unit".
    await session.getByLabel("Scenes", { exact: true }).getByRole("button", { name: "Recall Evening" }).click();
    const result = session.getByLabel("Scene result");
    // -- WP AUSB1 -- the demo's USB-direct units take it at a USB frame
    await expect(result).toContainText(
      /Evening recalled: at USB frame \d+ on Rear L\/R, Height; at audio link frame \d+ through the sender on the others; applied now on Plain unit \(no audio link\)\./,
    );
    await expect(result).not.toContainText("on time on every unit");
    await expect(result.getByRole("list", { name: "Per unit" }).getByLabel("Result for Plain unit")).toContainText("Master volume -12.0 dB: Applied now");
    await shot(page, "hwlk1f-scene");
    // Aligning: written on the others, skipped on P, and why.
    await setMode(session, page, "Installer");
    await openPage(session, "Clock and Links"); // -- WP WEB7 -- Qt's tab
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    const align = panel.getByRole("region", { name: "Alignment" });
    await align.getByRole("button", { name: "Align audio link delays…", exact: true }).click();
    await page.getByRole("dialog", { name: "Align audio link delays?" }).getByRole("button", { name: "Align" }).click();
    await expect(align.getByLabel("Alignment result")).toContainText("skipped on Plain unit (no audio link (firmware))");
    await expect(align.getByLabel("Alignment result")).toContainText("The audio link delays were written on Main left");
    // Saving the installation keeps P, with the hub's warning.
    const measured = panel.getByRole("table", { name: "Units on the audio link" }).getByLabel("Measured latency of Main right");
    await measured.fill("250");
    await measured.press("Enter");
    await panel.getByRole("button", { name: "Save" }).click();
    await expect(panel.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    await expect(panel.getByRole("status", { name: "Installation warnings" })).toContainText(
      "Plain unit (DEMO-PLAIN): its firmware has no audio link, so it cannot be a receiver on it.",
    );
    await shot(page, "hwlk1f-align-and-warning");
  });

  // -- WP AUSB1 -- the demo installation's USB-direct units (Rear L/R, Height) beside the audio link's
  test("AUSB1: USB-direct units are shown with their state and statistics, take scenes at a USB frame, and their mode is set", async ({ page }) => {
    test.setTimeout(90_000);
    hub = await linkHub(["--open", "--forget-clients", "--with-plain-unit"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const system = session.getByRole("region", { name: "System" });
    const main = system.getByRole("region", { name: "Room Main room" });
    const rear = main.getByLabel("Unit Rear L/R");
    await expect(rear).toContainText("Connected");
    await expect(rear).toContainText("USB-direct: Phase-locked");
    await expect(rear).toContainText(/host -11\.5 ppm · VCXO -11\.\d ppm · phase error [+-]\d\.\d\d samples · gaps 0, missed 0, late 0 samples/);
    await expect(main.getByLabel("Unit Main left")).toContainText("Audio link: Locked");
    const topo = system.getByRole("list", { name: "Topology" });
    await expect(topo.getByLabel("USB-direct Height")).toContainText("Phase-locked");
    await expect(system.getByRole("region", { name: "Room Bar" }).getByLabel("Unit Plain unit")).toContainText("No audio link (firmware)");
    await ausb1Shot(page, "system-view-mixed");
    // A scene for every unit: at a USB frame on the USB-direct units, at an audio link frame through the sender, at once on P.
    const bar = session.getByLabel("Scenes", { exact: true });
    await bar.getByRole("button", { name: "Recall Evening" }).click();
    const result = session.getByLabel("Scene result");
    await expect(result).toContainText(/at USB frame \d+ on Rear L\/R, Height; at audio link frame \d+ through the sender on the others; applied now on Plain unit \(no audio link\)/);
    const perUnit = result.getByRole("list", { name: "Per unit" });
    await expect(perUnit.getByLabel("Result for Rear L/R")).toContainText("Master volume -12.0 dB: Queued");
    await expect(perUnit.getByLabel("Result for Sender")).toContainText("Scene Evening: Queued");
    await ausb1Shot(page, "scene-mixed");
    // The Installer's panel: the USB-direct units, their delay against the system delay, the start offset, the stats.
    await setMode(session, page, "Installer");
    await openPage(session, "Clock and Links"); // -- WP WEB7 -- Qt's tab
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    const usb = panel.getByRole("table", { name: "USB-direct units" });
    await expect(usb.getByRole("row", { name: "USB-direct clock of Rear L/R" })).toContainText("170..399");
    await expect(usb.getByRole("row", { name: "USB-direct clock of Rear L/R" })).toContainText("144 samples");
    await expect(usb.getByRole("row", { name: "USB-direct clock of Height" })).toContainText("Phase-locked");
    await expect(panel.getByRole("table", { name: "Units on the audio link" }).getByRole("row", { name: "Clock of Rear L/R" })).toHaveCount(0);
    const stats = panel.getByRole("table", { name: "USB-direct stats" });
    await expect(stats.getByRole("row", { name: "USB-direct stats of Height" })).toContainText("-11.5 ppm");
    await expect(stats.getByRole("row", { name: "USB-direct stats of Height" })).toContainText("0 (last: none)");
    // Height to rate lock: saved on the hub (by revision), written with the alignment.
    await usb.getByLabel("USB-direct mode of Height").selectOption("rate_lock");
    await panel.getByRole("button", { name: "Save" }).click();
    await expect(panel.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    const align = panel.getByRole("region", { name: "Alignment" });
    await align.getByRole("button", { name: "Align audio link delays…", exact: true }).click();
    await page.getByRole("dialog", { name: "Align audio link delays?" }).getByRole("button", { name: "Align" }).click();
    const writes = align.getByRole("list", { name: "Alignment writes" });
    await expect(writes.getByLabel("Result for Height")).toContainText("USB-direct mode and delay: Done");
    await expect(writes.getByLabel("Result for Rear L/R")).toContainText("Delay OUT1: Done");
    await expect(usb.getByRole("row", { name: "USB-direct clock of Height" })).toContainText("Rate-locked", { timeout: 10_000 });
    await openPage(session, "System"); // -- WP WEB7 -- the units are on their own tab
    await expect(main.getByLabel("Unit Height")).toContainText("USB-direct: Rate-locked", { timeout: 10_000 });
    await ausb1Shot(page, "clock-and-audio-links-usb");
  });

  // -- WP AUSB2 -- a USB-direct unit that can't lock: said, alarmed, and its action set by the Installer
  test("AUSB2: a USB-direct unit that can't lock is shown with the hub's alarm, and the Installer sets what it does", async ({ page }) => {
    test.setTimeout(90_000);
    hub = await linkHub(["--open", "--forget-clients", "--with-usb-lock-error"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const system = session.getByRole("region", { name: "System" });
    const main = system.getByRole("region", { name: "Room Main room" });
    const rear = main.getByLabel("Unit Rear L/R");
    await expect(rear).toContainText("USB-direct: Can't lock: muted");
    await expect(rear).toContainText("can't lock: the host's rate is out of range");
    await expect(rear).toContainText("lock errors 1");
    const rail = session.getByRole("complementary", { name: "Status" });
    const alarm = rail.getByRole("list", { name: "Alarm list" }).getByLabel("Alarm usb_lock_error on Rear L/R");
    await expect(alarm).toContainText("USB-direct unit Rear L/R can't lock — muted (the host's rate is out of range)");
    await expect(alarm).toHaveAttribute("data-state", "raised");
    await ausb1Shot(page, "ausb2-lock-error-muted");
    // The Installer: Rear L/R plays unaligned when it can't lock; saved on the hub, which writes it to the unit.
    await setMode(session, page, "Installer");
    await openPage(session, "Clock and Links"); // -- WP WEB7 -- Qt's tab
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    const usb = panel.getByRole("table", { name: "USB-direct units" });
    const row = usb.getByRole("row", { name: "USB-direct clock of Rear L/R" });
    await expect(row).toContainText("Can't lock: muted (the host's rate is out of range)");
    await expect(row.getByLabel("When Rear L/R can't lock")).toHaveValue("mute");
    await row.getByLabel("When Rear L/R can't lock").selectOption("play_unaligned");
    await panel.getByRole("button", { name: "Save" }).click();
    await expect(panel.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    // The hub's own poll (every 5 s) writes it: the unit then plays unaligned, and the alarm says so.
    await expect(row).toContainText("Can't lock: playing unaligned", { timeout: 20_000 });
    await expect(row).not.toContainText("(unit: Mute)");
    await openPage(session, "System"); // -- WP WEB7 -- the units are on their own tab
    await expect(rear).toContainText("USB-direct: Can't lock: playing unaligned");
    await expect(alarm).toContainText("USB-direct unit Rear L/R can't lock — playing unaligned");
    await openPage(session, "Clock and Links"); // -- WP WEB7 --
    const stats = panel.getByRole("table", { name: "USB-direct stats" });
    await expect(stats.getByRole("row", { name: "USB-direct stats of Rear L/R" })).toContainText("Can't lock: playing unaligned");
    await ausb1Shot(page, "ausb2-lock-error-playing");
  });

  test("SW3: Save never overwrites a newer installation; the page says so and offers Reload", async ({ page, browser }) => {
    // -- fixup fs1 -- two hub sessions and two Installer dialogs: Firefox on a busy host needs more than 30 s
    test.setTimeout(60_000);
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    await setMode(session, page, "Installer");
    await openPage(session, "Clock and Links"); // -- WP WEB7 -- Qt's tab
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    await panel.getByRole("table", { name: "Audio link slots" }).getByRole("button", { name: "Bar IN3 from Ch 7" }).click();
    await expect(panel.getByRole("group", { name: "Unsaved installation" })).toBeVisible();
    // Another installer changes the installation meanwhile, on another page.
    const other = await browser.newPage();
    try {
      const s2 = await openUnit(other, hub, "DEMO-ALINK-B");
      await setMode(s2, other, "Installer");
      await openPage(s2, "Clock and Links"); // -- WP WEB7 --
      const p2 = s2.getByRole("region", { name: "Clock and audio links" });
      const theirs = p2.getByRole("table", { name: "Audio link slots" }).getByRole("button", { name: "Terrace IN4 from Ch 7" });
      await theirs.click();
      await p2.getByRole("button", { name: "Save" }).click();
      await expect(p2.getByRole("group", { name: "Unsaved installation" })).toHaveCount(0);
    } finally {
      await other.close();
    }
    // At the next poll this page sees the newer revision: no Save, a plain warning and Reload.
    const stale = panel.getByRole("alert", { name: "Installation changed on the hub" });
    await expect(stale).toContainText("changed on the hub while you edited it", { timeout: 10_000 });
    await expect(panel.getByRole("button", { name: "Save" })).toHaveCount(0);
    await shot(page, "save-stale");
    await stale.getByRole("button", { name: "Reload" }).click();
    const matrix = panel.getByRole("table", { name: "Audio link slots" });
    await expect(matrix.getByRole("button", { name: "Terrace IN4 from Ch 7" })).toHaveAttribute("aria-pressed", "true");
    await expect(matrix.getByRole("button", { name: "Bar IN3 from Ch 7" })).toHaveAttribute("aria-pressed", "false");
  });

  // Review sw3x's probe, kept: the hub's role changed while the page is away (`uconsole hub
  // role` with the hub stopped), then the same hub again; the kept token reconnects as a Viewer.
  test("SW3: a Control role may mute; made a Viewer, the page offers nothing that writes", async ({ page }) => {
    test.setTimeout(120_000);
    hub = await linkHub(["--pair"], true);
    const session = await openUnit(page, hub, "DEMO-ALINK-D");
    const rail = session.getByRole("complementary", { name: "Status" });
    const mute = rail.getByRole("button", { name: "System mute" });
    await mute.click();
    await expect(mute).toContainText(": on", { timeout: 10_000 });
    const alarm = rail.getByRole("list", { name: "Alarm list" }).getByLabel("Alarm system_muted on Sender");
    await expect(alarm.getByRole("button", { name: "Acknowledge" })).toBeEnabled({ timeout: 10_000 });
    await mute.click();
    await expect(mute).toContainText(": off", { timeout: 10_000 });
    await expect(session.getByLabel("Scenes", { exact: true }).getByRole("button", { name: "Recall Evening" })).toBeEnabled();
    const { port, store } = hub;
    await stopHub(hub);
    hub = undefined;
    const listed = JSON.parse(execFileSync(process.env.DSPI_TERM!, ["hub", "clients", "--store", store, "--json"], { encoding: "utf8" }).trim().split("\n").pop()!) as {
      clients: { cid: number; role: string }[];
    };
    expect(listed.clients.map((c) => c.role)).toEqual(["control"]);
    execFileSync(process.env.DSPI_TERM!, ["hub", "role", String(listed.clients[0]!.cid), "viewer", "--store", store]);
    hub = await linkHub([], false, store, port);
    const s2 = await openUnit(page, hub, "DEMO-ALINK-D", false);
    const rail2 = s2.getByRole("complementary", { name: "Status" });
    await expect(rail2.getByLabel("Connection")).toContainText("Connected");
    const system = s2.getByRole("region", { name: "System" });
    await expect(system.getByRole("region", { name: "Room Bar" }).getByLabel("Unit Bar")).toContainText("Connected");
    await expect(rail2.getByRole("button", { name: "System mute" })).toBeDisabled();
    await expect(rail2.getByRole("button", { name: "System mute" })).toHaveAttribute("title", "This needs the hub's Control role");
    const bar2 = s2.getByLabel("Scenes", { exact: true });
    for (const name of ["Evening", "Quiet inside", "Closed", "Test click"]) {
      await expect(bar2.getByRole("button", { name: `Recall ${name}` })).toBeDisabled();
    }
    await setMode(s2, page, "Installer");
    await openPage(s2, "Clock and Links"); // -- WP WEB7 -- Qt's tab
    const panel = s2.getByRole("region", { name: "Clock and audio links" });
    await expect(panel.getByText("Changing the installation needs the hub's Admin role: this panel is read only.")).toBeVisible();
    await expect(panel.getByRole("button", { name: "Compute" })).toBeDisabled();
    await expect(panel.getByRole("button", { name: "Align audio link delays…", exact: true })).toBeDisabled();
    const cells = panel.getByRole("table", { name: "Audio link slots" }).getByRole("button");
    expect(await cells.evaluateAll((els) => els.every((e) => (e as HTMLButtonElement).disabled))).toBe(true);
    await expect(panel.getByLabel(/^Measured latency of/)).toHaveCount(0);
    // Reads stay: the status's computation and the link stats.
    await expect(panel.getByRole("row", { name: "Clock of Terrace" })).toContainText("3 frames");
    await expect(panel.getByRole("table", { name: "Audio link stats" }).getByRole("row", { name: "Audio link stats of Bar" })).toContainText("locked");
  });
});

test("SW3, WEB7: a device on its own is not on a hub (Qt's empty state); Simple has no System view", async ({ page }) => {
  await page.goto("/");
  await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session.locator(".device-meta")).toBeVisible();
  // Today's main window stays first; the System view follows.
  await expectPage(session, "Main");
  await openPage(session, "System");
  // -- WP WEB7 -- Qt's System on a device the console holds itself: one empty state on Units, Clock and Links
  // and Installation, with the way to the hubs (it was a system of one unit: Qt shows none)
  for (const tab of ["System", "Clock and Links", "Installation"]) {
    await openPage(session, tab);
    const none = session.getByRole("status").filter({ hasText: "Not on a hub" });
    await expect(none).toContainText("Open a DSPi on a hub with an audio link to see its installation here.");
    await expect(none.getByRole("button", { name: "Hubs and Their Devices" })).toBeVisible();
  }
  await session.getByRole("button", { name: "Hubs and Their Devices" }).click();
  await expectPage(session, "Connect");
  await openPage(session, "System");
  const rail = session.getByRole("complementary", { name: "Status" });
  await expect(rail.getByLabel("Alarms")).toHaveText("No hub alarms");
  await expect(rail.getByRole("button", { name: "System mute" })).toHaveCount(0);
  await expect(session.getByLabel("Scenes", { exact: true })).toHaveCount(0);
  await shot(page, "single-unit");
  await setMode(session, page, "Simple");
  expect(await hasPage(session, "System")).toBe(false);
  expect(await hasPage(session, "Main")).toBe(true);
  // Remembered for this device.
  await page.reload();
  await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  await expect(page.getByRole("article").last().getByRole("button", { name: "Mode: Simple" })).toBeVisible();
});
