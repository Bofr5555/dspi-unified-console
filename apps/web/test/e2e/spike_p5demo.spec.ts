// WP P5DEMO: the P5SPIKE page on the demo DSPi with the demo's simulated microphone
// (`measureSimSet`): P5SIM's three-way speaker in a room answers the generator inside the core,
// so the start-level-only run reaches a timing verdict (spike_p5b's fake microphone, a looped
// file, cannot). No microphone is opened and nothing is played: the page only ticks.

import { expect, test, type Page } from "@playwright/test";

// The run takes about 20 s: Chromium and Firefox at desktop width, not again at 375 px.
test.skip(({ viewport }) => (viewport?.width ?? 0) < 500, "desktop width only");

interface SpikeRecord {
  phases: { phase: string }[];
  maxLevelDb: number | null;
  verdict: string;
  pushFailures: number;
  capture: unknown;
  simulatedMicrophone: { on: boolean; label: string } | null;
  setup: { startLevelOnly?: boolean };
  timing: { reliable: boolean; ratioPpm: number; cycleResidualsUs: number[]; maxResidualUs: number; residualBoundUs: number } | null;
  state: { phase: string; generatorStarts: number; frameFaults: number };
  generatorAfter: { state: number };
}

const record = async (page: Page): Promise<SpikeRecord | null> => {
  const text = await page.locator("#timing").getAttribute("data-record");
  return text ? (JSON.parse(text) as SpikeRecord) : null;
};

test("P5DEMO: the spike page reaches a timing verdict with the demo's simulated microphone", async ({ page }) => {
  test.setTimeout(150_000);
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.goto("/spike/measure-spike.html");
  await page.getByRole("button", { name: "The demo DSPi (no hardware)" }).click();
  await expect(page.locator("#device")).toContainText("The demo DSPi is open");
  await page.getByRole("button", { name: "The demo's simulated microphone" }).click();
  await expect(page.locator("#mic")).toContainText("Simulated microphone");
  // Short sweeps on a coupler; the page forces the start level only (-60 dBFS).
  await page.locator("#setup").fill(
    JSON.stringify({
      kind: "walked",
      outputs: [
        { output: 0, role: "full" },
        { output: 1, role: "full" },
      ],
      placement: "headphones",
      coupler: "flatPlate",
      passes: 3,
      sweepMs: 1000,
      gapMs: 300,
      captureRateHz: 48000,
    }),
  );
  await page.getByRole("button", { name: "Preflight" }).click();
  await expect(page.locator("#state")).toContainText('"clear": true');
  await page.getByRole("button", { name: "Start the run" }).click();
  await expect.poll(() => record(page), { timeout: 120_000 }).not.toBeNull();
  const r = (await record(page))!;
  const seen = r.phases.map((p) => p.phase);
  for (const phase of ["preRoll", "ramp", "run", "analysing", "done"]) expect(seen, JSON.stringify(r.phases)).toContain(phase);
  expect(r.simulatedMicrophone).toMatchObject({ on: true, label: "Simulated microphone" });
  expect(r.capture).toBeNull();
  expect(r.pushFailures).toBe(0);
  expect(r.setup.startLevelOnly).toBe(true);
  expect(r.maxLevelDb).toBe(-60);
  expect(r.state).toMatchObject({ phase: "done", generatorStarts: 3, frameFaults: 0 });
  expect(r.verdict).toBe("timing reliable");
  expect(r.timing!.reliable).toBe(true);
  expect(r.timing!.cycleResidualsUs).toHaveLength(6);
  expect(r.timing!.maxResidualUs).toBeLessThanOrEqual(r.timing!.residualBoundUs);
  // The room's microphone runs 37.5 ppm fast.
  expect(Math.abs(r.timing!.ratioPpm - 37.5)).toBeLessThan(5);
  expect(r.generatorAfter.state).toBe(0);
  await expect(page.getByRole("button", { name: "Download the record" })).toBeEnabled();
  expect(errors).toEqual([]);
});
