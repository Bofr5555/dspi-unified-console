// -- INT016 -- The README's two web pictures (docs/images/web-eq.png, web-system.png), in the Modern
// web console: the core's demo DSPi on Equaliser › PEQ, and on `uconsole --demo hub --alink` (a
// sender, a fan-out, four units and the system model) unit DEMO-ALINK-A's System › Units as Admin
// (an open hub). They came from the old screenshot tour (tour.spec.ts), which predated the Modern
// console and is gone; parity_web.spec.ts pictures every page and part on the demo. Run through
// tools/shots/readme.sh, or alone:
//
//   PW_ARGS="test/shots/readme_web.spec.ts --project desktop" tools/shots/web.sh <out dir>
//
// Files: `web-<project>-<name>-<scheme>.png` in $SHOTS_DIR (the tour's names, which readme.sh
// crops: web-desktop-demo-eq-light.png, web-desktop-hub-system-admin-light.png), dark and light.
// Only pictures: each step waits for what it shows, and fails rather than picture something else.
// With SHOTS_ONLY=<regex> only the pictures whose name matches. DSPI_TERM: the `uconsole` binary.

import { spawn, type ChildProcess } from "node:child_process";
import { mkdirSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openOnHub, openPage } from "../nav";

type OwnHub = { port: number; child: ChildProcess };

/** `uconsole --demo hub --web --alink --open` on a free port, once it is up: every page is Admin. */
async function openHub(): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/readme-shots-${process.pid}-${Math.random().toString(36).slice(2)}`;
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
      if (e.event === "status") ok(e.port ?? 0);
    });
  });
  return { port, child };
}

async function stopHub(h: OwnHub): Promise<void> {
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

async function openDemo(page: Page): Promise<Locator> {
  await page.goto("/");
  const picker = page.getByRole("navigation", { name: "DSPi devices" });
  await picker.getByRole("listitem").filter({ hasText: "Demo DSPi" }).getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  await expect(session).toHaveAttribute("data-status", "open");
  return session;
}

/** `web-<project>-<name>-<scheme>.png`, the whole page, dark then light. */
async function shot(page: Page, name: string) {
  const only = process.env.SHOTS_ONLY;
  if (only && !new RegExp(only).test(name)) return;
  const dir = process.env.SHOTS_DIR ?? "test-results/shots";
  mkdirSync(dir, { recursive: true });
  const project = test.info().project.name;
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.waitForTimeout(250);
    await page.screenshot({ path: `${dir}/web-${project}-${name}-${scheme}.png`, fullPage: true });
  }
  await page.emulateMedia({ colorScheme: null });
}

test("the Equaliser on the core's demo", async ({ page }) => {
  const session = await openDemo(page);
  await openPage(session, "EQ");
  await expect(session.getByRole("application", { name: /^EQ graph, / })).toBeVisible();
  await page.waitForTimeout(400); // the graph's first frames
  await shot(page, "demo-eq");
});

test.describe("the demo hub's audio link installation", () => {
  test.skip(!process.env.DSPI_TERM, "no uconsole binary (set DSPI_TERM)");
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("Admin: the System view", async ({ page }) => {
    hub = await openHub();
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);
    // The Fleet's Open (an open hub lists its DSPis to the page)
    const session = await openOnHub(page, { host: `127.0.0.1:${hub.port}`, serial: "DEMO-ALINK-A" });
    // Settled: connected, Link bound (its banner gone), the installation's first status in
    const rail = session.getByRole("complementary", { name: "Status" });
    await expect(rail.getByLabel("Connection")).toHaveText(/^Connected/);
    await expect(session.locator(".banner")).toHaveCount(0);
    await expect(rail.getByRole("button", { name: "System mute" })).toHaveText(/^System mute: (on|off)$/);
    await openPage(session, "System");
    const system = session.getByRole("region", { name: "System" });
    await expect(system.getByRole("region", { name: "Room Main room" }).getByLabel("Unit Main left")).toContainText("Audio link: Locked");
    await expect(system.getByRole("list", { name: "Topology" }).getByLabel("Fan-out Fan-out")).toContainText("+3 frames");
    await expect(rail.getByLabel("Alarms")).toHaveText("No alarms");
    await shot(page, "hub-system-admin");
  });
});
