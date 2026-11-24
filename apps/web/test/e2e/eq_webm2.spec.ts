// WP WEBM2 e2e: the Equaliser's PEQ, Crossover and Delay & Gain tabs and the inspector on the core's
// demo device, in every project. On a wide console the inspector sits beside the page and takes the
// band bar (with a shape grid) and the chosen channel's parts; hidden, or on a ~1000 px or 375 px
// console, the band bar stays under the graph and the inspector stacks under the page, with no
// sideways scroll. Every edit is read back from the core. Also RB's shell items: Save to device's
// colours are theme tokens, the tab's close button is reached by Tab, 44 px targets at 375 px.

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number };
type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = {
  numInputChannels: number;
  filters: Band[][];
  xover: Band[][];
  outputDelayMs: number[];
  outputMuted: boolean[];
  limiterEnabled: boolean[];
};

/** LR4 high pass, BW2 high pass (config.h's crossover numbering). */
const LR4_HP = 35;

async function core(page: Page): Promise<State> {
  const snap = (await page.evaluate(() => (globalThis as unknown as Clients).dspiClients[0]!.call("state", null))) as { state: State };
  return snap.state;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function noSidewaysScroll(page: Page) {
  expect(await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth)).toBeLessThanOrEqual(0);
}

const wide = () => test.info().project.name !== "narrow";

