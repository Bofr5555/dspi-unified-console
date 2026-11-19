// WP P5WAT e2e: the Measure screen's auto-tune and documents on the core's demo device, made the
// stereo three-way of P5SIM's room (the 3-way template; P5DEMO's simulated microphone hears it,
// nothing is opened or played): a tune measured, proposed in words, summed-checked, applied with
// its countdown, verified by itself (Keep not offered while it runs), kept (the second question
// when the verify did not pass) and A/B toggled; another applied and reverted, the settings back;
// a timing-unreliable copy of the measurement opened from a file, its timing writes withdrawn with
// the reason. Through a real hub (`uconsole --demo hub`) as Control: a document filed on the hub,
// renamed, and not offered for deletion. And 375 px with no sideways scroll.

import { spawn, type ChildProcess } from "node:child_process";
import { readFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { expect, test, type Locator, type Page } from "@playwright/test";

test.skip(({ browserName }) => browserName === "webkit", "Chromium and Firefox only");

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
const TEMPLATE = readFileSync("../../assets/templates/3way-rp2350.dspipreset", "utf8");

/** The demo as the three-way (the template), its volume below S2's limit, the Measure screen's Auto-tune tab. */
async function tuneScreen(page: Page): Promise<{ screen: Locator; tune: (name: string) => Locator }> {
  await page.goto("/test/embed.html?one");
  const one = page.locator("#one");
  await one.getByRole("region", { name: "Venue A" }).getByRole("button", { name: "Connect" }).click();
  const session = one.getByRole("article", { name: "DSPi" });
  await expect(session.getByRole("tab", { name: "EQ" })).toBeVisible();
  const token = await call<string>(page, "confirmToken", { op: "config_import" });
  await call(page, "configImport", { text: TEMPLATE, token });
  await call(page, "write", { path: "vol.user", value: { float: -20 } });
  await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Measure" }).click();
  const screen = session.getByRole("region", { name: "Measure" });
  await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Auto-tune" }).click();
  return { screen, tune: (name: string) => screen.getByRole("region", { name }) };
}

/** What a tune may change: the outputs' EQ, crossovers, delays and gains, the matrix. */
const CHAIN = ["filters", "xover", "outputDelayMs", "outputGainDb", "outputMuted", "matrixGain", "matrixInvert"];
const chainOf = async (page: Page) => {
  const st = (await call<{ state: Record<string, unknown> }>(page, "state")).state;
  return JSON.stringify(CHAIN.map((k) => st[k]));
};

/** The left speaker's layout measured (1 s sweeps), then a proposal; the summed check when it asks for one. */
async function measureAndPropose(page: Page, tune: (name: string) => Locator): Promise<void> {
  const layout = tune("Tune layout");
  await expect(layout.getByLabel("Layout recognised")).toContainText("a three-way speaker");
  // Programme feeds in words, the right speaker left out.
  await expect(layout.getByRole("list", { name: "Outputs" })).toContainText("fed by input");
  for (const item of await layout.getByRole("list", { name: "Outputs" }).getByRole("listitem").all()) {
    const speaker = await item.getByLabel(/^Speaker of/).inputValue();
    if (speaker !== "L") await item.getByRole("checkbox").uncheck();
  }
  const m = tune("Tune measurement");
  await m.getByLabel("Tune sweep length").fill("1");
  await m.getByRole("button", { name: "Measure this layout" }).click();
  await expect(m.getByLabel("Measurement to tune from")).toHaveAttribute("data-source", "yes", { timeout: 240_000 });
  await expect(m.getByRole("region", { name: "Measurement run" }).locator("[data-phase]")).toHaveAttribute("data-phase", "done");
  await tune("Tune options").getByRole("button", { name: "Propose a tune" }).click();
  const p = tune("Tune proposal");
  await expect(p.getByRole("list", { name: "Changes" })).toBeVisible();
  await expect(p.getByRole("img", { name: "Tune curves" })).toHaveAttribute("data-series", /measured:L now\|predicted:L predicted\|target:L target/);
  if ((await p.getByLabel("Summed check").getAttribute("data-check")) === "pending") {
    await expect(p.getByRole("button", { name: "Apply and verify" })).toBeDisabled();
    await p.getByRole("button", { name: "Run the summed check" }).click();
    await expect(p.getByLabel("Summed check")).not.toHaveAttribute("data-check", "pending", { timeout: 240_000 });
  }
}

/** Apply: the countdown, the verify by itself with no Keep while it runs, then its verdict. */
async function applyAndVerify(page: Page, tune: (name: string) => Locator): Promise<Locator> {
  await tune("Tune proposal").getByRole("button", { name: "Apply and verify" }).click();
  const g = tune("Tune guard");
  await expect(g.getByLabel("Guard countdown")).toContainText("s left to Keep or Revert");
  await expect(g).toHaveAttribute("data-stage", "verifying");
  await expect(g.getByRole("button", { name: "Keep", exact: true })).toHaveCount(0);
  await expect(g.getByRole("button", { name: "Revert" })).toBeVisible();
  await expect(g).toHaveAttribute("data-stage", "verified", { timeout: 300_000 });
  await expect(g.getByLabel("Verify")).toHaveAttribute("data-passed", /true|false/);
  return g;
}

test.afterEach(async ({ page }) => {
  await call(page, "measureTuneStop").catch(() => undefined);
  await call(page, "measureStop").catch(() => undefined);
  await call(page, "siggenControl", { action: "stopNow" }).catch(() => undefined);
});

test("P5WAT: a three-way tune measured, proposed, checked, applied, verified by itself, kept, and A/B toggled", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width), "desktop width");
  test.setTimeout(600_000);
  const errors = errorsOf(page);
  const { tune } = await tuneScreen(page);
  await measureAndPropose(page, tune);
  const before = await chainOf(page);
  const g = await applyAndVerify(page, tune);
  expect(await chainOf(page)).not.toBe(before);
  if ((await g.getByLabel("Verify").getAttribute("data-passed")) === "false") {
    // The core refuses the first Keep; the second question; only the person's own second press keeps.
    await g.getByRole("button", { name: "Keep", exact: true }).click();
    const q = g.getByRole("alertdialog", { name: "Keep anyway?" });
    await expect(q).toContainText("The verify did not confirm the tune");
    await expect(g).toHaveAttribute("data-stage", "verified");
    await expect(q.getByRole("button", { name: "Keep anyway" })).toBeEnabled({ timeout: 5000 });
    await page.waitForTimeout(300);
    await q.getByRole("button", { name: "Keep anyway" }).click();
  } else await g.getByRole("button", { name: "Keep", exact: true }).click();
  await expect(g).toHaveAttribute("data-stage", "kept");
  await expect(g.getByRole("button", { name: 'Download "before"' })).toBeVisible();
  const tuned = await chainOf(page);
  await g.getByRole("button", { name: "Compare A/B" }).click();
  const ab = g.getByRole("group", { name: "Tune A/B" });
  await expect(ab).toHaveAttribute("data-side", "a");
  await ab.getByRole("button", { name: "Switch to B" }).click();
  await expect(ab).toHaveAttribute("data-side", "b");
  expect(await chainOf(page)).not.toBe(tuned);
  await ab.getByRole("button", { name: "Switch to A" }).click();
  await expect(ab).toHaveAttribute("data-side", "a");
  await ab.getByRole("button", { name: "End on the tune" }).click();
  await expect(ab).toHaveCount(0);
  await expect.poll(() => chainOf(page)).toBe(tuned);
  expect((await call<{ state: number }>(page, "siggenStatus")).state).toBe(0);
  expect(errors).toEqual([]);
});

