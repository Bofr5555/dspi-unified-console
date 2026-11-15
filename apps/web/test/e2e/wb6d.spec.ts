// WP WB6d e2e: the System view's signal chain on the core's demo device, in Chromium, Firefox
// and Chromium at 375 px: every live input and every output with its meter, the routing
// summary, the processing blocks with their on or off state as the core's state has them,
// the outputs' gain, mute and delay; a change made elsewhere shows here; a block opens the
// screen that edits it (EQ at the channel and its PEQ or XO tab, the Matrix, the Main window).

import { expect, test, type Locator, type Page } from "@playwright/test";

type Band = { filterType: number; bypass: boolean };
type State = {
  numInputChannels: number;
  numOutputChannels: number;
  formatVersion: number;
  maxBands: number;
  filters: Band[][];
  xover: Band[][];
  outputGainDb: number[];
  outputDelayMs: number[];
  outputMuted: boolean[];
  outputEnabled: boolean[];
  limiterEnabled: boolean[];
  userVolumeDb: number;
};
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function core(page: Page, n = 0): Promise<State> {
  const snap = (await page.evaluate((n) => (globalThis as unknown as G).dspiClients[n]!.call("state", null), n)) as { state: State };
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

/** Console one alone on a venue's demo (Venue B: view only), on its System view: the session and the chain. */
async function openChain(page: Page, venue = "Venue A") {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await session.getByRole("tab", { name: "System", exact: true }).click();
  const chain = session.getByRole("region", { name: "Signal chain" });
  await expect(chain).toBeVisible();
  return { session, chain };
}

const outRow = (chain: Locator, o: number) => chain.getByRole("group", { name: `Chain OUT${o + 1}`, exact: true });
const block = (row: Locator, label: string) => row.getByRole("list", { name: /^Processing of / }).getByRole("listitem").filter({ hasText: new RegExp(`^${label}`) });

test("WB6d: the chain lists the live inputs and every output with meters, blocks, gain, mute and delay as the core has them", async ({ page }) => {
  const errors = errorsOf(page);
  const { chain } = await openChain(page);
  const s = await core(page);
  const inputs = chain.getByRole("region", { name: "Chain inputs" }).getByRole("group");
  // -- fixup batch3 F2 -- the source as the Inputs screen says it: the user's name, else its title ("USB"), never the registry key.
  const names = (await page.evaluate(() => (globalThis as unknown as G).dspiClients[0]!.call("inputSourceNames", null))) as { names: string[] };
  const src = (s as unknown as { inputSource: number }).inputSource;
  const title = (names.names[src] ?? "").trim() || (src === 0 ? "USB" : "");
  expect(title).not.toBe("");
  await expect(chain.getByRole("region", { name: "Chain inputs" }).getByRole("heading")).toHaveText(`Inputs · Source: ${title}`);
  // The demo streams two USB inputs until the device says more.
  await expect(inputs).toHaveCount(2);
  await expect(chain.getByRole("region", { name: "Chain outputs" }).getByRole("group")).toHaveCount(s.numOutputChannels);
  await expect(chain.getByRole("meter")).toHaveCount(2 + s.numOutputChannels);
  await expect(inputs.first().getByRole("meter")).toHaveAttribute("aria-valuemin", "-90");
  for (let o = 0; o < s.numOutputChannels; o++) {
    const row = outRow(chain, o);
    await expect(row).toHaveAttribute("data-enabled", String(s.outputEnabled[o] !== false));
    const level = row.getByLabel(`Level of OUT${o + 1}`);
    await expect(level).toContainText(`${(Math.round(s.outputGainDb[o]! * 10) / 10).toFixed(1)} dB`);
    await expect(level).toContainText(" ms");
    if (s.outputMuted[o]) await expect(level).toContainText("Muted");
    else await expect(level).not.toContainText("Muted");
    // Each output's EQ, crossover and (wire 32 on) limiter, with its state said.
    const peq = s.filters[s.numInputChannels + o]!.slice(0, s.maxBands || 10).filter((p) => p.filterType !== 0 && !p.bypass && !(p.filterType >= 32 && p.filterType <= 63)).length;
    await expect(block(row, "EQ")).toContainText(peq ? ": on" : ": off");
    const xo = (s.xover[s.numInputChannels + o] ?? []).filter((p) => p.filterType !== 0 && !p.bypass).length;
    await expect(block(row, "Crossover")).toContainText(xo ? ": on" : ": off");
    if (s.formatVersion >= 32) await expect(block(row, "Limiter")).toContainText(s.limiterEnabled[o] ? ": on" : ": off");
    // The routing summary, the core's label of what feeds it.
    await expect(row.getByRole("button", { name: /^Routing of OUT\d+: / })).toBeVisible();
  }
  // The device-wide effects, each with its state.
  const fx = chain.getByRole("list", { name: "Effects" }).getByRole("listitem");
  expect(await fx.count()).toBeGreaterThan(0);
  for (const t of await fx.allInnerTexts()) expect(t).toMatch(/: (on|off)/);
  await expect(chain.getByRole("button", { name: /^Volume / })).toContainText(`${s.userVolumeDb.toFixed(1)} dB`);
  // No horizontal page scroll, at 375 px too.
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
  expect(errors).toEqual([]);
});

test("WB6d: a change made on another screen shows in the chain (limiter, mute)", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, chain } = await openChain(page);
  const s = await core(page);
  test.skip(s.formatVersion < 32, "the demo's firmware has no limiter");
  const o = s.outputEnabled.findIndex((e) => e !== false);
  const row = outRow(chain, o);
  const wasOn = s.limiterEnabled[o] === true;
  await expect(block(row, "Limiter")).toContainText(wasOn ? ": on" : ": off");
  // OUT1's limiter and mute, on its card on the EQ screen (opened from the chain).
  const n = `OUT${o + 1}`;
  await row.getByRole("button", { name: new RegExp(`^${n} `) }).click();
  await expect(session.getByRole("tab", { name: "EQ", exact: true })).toHaveAttribute("aria-selected", "true");
  const card = session.getByRole("region", { name: new RegExp(`^Output settings, ${n}`) });
  await card.getByRole("button", { name: new RegExp(`^Limiter ${n}`) }).click();
  await expect.poll(async () => (await core(page)).limiterEnabled[o]).toBe(!wasOn);
  const muted = s.outputMuted[o] === true;
  await card.getByRole("button", { name: "Mute", exact: true }).click();
  await expect.poll(async () => (await core(page)).outputMuted[o]).toBe(!muted);
  // Back on the System view: the chain says so.
  await session.getByRole("tab", { name: "System", exact: true }).click();
  await expect(block(outRow(chain, o), "Limiter")).toContainText(!wasOn ? ": on" : ": off");
  if (!muted) await expect(outRow(chain, o).getByLabel(`Level of ${n}`)).toContainText("Muted");
  else await expect(outRow(chain, o).getByLabel(`Level of ${n}`)).not.toContainText("Muted");
  expect(errors).toEqual([]);
});

