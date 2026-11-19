// WP P5WMS e2e: the Measure screen on the core's demo device. With the demo's simulated
// microphone (P5DEMO: P5SIM's room answers the generator inside the core, nothing is opened or
// played): a measurement end to end with its curves drawn and its timing verdict, Stop stopping
// the generator, the live view's "cut this peak" applied and undone. With the browsers' fake
// microphone (P5WCAP; Chromium plays a P5SIM rendering, Firefox a tone): the capture's blocks
// reach the core through the Worker's port (they cannot answer the generator, so the run stops
// with its reason in words, or ends). The results are the core's curves (`measureDocCurves`),
// smoothed, with the target. A view-only session and a Viewer through a real hub (`uconsole --demo
// hub`) get the passive microphone RTA, computed by the core from the fake microphone, and
// nothing that plays. And 375 px with no sideways scroll. What the page did is read back from the core (a direct call).

import { execFileSync, spawn, type ChildProcess } from "node:child_process";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

test.skip(({ browserName }) => browserName === "webkit", "Chromium and Firefox only");
// Chromium's headless shell has no getUserMedia: the full Chromium, headless (as capture_p5wcap).
test.use({ channel: [async ({ browserName }, use) => use(browserName === "chromium" ? "chromium" : undefined), { scope: "worker" }] });

type Client = { call(m: string, p: unknown): Promise<unknown> };
type G = { dspiClients: Client[] };

async function call<T>(page: Page, method: string, params: unknown = null): Promise<T> {
  return (await page.evaluate(([m, p]) => (globalThis as unknown as G).dspiClients[0]!.call(m as string, p), [method, params] as const)) as T;
}

function errorsOf(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

const desktop = (w: number | undefined) => (w ?? 0) >= 500;

/** Console one alone, the venue connected, the Measure screen open; the demo's volume below S2's limit. */
async function measureScreen(page: Page, venue = "Venue A", lowerVolume = true): Promise<Locator> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: venue }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.getByRole("tab", { name: "EQ" })).toBeVisible();
  if (lowerVolume) await call(page, "write", { path: "vol.user", value: { float: -20 } });
  await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Measure" }).click();
  const screen = session.getByRole("region", { name: "Measure" });
  await expect(screen.getByRole("tablist", { name: "Measurement" })).toBeVisible();
  return screen;
}

// -- WP P5FOLLOW --
/** No placement is assumed: Check and Start wait for the person, said in words; then speakers at the listening position. */
async function choosePlacement(setup: Locator): Promise<void> {
  await expect(setup.getByRole("radiogroup", { name: "Placement" }).getByRole("radio", { checked: true })).toHaveCount(0);
  await expect(setup.getByRole("button", { name: "Start measuring" })).toBeDisabled();
  await expect(setup.getByRole("button", { name: "Check" })).toBeDisabled();
  await expect(setup).toContainText("Choose the placement first: speakers or headphones on a coupler.");
  await setup.getByRole("radio", { name: "Listening position" }).check();
  await expect(setup.getByRole("button", { name: "Start measuring" })).toBeEnabled();
}

async function noSidewaysScroll(page: Page) {
  const over = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(over).toBeLessThanOrEqual(0);
}

test.afterEach(async ({ page }) => {
  await call(page, "measureStop").catch(() => undefined);
  await call(page, "siggenControl", { action: "stopNow" }).catch(() => undefined);
});

