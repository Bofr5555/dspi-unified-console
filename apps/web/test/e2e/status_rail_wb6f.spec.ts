// WP WB6f e2e: the status rail's health row and panel, in Chromium, Firefox and
// Chromium at 375 px. On the core's demo device through the embedding page: the
// device, the preset and its unsaved state, the two cores' load, a clip that ends
// and stays listed until acknowledged, a clip acknowledged while on. A lost DSPi Link on dspi-link's fake hub (DSPI_FAKEHUB, as
// link.spec.ts), in CG10's words: raised while the core reconnects, kept after it
// is back until acknowledged.
//
// Screenshots for the report, dark and light, when WB6F_SHOTS names a directory.

import { mkdirSync } from "node:fs";
import { expect, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { openPage, rail as sections } from "../nav";

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };

const linkWords = (await import("../../../../ci/generated/web/link-words.json", { with: { type: "json" } })).default as {
  link: { key: string; text: string; action: string | null }[];
};
const words = (key: string) => linkWords.link.find((w) => w.key === key)!;

function core(page: Page, method: string, params: unknown = null, n = 0) {
  return page.evaluate(
    ([n, method, params]) => (globalThis as unknown as Clients).dspiClients[n as number]!.call(method as string, params),
    [n, method, params] as const,
  );
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    // A browser logs a WebSocket that fails or drops; that is the Link test's own doing.
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

async function shot(page: Page, name: string, of: Locator) {
  const dir = process.env.WB6F_SHOTS;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    await of.screenshot({ path: `${dir}/wb6f-${project}-${name}-${scheme}.png` });
  }
  await page.emulateMedia({ colorScheme: null });
}

const rail = (s: Locator) => s.getByRole("complementary", { name: "Status" });
const healthButton = (s: Locator) => rail(s).getByRole("button", { name: /^Health:/ });
const panel = (s: Locator) => rail(s).getByRole("region", { name: "Health" });

async function openDemo(page: Page) {
  const narrow = test.info().project.name === "narrow";
  await page.goto(`/test/embed.html${narrow ? "?one" : ""}`);
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.locator(".volume-value")).toBeVisible();
  return session;
}

test("WB6F rail: the device, the preset and its unsaved changes, the two cores' load, health OK", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  const r = rail(session);
  const snap = (await core(page, "state")) as { device: { name: string }; state: { activePresetSlot: number } };
  await expect(r.getByLabel("Device")).toHaveText(snap.device.name);
  await expect(r.getByLabel("Preset")).toContainText(`Preset ${snap.state.activePresetSlot + 1}:`);
  await expect(r.getByLabel("Core load")).toHaveText(/^C0 \d+% · C1 \d+%$/);
  await expect(healthButton(session)).toHaveText("Health: OK");
  await expect(panel(session)).toHaveCount(0);
  // A change makes the preset unsaved (an output trim, as main.spec.ts): the rail says so.
  await expect(r.getByLabel("Preset")).not.toContainText("unsaved");
  const slider = session.getByRole("region", { name: "Outputs" }).getByRole("slider").first();
  await slider.fill("-6");
  await slider.dispatchEvent("change");
  await expect(r.getByLabel("Preset")).toContainText("unsaved changes");
  // The panel opens on request, empty.
  await healthButton(session).click();
  await expect(panel(session)).toContainText("No problems.");
  await panel(session).getByRole("button", { name: "Hide" }).click();
  await expect(panel(session)).toHaveCount(0);
  // No page scrolls sideways for the rail.
  const wide = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(wide).toBeLessThanOrEqual(0);
  await shot(page, "rail", r);
  expect(errors).toEqual([]);
});

// -- fixup WB6F -- a demo clip ends once the core's clip hold is over (the fake's
// REQ_CLEAR_CLIPS clears the latched flags): it stays listed until acknowledged.
test("WB6F health: a clip that ends stays listed, marked ended, until acknowledged", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // A short clip hold, so the clip ends within the test.
  await core(page, "setMeterBallistics", { holdMs: 1000, decayPerS: 0.5, clipHoldMs: 800 });
  const names = ((await core(page, "state")) as { state: { channelNames: string[] } }).state.channelNames;
  await core(page, "demo", { action: "clip", mask: 1 });
  const p = panel(session);
  const item = p.getByRole("listitem").filter({ hasText: `Clipping on ${names[0]}` });
  await expect(item).toBeVisible({ timeout: 10_000 });
  await expect(item).toHaveAttribute("data-state", "ended", { timeout: 10_000 });
  await expect(item).toContainText(/since \d\d:\d\d:\d\d · ended \d\d:\d\d:\d\d/);
  await expect(healthButton(session)).toHaveText("Health: 1 problem");
  // It stays: still listed a while later, and on another screen.
  await page.waitForTimeout(1500);
  await openPage(session, "EQ");
  await expect(item).toHaveAttribute("data-state", "ended");
  await shot(page, "clip-ended", rail(session));
  await item.getByRole("button", { name: "Acknowledge" }).click();
  await expect(p.getByRole("listitem")).toHaveCount(0);
  await expect(healthButton(session)).toHaveText("Health: OK");
  expect(errors).toEqual([]);
});