test("WB6d: a block opens its screen: EQ at the channel and tab, the Matrix, the Effects, the Main window", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, chain } = await openChain(page);
  const s = await core(page);
  // IN2's EQ: the EQ screen with IN2 chosen, its PEQ bands.
  await block(chain.getByRole("group", { name: /^Chain IN2 / }), "EQ").getByRole("button").click();
  await expect(session.getByRole("tab", { name: "EQ", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(session.locator(".eq-pick", { hasText: "IN2" })).toHaveAttribute("aria-pressed", "true");
  // OUT1's crossover: OUT1 chosen, the XO tab.
  await session.getByRole("tab", { name: "System", exact: true }).click();
  const o1 = s.outputEnabled.findIndex((e) => e !== false);
  await block(outRow(chain, o1), "Crossover").getByRole("button").click();
  await expect(session.locator(".eq-pick", { hasText: `OUT${o1 + 1}` })).toHaveAttribute("aria-pressed", "true");
  const tabs = session.getByRole("tablist", { name: "Bands" });
  if (await tabs.count()) await expect(tabs.getByRole("tab", { name: "XO", exact: true })).toHaveAttribute("aria-selected", "true");
  // A tab chosen by hand opens its screen as it was: PEQ on the first channel.
  await session.getByRole("tab", { name: "Main", exact: true }).click();
  await session.getByRole("tab", { name: "EQ", exact: true }).click();
  await expect(session.locator(".eq-pick").first()).toHaveAttribute("aria-pressed", "true");
  // The routing summary: the Matrix.
  await session.getByRole("tab", { name: "System", exact: true }).click();
  await outRow(chain, o1).getByRole("button", { name: /^Routing of / }).click();
  await expect(session.getByRole("tab", { name: "Matrix", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(session.getByRole("region", { name: "Matrix Mixer" })).toBeVisible();
  // -- fixup b4fix N3 -- a device-wide effect and an output's effect block: the Effects screen at that effect.
  for (const list of [chain.getByRole("list", { name: "Effects" }), chain.getByRole("list", { name: /^Processing of / })]) {
    await session.getByRole("tab", { name: "System", exact: true }).click();
    const fxBlock = list.locator("button[data-block]:not([data-block=eq]):not([data-block=xo]):not([data-block=limiter])").first();
    if (!(await fxBlock.count())) continue;
    const id = (await fxBlock.getAttribute("data-block"))!;
    await fxBlock.click();
    await expect(session.getByRole("tab", { name: "Effects", exact: true })).toHaveAttribute("aria-selected", "true");
    await expect(session.locator(`.fx-pick[data-effect="${id}"]`)).toHaveAttribute("aria-selected", "true");
  }
  // The volume: the Main window.
  await session.getByRole("tab", { name: "System", exact: true }).click();
  await chain.getByRole("button", { name: /^Volume / }).click();
  await expect(session.getByRole("tab", { name: "Main", exact: true })).toHaveAttribute("aria-selected", "true");
  expect(errors).toEqual([]);
});

test("WB6d: a read-only session shows the chain, and its blocks still open their screens", async ({ page }) => {
  const errors = errorsOf(page);
  const { session, chain } = await openChain(page, "Venue B");
  await expect(session.getByText("You can view this DSPi but not change it").first()).toBeVisible();
  await expect(chain.getByRole("meter").first()).toBeVisible();
  const s = await core(page);
  const o = s.outputEnabled.findIndex((e) => e !== false);
  await block(outRow(chain, o), "EQ").getByRole("button").click();
  await expect(session.getByRole("tab", { name: "EQ", exact: true })).toHaveAttribute("aria-selected", "true");
  expect(errors).toEqual([]);
});