test("P5WMS: a measurement end to end on the simulated microphone: preflight in words, the run, the curves and the timing", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width), "desktop width (the 375 px test is below)");
  test.setTimeout(150_000);
  const errors = errorsOf(page);
  const screen = await measureScreen(page, "Venue A", false);
  const setup = screen.getByRole("region", { name: "Measurement setup" });
  await expect(setup.getByLabel("Microphone")).toHaveValue("sim");
  await expect(setup.getByLabel("Microphone").locator("option:checked")).toHaveText(/Simulated microphone/);
  await choosePlacement(setup); // -- WP P5FOLLOW --
  // The demo's volume is too high (S2): the preflight says so in words and nothing plays.
  await setup.getByRole("button", { name: "Check" }).click();
  const preflight = setup.getByRole("list", { name: "Preflight" });
  await expect(preflight.locator('[data-key="measure.volume_high"]')).toContainText("Volume too high to measure");
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  await call(page, "write", { path: "vol.user", value: { float: -20 } });
  await setup.getByLabel("Sweep length").fill("1");
  await setup.getByRole("button", { name: "Check" }).click();
  await expect(preflight).toContainText("Ready: nothing blocks the measurement.");
  await setup.getByRole("button", { name: "Start measuring" }).click();
  // Stop is shown while it runs, above the tabs.
  await expect(screen.getByRole("status", { name: "Measuring" }).getByRole("button", { name: "Stop" })).toBeVisible();
  const run = screen.getByRole("region", { name: "Measurement run" });
  await expect(run.getByRole("button", { name: "See the results" })).toBeVisible({ timeout: 120_000 });
  await expect(run.getByLabel("Timing")).toHaveAttribute("data-reliable", "true");
  await expect(screen.getByRole("status", { name: "Measuring" })).toHaveCount(0);
  await run.getByRole("button", { name: "See the results" }).click();
  const results = screen.getByRole("region", { name: "Measurement results" });
  await expect(results.getByLabel("Timing")).toContainText("Timing reliable");
  const graph = results.getByRole("img", { name: "Measured responses" });
  // The core's curves: smoothed (1/6 octave by default), each output with its levelled target.
  await expect(results.getByLabel("Curves drawn")).toHaveAttribute("data-smoothing", "6");
  await expect(graph).toHaveAttribute("data-series", /^measured:OUT1 [^|]*\|target:Target for OUT1 [^|]*\|measured:OUT2 [^|]*\|target:Target for OUT2 /);
  expect(await graph.locator('path[data-kind="target"]').count()).toBeGreaterThan(1);
  await results.getByLabel("Results smoothing").selectOption("24");
  await expect(results.getByLabel("Curves drawn")).toHaveAttribute("data-smoothing", "24");
  await results.getByLabel("Show the target").uncheck();
  await expect(graph).toHaveAttribute("data-series", /^measured:OUT1 [^|]*\|measured:OUT2 /);
  // The curves drawn, the untrusted points faint.
  expect(await graph.locator("path.trusted").count()).toBeGreaterThan(1);
  expect(await graph.locator("path.untrusted").count()).toBeGreaterThan(0);
  expect(await graph.locator("path.trusted").first().getAttribute("d")).toMatch(/^M[\d.]+,[\d.]+(L[\d.]+,[\d.]+){50,}/);
  // No hub here: the document is downloaded.
  await expect(results.getByRole("button", { name: "Save to the hub" })).toHaveCount(0);
  const dl = page.waitForEvent("download");
  await results.getByRole("button", { name: "Download" }).click();
  expect((await dl).suggestedFilename()).toMatch(/\.dspipreset$/);
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  expect(errors).toEqual([]);
});

// -- WP P5FOLLOW --
test("P5FOLLOW: nothing is measured before the placement is chosen; headphones show the coupler; two passes show the timing hint", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width), "desktop width");
  const errors = errorsOf(page);
  const screen = await measureScreen(page);
  const setup = screen.getByRole("region", { name: "Measurement setup" });
  await expect(setup.getByLabel("Placement detail")).toContainText("Where is the microphone?");
  await expect(setup.getByRole("button", { name: "Start measuring" })).toBeDisabled();
  await expect(setup.getByRole("button", { name: "Check" })).toBeDisabled();
  await expect(setup).toContainText("Choose the placement first");
  expect((await call<{ active: boolean }>(page, "measureState")).active).toBe(false);
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  await setup.getByRole("radio", { name: "Headphones on a coupler" }).check();
  await expect(setup.getByLabel("Placement detail")).toContainText("On the coupler, not on a head");
  await expect(setup.getByText("Coupler", { exact: true })).toBeVisible();
  await expect(setup.getByRole("button", { name: "Start measuring" })).toBeEnabled();
  await expect(setup.getByLabel("Passes hint")).toHaveCount(0);
  await setup.getByLabel("Passes").fill("2");
  await expect(setup.getByLabel("Passes hint")).toHaveText("Two passes cannot judge timing; use three or more for auto-tune.");
  await expect(setup.getByRole("button", { name: "Start measuring" })).toBeEnabled();
  await setup.getByLabel("Passes").fill("3");
  await expect(setup.getByLabel("Passes hint")).toHaveCount(0);
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  expect(errors).toEqual([]);
});