test("WB6F health: a clip opens the panel; acknowledged while on, it stays marked; a new one opens it again", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // A long clip hold: the clip stays on for the whole test.
  await core(page, "setMeterBallistics", { holdMs: 1000, decayPerS: 0.5, clipHoldMs: 60_000 });
  const names = ((await core(page, "state")) as { state: { channelNames: string[] } }).state.channelNames;
  await core(page, "demo", { action: "clip", mask: 1 });
  const p = panel(session);
  const item = p.getByRole("listitem").filter({ hasText: `Clipping on ${names[0]}` });
  await expect(item).toBeVisible({ timeout: 10_000 });
  await expect(item).toHaveAttribute("data-state", "on");
  await expect(item).toContainText(/since \d\d:\d\d:\d\d · now/);
  await expect(healthButton(session)).toHaveText("Health: 1 problem");
  await expect(healthButton(session)).toHaveAttribute("aria-expanded", "true");
  await shot(page, "clip-on", rail(session));
  // Another screen: still there (the feed is the session's, not a screen's).
  await openPage(session, "EQ");
  await expect(item).toBeVisible();
  await item.getByRole("button", { name: "Acknowledge" }).click();
  await expect(item).toHaveAttribute("data-state", "acknowledged");
  await expect(item.getByRole("button", { name: "Acknowledge" })).toHaveCount(0);
  await expect(healthButton(session)).toHaveText("Health: 1 acknowledged");
  // Hidden, the panel opens again by itself for a new problem.
  await p.getByRole("button", { name: "Hide" }).click();
  await expect(p).toHaveCount(0);
  await core(page, "demo", { action: "clip", mask: 0b10 });
  await expect(p.getByRole("listitem").filter({ hasText: `Clipping on ${names[1]}` })).toBeVisible({ timeout: 10_000 });
  await expect(healthButton(session)).toHaveText("Health: 1 problem");
  await shot(page, "clip-two", rail(session));
  expect(errors).toEqual([]);
});

test("WB6F health: Acknowledge All marks every problem that is still on", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // A long clip hold: the clip stays on for the whole test.
  await core(page, "setMeterBallistics", { holdMs: 1000, decayPerS: 0.5, clipHoldMs: 60_000 });
  await core(page, "demo", { action: "clip", mask: 0b11 });
  const p = panel(session);
  await expect(p.getByRole("listitem")).toHaveCount(2, { timeout: 10_000 });
  await expect(healthButton(session)).toHaveText("Health: 2 problems");
  await p.getByRole("button", { name: "Acknowledge All" }).click();
  const items = p.getByRole("listitem");
  await expect(items).toHaveCount(2);
  for (const i of await items.all()) {
    await expect(i).toHaveAttribute("data-state", "acknowledged");
    await expect(i.getByRole("button", { name: "Acknowledge" })).toHaveCount(0);
  }
  await expect(healthButton(session)).toHaveText("Health: 2 acknowledged");
  expect(errors).toEqual([]);
});

// ---------------------------------------------------------------- DSPi Link

const HUB = `http://127.0.0.1:${Number(process.env.HUB_PORT ?? 8738)}`;
const SERIAL = "0123456789ABCDEF";

async function newHub(request: APIRequestContext, args: string): Promise<number> {
  const r = await request.post(`${HUB}/__hub?args=${encodeURIComponent(args)}`);
  expect(r.ok()).toBe(true);
  return ((await r.json()) as { id: number }).id;
}

async function fault(request: APIRequestContext, id: number, line: string): Promise<void> {
  const r = await request.post(`${HUB}/__fault?hub=${id}`, { data: line });
  expect(await r.text()).toBe("ok");
}

test("WB6F health: a lost DSPi Link in CG10's words, kept after the reconnect until acknowledged", async ({ page, request }) => {
  test.skip(!process.env.DSPI_FAKEHUB, "no fake hub (set DSPI_FAKEHUB)");
  const errors = errorsOf(page);
  const id = await newHub(request, "--auth none");
  await page.goto(`${HUB}/?hub=${id}`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: "Hub 127.0.0.1:" });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article");
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: SERIAL.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(SERIAL);
  await expect(healthButton(session)).toHaveText("Health: OK");

  await fault(request, id, "refuse 503");
  await fault(request, id, "drop");
  const item = panel(session).getByRole("listitem").filter({ hasText: `DSPi Link: ${words("reconnecting").text}` });
  await expect(item).toBeVisible({ timeout: 10_000 });
  await expect(item).toHaveAttribute("data-state", "on");
  await shot(page, "dlink-lost", rail(session));

  // Back by itself: the banner goes, the problem stays until someone has seen it.
  await fault(request, id, "refuse ok");
  await expect(session.locator(".banner")).toHaveCount(0, { timeout: 20_000 });
  await expect(item).toHaveAttribute("data-state", "ended");
  await item.getByRole("button", { name: "Acknowledge" }).click();
  await expect(healthButton(session)).toHaveText("Health: OK");
  expect(errors).toEqual([]);
});
