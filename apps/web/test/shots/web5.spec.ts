// -- WP WEB5 -- Before/after pictures of the hub, hub settings and installer pages (web parity with
// the Modern design): on the demo DSPi with no hub (the installer, this console's copy), and on
// `uconsole --demo hub --web` (DSPI_TERM) as an Admin with an offline DSPi, an alarm, the rollout
// and an empty hub (no alarms, no schedules); the hub page once the hub stops answering. Dark and
// light. Run alone through tools/shots/web.sh:
//
//   PW_ARGS="test/shots/web5.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `<project>-<name>-<scheme>.png` in $SHOTS_DIR; `-full` ones are the whole page.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

const DEMO = "DEMO0000DSPI0001";
const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

async function shot(page: Page, name: string, full = false) {
  const only = process.env.SHOTS_ONLY;
  if (only && !new RegExp(only).test(name)) return;
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/${project}-${name}-${scheme}.png`, fullPage: full });
  }
  await page.emulateMedia({ colorScheme: null });
}

/** The page's own scroll box to its top, then the viewport and the whole page. */
async function both(page: Page, session: Locator, name: string) {
  await session.evaluate((e) => e.querySelectorAll<HTMLElement>("*").forEach((x) => x.scrollTop && (x.scrollTop = 0)));
  await shot(page, name);
  await shot(page, `${name}-full`, true);
}

async function setMode(session: Locator, page: Page, mode: "Simple" | "Advanced" | "Installer") {
  await session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ }).click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

type OwnHub = { port: number; child: ChildProcess };

function store(seed: boolean): string {
  const dir = `${process.env.TMPDIR ?? "test-results"}/web5-${process.pid}-${Math.random().toString(36).slice(2)}`;
  mkdirSync(dir, { recursive: true });
  if (seed) {
    const seen = Math.floor(Date.parse("2026-11-10T09:30:00Z") / 1000);
    writeFileSync(
      `${dir}/inventory.json`,
      JSON.stringify({
        v: 1,
        devices: [{ serial: "DEMO-OFFLINE-0042", name: "Bar", platform: 1, fw: "1.1.6", outputs: 9, location: "Bar", first_seen_s: seen - 86400, last_seen_s: seen }],
      }),
    );
    writeFileSync(`${dir}/alarms.json`, JSON.stringify({ v: 1, rules: { temperature: { on: true, above_c: 0 } } }));
  }
  return dir;
}

async function startHub(extra: string[], seed: boolean): Promise<OwnHub> {
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", "--open", "--forget-clients", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", store(seed)],
    { stdio: ["ignore", "pipe", "inherit"] },
  );
  const lines = createInterface({ input: child.stdout! });
  const port = await new Promise<number>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      try {
        const e = JSON.parse(line) as { event?: string; port?: number };
        if (e.event === "status" && e.port) ok(e.port);
      } catch {
        // not a JSON line
      }
    });
  });
  return { port, child };
}

async function stopHub(h: OwnHub | undefined): Promise<void> {
  if (!h || h.child.exitCode !== null) return;
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** The console served by the hub: the demo DSPi opened from the hub's list. */
async function openOnHub(page: Page, hub: OwnHub, scheme: "http" | "https"): Promise<Locator> {
  await page.goto(`${scheme}://127.0.0.1:${hub.port}/console/`);
  const online = picker(page).getByRole("listitem").filter({ hasText: "· online" });
  if (await online.count()) {
    await online.getByRole("button", { name: "Connect" }).click();
  } else {
    await picker(page).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` }).getByRole("button", { name: "Connect" }).click();
    const s = page.getByRole("article").last();
    await s.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) }).getByRole("button", { name: "Open" }).click();
  }
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-serial", new RegExp(DEMO));
  return session;
}

test("the installer on the demo DSPi (this console's copy)", async ({ page }) => {
  test.setTimeout(120_000);
  await page.goto("/");
  await picker(page).getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-status", "open");
  await setMode(session, page, "Installer");
  await openPage(session, "Installer");
  await expect(session.getByRole("region", { name: "Limits" })).toBeVisible();
  await page.waitForTimeout(800);
  await both(page, session, "installer");
});

test.describe("on a hub", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  test.use({ ignoreHTTPSErrors: true });
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    await stopHub(hub);
    hub = undefined;
  });

  test("Admin: the hub page with an offline DSPi, an alarm and the rollout; the hub's settings; the installer on the hub's copy; the hub gone", async ({ page }) => {
    test.setTimeout(240_000);
    hub = await startHub(["--tls", "on", "--rollout", "--allow-unverified-firmware", "--with-rollout-bench"], true);
    const session = await openOnHub(page, hub, "https");
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub");
    const board = session.getByRole("region", { name: "Alarms" });
    await expect(board.getByRole("listitem").filter({ hasText: "Running hot" }).first()).toBeVisible({ timeout: 30_000 });
    await expect(session.getByRole("region", { name: "Devices on the hub" }).getByRole("row").filter({ hasText: DEMO })).toContainText("°C", { timeout: 20_000 });
    await page.waitForTimeout(800);
    await both(page, session, "hub");
    await openPage(session, "Hub Settings");
    await page.waitForTimeout(1500);
    await both(page, session, "hub-settings");
    await setMode(session, page, "Installer");
    await openPage(session, "Installer");
    await expect(session.getByRole("region", { name: "Limits" })).toBeVisible();
    await page.waitForTimeout(1500);
    await both(page, session, "installer-hub");
    // The hub stops: what the hub page says then
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub");
    await stopHub(hub);
    hub = undefined;
    await expect(session.getByRole("status").filter({ hasText: "Not connected to the hub" }).or(session.getByText("Connection lost")).first()).toBeVisible({ timeout: 10_000 });
    await page.waitForTimeout(1500);
    await shot(page, "hub-gone");
  });

  test("Admin on an empty hub: no alarms, no groups, no schedules", async ({ page }) => {
    test.setTimeout(120_000);
    hub = await startHub([], false);
    const session = await openOnHub(page, hub, "http");
    await setMode(session, page, "Advanced");
    await openPage(session, "Hub");
    await expect(session.getByRole("region", { name: "Devices on the hub" })).toBeVisible();
    await page.waitForTimeout(1500);
    await both(page, session, "hub-empty");
    await openPage(session, "Hub Settings");
    await page.waitForTimeout(1500);
    await both(page, session, "hub-settings-empty");
  });
});
