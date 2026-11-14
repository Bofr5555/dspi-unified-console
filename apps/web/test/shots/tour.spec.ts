// The screenshot tour (playwright.shots.config.ts): what the console shows on the core's demo
// device and on `uconsole --demo hub --alink` (a sender, a fan-out, four units and the
// system model), dark and light, at the project's width. Only pictures: each step waits for what
// it shows, and a step that cannot show its state fails rather than taking a picture of
// something else.
//
// Environment: DSPI_TERM (the `uconsole` binary, as for the e2e), SHOTS_DIR (where the PNGs
// go), DSPI_LINK_SRC (upstream's @dspi/link source, `js/packages/dspi-link/src`) for the steps
// that make a unit lose its link and lock again through the fake device's simulation
// (`EXT_FAKE_ALINK_SIM`, a tunnelled extension write an Admin session may send).

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

type OwnHub = { port: number; pin: string; child: ChildProcess; store: string };

const REQ_EXTENSION = 0x00;
const EXT_FAKE_ALINK_SIM = 0xfff7;
const ALINK_SIM_LOSE = 0;
const ALINK_SIM_LOCK = 1;

async function linkHub(extra: string[], pairing = false, store?: string, port0 = 0): Promise<OwnHub> {
  store ??= `${process.env.TMPDIR ?? "test-results"}/shots-${process.pid}-${Math.random().toString(36).slice(2)}`;
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

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** The fake unit `serial` on an open hub plays its link: `ALINK_SIM_LOSE` or `ALINK_SIM_LOCK`. */
async function alinkSim(hub: OwnHub, serial: string, op: number): Promise<void> {
  const src = process.env.DSPI_LINK_SRC;
  if (!src) throw new Error("no DSPI_LINK_SRC: cannot play the link");
  const mod = (await import(`${src}/index.ts`)) as Record<string, unknown> & { default?: Record<string, unknown> };
  const { LinkClient } = (mod.LinkClient ? mod : mod.default!) as unknown as {
    LinkClient: new (o: { webSocket: unknown }) => {
      connect(url: string, o: { clientName: string; app: string; version: string }): Promise<void>;
      refreshDevices(): Promise<{ handle: number; serial: string }[]>;
      command(r: Record<string, unknown>): Promise<{ status: number }>;
      disconnect(): void;
    };
  };
  const c = new LinkClient({ webSocket: WebSocket });
  try {
    await c.connect(`ws://127.0.0.1:${hub.port}/dspi/v1`, { clientName: "shots", app: "shots", version: "0" });
    const dev = (await c.refreshDevices()).find((d) => d.serial === serial);
    if (!dev) throw new Error(`no ${serial} on the hub`);
    const r = await c.command({ handle: dev.handle, direction: "set", bRequest: REQ_EXTENSION, wValue: EXT_FAKE_ALINK_SIM, wIndex: 0, payload: new Uint8Array([op]) });
    if (r.status !== 0) throw new Error(`link sim ${op} on ${serial}: status ${r.status}`);
  } finally {
    c.disconnect();
  }
}

const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

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
  return session;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session.locator(".device-meta")).toBeVisible();
  return session;
}

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced" | "Installer", pictureAs?: string) {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  if (pictureAs) await shot(page, pictureAs, { full: false });
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

/** `<project>-<name>-<scheme>.png`, the whole page (or `of`, one part of it), dark then light. */
async function shot(page: Page, name: string, o: { of?: Locator; full?: boolean } = {}) {
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(200);
    const path = `${dir}/web-${project}-${name}-${scheme}.png`;
    if (o.of) await o.of.screenshot({ path });
    else await page.screenshot({ path, fullPage: o.full ?? true });
  }
  await page.emulateMedia({ colorScheme: null });
}

const tab = (session: Locator, name: string) => session.getByRole("tab", { name });