test("P5WAT: a tune reverted puts the settings back; a timing-unreliable measurement's timing writes are withdrawn with the reason", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width), "desktop width");
  test.setTimeout(600_000);
  const errors = errorsOf(page);
  const { screen, tune } = await tuneScreen(page);
  await measureAndPropose(page, tune);
  const before = await chainOf(page);
  const g = await applyAndVerify(page, tune);
  await g.getByRole("button", { name: "Revert" }).click();
  await expect(g).toHaveAttribute("data-stage", "reverted");
  await expect.poll(() => chainOf(page)).toBe(before);
  // The measurement from the Documents tab, downloaded, marked unreliable, opened, tuned from.
  await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Documents" }).click();
  const docs = screen.getByRole("region", { name: "Measurement documents" });
  const row = docs.getByRole("list", { name: "Measurements" }).getByRole("listitem").first();
  await expect(row).toHaveAttribute("data-where", "local");
  const dl = page.waitForEvent("download");
  await row.getByRole("button", { name: "Download" }).click();
  const text = readFileSync((await (await dl).path())!, "utf8");
  const doc = JSON.parse(text) as { meta: { name: string }; measurement: { capture: { timingReliable: boolean } } };
  doc.measurement.capture.timingReliable = false;
  doc.meta.name = "Unreliable copy";
  await docs.getByLabel("Open a document file").setInputFiles({ name: "unreliable.dspipreset", mimeType: "application/json", buffer: Buffer.from(JSON.stringify(doc)) });
  const copy = docs.getByRole("listitem", { name: "Unreliable copy" });
  await expect(copy).toBeVisible();
  await copy.getByRole("button", { name: "Tune from this" }).click();
  await expect(tune("Tune measurement").getByLabel("Measurement to tune from")).toContainText("Unreliable copy");
  await tune("Tune options").getByRole("button", { name: "Propose a tune" }).click();
  const p = tune("Tune proposal");
  await expect(p.getByLabel("Proposal timing")).toHaveAttribute("data-reliable", "false");
  const withdrawn = p.getByRole("list", { name: "Withdrawn" });
  await expect(withdrawn.locator('[data-key="measure.tune.timing_unreliable"]').first()).toContainText("No timing changes from this measurement");
  await expect(withdrawn).toContainText("no delay, polarity or crossover change is offered");
  expect(await p.locator('[aria-label="Changes"] [data-timing="true"]').count()).toBe(0);
  expect(await chainOf(page)).toBe(before);
  expect(errors).toEqual([]);
});

