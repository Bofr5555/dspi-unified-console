// WP WBCLIP2 e2e: one channel clip per PAGE (review-p3c N9). The embedding
// page holds two consoles, each with its own demo DSPi on its own core: a
// channel copied in console one pastes onto console two's device, by menu
// and by key, one undo step there, the source untouched; console two's Paste
// entry names the source device and channel, console one's own stays plain.
// Every result is read back from each core's own state (a direct call).

import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type Band = { filterType: number; bypass: boolean; freq: number; q: number; gain: number; qp: number };
type State = { numInputChannels: number; filters: Band[][]; outputGainDb: number[] };
type Snap = { device: { name: string; serial: string }; state: State };
type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function snap(page: Page, n: number): Promise<Snap> {
  return (await page.evaluate((i) => (globalThis as unknown as G).dspiClients[i]!.call("state", null), n)) as Snap;
}
const core = async (page: Page, n: number) => (await snap(page, n)).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

/** Connect console `id`'s Venue A and show its EQ screen. */
async function openEq(page: Page, id: "#one" | "#two") {
  const host = page.locator(id);
  await host.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = host.getByRole("article", { name: "DSPi" });
  await openPage(session, "EQ");
  await expect(session.locator(".eq-pick").first()).toBeVisible();
  return session;
}

async function pick(session: Locator, desc: string) {
  const b = session.locator(".eq-pick", { hasText: desc });
  await b.click();
  await expect(b).toHaveAttribute("aria-pressed", "true");
}
const channelMenu = (session: Locator) => session.locator(".eq-tools").getByRole("button", { name: "Channel" });
const item = (session: Locator, name: string | RegExp) => session.getByRole("menuitem", { name });
const undo = (s: Locator) => s.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" });
const named = (s: Snap) => (s.device.serial && s.device.serial !== s.device.name ? `${s.device.name} (${s.device.serial})` : s.device.name);

test("WBCLIP2 page clip: OUT1 copied in console one pastes onto console two's OUT2, named, one undo step there", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html");
  const one = await openEq(page, "#one");
  const two = await openEq(page, "#two");
  // Console one's device gets a band and a gain on OUT1 (copy reads the core's mirror).
  await page.evaluate(async () => {
    const c = (globalThis as unknown as G).dspiClients[0]!;
    const s = ((await c.call("state", null)) as { state: State }).state;
    await c.call("filter", { channel: s.numInputChannels, band: 2, filterType: 1, bypass: false, freq: 315, q: 1.4, gain: 5, qp: 0.707 });
    await c.call("write", { path: "out.gain", indices: [0], value: { float: -7 } });
  });
  const src = await snap(page, 0);
  const t0 = await core(page, 1);
  const out2 = t0.numInputChannels + 1;
  const before = { gain: t0.outputGainDb[1], bands: t0.filters[out2] };

  await pick(two, "OUT2");
  await channelMenu(two).click();
  await expect(item(two, "Paste Parameters")).toBeDisabled(); // nothing copied yet
  await page.keyboard.press("Escape");

  await pick(one, "OUT1");
  await channelMenu(one).click();
  await item(one, "Copy Parameters").click();
  // Console one's own clip: the plain words.
  await channelMenu(one).click();
  await expect(item(one, "Paste Parameters")).toHaveText("Paste Parameters");
  await page.keyboard.press("Escape");

  await channelMenu(two).click();
  const paste = item(two, `Paste Parameters from ${named(src)} OUT1`);
  await expect(paste).toBeEnabled();
  await paste.click();
  await expect.poll(async () => (await core(page, 1)).outputGainDb[1]).toBe(-7);
  expect((await core(page, 1)).filters[out2]![2]!.freq).toBe(315);
  // The source is untouched.
  expect((await core(page, 0)).filters).toEqual(src.state.filters);

  await undo(two).click();
  await expect.poll(async () => (await core(page, 1)).outputGainDb[1]).toBe(before.gain);
  expect((await core(page, 1)).filters[out2]).toEqual(before.bands);
  expect(errors).toEqual([]);
});

test("WBCLIP2 page clip by key: Ctrl/Cmd+C in console one, Ctrl/Cmd+V in console two", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html");
  const one = await openEq(page, "#one");
  const two = await openEq(page, "#two");
  await page.evaluate(async () => {
    const c = (globalThis as unknown as G).dspiClients[0]!;
    await c.call("write", { path: "out.gain", indices: [1], value: { float: -9 } });
  });
  await pick(one, "OUT2");
  await page.keyboard.press("ControlOrMeta+c");
  await pick(two, "OUT1");
  await page.keyboard.press("ControlOrMeta+v");
  await expect.poll(async () => (await core(page, 1)).outputGainDb[0]).toBe(-9);
  expect(errors).toEqual([]);
});
