// WP P1WB2 e2e: the Installer page (limits with their clamp report, a refused write's notice,
// protect all outputs, power-on), tokens on, Clear All as one undo, AutoEQ's preview before
// Apply, and the mode switch from the core's manifest; on the core's demo device in every
// project (Chromium, Firefox, Chromium at 375 px, without sideways scroll). Then the lock and a
// pinned mode: through `uconsole --demo hub` with a PIN and a pinned mode in its store (the
// locked-user view, the one unlock prompt naming the hub), and through dspi-link's fake hub
// with a fleet backend that pins the mode (test/pages/fleet-hub.ts).

import { spawn, type ChildProcess } from "node:child_process";
import { pbkdf2Sync, randomBytes } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { hasPage, openPage, rail as sections } from "../nav";
import { presetsButton, presetsReady } from "../presets"; // -- WP INTD --

type Clients = { dspiClients: { call(m: string, p: unknown): Promise<unknown> }[] };
type State = { outputGainDb: number[]; presetDefaultSlot: number; presetStartupMode: number; activePresetSlot: number; filters: { filterType: number }[][]; limiterEnabled: boolean[]; numOutputChannels: number };

async function call<T>(page: Page, method: string, params: unknown = null, n = 0): Promise<T> {
  return (await page.evaluate(([n, m, p]) => (globalThis as unknown as Clients).dspiClients[n as number]!.call(m as string, p), [n, method, params] as const)) as T;
}
const coreState = async (page: Page, n = 0) => (await call<{ state: State }>(page, "state", null, n)).state;

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && !/WebSocket/i.test(m.text())) errors.push(m.text());
  });
  return errors;
}