/** Screenshots in both themes when SHOTS_DIR names a directory (not in Firefox). */
async function shot(page: Page, name: string) {
  const dir = process.env.SHOTS_DIR;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  for (const scheme of ["light", "dark"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await page.screenshot({ path: `${dir}/webm2-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** One console with Venue A's demo device, on its PEQ page with OUT1 chosen. */
async function openEq(page: Page) {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await openPage(session, "EQ");
  await session.locator(".eq-pick", { hasText: "OUT1" }).click();
  const graph = session.getByRole("application", { name: /^EQ graph, OUT1/ });
  await expect(graph).toBeVisible();
  const inspector = session.getByRole("complementary", { name: "Inspector" });
  return { one, session, graph, inspector };
}

/** A point on the graph at `freq` Hz and `db` dB, from its drawn range (the default 20 Hz..20 kHz, ±25 dB). */
async function at(graph: Locator, freq: number, db: number) {
  const r = (await graph.boundingBox())!;
  const fx = Math.log10(freq / 20) / Math.log10(1000);
  return { x: r.x + fx * r.width, y: r.y + ((25 - db) / 50) * r.height };
}

test("WEBM2: the inspector beside a wide page has the band bar and a shape grid; the label on the graph; hidden, the bar is back under the graph", async ({
  page,
}) => {
  test.skip(!wide(), "a wide console");
  const errors = errorsOf(page);
  const { session, graph, inspector } = await openEq(page);
  await expect(inspector).toBeVisible();
  await expect(inspector).toHaveAttribute("data-placement", "beside");
  const g = (await graph.boundingBox())!;
  const i = (await inspector.boundingBox())!;
  expect(i.x).toBeGreaterThan(g.x + g.width - 1);
  // Double click: a bell, chosen, its fields in the inspector, its label by its dot.
  const slot = (await core(page)).filters[(await core(page)).numInputChannels]!.findIndex((b) => b.filterType === 0);
  expect(slot).toBeGreaterThanOrEqual(0);
  await graph.evaluate((e) => e.scrollIntoView({ block: "center" }));
  const p = await at(graph, 1000, 6);
  await page.mouse.dblclick(p.x, p.y);
  const w = (await core(page)).numInputChannels;
  await expect.poll(async () => (await core(page)).filters[w]![slot]!.filterType).toBe(1);
  const chip = inspector.getByRole("group", { name: `Band ${slot + 1}` });
  await expect(chip).toBeVisible();
  await expect(session.locator(".eq-graph-card .band-chip")).toHaveCount(0);
  await expect(graph.locator(".eq-band-tag")).toContainText(`${slot + 1} · Bell · `);
  // Its Q typed in the inspector, then a shape from the grid: read back from the core.
  await chip.getByRole("textbox", { name: "Q", exact: true }).fill("2.5");
  await chip.getByRole("textbox", { name: "Q", exact: true }).press("Enter");
  await expect.poll(async () => (await core(page)).filters[w]![slot]!.q).toBeCloseTo(2.5, 2);
  await inspector.getByRole("radiogroup", { name: "Shapes" }).getByRole("radio", { name: "Low Shelf" }).click();
  await expect.poll(async () => (await core(page)).filters[w]![slot]!.filterType).toBe(2);
  await expect(inspector.getByRole("radio", { name: "Low Shelf" })).toHaveAttribute("aria-checked", "true");
  await expect(graph.locator(".eq-band-tag")).toContainText(`${slot + 1} · Low Shelf · `);
  await shot(page, "peq-inspector");
  // Hidden: the band bar under the graph again; kept across a reload, then shown again.
  await inspector.getByRole("button", { name: "Hide" }).click();
  await expect(inspector).toHaveAttribute("data-placement", "hidden");
  await expect(session.locator(".eq-graph-card").getByRole("group", { name: /^(Band \d+|No band chosen)$/ })).toBeVisible();
  await page.reload();
  const again = await openEq(page);
  await expect(again.inspector).toHaveAttribute("data-placement", "hidden");
  await again.inspector.getByRole("button", { name: "Show" }).click();
  await expect(again.inspector).toHaveAttribute("data-placement", "beside");
  expect(errors).toEqual([]);
});

test("WEBM2: an output's inspector sets its high pass and limiter; an input's lists what it feeds", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, inspector } = await openEq(page);
  const w = (await core(page)).numInputChannels;
  const xo = inspector.getByRole("group", { name: "Crossover" });
  // From Off (the demo may have one already: Off first).
  await xo.getByRole("combobox", { name: "High pass", exact: true }).selectOption({ label: "Off" });
  await expect.poll(async () => (await core(page)).xover[w]!.some((b) => b.filterType % 2 === 1 && b.filterType >= 32)).toBe(false);
  await xo.getByRole("combobox", { name: "High pass", exact: true }).selectOption({ label: "LR4 24 dB/oct" });
  await expect.poll(async () => (await core(page)).xover[w]!.some((b) => b.filterType === LR4_HP)).toBe(true);
  await xo.getByRole("textbox", { name: "High pass frequency" }).fill("90");
  await xo.getByRole("textbox", { name: "High pass frequency" }).press("Enter");
  await expect.poll(async () => (await core(page)).xover[w]!.find((b) => b.filterType === LR4_HP)?.freq).toBeCloseTo(90, 0);
  await expect(xo).toContainText("HP LR4 90.00 Hz");
  await shot(page, "output-inspector");
  // The limiter's switch.
  const lim = inspector.getByRole("group", { name: "Limiter" });
  const was = (await core(page)).limiterEnabled[0]!;
  await lim.getByRole("switch", { name: "Output limiter" }).click();
  await expect.poll(async () => (await core(page)).limiterEnabled[0]).toBe(!was);
  // An input: what it feeds; one chosen from there is the page's channel.
  await session.locator(".eq-pick", { hasText: "IN1" }).click();
  const feeds = inspector.getByRole("group", { name: "Feeds" });
  await expect(feeds.getByRole("button", { name: /^OUT1 / })).toBeVisible();
  await feeds.getByRole("button", { name: /^OUT1 / }).click();
  await expect(session.locator(".eq-pick", { hasText: "OUT1" })).toHaveAttribute("aria-pressed", "true");
  expect(errors).toEqual([]);
});

test("WEBM2: the Crossover tab draws every output and sets a high pass; an output's name opens its XO bands", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openEq(page);
  const w = (await core(page)).numInputChannels;
  await openPage(session, "Crossover");
  await expect(session.locator(".xo-plot path.xo-curve").first()).toHaveAttribute("d", /^M/);
  const table = session.getByRole("region", { name: "Crossovers of every output" });
  const hp = table.getByRole("combobox", { name: /^OUT1 .* high pass$/ });
  await hp.selectOption({ label: "Off" });
  await hp.selectOption({ label: "BW2 12 dB/oct" });
  await expect.poll(async () => (await core(page)).xover[w]!.some((b) => b.filterType === 43)).toBe(true);
  await shot(page, "crossover");
  await noSidewaysScroll(page);
  await table.getByRole("button", { name: /^OUT1 / }).click();
  await expect(session.locator(".page[data-page]")).toHaveAttribute("data-page", "eq");
  await expect(session.getByRole("tablist", { name: "Bands" }).getByRole("tab", { name: "XO" })).toHaveAttribute("aria-selected", "true");
  expect(errors).toEqual([]);
});

test("WEBM2: Delay & Gain writes a distance as its delay, the arrival bars follow, mute and gain read back", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openEq(page);
  await openPage(session, "Delay & Gain");
  const table = session.getByRole("region", { name: "Delay and gain of every output" });
  const row = table.getByRole("row", { name: /^OUT1 / });
  await row.getByRole("textbox", { name: /distance$/ }).fill("1.715 m");
  await row.getByRole("textbox", { name: /distance$/ }).press("Enter");
  await expect.poll(async () => (await core(page)).outputDelayMs[0]).toBeCloseTo(5, 1);
  await expect(row.getByRole("textbox", { name: /delay$/ })).toHaveValue(/^5(\.0+)? ms$/);
  await row.getByRole("textbox", { name: /gain$/ }).fill("-3");
  await row.getByRole("textbox", { name: /gain$/ }).press("Enter");
  // The longest delay fills its bar (OUT1's now, unless another output has a longer one).
  const delays = (await core(page)).outputDelayMs;
  if (Math.max(...delays) === delays[0]) await expect(row.getByRole("meter")).toHaveAttribute("aria-valuenow", "100");
  const muted = (await core(page)).outputMuted[0]!;
  await row.getByRole("button", { name: /^Mute OUT1/ }).click();
  await expect.poll(async () => (await core(page)).outputMuted[0]).toBe(!muted);
  await shot(page, "delays");
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

// -- WP REVEFIX -- (REVE C1, USR1's class)
test("REVEFIX: tabbing through Delay & Gain writes nothing back: the fields show rounded values", async ({ page }) => {
  const errors = errorsOf(page);
  const { session } = await openEq(page);
  const call = (m: string, p: unknown) => page.evaluate(([m, p]) => (globalThis as unknown as Clients).dspiClients[0]!.call(m, p), [m, p] as const);
  await call("write", { path: "out.delay", indices: [0], value: { float: 4.1234 } });
  await openPage(session, "Delay & Gain");
  const before = (await core(page)).outputDelayMs[0]!;
  expect(before).toBeCloseTo(4.1234, 3);
  const dirty = ((await call("presetChanges", null)) as { changes: string[] }).changes.length;
  const row = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("row", { name: /^OUT1 / });
  // Into the delay field and out through distance and gain, typing nothing; Enter in one of them too
  await row.getByRole("textbox", { name: /delay$/ }).focus();
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  for (let i = 0; i < 3; i++) await page.keyboard.press("Tab");
  await expect(row.getByRole("textbox", { name: /delay$/ })).not.toBeFocused();
  await page.waitForTimeout(300);
  expect((await core(page)).outputDelayMs[0]).toBe(before);
  expect(((await call("presetChanges", null)) as { changes: string[] }).changes.length).toBe(dirty);
  expect(errors).toEqual([]);
});

test("WEBM2: at ~1000 px the inspector stacks under the page, the band bar stays under the graph, no sideways scroll", async ({ page }) => {
  test.skip(!wide(), "the 375 px test below is the narrow one");
  await page.setViewportSize({ width: 1000, height: 760 });
  const { session, graph, inspector } = await openEq(page);
  await expect(inspector).toHaveAttribute("data-placement", "under");
  const g = (await graph.boundingBox())!;
  const i = (await inspector.boundingBox())!;
  expect(i.y).toBeGreaterThan(g.y + g.height);
  await expect(session.locator(".eq-graph-card .band-chip")).toHaveCount(1);
  await shot(page, "1000px");
  for (const p of ["EQ", "Crossover", "Delay & Gain"]) {
    await openPage(session, p);
    await noSidewaysScroll(page);
  }
});

test("WEBM2: at 375 px: stacked, the band bar under the graph, 44 px targets, the tab's close button by Tab, no sideways scroll", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 800 });
  const { one, session, inspector } = await openEq(page);
  await expect(inspector).toHaveAttribute("data-placement", "under");
  await expect(session.locator(".eq-graph-card .band-chip")).toHaveCount(1);
  await noSidewaysScroll(page);
  await shot(page, "375px");
  for (const p of ["Crossover", "Delay & Gain"]) {
    await openPage(session, p);
    await noSidewaysScroll(page);
  }
  // RB: the dock's buttons and the tab's close are 44 px targets here.
  for (const b of await session.locator(".dock button:visible").all()) expect((await b.boundingBox())!.height).toBeGreaterThanOrEqual(43.5);
  const close = one.locator(".device-tab-close").first();
  const box = (await close.boundingBox())!;
  expect(Math.min(box.width, box.height)).toBeGreaterThanOrEqual(43.5);
  // RB: Tab from the tab on show reaches its close button.
  await one.getByRole("tablist", { name: "Open DSPi" }).getByRole("tab").first().focus();
  await page.keyboard.press("Tab");
  await expect(close).toBeFocused();
});

test("WEBM2: Save to device takes its colours from the theme's tokens", async ({ page }) => {
  const { one } = await openEq(page);
  const save = one.locator(".bar-save");
  const bg = () => save.evaluate((e) => getComputedStyle(e).backgroundColor);
  const token = () => save.evaluate((e) => getComputedStyle(e).getPropertyValue("--save-bg").trim());
  await page.emulateMedia({ colorScheme: "light" });
  expect(await token()).toBe("#2563eb");
  expect(await bg()).toBe("rgb(37, 99, 235)");
  await page.emulateMedia({ colorScheme: "dark" });
  expect(await token()).toBe("#2f6fdb");
  expect(await bg()).toBe("rgb(47, 111, 219)");
  await page.emulateMedia({ colorScheme: null });
});
