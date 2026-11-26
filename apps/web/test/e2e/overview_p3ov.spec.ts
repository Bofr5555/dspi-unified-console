// WP P3OV e2e: the Overview (PLAN-P3 U1) on the core's demo device in Chromium,
// Firefox and Chromium at 375 px. Every output in one table without sideways
// scroll at 1280 px, a card per output at 375 px; an inline gain edit lands on
// the device and shows on the Outputs page (the EQ screen's output card), an
// inline mute is one undo step; a read-only session edits nothing; the meters
// move (the tests' fake core, whose meters have a signal: the demo's capture
// is silent); and on a real hub (`uconsole --demo hub --web`, DSPI_TERM) a
// change made by another session updates the row.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openOnHub, openPage } from "../nav"; // -- WP UB6 -- openOnHub

type State = { numOutputChannels: number; outputGainDb: number[]; outputMuted: boolean[] };
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function core(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate((n) => (globalThis as unknown as G).dspiClients[n]!.call("state", null), n)) as { state: State };
  return snap.state;
}

const narrow = () => test.info().project.name === "narrow";

async function noSidewaysScroll(page: Page, overview: Locator) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
  // Nothing in the Overview sticks out of it (no inner scroller either).
  const over = await overview.evaluate((el) => {
    const r = el.getBoundingClientRect();
    return [...el.querySelectorAll("*")].filter((e) => e.getBoundingClientRect().right > r.right + 1).map((e) => `${e.tagName.toLowerCase()}.${(e as HTMLElement).className}`).slice(0, 5);
  });
  expect(over).toEqual([]);
}

/** Console one alone, the page's width, on Venue A (control) or Venue B (view only), on its Overview. */
async function openOverview(page: Page, venue = "Venue A", query = "one") {
  await page.goto(`/test/embed.html?${query}`);
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await openPage(session, "Overview");
  const overview = session.getByRole("region", { name: "Overview" });
  await expect(overview).toBeVisible();
  return { session, overview };
}

const rowOf = (overview: Locator, o: number) => overview.locator(`tr.ov-row[data-output="${o}"]`);

test("P3OV: every output in one table, as the core has them, without sideways scroll (1280 px) or as cards (375 px)", async ({ page }) => {
  const errors = errorsOf(page);
  const { overview } = await openOverview(page);
  const s = await core(page);
  const table = overview.getByRole("table", { name: "Outputs" });
  await expect(table.locator("tbody tr")).toHaveCount(s.numOutputChannels);
  expect(s.numOutputChannels).toBeGreaterThanOrEqual(8);
  for (let o = 0; o < s.numOutputChannels; o++) {
    const row = rowOf(overview, o);
    await expect(row.getByRole("textbox", { name: new RegExp(`^OUT${o + 1} .* gain$`) })).toHaveValue(`${(Math.round(s.outputGainDb[o]! * 10) / 10).toFixed(1)} dB`);
    await expect(row.getByRole("button", { name: new RegExp(`^OUT${o + 1} .* mute$`) })).toHaveAttribute("aria-pressed", String(s.outputMuted[o] === true));
    await expect(row.locator(".ov-delay")).toContainText(" ms");
    await expect(row.getByRole("meter")).toHaveAttribute("aria-valuemin", "-90");
  }
  // The inputs' meters above the table.
  expect(await overview.getByRole("region", { name: "Input meters" }).getByRole("meter").count()).toBeGreaterThanOrEqual(2);
  // The registry's units in the headers (hidden on cards, where each cell says what it is).
  await expect(table.getByRole("columnheader", { name: /^Gain \(dB\)$/ })).toHaveCount(1);
  if (narrow()) {
    // A card per output: the rows are boxes stacked, each wider than half the page.
    const box = (await rowOf(overview, 0).boundingBox())!;
    const next = (await rowOf(overview, 1).boundingBox())!;
    expect(next.y).toBeGreaterThanOrEqual(box.y + box.height - 1);
    expect(box.width).toBeGreaterThan(300);
    expect(await rowOf(overview, 0).evaluate((e) => getComputedStyle(e).display)).toBe("grid");
  } else {
    // One table: every row's cells side by side on one line.
    const a = (await rowOf(overview, 0).locator(".ov-name").boundingBox())!;
    const b = (await rowOf(overview, 0).locator(".ov-level").boundingBox())!;
    expect(Math.abs(a.y - b.y)).toBeLessThan(a.height);
    expect(page.viewportSize()!.width).toBe(1280);
  }
  await noSidewaysScroll(page, overview);
  expect(errors).toEqual([]);
});

test("P3OV: an inline gain edit lands and shows on the Outputs page; an inline mute is one undo step", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, overview } = await openOverview(page);
  const row = rowOf(overview, 1);
  const field = row.getByRole("textbox", { name: /^OUT2 .* gain$/ });
  await field.click();
  await field.fill("-4.5");
  await field.press("Enter");
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBe(-4.5);
  await expect(field).toHaveValue("-4.5 dB");
  // The + step: to the next half dB.
  await row.getByRole("button", { name: /^OUT2 .* gain up$/ }).click();
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBe(-4);
  // Past the registry's range: kept inside it (the core checks it again).
  await field.click();
  await field.fill("+99");
  await field.press("Enter");
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBeLessThan(99);
  const top = (await core(page)).outputGainDb[1]!;
  await field.click();
  await field.fill("-4");
  await field.press("Enter");
  await expect.poll(async () => (await core(page)).outputGainDb[1]).toBe(-4);
  expect(top).toBeGreaterThan(-4);

  // The Outputs page (OUT2's output card on the EQ screen) shows it.
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT2" }).click();
  const card = session.getByRole("region", { name: /^Output settings, OUT2/ });
  await expect(card.getByRole("textbox", { name: "Gain" })).toHaveValue("-4.0 dB");

  // Mute inline, then Undo: the same journal as every write.
  await openPage(session, "Overview");
  const mute = rowOf(overview, 1).getByRole("button", { name: /^OUT2 .* mute$/ });
  const was = (await core(page)).outputMuted[1] === true;
  await mute.click();
  await expect.poll(async () => (await core(page)).outputMuted[1]).toBe(!was);
  await expect(mute).toHaveAttribute("aria-pressed", String(!was));
  await session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await core(page)).outputMuted[1] === true).toBe(was);
  await expect(mute).toHaveAttribute("aria-pressed", String(was));
  expect(errors).toEqual([]);
});

