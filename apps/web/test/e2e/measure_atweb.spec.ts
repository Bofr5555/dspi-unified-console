// WP ATWEB e2e: the Measure screen's "Several units" step against the real hub: `uconsole --demo
// hub --web --alink --with-measure-units`, WP ATFAKE's units on the time-base contract (three on
// one audio link time base, DEMO-TB-C re-anchoring every 15 s; DEMO-TB-X on another sender's; an
// S/PDIF source and a follower that hears no DSPi time base). Every unit is judged by its own
// connection; the hub's `system.measure.*` starts each unit's (arithmetic) generator on the shared
// counter. The browsers' fake microphone cannot answer the generators, so a run here is planned,
// started (the settle wait in words) and stopped; the result, the proposal and Admin's apply are
// the unit tests' (test/unit/measure_atweb.test.ts) and ATE2E's in Rust. Shown here: the placement
// chosen before anything plays, the plan and its run, Stop reaching every unit, each named notice
// ("needs the DSPi hub", "these units do not share a clock", "unit lost lock", "needs N runs",
// "this S/PDIF source does not carry a DSPi time base"), Control measuring, and 375 px.

import { spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

test.skip(({ browserName }) => browserName === "webkit", "Chromium and Firefox only");
// Chromium's headless shell has no getUserMedia: the full Chromium, headless (as P5WMS).
test.use({ channel: [async ({ browserName }, use) => use(browserName === "chromium" ? "chromium" : undefined), { scope: "worker" }] });

type OwnHub = { port: number; pin: string; child: ChildProcess };

const A = "DEMO-TB-A";
const B = "DEMO-TB-B";
const C = "DEMO-TB-C";
const X = "DEMO-TB-X";

/** `uconsole --demo hub --web --alink --with-measure-units ...` on a free port (with `pairing`, its PIN). */
async function measureHub(extra: string[], pairing = false): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/atweb-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--alink", "--with-measure-units", "--console", "dist", ...extra, "--port", "0", "--json", "--no-keep-awake", "--store", store],
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

const picker = (page: Page) => page.getByRole("navigation", { name: "DSPi devices" });

/** Connect the serving hub, pair (`pair`), open unit `serial`'s session. */
async function openUnit(page: Page, hub: OwnHub, serial: string, pair = !!hub.pin): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = picker(page).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (pair) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  // The list shows a serial's last 8 characters.
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: serial.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(serial);
  const rail = session.getByRole("complementary", { name: "Status" });
  await expect(rail.getByLabel("Connection")).toHaveText(/^Connected/);
  await expect(session.locator(".banner")).toHaveCount(0);
  return session;
}

async function setMode(session: Locator, page: Page, mode: "Advanced" | "Installer") {
  const button = session.getByRole("complementary", { name: "Status" }).getByRole("button", { name: /^Mode:/ });
  if ((await button.textContent())?.includes(mode)) return;
  await button.click();
  const dialog = page.getByRole("dialog", { name: "Mode" });
  await dialog.getByRole("radio", { name: mode }).check();
  await dialog.getByRole("button", { name: "Use" }).click();
  await expect(session.getByRole("button", { name: `Mode: ${mode}` })).toBeVisible();
}

/** The Measure screen's "Several units" tab. */
async function severalUnits(session: Locator, page: Page): Promise<Locator> {
  await setMode(session, page, "Advanced");
  await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Measure" }).click();
  const screen = session.getByRole("region", { name: "Measure" });
  await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Several units" }).click();
  const tab = screen.getByRole("region", { name: "Several units" });
  await expect(tab.getByRole("group", { name: `Unit ${A}` })).toBeVisible();
  return tab;
}

/**
 * Calls on unit `serial` through a core Worker of the test's own, on the page's hub connection
 * (the page's token when it paired): what a person would do on that unit's own page.
 */