type OwnHub = { port: number; pin: string; child: ChildProcess; store: string };
const DEMO = "DEMO0000DSPI0001";

/** `uconsole --demo hub --web --loopback`, serving this console, on a free port, paired. */
async function demoHub(): Promise<OwnHub> {
  const store = `${process.env.TMPDIR ?? "test-results"}/p5wat-${process.pid}-${Math.random().toString(36).slice(2)}`;
  const child = spawn(process.env.DSPI_TERM!, ["--demo", "hub", "--web", "--loopback", "--console", "dist", "--pair", "--port", "0", "--json", "--no-keep-awake", "--store", store], {
    stdio: ["ignore", "pipe", "inherit"],
  });
  const lines = createInterface({ input: child.stdout! });
  let port = 0;
  const pin = await new Promise<string>((ok, fail) => {
    child.on("exit", (code) => fail(new Error(`uconsole hub exited ${code}`)));
    lines.on("line", (line) => {
      const e = JSON.parse(line) as { event?: string; port?: number; pin?: string };
      if (e.event === "status") port = e.port ?? 0;
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

test("P5WAT: through the hub as Control: a document filed and renamed on the hub, but Control cannot delete it", async ({ page, viewport }) => {
  test.skip(!desktop(viewport?.width) || !process.env.DSPI_TERM, "desktop width, with uconsole");
  test.setTimeout(120_000);
  const errors = errorsOf(page);
  const hub = await demoHub();
  try {
    await page.goto(`http://127.0.0.1:${hub.port}/console/`);
    const entryRow = page.getByRole("navigation", { name: "DSPi devices" }).getByRole("listitem").filter({ hasText: `Hub 127.0.0.1:${hub.port}` });
    await entryRow.getByRole("button", { name: "Connect" }).click();
    const session = page.getByRole("article").last();
    await session.getByLabel("PIN").fill(hub.pin);
    await session.getByRole("button", { name: "Pair" }).click();
    await session.getByRole("list", { name: /DSPis on/ }).getByRole("listitem").filter({ hasText: DEMO.slice(-8) }).getByRole("button", { name: "Open" }).click();
    await expect(session.locator(".device-meta")).toContainText(DEMO);
    await session.getByRole("tablist", { name: "Screens" }).getByRole("tab", { name: "Measure" }).click();
    const screen = session.getByRole("region", { name: "Measure" });
    await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Documents" }).click();
    const docs = screen.getByRole("region", { name: "Measurement documents" });
    await expect(docs).toContainText("only the hub's Admin deletes them");
    // A microphone's calibration (REW text) imported, then filed on the hub.
    const cal = Array.from({ length: 40 }, (_, i) => `${(20 * 1.2 ** i).toFixed(2)} ${(Math.sin(i / 4) * 0.5).toFixed(2)}`).join("\n");
    await docs.getByLabel("Import a microphone calibration").setInputFiles({ name: "P5WAT mic.txt", mimeType: "text/plain", buffer: Buffer.from(`* P5WAT test calibration\n${cal}\n`) });
    const local = docs.getByRole("list", { name: "Microphones" }).locator('[data-where="local"]');
    await expect(local).toHaveCount(1);
    await local.getByRole("button", { name: "Save to the hub" }).click();
    await expect(docs.getByRole("status", { name: "Documents outcome" })).toContainText("Saved on the hub");
    const onHub = docs.getByRole("list", { name: "Microphones" }).locator('[data-where="hub"]');
    await expect(onHub).toHaveCount(1);
    // Control renames it on the hub...
    await onHub.getByRole("button", { name: "Rename" }).click();
    await onHub.getByLabel("New name").fill("Stand microphone");
    await onHub.getByRole("button", { name: "Save" }).click();
    await expect(onHub).toContainText("Stand microphone");
    // ...but cannot delete it: no Delete, the reason in words (the hub's own role check refuses
    // Control's delete too: P5HUB's tests). After a refresh it is still on the hub.
    await expect(onHub.getByRole("button", { name: "Delete" })).toHaveCount(0);
    await expect(onHub.getByLabel("Delete")).toContainText("Only the hub's Admin deletes it.");
    await docs.getByRole("button", { name: "Refresh" }).click();
    await expect(onHub).toHaveCount(1);
    await expect(onHub).toContainText("Stand microphone");
    expect(errors).toEqual([]);
  } finally {
    await stopHub(hub);
  }
});

test("P5WAT: 375 px: the auto-tune and the documents without sideways scroll", async ({ page, viewport }) => {
  test.skip(desktop(viewport?.width), "the narrow project");
  const errors = errorsOf(page);
  const { screen, tune } = await tuneScreen(page);
  await expect(tune("Tune layout").getByRole("list", { name: "Outputs" })).toBeVisible();
  const over = () => page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(await over()).toBeLessThanOrEqual(0);
  await screen.getByRole("tablist", { name: "Measurement" }).getByRole("tab", { name: "Documents" }).click();
  await expect(screen.getByRole("region", { name: "Measurement documents" })).toBeVisible();
  expect(await over()).toBeLessThanOrEqual(0);
  expect(errors).toEqual([]);
});
