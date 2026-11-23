// WP ODEC e2e: the screens follow an AI agent (owner 2026-11-23, answer 9), on the web as on the
// desktop. Against the real hub (`uconsole --demo hub --web --alink`): another console says on the
// hub that its agent works on DEMO-ALINK-A (presence, as the desktop app does) with a test-signal
// grant in its words; a change reaching this page's session of that unit from elsewhere (a second
// page, standing in for the agent's console) brings this page to the screen it is on. The strip
// shows the grant apart from the work.
//
// The hub helpers are FEATUI's (installation_featui.spec.ts), copied: that spec exports none.

import { spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";
import { openPage } from "../nav";

type OwnHub = { port: number; pin: string; child: ChildProcess };

async function startHub(extra: string[], pairing = false): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/instw-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", store],
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

/** Connect the serving hub (pairing with its PIN when it has one) and open unit `serial`, or the hub's one device. */
async function openOn(page: Page, hub: OwnHub, serial: string): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (hub.pin) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  const meta = session.locator(".device-meta");
  // A hub with one DSPi opens it at once; one with several lists them.
  await expect(entry.or(meta.filter({ hasText: serial }))).toBeVisible();
  if (await entry.isVisible()) await entry.getByRole("button", { name: "Open" }).click();
  await expect(meta).toContainText(serial);
  const rail = session.getByRole("complementary", { name: "Status" });
  await expect(rail.getByLabel("Connection")).toHaveText(/^Connected/);
  await expect(session.locator(".banner")).toHaveCount(0);
  return session;
}

test.describe("ODEC: the screens follow an AI agent", () => {
  test.describe.configure({ mode: "serial" });
  let hub: OwnHub | undefined;
  test.afterEach(async () => {
    if (hub) await stopHub(hub);
    hub = undefined;
  });

  test("ODEC: a change from elsewhere while an agent works shows its page; the grant in the strip", async ({ page, context }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1280, height: 900 });
    hub = await startHub(["--alink", "--open", "--forget-clients"]);
    const session = await openOn(page, hub, "DEMO-ALINK-A");
    await openPage(session, "System");
    await expect(session.locator('.page[data-page="system"]')).toBeVisible();
    // The agent's console says so on the hub
    const ws = new WebSocket(`ws://127.0.0.1:${hub.port}/dspi/v1`, "dspi-link-1");
    const answers: Record<string, unknown>[] = [];
    // Text frames only: the hub's binary data plane is not this test's
    ws.onmessage = (m) => typeof m.data === "string" && answers.push(JSON.parse(m.data) as Record<string, unknown>);
    await new Promise<void>((ok, fail) => {
      ws.onopen = () => ok();
      ws.onerror = () => fail(new Error("no WebSocket"));
    });
    ws.send(JSON.stringify({ t: "hello", proto: { major: 1, minor: 3 }, client: { name: "Studio Mac", app: "uConsole", version: "test" } }));
    await expect.poll(() => answers.some((a) => a.t === "hello")).toBe(true);
    ws.send(
      JSON.stringify({ t: "presence.set", id: 1, kind: "agent", label: "Claude", serial: "DEMO-ALINK-A", what: "Changing the volume · test signals allowed, 9 min left", ttl_s: 60 }),
    );
    await expect.poll(() => answers.find((a) => a.id === 1)?.t).toBe("ok");
    const strip = session.getByRole("status", { name: "AI agent working" });
    await expect(strip).toContainText("is working on this device: Changing the volume");
    await expect(strip.locator(".agent-grant")).toHaveText(/test signals allowed, 9 min left/);
    // The change itself, from another page on the same unit
    const other = await context.newPage();
    const there = await openOn(other, hub, "DEMO-ALINK-A");
    await openPage(there, "Main");
    const slider = there.getByRole("region", { name: "Volume" }).getByRole("slider", { name: "Volume", exact: true });
    await slider.focus();
    // While the person here types in a field, this page stays (only the strip says it)
    await page.evaluate(() => {
      const i = document.createElement("input");
      i.id = "odec-typing";
      i.setAttribute("aria-label", "typing");
      document.querySelector(".dspi-web")!.appendChild(i);
      i.focus();
    });
    await slider.press("ArrowDown");
    await expect.poll(async () => (await there.getByRole("region", { name: "Volume" }).textContent()) ?? "").toBeTruthy();
    await page.waitForTimeout(2_500);
    await expect(session.locator('.page[data-page="system"]')).toBeVisible();
    // Done typing: the next change brings this page to the Dashboard, where the volume is
    await page.evaluate(() => document.getElementById("odec-typing")!.remove());
    await slider.press("ArrowDown");
    await expect(session.locator('.page[data-page="main"]')).toBeVisible({ timeout: 15_000 });
    ws.close();
    await other.close();
  });
});