async function onUnit<T>(page: Page, hub: OwnHub, serial: string, calls: [string, unknown][]): Promise<T[]> {
  return page.evaluate(
    async ([port, serial, calls]) => {
      const w = new Worker(new URL("worker.js", location.href), { type: "module" });
      let next = 0;
      const pending = new Map<number, { ok: (v: unknown) => void; fail: (e: Error) => void }>();
      w.onmessage = (ev: MessageEvent<{ reply?: { id: number; ok: boolean; value?: unknown; error?: { message: string } } }>) => {
        const r = ev.data.reply;
        if (!r) return;
        const p = pending.get(r.id);
        pending.delete(r.id);
        if (r.ok) p?.ok(r.value);
        else p?.fail(new Error(r.error?.message ?? "refused"));
      };
      const call = (method: string, params: unknown) =>
        new Promise<unknown>((ok, fail) => {
          const id = ++next;
          pending.set(id, { ok, fail });
          w.postMessage({ id, method, params });
        });
      const url = `ws://127.0.0.1:${port}/dspi/v1`;
      let token = { token: "", hubId: "" };
      try {
        const all = JSON.parse(localStorage.getItem("dspi-web.hub-tokens.v1") ?? "{}") as Record<string, { token: string; hubId: string }>;
        token = Object.values(all)[0] ?? token;
      } catch {
        // none kept: an open hub
      }
      try {
        await call("linkConnect", { url, token: token.token, hubId: token.hubId });
        await call("open", { kind: "hub", url, deviceId: serial });
        const out: unknown[] = [];
        for (const [m, p] of calls) out.push(await call(m, p));
        await call("close", undefined);
        return out;
      } finally {
        w.terminate();
      }
    },
    [hub.port, serial, calls] as const,
  ) as Promise<T[]>;
}

const lowerVolume = (page: Page, hub: OwnHub, serial: string) => onUnit(page, hub, serial, [["write", { path: "vol.user", value: { float: -20 } }]]);

/** Choose the listening position, units `serials` (each with its first output). */
async function chooseGroup(tab: Locator, serials: string[]) {
  await tab.getByLabel("Placement across units").selectOption("listeningPosition");
  for (const s of serials) {
    await tab.getByRole("checkbox", { name: `Measure ${s}` }).check();
    await expect(tab.getByRole("checkbox", { name: `${s} output 1` })).toBeChecked();
  }
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  return errors;
}

const desktop = (w: number | undefined) => (w ?? 0) >= 500;

/** Start the checked group, see the settle wait in words, Stop it; every unit's generator stopped. */
async function measureAndStop(page: Page, hub: OwnHub, tab: Locator, browserName: string) {
  await tab.getByLabel("Sweep length across units").fill("1");
  await tab.getByRole("button", { name: "Measure the units" }).click();
  if (browserName === "firefox") {
    // The Playwright image has no audio device: Firefox's real-time context never starts there
    // (as P5WMS's). The microphone comes first, so the refusal is in words and nothing started.
    await expect(tab.locator('p.error[role="alert"]').first()).toHaveAttribute("data-key", "mic_not_running", { timeout: 20_000 });
    await expect(tab.getByRole("status", { name: "Measuring across units" })).toHaveCount(0);
    for (const s of [A, B]) {
      const [st] = await onUnit<{ state: number }>(page, hub, s, [["siggenStatus", undefined]]);
      expect(st!.state, `${s}'s generator`).toBe(0);
    }
    return;
  }
  // It runs; Stop is in view; the hub holds the start until the clocks settle, said in words.
  const banner = tab.getByRole("status", { name: "Measuring across units" });
  await expect(banner).toBeVisible({ timeout: 20_000 });
  await expect(banner.getByLabel("Phase")).toContainText("waiting for the units' clocks to settle", { timeout: 20_000 });
  await banner.getByRole("button", { name: "Stop" }).click();
  await expect(banner).toHaveCount(0);
  await expect(tab.getByRole("alert", { name: "Why it stopped" })).toHaveAttribute("data-key", "measure.stopped");
  // Every unit's generator is stopped (the hub's abort), read on each unit itself.
  for (const s of [A, B]) {
    const [st] = await onUnit<{ state: number }>(page, hub, s, [["siggenStatus", undefined]]);
    expect(st!.state, `${s}'s generator`).toBe(0);
  }
}

