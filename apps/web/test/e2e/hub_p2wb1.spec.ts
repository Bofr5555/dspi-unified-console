// WP P2WB1 e2e: the console served by a real hub (`uconsole --demo hub --web`, DSPI_TERM)
// with the hub's own families: the page lists the hub's inventory as an installation (the
// demo DSPi online, a DSPi the hub saw before offline, listed and not offered); the hub
// screen's device table with the offline DSPi and Identify, which answers; an alarm raised
// by the hub (the test's rule: temperature at or above 0 °C) on the board and in the banner,
// acknowledged; the events view's "On the hub" source with the hub's records and who caused
// them (`by`). Chromium, Firefox and 375 px with no sideways scroll.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { fleetOf, hubDetails, openHubPart, openPage, partsOf } from "../nav"; // -- WP UB6 -- the Fleet, the hub page's parts

const DEMO = "DEMO0000DSPI0001";
const AWAY = "DEMO-OFFLINE-0042";
const WEB_CLIENT = "uConsole for DSPi (web)";

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
  // What sticks out, to say where when it fails.
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

type OwnHub = { port: number; child: ChildProcess };

/** A store the hub starts from: a DSPi it saw before (offline now) and an alarm rule the demo trips. */
function seededStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/p2wb1-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  const seen = Math.floor(Date.parse("2026-11-10T09:30:00Z") / 1000);
  writeFileSync(
    `${store}/inventory.json`,
    JSON.stringify({
      v: 1,
      devices: [{ serial: AWAY, name: "Bar", platform: 1, fw: "1.1.6", outputs: 9, location: "Bar", first_seen_s: seen - 86400, last_seen_s: seen }],
    }),
  );
  // The demo reads 35 °C: at or above 0 °C raises `temperature`.
  writeFileSync(`${store}/alarms.json`, JSON.stringify({ v: 1, rules: { temperature: { on: true, above_c: 0 } } }));
  return store;
}

