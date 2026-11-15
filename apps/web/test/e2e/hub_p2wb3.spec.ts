// WP P2WB3 e2e: the console served by a real hub (`uconsole --demo hub --web`, DSPI_TERM). An
// Admin's hub screen: Identify blinks the demo DSPi (any device through hubIdentifySerial); the
// details of a DSPi edited (location, note, tags); the hub renamed; an offline DSPi forgotten after
// the confirmation. A second console on the same hub: its Identify reaches the first console's
// "On the hub" history as the hub sends it (`event.added`), its snapshot saved to the hub reaches
// the first console's "On the hub" list (`snapshot.changed`); recalled on the hub with the hub's
// guard, the countdown bar's Revert puts it back. The MQTT wording. Chromium, Firefox and 375 px.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

const DEMO = "DEMO0000DSPI0001";
const AWAY = "DEMO-OFFLINE-0042";

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

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

type OwnHub = { port: number; child: ChildProcess };

/** A store the hub starts from: a DSPi it saw before, offline now. */
function seededStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/p2wb3-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  const seen = Math.floor(Date.parse("2026-11-10T09:30:00Z") / 1000);
  writeFileSync(
    `${store}/inventory.json`,
    JSON.stringify({
      v: 1,
      devices: [{ serial: AWAY, name: "Bar", platform: 1, fw: "1.1.6", outputs: 9, location: "Bar", first_seen_s: seen - 86400, last_seen_s: seen }],
    }),
  );
  return store;
}

async function startHub(store: string): Promise<OwnHub> {
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

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** The demo DSPi opened from the hub's inventory, in Advanced. */
async function openDemo(page: Page, hub: OwnHub): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "· online" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session.locator(".device-meta")).toContainText(DEMO);
  await setMode(session, page, "Advanced");
  return session;
}

const tab = (session: Locator, name: string) => session.getByRole("tab", { name, exact: true });