test("P3OV: a change made outside the Overview updates its row", async ({ page }) => {
  const errors = errorsOf(page);
  const { overview } = await openOverview(page);
  await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("write", { path: "out.gain", indices: [0], value: { float: -12 } }));
  // The core's write from outside the page arrives as the session's notification.
  await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("demo", { action: "renameElsewhere", name: "Stage left" }));
  await expect(rowOf(overview, 0).getByRole("textbox", { name: /^OUT1 .* gain$/ })).toHaveValue("-12.0 dB");
  expect(errors).toEqual([]);
});

test("P3OV: a read-only session shows the Overview with nothing editable", async ({ page }) => {
  const errors = errorsOf(page);
  const { overview } = await openOverview(page, "Venue B");
  const row = rowOf(overview, 0);
  await expect(row.getByRole("textbox", { name: /^OUT1 .* gain$/ })).toBeDisabled();
  await expect(row.getByRole("button", { name: /^OUT1 .* mute$/ })).toBeDisabled();
  await expect(row.getByRole("button", { name: /^OUT1 .* gain up$/ })).toBeDisabled();
  expect(errors).toEqual([]);
});

test("P3OV: the meters move, with a peak hold never below the level", async ({ page }) => {
  const errors = errorsOf(page);
  // The tests' fake core: its meters carry a moving signal.
  const { overview } = await openOverview(page, "Venue A", "one&core=stub");
  const meter = overview.getByRole("region", { name: "Input meters" }).getByRole("meter").first();
  await expect(meter).toBeVisible();
  const seen = new Set<string>();
  await expect
    .poll(async () => {
      seen.add((await meter.getAttribute("aria-valuenow")) ?? "");
      return seen.size;
    }, { intervals: [120], timeout: 10_000 })
    .toBeGreaterThan(2);
  // Level and hold read together (one render), for every meter.
  for (let k = 0; k < 5; k++) {
    const pairs = await overview.evaluate((el) => [...el.querySelectorAll('[role="meter"]')].map((m) => [Number(m.getAttribute("aria-valuenow")), Number(m.getAttribute("data-hold"))]));
    expect(pairs.length).toBeGreaterThan(2);
    for (const [now, hold] of pairs) expect(hold!).toBeGreaterThanOrEqual(now!);
  }
  // An output's meter moves too.
  const out = rowOf(overview, 0).getByRole("meter");
  const outSeen = new Set<string>();
  await expect
    .poll(async () => {
      outSeen.add((await out.getAttribute("aria-valuenow")) ?? "");
      return outSeen.size;
    }, { intervals: [120], timeout: 10_000 })
    .toBeGreaterThan(2);
  expect(errors).toEqual([]);
});

// -- On a real hub: another session's change --

type OwnHub = { port: number; child: ChildProcess };

async function startHub(): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/p3ov-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
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

async function hubOverview(page: Page, hub: OwnHub): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const session = await openOnHub(page, { host: `127.0.0.1:${hub.port}`, serial: "DEMO0000DSPI0001" }); // -- WP UB6 -- the Fleet's Open
  await openPage(session, "Overview");
  const overview = session.getByRole("region", { name: "Overview" });
  await expect(overview).toBeVisible();
  return overview;
}

if (!process.env.DSPI_TERM) {
  test("P3OV: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P3OV on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("P3OV: a change made by another session on the hub updates the row", async ({ page }) => {
    test.setTimeout(90_000);
    const errors = errorsOf(page);
    hub = await startHub();
    const first = await hubOverview(page, hub);
    const other = await page.context().newPage();
    const second = await hubOverview(other, hub);
    // The second session sets OUT3's gain and mutes it from its own Overview.
    const theirs = rowOf(second, 2);
    const field = theirs.getByRole("textbox", { name: /^OUT3 .* gain$/ });
    await field.click();
    await field.fill("-8.5");
    await field.press("Enter");
    await expect(field).toHaveValue("-8.5 dB");
    const mine = rowOf(first, 2);
    await expect(mine.getByRole("textbox", { name: /^OUT3 .* gain$/ })).toHaveValue("-8.5 dB", { timeout: 15_000 });
    const was = (await mine.getByRole("button", { name: /^OUT3 .* mute$/ }).getAttribute("aria-pressed")) === "true";
    await theirs.getByRole("button", { name: /^OUT3 .* mute$/ }).click();
    await expect(mine.getByRole("button", { name: /^OUT3 .* mute$/ })).toHaveAttribute("aria-pressed", String(!was), { timeout: 15_000 });
    await other.close();
    await noSidewaysScroll(page, first);
    expect(errors).toEqual([]);
  });
});