/** Pictures for the report (Chromium and narrow), when P1WB2_SHOTS names a directory. */
async function shot(page: Page, name: string, of?: Locator) {
  const dir = process.env.P1WB2_SHOTS;
  const project = test.info().project.name;
  if (!dir || project === "firefox") return;
  mkdirSync(dir, { recursive: true });
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(150);
    const path = `${dir}/p1wb2-${project}-${name}-${scheme}.png`;
    if (of) await of.screenshot({ path });
    else await page.screenshot({ path, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await presetsReady(session); // -- WP INTD --
  return session;
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

test("P1WB2 Installer: a limit narrowed below the device is held and reported; a write past it shows the notice", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await call(page, "write", { path: "out.gain", indices: [1], value: { float: 8 } });
  // The mode switch is the manifest's: its three modes.
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await expect(dialog.getByRole("radio")).toHaveCount(3);
  await dialog.getByRole("button", { name: "Cancel" }).click();
  expect(await hasPage(session, "Installer")).toBe(false);
  await setMode(session, page, "Installer");
  await openPage(session, "Installer");
  const limits = session.getByRole("region", { name: "Limits" });
  // No PIN on the web, nothing locked, nothing asked.
  await expect(session.getByRole("region", { name: "Installer lock" })).toContainText("No installer PIN is set: nothing is locked.");
  await expect(page.getByRole("dialog", { name: "Unlock" })).toHaveCount(0);

  const out2 = limits.getByRole("group", { name: /^OUT2/ });
  const highest = out2.getByLabel("Gain at most (dB)");
  // -- WP P1EFIX -- the ranges are the core's one table (-- WP WEB -- on the text field, which reads "−2,5 dB" as Qt's fields do)
  await expect(highest).toHaveAttribute("data-max", "20");
  await expect(highest).toHaveAttribute("data-min", "-60");
  await expect(limits.getByLabel("Master volume at most (dB)")).toHaveAttribute("data-min", "-128");
  await expect(out2.getByLabel("Delay at most (ms)")).toHaveAttribute("data-max", "85");
  await highest.fill("2,0 dB");
  await highest.dispatchEvent("change");
  const report = limits.getByRole("status", { name: "Limits report" });
  await expect(report).toContainText("Held to the new limits");
  await expect(report).toContainText("out.gain[1] 8 → 2 dB");
  await expect.poll(async () => (await coreState(page)).outputGainDb[1]).toBe(2);
  await shot(page, "limits-held", limits);
  await noSidewaysScroll(page);

  // A write past it: refused, in the notice table's words.
  const r = await page.evaluate(() =>
    (globalThis as unknown as Clients).dspiClients[0]!.call("write", { path: "out.gain", indices: [1], value: { float: 6 } }).then(
      () => "written",
      (e: { noticeKey?: string }) => e.noticeKey,
    ),
  );
  expect(r).toBe("limit_exceeded");
  // -- WP INTD -- OUT2's gain typed on Delay & gain (the Overview's level cards are gone)
  await openPage(session, "Delay & gain");
  const field = session.getByRole("region", { name: "Delay and gain of every output" }).getByRole("row", { name: /^OUT2 / }).getByRole("textbox", { name: /gain$/ });
  await field.fill("6");
  await field.press("Enter");
  const alert = session.locator("p.error.notice");
  await expect(alert).toHaveAttribute("data-key", "limit_exceeded");
  await expect(alert).toContainText("Outside the limits.");
  await shot(page, "limit-refused-notice", alert);

  // Kept by this page for the device: open it again and the core holds it at once.
  await page.reload();
  const again = await openDemo(page);
  await expect.poll(async () => (await call<{ text: string }>(page, "limitsProfile")).text).toContain('"maxGainDb": 2');
  await setMode(again, page, "Installer");
  await openPage(again, "Installer");
  await expect(again.getByRole("region", { name: "Limits" }).getByRole("group", { name: /^OUT2/ }).getByLabel("Gain at most (dB)")).toHaveValue("2");
  await again.getByRole("region", { name: "Limits" }).getByRole("button", { name: "Remove All Limits" }).click();
  await expect.poll(async () => (await call<{ text: string }>(page, "limitsProfile")).text).toBe("");
  expect(errors).toEqual([]);
});

test("P1WB2 Installer: outputs without a limiter, Protect all outputs; power-on applies at once; Test power-on", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await call(page, "write", { path: "limit.on", indices: [255], value: { bool: false } });
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: 0 } });
  await setMode(session, page, "Installer");
  await openPage(session, "Installer");
  const limits = session.getByRole("region", { name: "Limits" });
  const warning = limits.getByRole("status", { name: "Outputs without a limiter" });
  await expect(warning).toContainText("above -10 dB of gain");
  await shot(page, "protect-warning", limits);
  await warning.getByRole("button", { name: "Protect all outputs" }).click();
  await expect(warning).toHaveCount(0);
  const s = await coreState(page);
  expect(s.limiterEnabled.slice(0, s.numOutputChannels).every(Boolean)).toBe(true);
  // -- WP P1EFIX -- one undo says the limiters stay on (the core's history note)
  await session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" }).click();
  await expect(session.getByText(/The limiters stay on/)).toBeVisible();
  const kept = await coreState(page);
  expect(kept.limiterEnabled.slice(0, kept.numOutputChannels).every(Boolean)).toBe(true);

  // At power-on: the startup preset, written at once.
  const power = session.getByRole("region", { name: "At power-on" });
  await power.getByLabel("Startup preset").selectOption("2");
  await expect.poll(async () => (await coreState(page)).presetDefaultSlot).toBe(2);
  await power.getByLabel("Master volume").selectOption("1");
  await expect.poll(async () => ((await coreState(page)) as unknown as { masterVolumeMode: number }).masterVolumeMode).toBe(1);
  await expect(power).toContainText("S/PDIF and ADAT inputs stay muted until they lock to their signal.");
  // The DAC's hardware mute, read back from the device.
  const dac = power.getByRole("group", { name: "DAC hardware mute" });
  if (await dac.count()) {
    await dac.getByRole("checkbox", { name: /Mute the DAC/ }).check();
    const hold = dac.getByLabel("Hold before the clock stops (ms)");
    await hold.fill("25");
    await hold.dispatchEvent("change");
    await expect.poll(async () => (await call<{ holdMs: number }>(page, "dacMuteFetch")).holdMs).toBe(25);
  }
  await shot(page, "power-on", power);
  // A copy of the running preset in slot 3, so loading it is a change the device makes.
  await call(page, "presetCopy", { source: (await coreState(page)).activePresetSlot, dest: 2, token: await call(page, "confirmToken", { op: "copy_preset" }) });
  await power.getByRole("button", { name: "Test Power-on" }).click();
  const ask = page.getByRole("dialog", { name: "Unsaved Changes" });
  if (await ask.isVisible().catch(() => false)) await ask.getByRole("button", { name: "Discard" }).click();
  await expect.poll(async () => (await coreState(page)).activePresetSlot).toBe(2);
  await expect(power.getByRole("status")).toContainText("as at power-on");
  await noSidewaysScroll(page);
  expect(errors).toEqual([]);
});