/** `--open` (every session an Admin) or `--pair` (a PIN pairs, as Control). */
async function startHub(store: string, access: "--open" | "--pair" = "--open"): Promise<OwnHub & { pin: string }> {
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", access, "--forget-clients", "--port", "0", "--json", "--no-keep-awake", "--store", store],
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

if (!process.env.DSPI_TERM) {
  test("P2WB1: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P2WB1 on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("P2WB1: the hub's devices (an offline one listed), Identify, an alarm acknowledged, the hub's history with who did it", async ({ page }) => {
    test.setTimeout(120_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore());
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);

    // -- WP UB6 -- (UXP B6, D13) the page starts on the hub's Fleet: the offline DSPi listed, not offered
    const fleet = fleetOf(page);
    await expect(fleet.getByRole("row", { name: "Bar" })).toContainText("Offline");
    await expect(fleet.getByRole("row", { name: "Bar" }).getByRole("button")).toHaveCount(0);
    // The picker: the hub's inventory as an installation of its own; the offline DSPi listed, not offered.
    await page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "This Computer" }).click();
    const picker = page.getByRole("navigation", { name: "DSPi devices" });
    const away = picker.getByRole("listitem").filter({ hasText: "Bar · offline" });
    await expect(away).toContainText("Not attached to the hub");
    await expect(away.getByRole("button")).toHaveCount(0);
    // -- WP UB6 -- the hub listed once: its own entry (Link's choose step) left out beside its DSPis
    await expect(picker.getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` })).toHaveCount(0);
    await noSidewaysScroll(page);

    // Open the demo DSPi from the inventory: no choose step.
    await picker.getByRole("listitem").filter({ hasText: "· online" }).getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));

    // The hub screen, in Advanced (monitoring, as the event log).
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub");
    const devices = session.getByRole("region", { name: "Devices on the hub" });
    const awayRow = devices.getByRole("row", { name: "Bar" });
    await expect(awayRow).toContainText("Offline");
    await expect(awayRow).toContainText(AWAY);
    // -- WP P2WB3 -- an offline DSPi is not identified or opened; an Admin edits or forgets it (-- WP UB6 -- in its details)
    await expect(awayRow.getByRole("button", { name: /^(Identify|Open)$/ })).toHaveCount(0);
    const awayBody = await hubDetails(devices, "Bar");
    await expect(awayBody.getByRole("button", { name: /^(Edit the details of|Forget) Bar$/ })).toHaveCount(2);
    const own = devices.getByRole("row").filter({ hasText: DEMO });
    await expect(own).toContainText("Online (open here)");
    await expect(own).toContainText("1.1.6");
    // Health from the hub's poll: the demo's temperature.
    await expect(own).toContainText("35.0 °C", { timeout: 20_000 });
    await expect(devices.locator(".hub-role-tag")).toHaveText("Admin");

    // Identify answers: the blink, or (firmware without EXT_IDENTIFY, as the demo) the hub's refusal in the core's words.
    await own.getByRole("button", { name: "Identify" }).click();
    const said = devices.getByRole("status", { name: "Action result" });
    await expect(said).toBeVisible();
    await expect(said).not.toBeEmpty();

    // An Admin sees the hub's statistics (-- WP UB6 -- the Hub part).
    await openHubPart(session, "Hub");
    await expect(session.getByRole("region", { name: "Hub statistics" })).toContainText(/session/);

    // The alarm the test's rule raises: on the board, in the banner; acknowledged. -- WP UB6 -- the banner's Show
    // Alarms opens the Alarms part
    const banner0 = session.getByRole("status", { name: "Hub alarms" });
    await expect(banner0).toContainText("alarm", { timeout: 30_000 });
    await banner0.getByRole("button", { name: "Show Alarms" }).click();
    await expect(partsOf(session, "Hub").getByRole("tab", { name: "Alarms", exact: true })).toHaveAttribute("aria-selected", "true");
    const board = session.getByRole("region", { name: "Alarms" });
    const hot = board.getByRole("listitem").filter({ hasText: "Running hot" });
    await expect(hot).toHaveCount(1, { timeout: 30_000 });
    await expect(hot).toHaveAttribute("data-key", "alarm.temperature");
    await expect(hot).toContainText("Raised");
    const banner = session.getByRole("status", { name: "Hub alarms" });
    await expect(banner).toContainText("alarm");
    // -- WP WEB7 -- (REVE D5) the status rail counts what the banner counts, never "No hub alarms" under it
    const railAlarms = session.getByRole("complementary", { name: "Status" }).getByLabel("Alarms");
    await expect(railAlarms).toHaveText(/^[1-9]\d* hub alarms?$/);
    await noSidewaysScroll(page);
    await hot.getByRole("button", { name: "Acknowledge" }).click();
    await expect(hot).toContainText("Acknowledged");
    // By the session's name at the hub: its client name, or "Session <n>" on an open hub.
    await expect(hot).toContainText(/acknowledged by \S/);
    const who = /acknowledged by (.+)$/.exec((await hot.locator(".hub-alarm-time").textContent()) ?? "")?.[1] ?? "";
    expect(who).toMatch(new RegExp(`^(Session \\d+|${WEB_CLIENT.replace(/[()]/g, "\\$&")})$`));
    await expect(hot.getByRole("button", { name: "Acknowledge" })).toHaveCount(0);
    await expect(banner).toHaveCount(0);
    await expect(railAlarms).toHaveText("No hub alarms"); // -- WP WEB7 --

    // The events view: "On the hub", the hub's records of this DSPi with who did it.
    await openPage(session, "Events");
    const log = session.getByRole("region", { name: "Event log" });
    await log.getByLabel("Source").selectOption("hub");
    const acked = log.locator(".hub-event-list tbody tr").filter({ hasText: "Alarm acknowledged" });
    await expect(acked.first()).toBeVisible({ timeout: 20_000 });
    await expect(acked.first().locator(".event-by")).toHaveText(who);
    await expect(log.locator(".hub-event-list tbody tr").filter({ hasText: "Alarm raised" }).first()).toBeVisible();
    await noSidewaysScroll(page);
    // Back to this console's own log.
    await log.getByLabel("Source").selectOption("console");
    await expect(log.locator(".hub-event-list")).toHaveCount(0);
    expect(errors).toEqual([]);
  });

  test("P2WB1 Control: paired with the hub's PIN: no statistics, alarms acknowledged; the inventory once paired", async ({ page }) => {
    test.setTimeout(120_000);
    const errors = errorsOf(page);
    const h = await startHub(seededStore(), "--pair");
    hub = h;
    await page.goto(`http://127.0.0.1:${h.port}/console/`);
    const picker = page.getByRole("navigation", { name: "DSPi devices" });
    const hubRow = picker.getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${h.port}` });
    await expect(hubRow).toHaveCount(1);
    // A hub that wants its PIN first lists no inventory of its own: the hub's entry pairs.
    await expect(picker.getByRole("listitem").filter({ hasText: "· online" })).toHaveCount(0);
    await hubRow.getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await session.getByLabel("PIN").fill(h.pin);
    await session.getByRole("button", { name: "Pair" }).click();
    await session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) }).getByRole("button", { name: "Open" }).click();
    await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub");
    const devices = session.getByRole("region", { name: "Devices on the hub" });
    await expect(devices.locator(".hub-role-tag")).toHaveText("Control");
    await expect(devices.getByRole("row", { name: "Bar" })).toContainText("Offline");
    await openHubPart(session, "Hub"); // -- WP UB6 --
    await expect(session.getByRole("region", { name: "Hub statistics" })).toHaveCount(0);
    await openHubPart(session, "Alarms"); // -- WP UB6 --
    const hot = session.getByRole("region", { name: "Alarms" }).getByRole("listitem").filter({ hasText: "Running hot" });
    await expect(hot).toHaveCount(1, { timeout: 30_000 });
    await hot.getByRole("button", { name: "Acknowledge" }).click();
    await expect(hot).toContainText("Acknowledged");
    await noSidewaysScroll(page);

    // Paired now: the page asks the hub with its token, and lists the hub's inventory (-- WP UB6 -- the Fleet first).
    await page.reload();
    await expect(fleetOf(page).getByRole("row", { name: "Bar" })).toContainText("Offline");
    await page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "This Computer" }).click();
    await expect(picker.getByRole("listitem").filter({ hasText: "Bar · offline" })).toContainText("Not attached to the hub");
    await expect(picker.getByRole("listitem").filter({ hasText: "· online" }).getByRole("button", { name: "Connect" })).toBeVisible();
    expect(errors).toEqual([]);
  });
});
