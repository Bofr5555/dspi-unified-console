// WP P3WGR e2e: the hub screen's groups on the console served by a real hub (`uconsole --demo
// hub --alink --web`, DSPI_TERM, whose simulated installation gives several demo DSPis): an
// Admin makes a group of two demo DSPis and a DSPi the hub saw before (offline now); a volume
// with an offset on one member lands on both online ones, each row with the value it got, and
// the offline member's row says so (kept until dismissed); a guarded bulk apply of a hub
// snapshot is kept with one Keep, another reverted with one Revert Now (both devices' rows say
// so); the fleet history shows the writes with the client's name; the health board filters by
// the group. A Control client (PIN pairing) sees the group read-only, with the actions. Chromium,
// Firefox and 375 px with no sideways scroll.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

const ONE = "DEMO-ALINK-A";
const TWO = "DEMO-ALINK-D";
const AWAY = "DEMO-OFFLINE-0042";
const BY = /^(Session \d+|uConsole for DSPi \(web\))$/;

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
  const wide = over > 0
    ? await page.evaluate(() => {
        const w = document.documentElement.clientWidth;
        return [...document.querySelectorAll("body *")]
          .filter((e) => e.getBoundingClientRect().right > w + 1 && !(e.parentElement && e.parentElement.getBoundingClientRect().right > w + 1))
          .slice(0, 5)
          .map((e) => `${e.tagName.toLowerCase()}.${(e as HTMLElement).className}`);
      })
    : [];
  expect(over, wide.join(", ")).toBeLessThanOrEqual(0);
}

type OwnHub = { port: number; child: ChildProcess; pin: string };

/** A store the hub starts from: a DSPi it saw before (offline now) and, for Control, a group. */
function seededStore(withGroup: boolean): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/p3wgr-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  const seen = Math.floor(Date.parse("2026-11-10T09:30:00Z") / 1000);
  writeFileSync(
    `${store}/inventory.json`,
    JSON.stringify({
      v: 1,
      devices: [{ serial: AWAY, name: "Bar", platform: 1, fw: "1.1.6", outputs: 9, location: "Bar", first_seen_s: seen - 86400, last_seen_s: seen }],
    }),
  );
  if (withGroup) {
    writeFileSync(`${store}/groups.json`, JSON.stringify({ v: 1, next_id: 2, groups: [{ id: "g1", name: "Front", note: "The front of house", members: [ONE, TWO, AWAY] }] }));
  }
  return store;
}

/** `--open` (every session an Admin) or `--pair` (a PIN pairs, as Control). */
async function startHub(store: string, access: "--open" | "--pair"): Promise<OwnHub> {
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--alink", "--console", "dist", access, "--forget-clients", "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
      if (e.event === "status" && e.port) {
        port = e.port;
        if (access === "--open") ok("");
      }
      if (e.event === "pairing" && e.pin) ok(e.pin);
    });
  });
  return { port, child, pin };
}

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** Connect the serving hub (pairing with its PIN when it asks), open `serial`, Advanced mode. */
async function openUnit(page: Page, hub: OwnHub, serial: string): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (hub.pin) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(serial));
  const rail = session.getByRole("complementary", { name: "Status" });
  await expect(rail.getByLabel("Connection")).toHaveText(/^Connected/);
  await expect(session.locator(".banner")).toHaveCount(0);
  await setMode(session, page, "Advanced");
  return session;
}

const line = (panel: Locator, serial: string) => panel.locator(`li.hub-result[data-serial="${serial}"]`);

