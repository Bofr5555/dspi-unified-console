// WP P5WCAP e2e: the microphone capture (src/core/capture.ts, capture-worklet.ts) in real
// browsers on their fake microphone (playwright.config.ts). Chromium plays P5SIM's rendering
// (test/fixtures/p5wcap-sim.wav, looped): the blocks carry exactly its samples with contiguous
// frame indices. Firefox's fake stream is a tone: the flow only. In both: a denied permission is
// the notice, a stop ends the track and closes the port, the graph connects nothing to a
// destination, and `sinkId` is "none" where the browser has it. Nothing is played.

import { expect, test, type Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

test.skip(({ browserName }) => browserName === "webkit", "Chromium and Firefox only");
// Chromium's headless shell has no getUserMedia ("Not supported"): the full Chromium, headless.
test.use({ channel: [async ({ browserName }, use) => use(browserName === "chromium" ? "chromium" : undefined), { scope: "worker" }] });

interface Info {
  report: { rateHz: number; blockFrames: number; sinkNone: boolean; rateForced: boolean; echoCancellation?: boolean; noiseSuppression?: boolean; autoGainControl?: boolean } | null;
  blocks: { firstFrame: number; len: number; rateHz: number; clipped: boolean }[];
  samples: number;
  ended: boolean;
  events: string[];
  error: { code: string; noticeKey: string | null; message: string } | null;
  trackStates: string[];
  microphones: { deviceId: string; label: string; labelled: boolean }[];
  connections: { from: string; to: string; toDestination: boolean }[];
  continuity: { frames: number; blocks: number; gaps: number; repeats: number } | null;
}
type T = { captureTest: { start(id?: string): Promise<void>; stop(): Promise<void>; info(): Info; samples(from: number, n: number): number[] } };
const info = (page: Page) => page.evaluate(() => (globalThis as unknown as T).captureTest.info());

/** The fixture's samples, from its PCM 16-bit data chunk. */
function fixture(): Float32Array {
  const b = readFileSync(fileURLToPath(new URL("../fixtures/p5wcap-sim.wav", import.meta.url)));
  let at = 12;
  while (at + 8 <= b.length) {
    const id = b.toString("ascii", at, at + 4);
    const len = b.readUInt32LE(at + 4);
    if (id === "data") {
      const out = new Float32Array(len / 2);
      for (let i = 0; i < out.length; i++) out[i] = b.readInt16LE(at + 8 + 2 * i) / 32768;
      return out;
    }
    at += 8 + len + (len & 1);
  }
  throw new Error("no data chunk");
}

async function noPageErrors(page: Page) {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

async function capture(page: Page, seconds: number): Promise<Info> {
  await page.goto("/test/capture.html");
  await page.getByRole("button", { name: "Start" }).click();
  await expect(page.getByRole("status")).not.toHaveText("idle");
  expect((await info(page)).error).toBeNull();
  await expect(page.getByRole("status")).toHaveText("capturing");
  await expect.poll(async () => (await info(page)).samples, { timeout: 15_000 }).toBeGreaterThan(seconds * 44_100 + 4410);
  return info(page);
}

function checkGraph(i: Info, browserName: string) {
  // One connection: the microphone into the worklet; nothing to a destination.
  expect(i.connections.every((c) => !c.toDestination)).toBe(true);
  expect(i.connections.map((c) => `${c.from}>${c.to}`)).toEqual(["MediaStreamAudioSourceNode>AudioWorkletNode"]);
  if (browserName === "chromium") expect(i.report!.sinkNone).toBe(true);
}

/**
 * WCAPFIX: Chromium's fake microphone settles within this long of the capture's first sample.
 * Before that its start is its own: the first 600 to 1400 samples, frame-contiguous, are near
 * silence rather than the file (up to 6 codes off the fixture's lead-in), and under load the
 * source can stall once (one 2176-frame gap seen 640 frames in). Same on 67fa8fff and 915d1a65.
 * Our code passes both on honestly; the exact checks below hold from here on.
 */
const SETTLE_S = 0.2;

function checkFrames(i: Info) {
  const r = i.report!;
  expect(i.blocks.length).toBeGreaterThan(10);
  const settled = i.blocks[0]!.firstFrame + SETTLE_S * r.rateHz;
  let startGaps = 0;
  for (const [k, b] of i.blocks.entries()) {
    expect(b.rateHz).toBe(r.rateHz);
    if (k > 0) {
      const p = i.blocks[k - 1]!;
      // Never a repeat; a gap only while the source starts (WCAPFIX), counted below.
      if (b.firstFrame > p.firstFrame + p.len && p.firstFrame < settled) startGaps++;
      else expect(b.firstFrame, `block ${k}`).toBe(p.firstFrame + p.len);
    }
  }
  expect(startGaps, "at most one stall while the source starts").toBeLessThanOrEqual(1);
  // About 20 ms a block.
  const full = i.blocks.filter((b) => b.len === r.blockFrames).length;
  expect(full).toBeGreaterThanOrEqual(i.blocks.length - 2);
  expect(Math.abs(r.blockFrames / r.rateHz - 0.02)).toBeLessThan(0.003);
  expect(i.continuity).toMatchObject({ gaps: startGaps, repeats: 0 });
}

async function stopAndCheck(page: Page) {
  await page.getByRole("button", { name: "Stop" }).click();
  await expect(page.getByRole("status")).toHaveText("stopped");
  const i = await info(page);
  expect(i.trackStates.length).toBeGreaterThan(0);
  expect(i.trackStates.every((s) => s === "ended")).toBe(true);
  expect(i.events).toEqual(["stopped"]);
  expect(i.ended).toBe(true);
  // Nothing more arrives.
  const n = i.blocks.length;
  await page.waitForTimeout(300);
  expect((await info(page)).blocks.length).toBe(n);
}

test.describe("P5WCAP: granted", () => {
  // Firefox has no permission to grant here: its fake stream comes with the permission prompt off
  // (media.navigator.permission.disabled, playwright.config.ts).
  test.use({ permissions: async ({ browserName }, use) => use(browserName === "firefox" ? [] : ["microphone"]) });

  test("P5WCAP: Chromium: P5SIM's samples arrive exactly, frame-indexed, nothing played; stop ends the track", async ({ page, browserName }) => {
    test.skip(browserName !== "chromium", "the WAV feed is Chromium's");
    const errors = await noPageErrors(page);
    const i = await capture(page, 2);
    checkGraph(i, browserName);
    checkFrames(i);
    // The context runs at the microphone's own rate (Chromium's fake one runs at 44.1 kHz, as the fixture).
    expect(i.report).toMatchObject({ rateHz: 44_100, rateForced: true, echoCancellation: false, noiseSuppression: false, autoGainControl: false });
    expect(i.microphones.length).toBeGreaterThan(0);
    expect(i.microphones.every((m) => m.labelled)).toBe(true);

    // Content: the capture holds the fixture's samples exactly (Chromium loops the file).
    const want = fixture();
    const got = await page.evaluate(() => (globalThis as unknown as T).captureTest.samples(0, 88_200));
    expect(got.length).toBe(88_200);
    // Align on a loud stretch of the capture (the sweeps), then compare everything.
    let loud = 0;
    for (let k = 0; k + 480 < got.length; k += 480) {
      let e = 0;
      for (let j = 0; j < 480; j++) e += got[k + j]! ** 2;
      if (e > 480 * 0.003) {
        loud = k;
        break;
      }
    }
    const L = want.length;
    let best = -1;
    let bestErr = Infinity;
    for (let off = 0; off < L; off++) {
      let err = 0;
      for (let j = 0; j < 256 && err < bestErr; j++) err += Math.abs(got[loud + j]! - want[(off + j) % L]!);
      if (err < bestErr) {
        bestErr = err;
        best = off;
      }
    }
    // Compare the whole pass of the file around that point (Chromium's loop restarts at its own
    // buffer boundary, so the next pass is not exactly one file length later), from where the
    // source has settled (SETTLE_S, WCAPFIX: the first samples are Chromium's start, not ours).
    const from = Math.max(Math.round(SETTLE_S * i.report!.rateHz), loud - best);
    const to = Math.min(got.length, loud + (L - best));
    expect(to - from, "most of one pass of the file").toBeGreaterThan(L / 2);
    let maxDiff = 0;
    let diff2 = 0;
    let energy = 0;
    for (let k = from; k < to; k++) {
      const d = got[k]! - want[best + (k - loud)]!;
      maxDiff = Math.max(maxDiff, Math.abs(d));
      diff2 += d * d;
      energy += got[k]! ** 2;
    }
    expect(energy).toBeGreaterThan(1);
    // The same samples up to Chromium's own 16-bit conversion: a few codes at most, and well
    // under one code on average (the fixture's sweeps peak at about 8600 codes).
    expect(maxDiff, `aligned: capture ${loud} = file ${best}`).toBeLessThanOrEqual(4 / 32768);
    expect(Math.sqrt(diff2 / (to - from))).toBeLessThan(1 / 32768);
    expect(i.blocks.some((b) => b.clipped)).toBe(false);

    await stopAndCheck(page);
    expect(errors).toEqual([]);
  });

  test("P5WCAP: Firefox: the fake stream with processing off, its own rate, the graph without output; flow or a clear refusal", async ({ page, browserName }) => {
    test.skip(browserName !== "firefox", "Firefox's flow");
    const errors = await noPageErrors(page);
    await page.goto("/test/capture.html");
    await page.getByRole("button", { name: "Start" }).click();
    await expect(page.getByRole("status")).not.toHaveText("idle", { timeout: 10_000 });
    let i = await info(page);
    // The graph was built as on any machine: one connection, nothing to a destination.
    checkGraph(i, browserName);
    if ((await page.getByRole("status").textContent()) === "refused") {
      // The Playwright image has no audio device: Firefox's real-time context never starts there.
      // The capture refuses with its notice and releases the microphone, never hangs.
      expect(i.error).toMatchObject({ code: "unsupported", noticeKey: "mic_not_running" });
      await expect(page.getByRole("alert")).toContainText("Microphone capture did not start");
      expect(i.trackStates.length).toBeGreaterThan(0);
      expect(i.trackStates.every((s) => s === "ended")).toBe(true);
      expect(i.blocks).toEqual([]);
    } else {
      await expect.poll(async () => (await info(page)).samples, { timeout: 15_000 }).toBeGreaterThan(44_000);
      i = await info(page);
      checkFrames(i);
      // Firefox keeps the context's own rate: never forced.
      expect(i.report!.rateForced).toBe(false);
      expect(i.report).toMatchObject({ echoCancellation: false, noiseSuppression: false, autoGainControl: false });
      await stopAndCheck(page);
    }
    expect(errors).toEqual([]);
  });
});

test.describe("P5WCAP: denied", () => {
  test("P5WCAP: a denied permission is the notice, and nothing is opened", async ({ page, browserName, context }) => {
    test.skip(browserName === "firefox", "Firefox's prompt cannot be answered headless; its denial is a unit test");
    await context.clearPermissions();
    await page.goto("/test/capture.html");
    await page.getByRole("button", { name: "Start" }).click();
    await expect(page.getByRole("status")).toHaveText("refused");
    const i = await info(page);
    expect(i.error).toMatchObject({ code: "refused", noticeKey: "mic_denied" });
    await expect(page.getByRole("alert")).toContainText("Microphone access denied");
    expect(i.connections).toEqual([]);
    expect(i.blocks).toEqual([]);
  });
});
