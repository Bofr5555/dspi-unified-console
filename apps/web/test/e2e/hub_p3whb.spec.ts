// WP P3WHB e2e: the hub screen's health board on the console served by a real hub
// (`uconsole --demo hub --web`, DSPI_TERM) with the demo DSPi: a threshold from the hub's
// alarm rules (the test's rule: temperature at or above 0 °C; the demo reads 35 °C) colours
// the temperature cell; the alarm that rule raises shows in the DSPi's row and is
// acknowledged from it; the stream counters' columns are hidden until the hub gives
// counters, then shown; the offline DSPi listed below; sort and the tag filter; Chromium,
// Firefox and 375 px (cards) with no sideways scroll.

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

/** A store the hub starts from: a DSPi it saw before (offline now, tagged "bar") and an alarm rule the demo trips. */
function seededStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/p3whb-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  const seen = Math.floor(Date.parse("2026-11-10T09:30:00Z") / 1000);
  writeFileSync(
    `${store}/inventory.json`,
    JSON.stringify({
      v: 1,
      devices: [{ serial: AWAY, name: "Bar", platform: 1, fw: "1.1.6", outputs: 9, location: "Bar", tags: ["bar"], first_seen_s: seen - 86400, last_seen_s: seen }],
    }),
  );
  // The demo reads 35 °C: at or above 0 °C raises `temperature` and colours the cell.
  writeFileSync(`${store}/alarms.json`, JSON.stringify({ v: 1, rules: { temperature: { on: true, above_c: 0 } } }));
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

if (!process.env.DSPI_TERM) {
  test("P3WHB: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P3WHB on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("P3WHB: the health board: a rule's threshold colours a cell, an alarm acknowledged from the row, counters once the hub gives them", async ({ page }) => {
    test.setTimeout(180_000);
    const errors = errorsOf(page);
    hub = await startHub(seededStore());
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);
    const picker = page.getByRole("navigation", { name: "DSPi devices" });
    await picker.getByRole("listitem").filter({ hasText: "· online" }).getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await expect(session.locator(".device-meta")).toContainText(DEMO);
    await setMode(session, page, "Advanced");
    await session.getByRole("tab", { name: "Hub", exact: true }).click();

    const board = session.getByRole("region", { name: "Health board" });
    const own = board.locator("tr.hub-health-row").filter({ hasText: DEMO });
    const away = board.locator("tr.hub-health-row").filter({ hasText: AWAY });
    // The offline DSPi listed below the online one.
    await expect(own).toHaveCount(1);
    await expect(away).toContainText("Offline");
    const order = await board.locator("tr.hub-health-row .hub-serial").allTextContents();
    expect(order.indexOf(DEMO)).toBeLessThan(order.indexOf(AWAY));

    // The counters: the hub reads them on a 30 s cycle (the starvation total about 45 s after
    // the attach), so the columns are hidden at first, then shown with the demo's totals.
    const counterHead = board.locator("th").filter({ hasText: "Underruns" });
    await expect(counterHead).toHaveCount(0);
    await expect(own.locator("td.hub-health-underruns")).toHaveCount(0);

    // The rule's threshold (0 °C) colours the demo's 35 °C; CPU has no rule set here that the demo reaches.
    const temp = own.locator("td.hub-health-temp");
    await expect(temp).toContainText("35.0 °C", { timeout: 20_000 });
    await expect(temp).toHaveAttribute("data-tone", "bad", { timeout: 20_000 });
    await expect(temp).toHaveAttribute("title", /0 °C/);
    await expect(own.locator("td.hub-health-cpu")).not.toHaveAttribute("data-tone", /./);
    // The demo announces clip events: their count a minute (latch-only firmware: the latch's word).
    await expect(own.locator("td.hub-health-clip")).toHaveText(/^(\d+\/min|No|Clipped)$/);
    await noSidewaysScroll(page);

    // The alarm the rule raises shows in the row and is acknowledged from it.
    const hot = own.locator("li.hub-row-alarm").filter({ hasText: "Running hot" });
    await expect(hot).toHaveCount(1, { timeout: 30_000 });
    await expect(hot).toHaveAttribute("data-key", "alarm.temperature");
    await hot.getByRole("button", { name: "Acknowledge" }).click();
    await expect(hot).toContainText("Acknowledged");
    await expect(hot.getByRole("button", { name: "Acknowledge" })).toHaveCount(0);
    // The alarm board below says the same.
    await expect(session.getByRole("region", { name: "Alarms" }).getByRole("listitem").filter({ hasText: "Running hot" })).toContainText("Acknowledged");

    // Sort: the temperature, highest first; the offline DSPi stays below.
    await board.locator("th").filter({ hasText: "Temperature" }).getByRole("button").click();
    await expect(board.locator("th").filter({ hasText: "Temperature" })).toHaveAttribute("aria-sort", "descending");
    await board.locator("th").filter({ hasText: "Temperature" }).getByRole("button").click();
    await expect(board.locator("th").filter({ hasText: "Temperature" })).toHaveAttribute("aria-sort", "ascending");
    const sorted = await board.locator("tr.hub-health-row .hub-serial").allTextContents();
    expect(sorted.indexOf(DEMO)).toBeLessThan(sorted.indexOf(AWAY));

    // The tag filter: the offline DSPi's tag; then a tag nobody has.
    const filter = board.getByLabel("Filter by tag");
    await filter.fill("BAR");
    await expect(board.locator("tr.hub-health-row")).toHaveCount(1);
    await expect(away).toHaveCount(1);
    await filter.fill("nowhere");
    await expect(board).toContainText("No DSPi has that tag.");
    await filter.fill("");
    await expect(own).toHaveCount(1);

    // Now the counters arrive: the columns show, with the demo's totals.
    await expect(counterHead).toHaveCount(1, { timeout: 90_000 });
    await expect(own.locator("td.hub-health-starvation")).toHaveText(/^(\d+|–)$/);
    await expect(own.locator("td.hub-health-starvation")).toHaveText("0", { timeout: 20_000 });
    await noSidewaysScroll(page);
    expect(errors).toEqual([]);
  });
});