test.describe("one device: the core's demo", () => {
  test("main, EQ, matrix, a system of one unit, the three modes", async ({ page }) => {
    const session = await openDemo(page);
    await expect(session.getByRole("meter").first()).toBeVisible();
    await shot(page, "demo-main");
    await tab(session, "EQ").click();
    await expect(session.locator("canvas").first()).toBeVisible();
    await shot(page, "demo-eq");
    await tab(session, "Matrix").click();
    await shot(page, "demo-matrix");
    await tab(session, "System").click();
    await expect(session.getByRole("region", { name: "System" }).getByText("One device, shown as a single unit.")).toBeVisible();
    await shot(page, "demo-single-unit");
    // -- WP WB8 -- Undo and Redo beside the tabs, after a change; the event log and Save Diagnostics…
    await tab(session, "Main").click();
    const volume = session.getByRole("slider", { name: "Volume" });
    await volume.fill("150");
    await volume.dispatchEvent("change");
    const undo = session.getByRole("group", { name: "Undo and Redo" });
    await expect(undo.getByRole("button", { name: "Undo" })).toBeEnabled();
    await shot(page, "demo-undo", { full: false });
    await tab(session, "Events").click();
    const events = session.getByRole("region", { name: "Event log" });
    await expect(events.getByRole("row").filter({ hasText: "Connected" })).toHaveCount(1);
    await shot(page, "demo-events");
    await Promise.all([page.waitForEvent("download"), events.getByRole("button", { name: "Save Diagnostics…" }).click()]);
    await expect(events.getByRole("status").filter({ hasText: "Nothing was sent anywhere" })).toBeVisible();
    await shot(page, "demo-diagnostics-saved", { of: events });
    await setMode(session, page, "Installer", "mode-dialog");
    await shot(page, "demo-installer");
    await setMode(session, page, "Simple");
    await expect(tab(session, "System")).toHaveCount(0);
    await shot(page, "demo-simple");
  });
});

// -- WP P1WB1 -- snapshots, the A/B toggle in the scene bar, a safe apply's countdown, the import
// preview, an offline file
test.describe("snapshots and files on the core's demo", () => {
  test("snapshots, A/B, safe apply, import preview, an offline file", async ({ page }) => {
    const mac = new URL("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset", import.meta.url).pathname;
    const session = await openDemo(page);
    await tab(session, "Snapshots").click();
    const screen = session.getByRole("region", { name: "Snapshots" });
    const capture = async (name: string, note: string) => {
      await screen.getByLabel("Name").fill(name);
      await screen.getByLabel("Note").fill(note);
      await Promise.all([page.waitForEvent("download"), screen.getByRole("button", { name: "Capture" }).click()]);
    };
    await capture("Evening", "Warm, -3 dB on the tops");
    const gain = session.getByRole("region", { name: "Outputs" });
    await tab(session, "Main").click();
    await gain.getByRole("slider").first().fill("-4");
    await gain.getByRole("slider").first().dispatchEvent("change");
    await tab(session, "Snapshots").click();
    await capture("Bright", "Output 1 at -4 dB");
    await expect(screen.getByRole("list", { name: "Held snapshots" }).getByRole("listitem")).toHaveCount(2);
    await shot(page, "demo-snapshots");
    const row = (n: string) => screen.getByRole("list", { name: "Held snapshots" }).getByRole("listitem", { name: n });
    await row("Evening").getByRole("button", { name: "Compare" }).click();
    const bar = session.getByRole("group", { name: "A/B compare" });
    await bar.getByRole("button", { name: /switch/ }).click();
    await expect(bar).toContainText("Now B");
    const compare = screen.getByRole("region", { name: "A/B compare" });
    await compare.getByRole("button", { name: "Show Differences" }).click();
    await expect(compare.getByRole("table", { name: "From A to B" })).toBeVisible();
    await shot(page, "demo-ab");
    await shot(page, "demo-ab-bar", { of: bar });
    await bar.getByRole("button", { name: "Keep A" }).click();
    await expect(bar).toHaveCount(0);
    await row("Bright").getByRole("button", { name: "Recall with Rollback" }).click();
    const dialog = page.getByRole("dialog", { name: "Recall Snapshot" });
    const guard = session.getByRole("alert", { name: "Safe apply" });
    // Asked first when the settings are unsaved.
    await expect(dialog.or(guard)).toBeVisible();
    if (await dialog.isVisible()) await dialog.getByRole("button", { name: "Recall" }).click();
    await expect(guard).toContainText("reverts in");
    await shot(page, "demo-safe-apply", { full: false });
    await guard.getByRole("button", { name: "Revert" }).click();
    await expect(guard).toHaveCount(0);
    await tab(session, "Files").click();
    await session.getByRole("region", { name: "Files" }).getByLabel("Configuration file to import").setInputFiles(mac);
    const preview = session.getByRole("region", { name: "Import preview" });
    await preview.getByRole("group", { name: "What to import" }).getByRole("checkbox", { name: /^EQ/ }).uncheck();
    await expect(preview.getByRole("table", { name: "What the import changes" })).toBeVisible();
    await shot(page, "demo-import-preview");
    await preview.getByRole("button", { name: "Cancel" }).click();
    await page.getByRole("region", { name: "Files" }).first().getByLabel("File to open").setInputFiles(mac);
    const doc = page.getByRole("article").last();
    await expect(page.locator(".device-meta").filter({ hasText: "File: mac-swift-v32.dspipreset" })).toBeVisible();
    await expect(doc).toBeVisible();
    await shot(page, "offline-file");
  });
});

