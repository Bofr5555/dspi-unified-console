// WP P2WB2 e2e: the hub settings page and the shared snapshots, on a real hub (`uconsole --demo hub`,
// DSPI_TERM) serving the console, with the demo DSPi behind it.
//
// - An Admin (open access): a webhook target saved and tested against this test's own listener (the
//   secret shown once, the delivery signed with it), a schedule saved and run now (its results), an
//   API key minted (shown once) and revoked, the alarm rules saved; a snapshot saved to the hub,
//   listed as the hub's and recalled. No sideways scroll at 375 px.
// - A Control client (PIN pairing): the page read-only with the core's row, the Admin-only sections
//   as the hub answers them, nothing changed on the hub.
// - The demo DSPi straight (no hub): no hub settings page, no "On the hub" section.

import { spawn, type ChildProcess } from "node:child_process";
import { createHmac } from "node:crypto";
import { mkdirSync } from "node:fs";
import { createServer, type Server } from "node:http";
import type { AddressInfo } from "node:net";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { hasPage, openPage } from "../nav";

const DEMO = "DEMO0000DSPI0001";

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
  expect(over).toBeLessThanOrEqual(0);
}

// -- A hub of the test's own --

type OwnHub = { port: number; pin: string; child: ChildProcess };

function newStore(): string {
  const store = `${process.env.TMPDIR ?? "test-results"}/p2wb2-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(store, { recursive: true });
  return store;
}

async function startHub(extra: string[], pairing: boolean): Promise<OwnHub> {
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", newStore()],
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
  return { port, pin, child };
}

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** The console served by the hub: connect (pairing with its PIN when it asks), open the demo DSPi. */
async function openHub(page: Page, hub: OwnHub): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (hub.pin) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));
  return session;
}

// -- The webhook receiver --

type Hit = { headers: Record<string, string | string[] | undefined>; body: string };

async function listener(): Promise<{ server: Server; port: number; hits: Hit[] }> {
  const hits: Hit[] = [];
  const server = createServer((req, res) => {
    let body = "";
    req.on("data", (c: Buffer) => (body += c.toString("utf8")));
    req.on("end", () => {
      hits.push({ headers: req.headers, body });
      res.writeHead(200, { "content-type": "text/plain" });
      res.end("ok");
    });
  });
  await new Promise<void>((r) => server.listen(0, "127.0.0.1", () => r()));
  return { server, port: (server.address() as AddressInfo).port, hits };
}

if (!process.env.DSPI_TERM) {
  test("P2WB2: the Terminal's binary is there", () => {
    test.skip(!process.env.DSPI_TERM_REQUIRED, "no uconsole binary (set DSPI_TERM)");
    throw new Error("DSPI_TERM_REQUIRED, but no DSPI_TERM");
  });
}

test.describe("P2WB2 on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  let server: Server | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
    if (server) await new Promise<void>((r) => server!.close(() => r()));
    server = undefined;
  });

  test("P2WB2 Admin: a webhook saved and tested, a schedule run now, a key minted and revoked; a snapshot saved to the hub and recalled", async ({ page }) => {
    test.setTimeout(150_000);
    const errors = errorsOf(page);
    const recv = await listener();
    server = recv.server;
    hub = await startHub(["--open", "--forget-clients"], false);
    const session = await openHub(page, hub);
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub Settings");
    const screen = session.locator(".hub-settings-screen");
    await expect(screen.locator(".hub-role")).toContainText("Admin");
    await expect(screen.getByRole("note", { name: "Read-only" })).toHaveCount(0);

    // Notifications: a webhook to this test's listener, unencrypted (a loopback address).
    const out = screen.getByRole("region", { name: "Notifications" });
    await expect(out).toContainText("No targets");
    await out.getByRole("button", { name: "Add Target" }).click();
    const target = out.locator("fieldset.hub-target").first();
    await target.getByLabel("Name").fill("Desk");
    await target.getByLabel("URL").fill(`http://127.0.0.1:${recv.port}/hook`);
    await target.getByLabel("Unencrypted (http)").check();
    await expect(out.locator(".hub-plaintext")).toContainText("private");
    await out.getByRole("button", { name: "Save Targets" }).click();
    const secretCard = out.getByRole("status", { name: "New secret" });
    await expect(secretCard).toContainText("Desk");
    const secret = (await secretCard.locator("code").textContent()) ?? "";
    expect(secret).toMatch(/^[0-9a-f]{64}$/);
    await noSidewaysScroll(page);
    await secretCard.getByRole("button", { name: "Done" }).click();
    await expect(secretCard).toHaveCount(0);
    const saved = out.getByRole("group", { name: "Desk" });
    await expect(saved).toContainText("Unencrypted");
    await saved.getByRole("button", { name: "Test" }).click();
    await expect(saved.getByRole("status", { name: "Test result" })).toContainText("Test delivered");
    await expect.poll(() => recv.hits.length).toBeGreaterThan(0);
    const hit = recv.hits[recv.hits.length - 1]!;
    const ts = String(hit.headers["x-dspi-timestamp"]);
    const signed = `sha256=${createHmac("sha256", secret).update(`${ts}.${hit.body}`).digest("hex")}`;
    expect(hit.headers["x-dspi-signature"]).toBe(signed);
    expect(JSON.parse(hit.body).kind).toBe("test");
    await expect(saved).toContainText("Delivered");

    // Schedules: a rule for the demo DSPi, saved and run now.
    const sched = screen.getByRole("region", { name: "Schedules" });
    await expect(sched).toContainText("No schedules yet");
    await sched.getByRole("button", { name: "New Schedule…" }).click();
    const editor = sched.getByRole("form", { name: "Schedule rule" });
    await editor.getByLabel("Name").fill("Evening");
    await editor.getByLabel("Action 1 dB").fill("-30");
    await editor.getByLabel("Action 1 dB").dispatchEvent("change");
    await expect(editor.getByLabel("Targets")).toHaveValue(DEMO);
    await editor.getByRole("button", { name: "Save Schedule" }).click();
    const rule = sched.getByRole("list", { name: "Schedule rules" }).getByRole("listitem", { name: "Evening" });
    await expect(rule).toContainText("Mon, Tue, Wed, Thu, Fri at 18:00");
    await rule.getByRole("button", { name: "Run Now" }).click();
    const run = sched.getByRole("status", { name: "Schedule run" });
    await expect(run).toContainText("“Evening” ran");
    await expect(run).toContainText(`${DEMO}: 0:param.set: done`);
    await noSidewaysScroll(page);
    await run.getByRole("button", { name: "Done" }).click();

    // API keys: minted (shown once) and revoked.
    const keys = screen.getByRole("region", { name: "API keys" });
    await keys.getByLabel("Key name").fill("Script");
    await keys.getByLabel("Key role").selectOption("viewer");
    await keys.getByRole("button", { name: "Mint Key" }).click();
    const keyCard = keys.getByRole("status", { name: "New API key" });
    await expect(keyCard).toContainText("Script");
    expect(((await keyCard.locator("code").textContent()) ?? "").length).toBe(43);
    await keyCard.getByRole("button", { name: "Done" }).click();
    await expect(keyCard).toHaveCount(0);
    const keyRow = keys.getByRole("list", { name: "API keys on the hub" }).getByRole("listitem", { name: "Script" });
    await expect(keyRow).toContainText("Viewer");
    await keyRow.getByRole("button", { name: "Revoke Script" }).click();
    await page.getByRole("dialog", { name: "Revoke API Key" }).getByRole("button", { name: "Revoke" }).click();
    await expect(keyRow).toHaveCount(0);

    // Alarm rules: a threshold saved.
    const alarms = screen.getByRole("region", { name: "Alarm rules" });
    const cpu = alarms.getByRole("group", { name: "Processing load high" });
    await cpu.getByLabel("At or above (%)").fill("80");
    await cpu.getByLabel("At or above (%)").dispatchEvent("change");
    await alarms.getByRole("button", { name: "Save Alarm Rules" }).click();
    await expect(alarms.getByRole("status")).toContainText("saved");
    await expect(cpu.getByLabel("At or above (%)")).toHaveValue("80");

    // MQTT: shown, off.
    await expect(screen.getByRole("region", { name: "MQTT and Home Assistant" }).getByRole("status", { name: "Broker connection" })).toContainText("Off");

    // Shared snapshots: the DSPi now, saved to the hub, listed as the hub's, recalled.
    await openPage(session, "Snapshots");
    const onHub = session.getByRole("region", { name: "On the hub" });
    await expect(onHub).toContainText("No snapshots on the hub");
    await onHub.getByRole("button", { name: "Save to the Hub" }).click();
    await expect(onHub.getByRole("status")).toContainText("to the hub");
    const snap = onHub.getByRole("list", { name: "Snapshots on the hub" }).getByRole("listitem").first();
    await expect(snap).toHaveAttribute("data-where", "hub");
    await expect(snap).toContainText("On the hub");
    await noSidewaysScroll(page);
    await snap.getByRole("button", { name: "Recall" }).click();
    const ask = page.getByRole("dialog", { name: "Recall Snapshot" });
    const keep = session.getByRole("alert", { name: "Safe apply" }).getByRole("button", { name: "Keep" });
    const result = session.getByRole("status", { name: "Result" });
    await expect(keep.or(result).or(ask)).toBeVisible();
    if (await ask.isVisible()) await ask.getByRole("button", { name: "Recall" }).click();
    await expect(keep.or(result)).toBeVisible();
    if (await keep.isVisible()) await keep.click();
    await expect(result).toBeVisible();
    expect(errors).toEqual([]);
  });

  test("P2WB2 Control: the settings page is read-only with the core's row; Admin-only sections as the hub answers", async ({ page }) => {
    test.setTimeout(90_000);
    const errors = errorsOf(page);
    hub = await startHub(["--pair"], true);
    const session = await openHub(page, hub);
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub Settings");
    const screen = session.locator(".hub-settings-screen");
    const ro = screen.getByRole("note", { name: "Read-only" });
    await expect(ro).toHaveAttribute("data-key", "installer.hub_settings_role");
    await expect(ro).toContainText("Read-only: Admin needed");
    await expect(ro).toContainText("uconsole hub role <cid> admin");
    await expect(screen.locator(".hub-role")).toContainText("Control");
    // Every role reads the alarm rules and the schedule: shown, not editable.
    const alarms = screen.getByRole("region", { name: "Alarm rules" });
    await expect(alarms.getByRole("checkbox").first()).toBeDisabled();
    await expect(alarms.getByRole("button", { name: "Save Alarm Rules" })).toBeDisabled();
    await expect(screen.getByRole("region", { name: "Schedules" }).getByRole("button", { name: "New Schedule…" })).toBeDisabled();
    // The Admin-only reads: the hub's refusal, and no form to send.
    await expect(screen.getByRole("region", { name: "Notifications" }).getByRole("alert")).toBeVisible();
    await expect(screen.getByRole("region", { name: "Notifications" }).getByRole("button", { name: "Add Target" })).toHaveCount(0);
    await expect(screen.getByRole("region", { name: "MQTT and Home Assistant" }).getByRole("alert")).toBeVisible();
    const keys = screen.getByRole("region", { name: "API keys" });
    await expect(keys.getByRole("alert")).toBeVisible();
    await expect(keys.getByRole("button", { name: "Mint Key" })).toBeDisabled();
    await noSidewaysScroll(page);
    expect(errors).toEqual([]);
  });
});

test("P2WB2 direct: the demo DSPi has no hub settings page and no On the hub section", async ({ page }) => {
  const errors = errorsOf(page);
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article").first();
  await expect(session.getByRole("button", { name: /^Presets: / })).toBeVisible(); // -- WP INTD -- the session bar's presets (DL5)
  await setMode(session, page, "Advanced");
  expect(await hasPage(session, "Events")).toBe(true);
  expect(await hasPage(session, "Hub Settings")).toBe(false);
  await openPage(session, "Snapshots");
  await expect(session.getByRole("region", { name: "Snapshots" })).toBeVisible();
  await expect(session.getByRole("region", { name: "On the hub" })).toHaveCount(0);
  expect(errors).toEqual([]);
});