if (!process.env.DSPI_TERM) {
  test("P3WGR: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P3WGR on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("P3WGR Admin: a group made, a volume with an offset on two DSPis, a guarded bulk apply kept and another reverted, the fleet history with who did it", async ({ page }) => {
    test.setTimeout(240_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore(false), "--open");
    const session = await openUnit(page, hub, ONE);
    await openPage(session, "Hub");
    const groups = session.getByRole("region", { name: "Groups" });
    await expect(groups).toContainText("The hub has no groups yet.");

    // A group: two demo DSPis and the offline one, picked from the inventory.
    await groups.getByRole("button", { name: "New Group…" }).click();
    const form = groups.getByRole("form", { name: "New group" });
    await form.getByLabel("Name").fill("Front");
    await form.getByLabel("Note").fill("The front of house");
    for (const s of [ONE, TWO, AWAY]) await form.getByRole("checkbox", { name: new RegExp(s) }).check();
    await noSidewaysScroll(page);
    await form.getByRole("button", { name: "Save" }).click();
    await expect(groups.getByRole("status", { name: "Group result" })).toContainText("Made the group Front.");
    const front = groups.getByRole("group", { name: "Pick a group" }).getByRole("button", { name: /Front/ });
    await expect(front).toHaveAttribute("aria-pressed", "true");
    const details = groups.locator(".hub-group");
    await expect(details.locator(`li[data-serial="${AWAY}"]`)).toContainText("Offline");
    await expect(details.locator(`li[data-serial="${ONE}"]`)).toContainText("Online");
    await expect(details.locator(`li[data-serial="${TWO}"]`)).toContainText("Online");

    // The health board filters by the group: the offline member listed below.
    const board = session.getByRole("region", { name: "Health board" });
    await board.getByRole("combobox", { name: "Group", exact: true }).selectOption({ label: "Front" });
    await expect(board.locator("tr.hub-health-row")).toHaveCount(3);
    await board.getByRole("combobox", { name: "Group", exact: true }).selectOption({ label: "Every DSPi" });

    // A volume with an offset on the second DSPi: what each will get, then each row as the hub answered it.
    const actions = groups.locator(".hub-group-actions");
    await actions.getByLabel("Volume (dB)").fill("-20");
    const offsets = actions.locator("table.hub-offsets");
    await offsets.locator(`tr[data-serial="${TWO}"] input.hub-offset`).fill("-3");
    await expect(offsets.locator(`tr[data-serial="${ONE}"] .hub-will-get`)).toHaveText("-20.0 dB");
    await expect(offsets.locator(`tr[data-serial="${TWO}"] .hub-will-get`)).toHaveText("-23.0 dB");
    await noSidewaysScroll(page);
    await actions.getByRole("button", { name: "Set Volume" }).click();
    const result = actions.getByRole("status", { name: "Group action result" });
    await expect(line(result, ONE)).toContainText("-20.0 dB", { timeout: 30_000 });
    await expect(line(result, TWO)).toContainText("-23.0 dB");
    await expect(line(result, AWAY)).toHaveClass(/warn/);
    await expect(line(result, AWAY)).toHaveAttribute("data-key", "dlink.offline");
    await expect(result).toContainText("1 of 3 did not");
    await noSidewaysScroll(page);
    // Kept until dismissed.
    await actions.getByRole("button", { name: "Read Volume" }).click();
    await expect(line(result, TWO)).toContainText("-23.0 dB", { timeout: 30_000 });
    await result.getByRole("button", { name: "Dismiss" }).click();
    await expect(result).toHaveCount(0);

    // A snapshot of the open DSPi on the hub, for the bulk apply.
    await openPage(session, "Snapshots");
    const onHub = session.getByRole("region", { name: "On the hub" });
    await onHub.getByRole("button", { name: "Save to the Hub" }).click();
    await expect(onHub.getByRole("status")).toContainText("to the hub");
    await openPage(session, "Hub");

    // A guarded bulk apply, kept with one Keep.
    const bulk = groups.locator(".hub-bulk");
    await expect(bulk.getByLabel("Snapshot on the hub").locator("option")).not.toHaveCount(0, { timeout: 20_000 });
    await expect(bulk.getByLabel("Revert unless kept")).toBeChecked();
    await bulk.getByLabel("Keep within (seconds)").fill("120");
    await bulk.getByRole("button", { name: "Apply to the Group" }).click();
    const bar = bulk.getByRole("alert", { name: "Keep the group's changes" });
    await expect(bar).toContainText("Keep the changes on 2 DSPis?", { timeout: 60_000 });
    await expect(bar.locator(".hub-countdown")).toHaveText(/^[0-2]:\d\d$/);
    const applied = bulk.getByRole("status", { name: "Bulk apply result" });
    await expect(line(applied, ONE)).toContainText("Applied, waiting for Keep");
    await expect(line(applied, TWO)).toContainText("Applied, waiting for Keep");
    await expect(line(applied, AWAY)).toHaveAttribute("data-key", "dlink.offline");
    await noSidewaysScroll(page);
    await bar.getByRole("button", { name: "Keep" }).click();
    await expect(bar).toHaveCount(0);
    await expect(applied).toContainText("Kept the changes on Front");
    await expect(line(applied, ONE)).toContainText("Kept");
    await expect(line(applied, TWO)).toContainText("Kept");

    // Another, reverted with one Revert Now: both DSPis revert.
    await bulk.getByRole("button", { name: "Apply to the Group" }).click();
    await expect(bar).toContainText("Keep the changes on 2 DSPis?", { timeout: 60_000 });
    await bar.getByRole("button", { name: "Revert Now" }).click();
    await expect(bar).toHaveCount(0);
    await expect(applied).toContainText("Reverted the changes on Front");
    await expect(line(applied, ONE)).toContainText("Reverted");
    await expect(line(applied, TWO)).toContainText("Reverted");
    await applied.getByRole("button", { name: "Dismiss" }).click();

    // The fleet history: the writes on both DSPis, with the client's name.
    const history = groups.locator(".hub-fleet-history");
    await history.getByRole("button", { name: "Refresh" }).click();
    for (const s of [ONE, TWO]) {
      const rows = history.locator(`tr[data-serial="${s}"]`);
      await expect(rows.first()).toBeVisible({ timeout: 20_000 });
      const bys = await rows.locator(".event-by").allTextContents();
      expect(bys.some((b) => BY.test(b)), bys.join(" | ")).toBe(true);
    }
    await noSidewaysScroll(page);
    expect(errors).toEqual([]);
  });

  test("P3WGR Control: the group read-only, its actions offered", async ({ page }) => {
    test.setTimeout(120_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore(true), "--pair");
    const session = await openUnit(page, hub, ONE);
    await openPage(session, "Hub");
    await expect(session.getByRole("region", { name: "Devices on the hub" }).locator(".hub-role-tag")).toHaveText("Control");
    const groups = session.getByRole("region", { name: "Groups" });
    const details = groups.locator(".hub-group");
    await expect(details.getByRole("heading", { name: "Front" })).toBeVisible();
    await expect(details).toContainText("The front of house");
    await expect(details).toContainText("Groups are made and changed by an Admin of the hub.");
    await expect(details.locator(`li[data-serial="${AWAY}"]`)).toContainText("Offline");
    // No editor: no New, Edit or Delete, no form.
    await expect(groups.getByRole("button", { name: /^(New Group…|Edit…|Delete…)$/ })).toHaveCount(0);
    await expect(groups.getByRole("form")).toHaveCount(0);
    // The actions are a Control's.
    await expect(groups.locator(".hub-group-actions").getByRole("button", { name: "Set Volume" })).toBeEnabled();
    await expect(groups.locator(".hub-bulk")).toBeVisible();
    await noSidewaysScroll(page);
    expect(errors).toEqual([]);
  });
});