test.describe("an audio link installation on the hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("Admin: the System view, a scene through the sender's superframe, a refused scene, the rail", async ({ page }) => {
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    const system = session.getByRole("region", { name: "System" });
    await expect(system.getByRole("region", { name: "Room Main room" }).getByLabel("Unit Main left")).toContainText("Audio link: Locked");
    await expect(system.getByRole("list", { name: "Topology" }).getByLabel("Fan-out Fan-out")).toContainText("+3 frames");
    await shot(page, "hub-system-admin");
    const bar = session.getByLabel("Scenes", { exact: true });
    const result = session.getByLabel("Scene result");
    await bar.getByRole("button", { name: "Recall Evening" }).click();
    await expect(result).toContainText("on time on every unit");
    // At once, before Evening's frame has passed: the sender holds one scene at a time.
    await bar.getByRole("button", { name: "Recall Closed" }).click();
    await expect(result.getByRole("alert")).toContainText("Closed was refused");
    await shot(page, "hub-scene-refused");
    await shot(page, "hub-rail-scene-refused", { of: session.getByRole("complementary", { name: "Status" }) });
    // Evening again once its frame has passed: one op for every unit through the superframe.
    await page.waitForTimeout(1500);
    await bar.getByRole("button", { name: "Recall Evening" }).click();
    await expect(result).toContainText("on time on every unit");
    await expect(result.getByRole("alert")).toHaveCount(0);
    await shot(page, "hub-scene-superframe");
    await shot(page, "hub-scene-result", { of: result });
    // A zone and a room: each unit's own answer.
    await page.waitForTimeout(1500);
    await bar.getByRole("button", { name: "Recall Quiet inside" }).click();
    await expect(result.getByRole("list", { name: "Per unit" })).toBeVisible();
    await shot(page, "hub-scene-per-unit");
    // The system mute and its alarm.
    const rail = session.getByRole("complementary", { name: "Status" });
    await rail.getByRole("button", { name: "System mute" }).click();
    await expect(rail.getByRole("button", { name: "System mute" })).toContainText(": on", { timeout: 10_000 });
    await expect(rail.getByLabel("Alarms")).toHaveText("1 alarm", { timeout: 10_000 });
    await shot(page, "hub-rail-mute-alarm");
    // -- WP WB8 -- the event log: the alarm beside the unit's own records
    await tab(session, "Events").click();
    const events = session.getByRole("region", { name: "Event log" });
    await expect(events.getByRole("row").filter({ hasText: "Alarm" }).first()).toBeVisible();
    await shot(page, "hub-events");
  });

  // -- WP WB8 -- an undo another console stepped in on: left as they set it, and named
  test("undo: a value another console changed since", async ({ page, browser }) => {
    test.setTimeout(60_000);
    hub = await linkHub(["--open", "--forget-clients"]);
    const mine = await openUnit(page, hub, "DEMO-ALINK-A");
    await tab(mine, "Main").click();
    const set = async (s: Locator, pos: string) => {
      const v = s.getByRole("slider", { name: "Volume" });
      await v.fill(pos);
      await v.dispatchEvent("change");
    };
    const before = (await mine.locator(".volume-value").textContent())!;
    await set(mine, "128");
    await expect(mine.locator(".volume-value")).not.toHaveText(before);
    const mineShown = (await mine.locator(".volume-value").textContent())!;
    const other = await browser.newPage();
    try {
      const theirs = await openUnit(other, hub, "DEMO-ALINK-A");
      await tab(theirs, "Main").click();
      await set(theirs, "100");
      await expect(mine.locator(".volume-value")).not.toHaveText(mineShown, { timeout: 10_000 });
    } finally {
      await other.close();
    }
    await mine.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" }).click();
    await expect(mine.locator(".step-notice")).toContainText("since; it was left as they set it");
    await shot(page, "hub-undo-other-writer", { full: false });
  });

  test("Installer: Clock and audio links, audio link delay and hop offset, link stats, the alignment", async ({ page }) => {
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    await setMode(session, page, "Installer");
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    await expect(panel.getByRole("table", { name: "Units on the audio link" }).getByRole("row", { name: "Clock of Main right" })).toContainText("3 frames");
    await expect(panel.getByRole("table", { name: "Audio link stats" }).getByRole("row", { name: "Audio link stats of Main left" })).toContainText("locked");
    await shot(page, "hub-alink-clock");
    await shot(page, "hub-clock-units", { of: panel.getByRole("table", { name: "Units on the audio link" }) });
    await shot(page, "hub-alink-stats", { of: panel.getByRole("table", { name: "Audio link stats" }) });
    // The glossary's words, the firmware's names in a tooltip.
    await panel.getByRole("columnheader", { name: "Hop offset" }).locator("abbr").hover();
    await page.waitForTimeout(1200);
    await shot(page, "hub-clock-glossary-tooltip", { full: false });
    const align = panel.getByRole("region", { name: "Alignment" });
    await align.getByRole("button", { name: "Compute" }).click();
    await expect(align.getByLabel("Alignment result")).toContainText("Computed, not written");
    await align.getByRole("button", { name: "Align audio link delays…", exact: true }).click();
    const confirm = page.getByRole("dialog", { name: "Align audio link delays?" });
    await expect(confirm).toBeVisible();
    await shot(page, "hub-align-confirm", { full: false });
    await confirm.getByRole("button", { name: "Align" }).click();
    await expect(align.getByLabel("Alignment result")).toContainText("The audio link delays were written");
    await shot(page, "hub-alignment-written", { of: align });
    // An unsaved change to the slots.
    await panel.getByRole("table", { name: "Audio link slots" }).getByRole("button", { name: "Main right IN3 from Ch 6" }).click();
    await expect(panel.getByRole("group", { name: "Unsaved installation" })).toBeVisible();
    await shot(page, "hub-slots-unsaved", { of: panel.getByRole("table", { name: "Audio link slots" }) });
  });

  test("the audio link: a unit loses its link (holdover), then locks again", async ({ page }) => {
    test.skip(!process.env.DSPI_LINK_SRC, "no DSPI_LINK_SRC");
    hub = await linkHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, "DEMO-ALINK-A");
    await setMode(session, page, "Installer");
    const system = session.getByRole("region", { name: "System" });
    const right = system.getByRole("region", { name: "Room Main room" }).getByLabel("Unit Main right");
    await expect(right).toContainText("Audio link: Locked");
    await alinkSim(hub, "DEMO-ALINK-B", ALINK_SIM_LOSE);
    await expect(right).toContainText("Audio link: Holdover", { timeout: 15_000 });
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByLabel("Alarms")).not.toHaveText("No alarms", { timeout: 15_000 });
    await shot(page, "link-holdover");
    await shot(page, "link-holdover-rail", { of: rail });
    const stats = session.getByRole("region", { name: "Clock and audio links" }).getByRole("table", { name: "Audio link stats" });
    await expect(stats.getByRole("row", { name: "Audio link stats of Main right" })).toContainText("holdover", { timeout: 15_000 });
    await shot(page, "link-holdover-stats", { of: stats });
    await alinkSim(hub, "DEMO-ALINK-B", ALINK_SIM_LOCK);
    await expect(right).toContainText("Audio link: Locked", { timeout: 15_000 });
    await expect(stats.getByRole("row", { name: "Audio link stats of Main right" })).toContainText("locked", { timeout: 15_000 });
    await page.waitForTimeout(3500);
    await shot(page, "link-relocked");
    await shot(page, "link-relocked-stats", { of: stats });
    await shot(page, "link-relocked-rail", { of: rail });
  });

  test("Control: the installer panel read only, an admin-only scene refused", async ({ page }) => {
    hub = await linkHub(["--pair"], true);
    const session = await openUnit(page, hub, "DEMO-ALINK-C");
    await setMode(session, page, "Installer");
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    await expect(panel.getByText("this panel is read only")).toBeVisible();
    await session.getByLabel("Scenes", { exact: true }).getByRole("button", { name: "Recall Test click" }).click();
    await expect(session.getByLabel("Scene result").getByRole("alert")).toContainText("Test click: refused");
    await shot(page, "role-control");
  });

  test("Viewer: nothing that writes", async ({ page }) => {
    hub = await linkHub(["--pair"], true);
    await openUnit(page, hub, "DEMO-ALINK-D");
    const { port, store } = hub;
    await stopHub(hub);
    hub = undefined;
    const listed = JSON.parse(execFileSync(process.env.DSPI_TERM!, ["hub", "clients", "--store", store, "--json"], { encoding: "utf8" }).trim().split("\n").pop()!) as {
      clients: { cid: number }[];
    };
    execFileSync(process.env.DSPI_TERM!, ["hub", "role", String(listed.clients[0]!.cid), "viewer", "--store", store]);
    hub = await linkHub([], false, store, port);
    const session = await openUnit(page, hub, "DEMO-ALINK-D", false);
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByRole("button", { name: "System mute" })).toBeDisabled();
    await shot(page, "role-viewer");
    await setMode(session, page, "Installer");
    await expect(session.getByRole("region", { name: "Clock and audio links" }).getByRole("button", { name: "Compute" })).toBeDisabled();
    await shot(page, "role-viewer-installer");
  });
});