test.describe("ATWEB: measuring several units on the hub", () => {
  test.skip(!process.env.DSPI_TERM, "needs uconsole (DSPI_TERM)");
  // The browsers' fake microphone (Chromium's permission is the context's; Firefox asks nothing).
  test.use({ permissions: async ({ browserName }, use) => use(browserName === "firefox" ? [] : ["microphone"]) });
  let hub: OwnHub | undefined;
  test.afterEach(() => {
    hub?.child.kill();
    hub = undefined;
  });

  test("ATWEB: Control measures: the placement first, each unit checked on its own connection, the run with the settle wait in words, Stop reaching every unit", async ({ page, viewport, browserName }) => {
    test.skip(!desktop(viewport?.width), "desktop width (375 px below)");
    test.setTimeout(150_000);
    const errors = errorsOf(page);
    hub = await measureHub(["--pair"], true);
    const tab = await severalUnits(await openUnit(page, hub, A), page);
    // No placement assumed: nothing can be checked or started.
    await expect(tab.getByLabel("Placement across units")).toHaveValue("");
    await expect(tab.getByLabel("Placement problem")).toContainText("Choose where the microphone is first");
    await expect(tab.getByRole("button", { name: "Measure the units" })).toBeDisabled();
    await expect(tab.getByRole("button", { name: "Check every unit" })).toBeDisabled();
    await tab.getByLabel("Placement across units").selectOption("headphones");
    await expect(tab.getByLabel("Placement problem")).toContainText("Headphones are never measured across units");
    await expect(tab.getByRole("button", { name: "Measure the units" })).toBeDisabled();
    await chooseGroup(tab, [A, B]);
    await expect(tab.getByLabel("Placement problem")).toHaveCount(0);
    // Each unit checked on its own connection: the demo's volume is too high on both, each named.
    await tab.getByRole("button", { name: "Check every unit" }).click();
    const pre = tab.getByRole("list", { name: "Preflight across units" });
    await expect(pre.locator(`[data-key="measure.volume_high"][data-serial="${A}"]`)).toContainText(A);
    await expect(pre.locator(`[data-key="measure.volume_high"][data-serial="${B}"]`)).toContainText(B);
    // Lowered on each unit (with this page's pairing), then the console opened again to read them.
    await lowerVolume(page, hub, A);
    await lowerVolume(page, hub, B);
    const again = await severalUnits(await openUnit(page, hub, A, false), page);
    await chooseGroup(again, [A, B]);
    await again.getByRole("button", { name: "Check every unit" }).click();
    await expect(again.getByRole("list", { name: "Preflight across units" })).toContainText("Ready: nothing blocks the measurement across units.");
    // Control may measure.
    await measureAndStop(page, hub, again, browserName);
    // Control sees no Apply and no Revert on this tab (they are Admin's).
    await expect(again.getByRole("button", { name: "Apply the alignment" })).toHaveCount(0);
    await expect(again.getByRole("button", { name: "Revert" })).toHaveCount(0);
    expect(errors).toEqual([]);
  });

  test("ATWEB: the named notices: not one clock, a group that needs several runs, a unit that lost its lock", async ({ page, viewport, browserName }) => {
    test.skip(!desktop(viewport?.width), "desktop width");
    test.skip(browserName === "firefox", "Firefox cannot start audio in the Playwright image: a run's notices are Chromium's");
    test.setTimeout(180_000);
    hub = await measureHub(["--open", "--forget-clients"]);
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);
    for (const s of [A, B, C, X]) await lowerVolume(page, hub, s);
    const tab = await severalUnits(await openUnit(page, hub, A), page);
    const measure = tab.getByRole("button", { name: "Measure the units" });
    const banner = tab.getByRole("status", { name: "Measuring across units" });
    // Two senders' time bases: the hub refuses at open, in words.
    await chooseGroup(tab, [A, X]);
    await measure.click();
    const err = tab.locator('p.error[role="alert"]').first();
    await expect(err).toHaveAttribute("data-key", "measure.system.not_one_domain", { timeout: 20_000 });
    await expect(err).toContainText("These units do not share a clock");
    await tab.getByRole("checkbox", { name: `Measure ${X}` }).uncheck();
    // A group that does not fit one capture: several runs, said as soon as it starts.
    await tab.getByRole("checkbox", { name: `Measure ${B}` }).check();
    await tab.getByRole("checkbox", { name: `${A} output 2` }).check();
    await tab.getByRole("checkbox", { name: `${B} output 2` }).check();
    await tab.getByLabel("Sweep length across units").fill("15");
    await measure.click();
    const runs = tab.getByRole("note", { name: "Runs" });
    await expect(runs).toHaveAttribute("data-key", "measure_system_runs", { timeout: 20_000 });
    await expect(runs).toContainText(/this group needs \d+ runs, chained through the anchor output/);
    await banner.getByRole("button", { name: "Stop" }).click();
    await expect(banner).toHaveCount(0);
    // A unit whose time base re-anchors (DEMO-TB-C, every 15 s): the run ends with the unit named.
    await tab.getByRole("checkbox", { name: `Measure ${B}` }).uncheck();
    await tab.getByRole("checkbox", { name: `Measure ${C}` }).check();
    await expect(tab.getByRole("checkbox", { name: `${C} output 1` })).toBeChecked();
    await tab.getByLabel("Sweep length across units").fill("1");
    const notLocked = tab.locator('p.error[role="alert"][data-key="measure.system.not_locked"]');
    const why = tab.getByRole("alert", { name: "Why it stopped" });
    let lost = "";
    for (let attempt = 0; attempt < 4 && !lost; attempt++) {
      await measure.click();
      // It runs (or the hub found DEMO-TB-C anchoring at open: try again).
      await expect(banner.or(notLocked)).toBeVisible({ timeout: 20_000 });
      if (await notLocked.isVisible()) continue;
      await expect(banner).toHaveCount(0, { timeout: 60_000 });
      if ((await why.getAttribute("data-key")) === "measure.system.abort.disturbance") lost = (await why.textContent()) ?? "";
    }
    expect(lost, "a run ended on DEMO-TB-C's lost lock").toContain("Unit lost lock");
    expect(lost).toContain(C);
  });

  test("ATWEB: an S/PDIF follower that hears no DSPi time base says so in the Installer's clock panel", async ({ page, viewport }) => {
    test.skip(!desktop(viewport?.width), "desktop width");
    test.setTimeout(90_000);
    hub = await measureHub(["--open", "--forget-clients"]);
    const session = await openUnit(page, hub, A);
    await setMode(session, page, "Installer");
    await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "System" }).click();
    const panel = session.getByRole("region", { name: "Clock and audio links" });
    const table = panel.getByRole("table", { name: "S/PDIF units" });
    await expect(table.getByRole("row", { name: "S/PDIF clock of S/PDIF source" })).toContainText("Source");
    const rear = table.getByRole("row", { name: "S/PDIF clock of S/PDIF rear" });
    await expect(rear).toContainText("Lock error", { timeout: 30_000 });
    await expect(rear.locator('[data-key="spdif_no_time_base"]')).toContainText("This S/PDIF source does not carry a DSPi time base");
    // The unit fed by S/PDIF from its source: the source is chosen among the sources (Admin).
    await expect(rear.getByLabel("S/PDIF source of S/PDIF rear")).toHaveValue("S");
  });
});