test("P1WB2 tokens and groups: tokens on, Save asks and saves; an EQ Clear All is one undo", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  // -- WP P1TZ -- tokens are required by the core from the start (no switch to ask)
  // A gated call with no token is refused; the page's own prompt carries one.
  const refused = await page.evaluate(() =>
    (globalThis as unknown as Clients).dspiClients[0]!.call("write", { path: "preset.save", indices: [0], value: "trigger" }).then(
      () => "saved",
      (e: { noticeKey?: string }) => e.noticeKey,
    ),
  );
  expect(refused).toBe("confirmation_required");
  await call(page, "write", { path: "out.gain", indices: [0], value: { float: -2 } });
  await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
  await page.keyboard.press("Control+s");
  await page.getByRole("dialog", { name: "Save Preset" }).getByRole("button", { name: "Save" }).click();
  await expect(presetsButton(session)).not.toHaveAccessibleName(/unsaved/); // -- WP INTD --
  await expect(session.locator("p.error")).toHaveCount(0);

  // Clear All on the EQ: one undo puts every band back.
  await openPage(session, "EQ");
  for (const b of [0, 1, 2]) await call(page, "filter", { channel: 0, band: b, filterType: 1, freq: 300 + 200 * b, q: 1, gain: 4, bypass: false, qp: 1 });
  await call(page, "endGroup");
  const list = session.getByRole("region", { name: /^Bands of IN1/ });
  await list.getByRole("button", { name: /^Clear (\d+\/\d+ )?PEQ$/ }).click(); // -- WP WEB6 -- Qt's "Clear PEQ" ("Clear 1/2 PEQ" on a linked pair)
  const ask = page.getByRole("dialog", { name: "Clear All Bands?" });
  await expect(ask).toContainText("One Undo puts them all back.");
  await ask.getByRole("button", { name: "Clear All" }).click();
  await expect.poll(async () => (await coreState(page)).filters[0]!.slice(0, 3).every((b) => b.filterType === 0)).toBe(true);
  await session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => (await coreState(page)).filters[0]!.slice(0, 3).every((b) => b.filterType === 1)).toBe(true);
  expect(errors).toEqual([]);
});

test("P1WB2 AutoEQ: a profile previewed on the graph writes nothing; Apply writes the pair and keeps the state before", async ({ page }) => {
  const errors = errorsOf(page);
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await session.getByRole("button", { name: "Headphone Profile…" }).click(); // -- WP WEB6 -- Qt's tool on an input's page (was AutoEQ…)
  const panel = session.getByRole("region", { name: "AutoEQ" });
  const profile = { id: "o/Sony A", manufacturer: "Sony", model: "A", preamp: -4, filters: [{ type: "PK", freq: 105, q: 0.7, gain: 3 }] };
  await panel.getByLabel("AutoEQ profile").setInputFiles({ name: "sony-a.json", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(profile)) });
  await expect(panel).toContainText("Sony A");
  await expect(panel.getByLabel(/^Channel pair/)).toHaveValue("0");
  const before = JSON.stringify((await coreState(page)).filters);
  await panel.getByRole("button", { name: "Preview" }).click();
  const preview = panel.getByRole("status", { name: "AutoEQ preview" });
  await expect(preview).toContainText("Nothing is written until Apply.");
  await expect(preview).toContainText("preamp is set to the profile's -4.0 dB");
  expect(JSON.stringify((await coreState(page)).filters)).toBe(before);
  await shot(page, "autoeq-preview");
  await noSidewaysScroll(page);
  await panel.getByRole("button", { name: "Apply" }).click();
  const result = panel.getByRole("status", { name: "AutoEQ result" });
  await expect(result).toContainText("Applied Sony A to IN1 + IN2.");
  expect(JSON.stringify((await coreState(page)).filters)).not.toBe(before);
  const [download] = await Promise.all([page.waitForEvent("download"), result.getByRole("button", { name: "Download the state before" }).click()]);
  expect(download.suggestedFilename()).toBe("before-autoeq.dspipreset");
  // One undo takes it back.
  await session.getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: "Undo" }).click();
  await expect.poll(async () => JSON.stringify((await coreState(page)).filters)).toBe(before);
  expect(errors).toEqual([]);
});