test("P5WMS: Stop stops the generator at once, and the run says nothing was kept", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width), "desktop width");
  test.setTimeout(60_000);
  const errors = errorsOf(page);
  const screen = await measureScreen(page);
  const setup = screen.getByRole("region", { name: "Measurement setup" });
  await choosePlacement(setup); // -- WP P5FOLLOW --
  await setup.getByRole("button", { name: "Start measuring" }).click();
  const run = screen.getByRole("region", { name: "Measurement run" });
  // Once the generator plays (the ramp), Stop.
  await expect.poll(async () => (await call<{ state: number }>(page, "siggenStatus")).state, { timeout: 20_000 }).not.toBe(0);
  await screen.getByRole("status", { name: "Measuring" }).getByRole("button", { name: "Stop" }).click();
  await expect(run.locator("[data-phase]")).toHaveAttribute("data-phase", "aborted");
  await expect(run).toContainText("nothing was kept");
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  expect((await call<{ active: boolean }>(page, "measureState")).active).toBe(false);
  await expect(screen.getByRole("status", { name: "Measuring" })).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("P5WMS: the live view: a curve per sweep with the target, 'cut this peak' previewed, applied, and undone", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width), "desktop width");
  test.setTimeout(120_000);
  const errors = errorsOf(page);
  const screen = await measureScreen(page);
  await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Live" }).click();
  const live = screen.getByRole("region", { name: "Live response" });
  await live.getByLabel("Smoothing").selectOption("6");
  await live.getByRole("button", { name: "Start live" }).click();
  await expect.poll(async () => Number(await live.locator("[data-seq]").getAttribute("data-seq")), { timeout: 60_000 }).toBeGreaterThanOrEqual(2);
  const graph = live.getByRole("img", { name: "Live response" });
  await expect(graph).toHaveAttribute("data-series", /measured:.*\|target:Target/);
  const filters = async () => JSON.stringify((await call<{ state: { filters: unknown } }>(page, "state")).state.filters);
  const before = await filters();
  await live.getByRole("button", { name: "Find the highest peak" }).click();
  await expect(live.getByLabel("Peak frequency")).not.toHaveValue("");
  await live.getByRole("button", { name: "Cut this peak" }).click();
  const pending = live.getByRole("group", { name: "Pending change" });
  await expect(pending).toContainText(/PEQ band \d+: flat → peak -[\d.]+ dB/);
  await expect(graph).toHaveAttribute("data-series", /predicted:Predicted/);
  expect(await filters()).toBe(before);
  await pending.getByRole("button", { name: "Apply" }).click();
  await expect(live.getByRole("status")).toContainText("Undo takes it back as one step");
  expect(await filters()).not.toBe(before);
  await page.locator("#one").getByRole("group", { name: "Undo and Redo" }).getByRole("button", { name: /^Undo/ }).click();
  await expect.poll(filters).toBe(before);
  await live.getByRole("button", { name: "Stop live" }).click();
  await expect(live.getByRole("button", { name: "Start live" })).toBeVisible();
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  expect(errors).toEqual([]);
});