test("ATWEB: a console not on the DSPi hub says that measuring several units needs it", async ({ page }) => {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.getByRole("tab", { name: "EQ" })).toBeVisible();
  await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Measure" }).click();
  const screen = session.getByRole("region", { name: "Measure" });
  await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Several units" }).click();
  const note = screen.getByRole("region", { name: "Several units" }).getByRole("note");
  await expect(note).toHaveAttribute("data-key", "measure.system.needs_hub");
  await expect(note).toContainText("Measuring several units needs the DSPi hub");
  await expect(screen.getByRole("button", { name: "Measure the units" })).toHaveCount(0);
});

test("ATWEB: the Several units step at 375 px: no sideways scroll", async ({ page, viewport }) => {
  test.skip(!process.env.DSPI_TERM, "needs uconsole (DSPI_TERM)");
  test.skip(desktop(viewport?.width), "the narrow project");
  test.setTimeout(90_000);
  const hub = await measureHub(["--open", "--forget-clients"]);
  try {
    const tab = await severalUnits(await openUnit(page, hub, A), page);
    await chooseGroup(tab, [A, B]);
    await tab.getByRole("checkbox", { name: `${B} output 2` }).check();
    await expect(tab.getByLabel(`Role of ${B} output 2`)).toBeVisible();
    const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
    expect(over).toBeLessThanOrEqual(0);
  } finally {
    hub.child.kill();
  }
});