// -- The lock and a pinned mode on a hub --

type OwnHub = { port: number; pin: string; child: ChildProcess; store: string };

async function linkHub(extra: string[], pairing: boolean, store: string): Promise<OwnHub> {
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--alink", "--console", "dist", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", store],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
      if (e.event === "status") {
        port = e.port ?? 0;
        if (!pairing) ok("");
      }
      if (e.event === "pairing" && e.pin) ok(e.pin);
    });
  });
  return { port, pin, child, store };
}

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

const INSTALLER_PIN = "2468";

/**
 * A hub whose store holds, for `serial`, an installer PIN (as the hub keeps it: PBKDF2-SHA256
 * with a random salt and the serial), a profile whose lock covers a page, and Control pinned to
 * Simple: what an Admin sets with `limits.pin`, `limits.set` and `mode.pin`.
 */
async function lockedHub(serial: string): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/p1wb2-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  await stopHub(await linkHub(["--pair"], true, store));
  const file = `${store}/hub-store.json`;
  const data = JSON.parse(readFileSync(file, "utf8")) as Record<string, unknown>;
  const salt = randomBytes(16);
  const iterations = 2000;
  const hash = pbkdf2Sync(INSTALLER_PIN, Buffer.concat([salt, Buffer.from(serial)]), iterations, 32, "sha256");
  data.device_pins = { [serial]: { version: 1, kdf: "pbkdf2-sha256", iterations, salt: salt.toString("hex"), hash: hash.toString("hex") } };
  data.device_limits = { [serial]: { lockedPages: ["channels"], note: "Venue A: volume and presets only, please.", contact: "Installer on duty" } };
  data.role_modes = { control: "simple" };
  writeFileSync(file, JSON.stringify(data, null, 2));
  return linkHub(["--pair"], true, store);
}

async function openUnit(page: Page, hub: OwnHub, serial: string): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await session.getByLabel("PIN").fill(hub.pin);
  await session.getByRole("button", { name: "Pair" }).click();
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(serial);
  return session;
}