test.describe("the browsers' fake microphone", () => {
  test.use({ permissions: async ({ browserName }, use) => use(browserName === "firefox" ? [] : ["microphone"]) });

  test("P5WMS: the microphone's blocks reach the core through the Worker's port; the run ends or stops in words, the microphone closed", async ({ page, viewport, browserName }) => {
    test.skip(!desktop(viewport?.width), "desktop width");
    test.setTimeout(150_000);
    const errors = errorsOf(page);
    const screen = await measureScreen(page);
    const setup = screen.getByRole("region", { name: "Measurement setup" });
    await setup.getByLabel("Microphone").selectOption("mic:");
    await choosePlacement(setup); // -- WP P5FOLLOW --
    await setup.getByRole("button", { name: "Start measuring" }).click();
    const refused = screen.getByRole("alert").filter({ hasText: "Microphone capture did not start" });
    await expect(screen.getByRole("status", { name: "Measuring" }).or(refused)).toBeVisible({ timeout: 15_000 });
    if (await refused.isVisible()) {
      // The Playwright image has no audio device: Firefox's real-time context never starts there
      // (as capture_p5wcap). The refusal is in words, the microphone given back, nothing started.
      expect(browserName).toBe("firefox");
      expect((await call<{ active: boolean; phase: string }>(page, "measureState")).active).toBe(false);
      await expect(screen.getByRole("status", { name: "Measuring" })).toHaveCount(0);
      expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
      expect(errors).toEqual([]);
      return;
    }
    // The blocks arrive with their frame indices: the core counts the frames.
    await expect.poll(async () => (await call<{ frames: number }>(page, "measureState")).frames, { timeout: 20_000 }).toBeGreaterThan(20_000);
    expect((await call<{ frameFaults: number }>(page, "measureState")).frameFaults).toBe(0);
    // A looped file or a tone does not answer the generator as a room does: the core either stops
    // the run and says why, or (Chromium's P5SIM rendering can line up with the schedule) ends it
    // with a document whose timing verdict is in words. Either way the microphone is closed.
    const run = screen.getByRole("region", { name: "Measurement run" });
    await expect(run.locator("[data-phase]")).toHaveAttribute("data-phase", /aborted|failed|done/, { timeout: 90_000 });
    if ((await run.locator("[data-phase]").getAttribute("data-phase")) === "done") {
      await run.getByRole("button", { name: "See the results" }).click();
      const results = screen.getByRole("region", { name: "Measurement results" });
      await expect(results.getByLabel("Timing")).toHaveText(/^(Timing reliable|Timing unreliable|No timing)/);
      await expect(results.getByRole("img", { name: "Measured responses" })).toHaveAttribute("data-series", /measured:OUT1 /);
    } else await expect(run.getByRole("alert")).not.toBeEmpty();
    await expect(screen.getByRole("status", { name: "Measuring" })).toHaveCount(0);
    expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
    expect(errors).toEqual([]);
  });
});

/** The passive RTA listens (Chromium: the fake microphone's file; Firefox in the image cannot start audio). */
async function passiveListens(page: Page, screen: Locator, browserName: string) {
  const tabs = screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab");
  await expect(tabs).toHaveText(["Passive RTA", "Results", "Documents"]); // -- WP P5WAT -- its documents tab (a Viewer reads documents)
  await expect(screen.getByRole("button", { name: "Start measuring" })).toHaveCount(0);
  await expect(screen.getByRole("button", { name: "Start live" })).toHaveCount(0);
  const passive = screen.getByRole("region", { name: "Passive microphone RTA" });
  await expect(passive).toContainText(/only the passive microphone RTA and the documents/i);
  await passive.getByRole("button", { name: "Listen" }).click();
  const refused = screen.getByRole("alert").filter({ hasText: "Microphone capture did not start" });
  await expect(passive.getByRole("button", { name: "Stop listening" }).or(refused)).toBeVisible({ timeout: 15_000 });
  if (await refused.isVisible()) {
    expect(browserName).toBe("firefox");
    return;
  }
  await expect(screen.getByRole("status", { name: "Measuring" })).toContainText("nothing is played");
  // The core computes it: frames are averaged and its spectrum is drawn.
  await expect.poll(async () => Number(await passive.getByLabel("Frames averaged").getAttribute("data-frames")), { timeout: 20_000 }).toBeGreaterThan(0);
  const graph = passive.getByRole("img", { name: "Passive microphone RTA" });
  await expect(graph).toHaveAttribute("data-series", "measured:Microphone RTA");
  expect(await graph.locator('path[data-kind="measured"]').count()).toBeGreaterThan(0);
  await passive.getByRole("button", { name: "Hold", exact: true }).click();
  await expect(graph).toHaveAttribute("data-series", "measured:Microphone RTA|held:Held");
  await expect(passive.getByRole("img", { name: "Compared with the held spectrum" })).toBeVisible();
  await passive.getByRole("button", { name: "Stop listening" }).click();
  await expect(passive.getByRole("button", { name: "Listen" })).toBeVisible();
  await expect(screen.getByRole("status", { name: "Measuring" })).toHaveCount(0);
}

