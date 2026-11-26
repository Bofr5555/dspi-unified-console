// WP UB6 e2e (UXP B6, owner D13): a console served by a real hub (`uconsole --demo hub --web --alink`) starts on
// the hub's Fleet with no DSPi open: every DSPi the hub has seen (an offline one listed, not offered), the counts and
// the alarms to acknowledge, the strip saying what the hub shares (never "connect one over USB"); Open opens the DSPi
// in a tab; the hub listed once on This Computer. Then the hub page in parts (Devices · Alarms · … · Hub), one table
// whose details open under the row, the banner's Show Alarms opening the Alarms part; Hub Settings' one unsaved bar.
// Chromium, Firefox and 375 px with no sideways scroll. The pictures (dark and light) go to ub6-shots/ in the results.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { fleetOf, hubDetails, openHubPart, openPage, partsOf } from "../nav";

const AWAY = "DEMO-OFFLINE-0042";

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
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

/** The page in both schemes, for the report (Firefox draws the same page: Chromium's are kept). */
async function shots(page: Page, name: string) {
  if (test.info().project.name === "firefox") return;
  const dir = `${process.env.PW_OUTPUT_DIR ?? "test-results"}/ub6-shots`;
  mkdirSync(dir, { recursive: true });
  const size = test.info().project.name === "narrow" ? "phone" : "desktop";
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/${name}-${size}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: "dark" });
}

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced" | "Installer") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

type OwnHub = { port: number; child: ChildProcess };

/** A store the hub starts from: a DSPi it saw before (offline now) and an alarm rule the demo trips. */
function seededStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/ub6-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  const seen = Math.floor(Date.parse("2026-11-20T09:30:00Z") / 1000);
  writeFileSync(
    `${store}/inventory.json`,
    JSON.stringify({
      v: 1,
      devices: [{ serial: AWAY, name: "Bar", platform: 1, fw: "1.1.6", outputs: 9, location: "Bar", tags: ["bar"], first_seen_s: seen - 86400, last_seen_s: seen }],
    }),
  );
  writeFileSync(`${store}/alarms.json`, JSON.stringify({ v: 1, rules: { temperature: { on: true, above_c: 0 } } }));
  return store;
}