test.describe("P1WB2 on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("P1WB2 lock: the locked-user view, a refused write in the hub's words, the one prompt naming the hub; unlocked, the pinned mode", async ({ page }) => {
    test.setTimeout(90_000);
    const errors = errorsOf(page);
    const serial = "DEMO-ALINK-A";
    hub = await lockedHub(serial);
    const session = await openUnit(page, hub, serial);
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByRole("button", { name: "Mode: Locked" })).toBeVisible();
    const banner = session.getByRole("region", { name: "Locked" });
    await expect(banner).toContainText("Locked by the installer: only volume, mute and presets can be changed.");
    await expect(banner).toContainText("Venue A: volume and presets only, please.");
    await expect(sections(session).getByRole("button")).toHaveText(["Overview", "Devices"]);
    await expect(session.getByRole("tab")).toHaveCount(0);
    await shot(page, "locked-user");
    await noSidewaysScroll(page);
    // Volume passes; Bypass is refused, by the hub, in its notice row's words (-- WP INTD -- the
    // Overview's output trims are gone; the dock's Bypass is the write the lock leaves out there).
    const volume = session.getByRole("slider", { name: "Volume" });
    const shown = await session.locator(".volume-value").textContent();
    await volume.fill("100");
    await volume.dispatchEvent("change");
    await expect(session.locator(".volume-value")).not.toHaveText(shown!);
    await expect(session.locator("p.error")).toHaveCount(0);
    await session.getByRole("button", { name: "Bypass", exact: true }).click();
    const refused = session.locator("p.error.notice");
    await expect(refused).toHaveAttribute("data-key", /^installer_lock/);
    await shot(page, "locked-refused", refused);
    // The mode switch says why it is not switchable.
    await rail.getByRole("button", { name: "Mode: Locked" }).click();
    const modes = page.getByRole("dialog", { name: "Mode" });
    await expect(modes).toContainText("Locked by the installer: unlock to switch modes.");
    await expect(modes.getByRole("radio", { name: "Advanced" })).toBeDisabled();
    await modes.getByRole("button", { name: "Cancel" }).click();

    // The one prompt: the hub's PIN, said so.
    await banner.getByRole("button", { name: "Unlock…" }).click();
    const unlock = page.getByRole("dialog", { name: "Unlock" });
    await expect(unlock.locator(".lock-scope")).toContainText("hub");
    await unlock.getByLabel("Installer PIN").fill("0000");
    await unlock.getByRole("button", { name: "Unlock" }).click();
    await expect(unlock.getByRole("alert")).toContainText("Wrong PIN");
    await shot(page, "unlock-wrong-pin", unlock);
    await unlock.getByLabel("Installer PIN").fill(INSTALLER_PIN);
    await unlock.getByRole("button", { name: "Unlock" }).click();
    await expect(unlock).toHaveCount(0);
    // Unlocked: the hub still pins Control to Simple.
    await expect(rail.getByRole("button", { name: "Mode: Simple (pinned)" })).toBeVisible();
    await expect(session.getByRole("region", { name: "Locked" })).toHaveCount(0);
    await rail.getByRole("button", { name: /^Mode:/ }).click();
    await expect(page.getByRole("dialog", { name: "Mode" })).toContainText("This console is pinned to Simple for this DSPi.");
    await page.getByRole("dialog", { name: "Mode" }).getByRole("button", { name: "Cancel" }).click();
    // -- WP INTD -- unlocked, the Bypass the lock refused goes through
    await session.getByRole("button", { name: "Bypass", exact: true }).click();
    await expect(session.getByRole("button", { name: "Bypass", exact: true })).toHaveAttribute("aria-pressed", "true");
    await expect(session.locator("p.error")).toHaveCount(0);
    await shot(page, "unlocked-pinned");
    expect(errors).toEqual([]);
  });
});

// -- A fleet backend that pins the mode, through dspi-link's fake hub --

const HUB = `http://127.0.0.1:${Number(process.env.HUB_PORT ?? 8738)}`;
const FAKE_SERIAL = "0123456789ABCDEF";

async function newFakeHub(request: APIRequestContext, args: string): Promise<number> {
  const r = await request.post(`${HUB}/__hub?args=${encodeURIComponent(args)}`);
  expect(r.ok()).toBe(true);
  return ((await r.json()) as { id: number }).id;
}

test.describe("P1WB2 a fleet's pinned mode", () => {
  test.skip(!process.env.DSPI_FAKEHUB, "no fake hub (set DSPI_FAKEHUB)");

  test("P1WB2 fleet: the backend's pinned mode overrides the user's choice, and the switch says so", async ({ page, request }) => {
    const errors = errorsOf(page);
    const id = await newFakeHub(request, "--auth none");
    const addr = `ws://127.0.0.1:${new URL(HUB).port}/h/${id}/dspi/v1`;
    const q = new URLSearchParams({ a: addr, b: addr, device: FAKE_SERIAL, mode: "simple" });
    await page.goto(`${HUB}/test/fleet-hub.html?${q}`);
    await page.getByRole("region", { name: "Site A" }).getByRole("listitem").filter({ hasText: "Stage rack" }).getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article");
    await expect(session.locator(".device-meta")).toContainText(FAKE_SERIAL);
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByRole("button", { name: "Mode: Simple (pinned)" })).toBeVisible();
    expect(await hasPage(session, "System")).toBe(false);
    await rail.getByRole("button", { name: /^Mode:/ }).click();
    const dialog = page.getByRole("dialog", { name: "Mode" });
    await expect(dialog).toContainText("This console is pinned to Simple for this DSPi.");
    await expect(dialog.getByRole("radio", { name: "Installer" })).toBeDisabled();
    await dialog.getByRole("button", { name: "Cancel" }).click();
    await shot(page, "fleet-pinned");
    expect(errors).toEqual([]);
  });
});