if (!process.env.DSPI_TERM) {
  test("P2WB3: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P2WB3 on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("P2WB3 Admin: Identify blinks, a DSPi's details edited, the hub renamed, an offline DSPi forgotten", async ({ page }) => {
    test.setTimeout(120_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore());
    const session = await openDemo(page, hub);
    await tab(session, "Hub").click();
    const devices = session.getByRole("region", { name: "Devices on the hub" });
    const own = devices.getByRole("row").filter({ hasText: DEMO });
    await expect(own).toContainText("Online (open here)");

    // Identify: the demo blinks now (hubIdentifySerial).
    await own.getByRole("button", { name: "Identify" }).click();
    await expect(devices.getByRole("status", { name: "Action result" })).toContainText(/blinking for \d+ s\./);
    // An offline row: no Identify, an Admin's Edit and Forget.
    const away = devices.getByRole("row", { name: "Bar" });
    await expect(away.getByRole("button", { name: "Identify" })).toHaveCount(0);
    await expect(own.getByRole("button", { name: /^Forget/ })).toHaveCount(0);

    // The details: location, note, tags, saved at the hub and shown from its answer.
    await away.getByRole("button", { name: "Edit the details of Bar" }).click();
    const form = devices.getByRole("form", { name: "Details of Bar" });
    await form.getByLabel("Location").fill("Back bar");
    await form.getByLabel("Note").fill("Spare unit");
    await form.getByLabel("Tags (separated by commas)").fill("spare, bar ,spare");
    await noSidewaysScroll(page);
    await form.getByRole("button", { name: "Save" }).click();
    await expect(form).toHaveCount(0);
    await expect(away).toContainText("Back bar");
    await expect(away.locator(".hub-tags")).toHaveText("spare, bar");
    await expect(away.locator(".hub-note")).toHaveText("Spare unit");

    // Rename the hub.
    const own2 = session.getByRole("region", { name: "The hub" });
    await own2.getByRole("button", { name: "Rename the Hub…" }).click();
    await own2.getByLabel("New name").fill("Studio hub");
    await own2.getByRole("button", { name: "Rename", exact: true }).click();
    await expect(own2.getByRole("status", { name: "Rename" })).toContainText("Studio hub");
    await expect(own2.locator(".hub-own-name")).toHaveText("Studio hub");
    // The hub says so to every new session: /dspi/v1/info.
    const info = await page.evaluate(async (port) => (await fetch(`http://127.0.0.1:${port}/dspi/v1/info`)).json(), hub.port);
    expect(JSON.stringify(info)).toContain("Studio hub");

    // Forget the offline DSPi: asked first; Cancel keeps it.
    await away.getByRole("button", { name: "Forget Bar" }).click();
    const ask = page.getByRole("dialog", { name: "Forget the DSPi" });
    await expect(ask).toContainText("snapshots");
    await ask.getByRole("button", { name: "Cancel" }).click();
    await expect(away).toHaveCount(1);
    await away.getByRole("button", { name: "Forget Bar" }).click();
    await page.getByRole("dialog", { name: "Forget the DSPi" }).getByRole("button", { name: "Forget and Delete Its Snapshots" }).click();
    await expect(devices.getByRole("status", { name: "Action result" })).toContainText("Forgot Bar and its snapshots");
    await expect(devices.getByRole("row", { name: "Bar" })).toHaveCount(0);
    await noSidewaysScroll(page);

    // The MQTT wording.
    await tab(session, "Hub Settings").click();
    await expect(session.getByRole("region", { name: "MQTT and Home Assistant" })).toContainText("a broker not on the hub's machine");
    expect(errors).toEqual([]);
  });

  test("P2WB3 two consoles: the hub's history and snapshots follow the hub's events; a guarded recall on the hub reverted", async ({ page }) => {
    test.setTimeout(150_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore());
    const first = await openDemo(page, hub);
    const other = await page.context().newPage();
    const second = await openDemo(other, hub);

    // The first console shows the hub's history; the second identifies: the record comes as the hub sends it.
    await tab(first, "Events").click();
    const log = first.getByRole("region", { name: "Event log" });
    await log.getByLabel("Source").selectOption("hub");
    await expect(log.locator(".hub-history")).toBeVisible();
    await tab(second, "Hub").click();
    await second.getByRole("region", { name: "Devices on the hub" }).getByRole("row").filter({ hasText: DEMO }).getByRole("button", { name: "Identify" }).click();
    await expect(log.locator(".hub-event-list tbody tr").filter({ hasText: "Identify: blink" }).first()).toBeVisible({ timeout: 15_000 });

    // The first console's "On the hub" list; the second saves to the hub: it is listed here without a reload.
    await tab(first, "Snapshots").click();
    const onHub = first.getByRole("region", { name: "On the hub" });
    await expect(onHub).toContainText("No snapshots on the hub");
    await tab(second, "Snapshots").click();
    const theirs = second.getByRole("region", { name: "On the hub" });
    await theirs.getByRole("button", { name: "Save to the Hub" }).click();
    await expect(theirs.getByRole("status")).toContainText("to the hub");
    const snap = onHub.getByRole("list", { name: "Snapshots on the hub" }).getByRole("listitem").first();
    await expect(snap).toHaveAttribute("data-where", "hub", { timeout: 15_000 });
    await other.close();

    // Recall on the hub: guarded by the hub (the core adopted its guard); the bar's Revert has the hub put it back.
    await snap.getByRole("button", { name: "Recall" }).click();
    const ask = page.getByRole("dialog", { name: "Recall Snapshot" });
    const bar = first.getByRole("alert", { name: "Safe apply" });
    await expect(bar.or(ask)).toBeVisible();
    if (await ask.isVisible()) await ask.getByRole("button", { name: "Recall" }).click();
    await expect(bar.getByRole("button", { name: "Keep" })).toBeVisible();
    await noSidewaysScroll(page);
    await bar.getByRole("button", { name: "Revert" }).click();
    const result = first.getByRole("status", { name: "Result" });
    await expect(result).toBeVisible();
    await expect(bar.getByRole("button", { name: "Keep" })).toHaveCount(0);
    // The hub's history has the recall and its revert.
    await tab(first, "Events").click();
    await log.getByLabel("Source").selectOption("hub");
    await expect(log.locator(".hub-event-list tbody tr").filter({ hasText: /Recall/i }).first()).toBeVisible({ timeout: 15_000 });
    expect(errors).toEqual([]);
  });
});