async function startHub(store: string): Promise<OwnHub> {
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

if (!process.env.DSPI_TERM) {
  test("UB6: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("UB6 on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("UB6 D13: the page starts on the Fleet with no DSPi open; Open opens one; the hub listed once", async ({ page }) => {
    test.setTimeout(150_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore());
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);

    // The Fleet, Devices' first page: every DSPi the hub has seen, the offline one listed and not offered.
    const fleet = fleetOf(page);
    const rows = fleet.locator("tbody tr");
    await expect(rows.first()).toBeVisible({ timeout: 30_000 });
    await expect(page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "Fleet" })).toHaveAttribute("aria-selected", "true");
    const away = fleet.getByRole("row", { name: "Bar" });
    await expect(away).toContainText("Offline");
    await expect(away.getByRole("button")).toHaveCount(0);
    const online = fleet.getByRole("button", { name: /^Open\b/ });
    await expect.poll(() => online.count(), { timeout: 20_000 }).toBeGreaterThanOrEqual(4);
    const n = await online.count();
    await expect(rows).toHaveCount(n + 1);
    // The counts, and the alarms the rule raises (the demo reads 35 °C) to acknowledge.
    await expect(fleet.locator(".fleet-online")).toHaveText(`${n} online`);
    await expect(fleet.locator(".fleet-offline")).toHaveText("1 offline");
    await expect(fleet.getByLabel("Alarms to acknowledge")).toHaveText(/^[1-9]\d* alarms? to acknowledge$/, { timeout: 30_000 });
    await expect(fleet.locator(".fleet-alarm-count").first()).toHaveText(/^[1-9]\d* alarms?$/);
    // The strip says what the hub shares, with the way to it; never "connect one over USB".
    const strip = page.locator(".no-device-strip");
    await expect(strip).toHaveText(new RegExp(`^This hub shares ${n} DSPis\\.\\s*Open…$`));
    await expect(page.getByText("Connect one over USB")).toHaveCount(0);
    await noSidewaysScroll(page);
    await shots(page, "fleet");

    // This Computer: the hub's DSPis once (its inventory); its own entry, the same hub again, is not listed.
    await page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "This Computer" }).click();
    const picker = page.getByRole("navigation", { name: "DSPi devices" });
    await expect(picker.getByRole("listitem").filter({ hasText: "Bar · offline" })).toHaveCount(1);
    await expect(picker.getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` })).toHaveCount(0);
    await noSidewaysScroll(page);
    await shots(page, "this-computer");
    // The strip's Open… is the way back to the Fleet.
    await strip.getByRole("button", { name: "Open…" }).click();
    await expect(page.getByRole("tablist", { name: "Pages" }).getByRole("tab", { name: "Fleet" })).toHaveAttribute("aria-selected", "true");

    // Open: the DSPi in a tab of its own, with no choose step.
    const first = online.first();
    const name = ((await first.getAttribute("aria-label")) ?? "").replace(/^Open /, "");
    await first.click();
    const session = page.getByRole("article").last();
    await expect(session).toHaveAttribute("data-serial", /\S/);
    await expect(session.getByRole("tab", { selected: true }).first()).toBeVisible();
    await expect(fleet).toHaveCount(0);
    expect(name).not.toBe("");
    expect(errors).toEqual([]);
  });

  test("UB6 B6: the hub page in parts, one table with its details under the row; Hub Settings' one bar", async ({ page }) => {
    test.setTimeout(150_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore());
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);
    await fleetOf(page).getByRole("row").filter({ hasText: "DEMO-ALINK" }).first().getByRole("button", { name: /^Open\b/ }).click();
    const session = page.getByRole("article").last();
    await expect(session).toHaveAttribute("data-serial", /DEMO-ALINK/);
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub");

    // The parts the hub has families for; Devices first: one table (the health board's columns in it).
    const parts = partsOf(session, "Hub");
    await expect(parts.getByRole("tab").first()).toHaveText("Devices");
    await expect(parts.getByRole("tab").last()).toHaveText("Hub");
    await expect(parts.getByRole("tab", { name: "Alarms", exact: true })).toHaveCount(1);
    await expect(parts.getByRole("tab", { name: "Devices" })).toHaveAttribute("aria-selected", "true");
    const devices = session.getByRole("region", { name: "Devices on the hub" });
    await expect(session.locator("table")).toHaveCount(1);
    await expect(session.getByRole("region", { name: "Health board" })).toHaveCount(0);
    await expect(devices.locator("th")).toHaveText(["Name ▲", "Location", "State", "Temperature", "Alarms", "Firmware", "Actions"]);
    // An Admin's Edit… and Forget… in the offline DSPi's details.
    const body = await hubDetails(devices, "Bar");
    await expect(body.getByRole("button", { name: /^(Edit the details of|Forget) Bar$/ })).toHaveCount(2);
    await expect(body.locator(".hub-detail dt")).toContainText(["CPU", "Clip", "Uptime", "Hangs", "Last restart", "Last change", "Tags"]);
    await noSidewaysScroll(page);
    await shots(page, "hub-devices");

    // The banner's Show Alarms: the Alarms part.
    const banner = session.getByRole("status", { name: "Hub alarms" });
    await expect(banner).toContainText("alarm", { timeout: 30_000 });
    await banner.getByRole("button", { name: "Show Alarms" }).click();
    await expect(parts.getByRole("tab", { name: "Alarms" })).toHaveAttribute("aria-selected", "true");
    await expect(session.getByRole("region", { name: "Alarms" }).getByRole("listitem").first()).toBeVisible();
    await expect(devices).toHaveCount(0);
    await noSidewaysScroll(page);
    await shots(page, "hub-alarms");

    // Hub: the name, this console's role, the statistics.
    await openHubPart(session, "Hub");
    await expect(session.getByRole("region", { name: "The hub" }).locator(".hub-role-tag")).toHaveText("Admin");
    await expect(session.getByRole("region", { name: "Hub statistics" })).toContainText(/session/);
    await shots(page, "hub-hub");

    // Hub Settings: one bar for the page's changes, no Save per section.
    await openPage(session, "Hub Settings");
    const screen = session.locator(".hub-settings-screen");
    const bar = screen.getByRole("region", { name: "Unsaved hub settings" });
    await expect(bar).toHaveCount(0);
    await expect(screen.getByRole("button", { name: /^Save (Alarm Rules|Targets|MQTT Settings)$/ })).toHaveCount(0);
    const temp = screen.getByRole("region", { name: "Alarm rules" }).getByRole("group", { name: "Running hot" });
    await temp.getByLabel(/At or above \(°C\)/).fill("70");
    await temp.getByLabel(/At or above \(°C\)/).dispatchEvent("change");
    await screen.getByRole("region", { name: "MQTT and Home Assistant" }).getByLabel("Base topic").fill("venue");
    await expect(bar).toContainText("2 changes");
    await noSidewaysScroll(page);
    await shots(page, "hub-settings-bar");
    await bar.getByRole("button", { name: "Save", exact: true }).click();
    await expect(bar).toHaveCount(0, { timeout: 15_000 });
    await expect(screen.getByRole("region", { name: "MQTT and Home Assistant" }).getByLabel("Base topic")).toHaveValue("venue");
    expect(errors).toEqual([]);
  });
});