test.describe("passive", () => {
  test.use({ permissions: async ({ browserName }, use) => use(browserName === "firefox" ? [] : ["microphone"]) });

  test("P5WMS: a view-only session gets the passive RTA, computed from the microphone, and the results only: nothing plays", async ({ page, browserName }) => {
    test.setTimeout(60_000);
    const errors = errorsOf(page);
    const screen = await measureScreen(page, "Venue B", false);
    await passiveListens(page, screen, browserName);
    expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
    expect((await call<{ active: boolean }>(page, "measureState")).active).toBe(false);
    await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Results" }).click();
    await expect(screen.getByRole("region", { name: "Measurement results" })).toContainText("No measurement yet");
    expect(errors).toEqual([]);
  });

  // A real hub (`uconsole --demo hub`), paired as Control, then made a Viewer with the hub stopped
  // (`uconsole hub role`, as system.spec's SW3 does); the kept token reconnects as the Viewer.
  test("P5WMS: a Viewer through the hub gets the passive RTA computed and nothing that plays", async ({ page, browserName, viewport }) => {
    test.skip(!desktop(viewport?.width) || !process.env.DSPI_TERM, "desktop width, with uconsole");
    test.setTimeout(120_000);
    const errors = errorsOf(page);
    let hub = await demoHub(true);
    try {
      await hubSession(page, hub, true);
      const { port, store } = hub;
      await stopHub(hub);
      const listed = JSON.parse(execFileSync(process.env.DSPI_TERM!, ["hub", "clients", "--store", store, "--json"], { encoding: "utf8" }).trim().split("\n").pop()!) as { clients: { cid: number }[] };
      execFileSync(process.env.DSPI_TERM!, ["hub", "role", String(listed.clients[0]!.cid), "viewer", "--store", store]);
      hub = await demoHub(false, store, port);
      const session = await hubSession(page, hub, false);
      await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Measure" }).click();
      const screen = session.getByRole("region", { name: "Measure" });
      await expect(screen.getByRole("region", { name: "Passive microphone RTA" })).toContainText("hub's Viewer role");
      await passiveListens(page, screen, browserName);
      // Nothing plays: the generator stays idle (its own screen, which a Viewer may read).
      await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Signal Generator" }).click();
      const gen = session.getByRole("region", { name: "Signal Generator" });
      await expect(gen.getByRole("group", { name: "Transport" }).getByRole("button", { name: "Stop", exact: true })).toHaveCount(0);
      expect(errors).toEqual([]);
    } finally {
      await stopHub(hub);
    }
  });
});

type OwnHub = { port: number; pin: string; child: ChildProcess; store: string };
const DEMO = "DEMO0000DSPI0001";

/** `uconsole --demo hub --web --loopback`, serving this console, on a free port (or `port`); with `pairing`, its PIN. */
async function demoHub(pairing: boolean, store?: string, port0 = 0): Promise<OwnHub> {
  store ??= `${process.env.TMPDIR ?? "test-results"}/p5wms-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(
    process.env.DSPI_TERM!,
    ["--demo", "hub", "--web", "--loopback", "--console", "dist", ...(pairing ? ["--pair"] : []), "--port", String(port0), "--json", "--no-keep-awake", "--store", store],
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
  if (h.child.exitCode !== null) return;
  const exited = new Promise<void>((r) => h.child.once("exit", () => r()));
  h.child.kill();
  await exited;
}

/** The hub's page, its entry connected (paired when asked), the demo DSPi opened. */
async function hubSession(page: Page, hub: OwnHub, pair: boolean): Promise<Locator> {
  await page.goto(`http://127.0.0.1:${hub.port}/console/`);
  const row = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
  await row.getByRole("button", { name: "Connect" }).click();
  const session = page.getByRole("article").last();
  if (pair) {
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
  }
  const entry = session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) });
  await entry.getByRole("button", { name: "Open" }).click();
  await expect(session.locator(".device-meta")).toContainText(DEMO);
  await expect(session.locator(".banner")).toHaveCount(0, { timeout: 10_000 });
  return session;
}

test("P5WMS: 375 px: every tab of the Measure screen without sideways scroll", async ({ page, viewport }) => {
  test.skip(desktop(viewport?.width), "the narrow project");
  const errors = errorsOf(page);
  const screen = await measureScreen(page);
  await expect(screen.getByRole("region", { name: "Measurement setup" })).toBeVisible();
  await choosePlacement(screen.getByRole("region", { name: "Measurement setup" })); // -- WP P5FOLLOW --
  await screen.getByRole("region", { name: "Measurement setup" }).getByRole("button", { name: "Check" }).click();
  await expect(screen.getByRole("list", { name: "Preflight" })).toBeVisible();
  await noSidewaysScroll(page);
  for (const t of ["Results", "Live"]) {
    await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: t }).click();
    await noSidewaysScroll(page);
  }
  await expect(screen.getByRole("img", { name: "Live response" })).toBeVisible();
  expect(errors).toEqual([]);
});
