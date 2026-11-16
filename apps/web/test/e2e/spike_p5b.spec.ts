// Review P5-B (decision a): the P5SPIKE page (tools/spike) end to end on the demo DSPi with
// Chromium's fake microphone. The page drives the core's measurement session (P5SES) through
// the Worker: preflight, a start at the start level only, every capture block pushed with its
// frame index, the tick, the session's state and verdict in the record; Stop is the session's
// stop and leaves the generator idle. The fake microphone plays a looped file, not what the
// generator plays, so the run ends without timing: the timing fields with a microphone that
// answers the generator are the core's own test (wp_p5b_device_a_start_level_only_run_...),
// and the simulated microphone of the demo is P5DEMO's. Nothing is played.

import { expect, test, type Page } from "@playwright/test";

// The fake microphone with a file is Chromium's; Firefox's audio context never runs in the
// container (P5WCAP). Once is enough: not again at 375 px.
test.skip(({ browserName, viewport }) => browserName !== "chromium" || (viewport?.width ?? 0) < 500, "Chromium only, once");
// Chromium's headless shell has no getUserMedia: the full Chromium, headless.
test.use({ channel: "chromium", permissions: ["microphone"] });

interface SpikeRecord {
  phases: { phase: string }[];
  maxLevelDb: number | null;
  stoppedBy: string;
  verdict: string;
  pushFailures: number;
  frames: { blocks: number; frames: number; gaps: number; repeats: number } | null;
  setup: { startLevelOnly?: boolean };
  state: { phase: string; generatorStarts: number; frameFaults: number; frames: number; issue?: { key: string } | null };
  generatorAfter: { state: number; stopReason: number };
}

const record = async (page: Page): Promise<SpikeRecord | null> => {
  const text = await page.locator("#timing").getAttribute("data-record");
  return text ? (JSON.parse(text) as SpikeRecord) : null;
};

async function ready(page: Page): Promise<number> {
  await page.goto("/spike/measure-spike.html");
  await page.getByRole("button", { name: "The demo DSPi (no hardware)" }).click();
  await expect(page.locator("#device")).toContainText("The demo DSPi is open");
  await page.getByRole("button", { name: "Start the microphone" }).click();
  await expect(page.locator("#mic")).toContainText("rateHz", { timeout: 15_000 });
  const mic = JSON.parse((await page.locator("#mic").textContent()) ?? "{}") as { rateHz: number; sinkNone: boolean };
  expect(mic.sinkNone).toBe(true);
  // Short sweeps; a ceiling above the start level, which the page's startLevelOnly overrides.
  await page.locator("#setup").fill(
    JSON.stringify({
      kind: "walked",
      outputs: [
        { output: 0, role: "full" },
        { output: 1, role: "full" },
      ],
      placement: "headphones",
      coupler: "flatPlate",
      passes: 2,
      sweepMs: 1000,
      gapMs: 300,
      ceilingDb: -20,
      startLevelOnly: false,
      captureRateHz: mic.rateHz,
    }),
  );
  return mic.rateHz;
}

test("P5B: the spike page runs a start-level-only session on the demo DSPi to a verdict", async ({ page }) => {
  test.setTimeout(120_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  const rate = await ready(page);
  await page.getByRole("button", { name: "Preflight" }).click();
  await expect(page.locator("#state")).toContainText('"clear": true');
  await page.getByRole("button", { name: "Start the run" }).click();
  await expect.poll(() => record(page), { timeout: 90_000 }).not.toBeNull();
  const r = (await record(page))!;
  const seen = r.phases.map((p) => p.phase);
  for (const phase of ["preRoll", "ramp", "run", "analysing"]) expect(seen, JSON.stringify(r.phases)).toContain(phase);
  // The start level only (-60 dBFS on a coupler), whatever the text box said.
  expect(r.setup.startLevelOnly).toBe(true);
  expect(r.maxLevelDb).toBe(-60);
  expect(r.state.generatorStarts).toBe(3);
  // Every block reached the session in order.
  expect(r.state.frameFaults).toBe(0);
  expect(r.pushFailures).toBe(0);
  expect(r.frames).toMatchObject({ gaps: 0, repeats: 0 });
  expect(r.state.frames).toBeGreaterThan(5 * rate);
  // A verdict, and the generator idle.
  expect(["done", "failed", "aborted"]).toContain(r.state.phase);
  expect(r.verdict.length).toBeGreaterThan(0);
  expect(r.generatorAfter.state).toBe(0);
  await expect(page.getByRole("button", { name: "Download the record" })).toBeEnabled();
  expect(errors).toEqual([]);
});

test("P5B: Stop on the spike page stops the session and the generator", async ({ page }) => {
  test.setTimeout(120_000);
  await ready(page);
  await page.getByRole("button", { name: "Start the run" }).click();
  await expect(page.locator("#state")).toContainText('"phase": "run"', { timeout: 60_000 });
  await page.getByRole("button", { name: "Stop the run" }).click();
  await expect.poll(() => record(page), { timeout: 15_000 }).not.toBeNull();
  const r = (await record(page))!;
  expect(r.stoppedBy).toBe("user");
  expect(r.state.phase).toBe("aborted");
  expect(r.state.issue?.key).toBe("measure.stopped");
  // The generator was playing and the host's stop ended it (SIGGEN_STOP_HOST = 1).
  expect(r.generatorAfter).toMatchObject({ state: 0, stopReason: 1 });
});
