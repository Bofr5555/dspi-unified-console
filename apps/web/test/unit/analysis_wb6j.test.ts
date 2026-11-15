// WP WB6j unit tests: the analyser's request and drawing rules, the signal generator's draft and
// its blockers, Stats for nerbs' wording and what a failed read keeps, the monitor's lines, cards
// per row; and on the real core (as main.test.ts) the analyser, the generator with Identify and
// the stats through a Session on the demo device. No audio: the demo device only.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  Analyser,
  DEFAULT_SPECTRUM,
  barsOf,
  channelsOf,
  columnsFor,
  combine,
  followingText,
  levelFrac,
  loadSpectrum,
  populated,
  smooth,
  tidyChoice,
} from "../../src/screens/analysis/analyser";
import { columnsFor as cardColumns, loadPerRow, perRowLabel, perRowSummary, savePerRow, tidyPerRow } from "../../src/screens/analysis/dashboard";
import {
  FLAG_WALK,
  initialDraft,
  phaseText,
  playableMask,
  startBlocker,
  transportDetail,
  transportTitle,
  withLevel,
  withParam,
  withType,
} from "../../src/screens/analysis/siggen";
import { appendLines, copyText, eventCount, formatTemperature, mergeSlow, slowSections, starvationDelta } from "../../src/screens/analysis/stats";
import type { AnalyserView, SiggenCapsInfo, SiggenTypeInfo, StatsRead } from "../../src/core/tools-protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";

class MemStore {
  m = new Map<string, string>();
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
}

// ------------------------------------------------------------------ analyser

test("WB6j: the request is the newest view's tap with every view's channels on it; none releases", () => {
  assert.equal(combine([]), null);
  assert.deepEqual(combine([{ tap: 0, mask: 1 }]), { tap: 0, mask: 1 });
  assert.deepEqual(combine([{ tap: 0, mask: 1 }, { tap: 0, mask: 4 }]), { tap: 0, mask: 5 });
  // The newest view on the outputs wins the tap; the inputs' channels are not mixed in.
  assert.deepEqual(combine([{ tap: 0, mask: 3 }, { tap: 1, mask: 2 }]), { tap: 1, mask: 2 });
  assert.deepEqual(channelsOf(0b1010), [1, 3]);
  assert.equal(columnsFor(1), 1);
  assert.equal(columnsFor(9), 4);
});

test("WB6j: a level's height between floor and ceiling; populated bands; smoothing keeps 0.35", () => {
  assert.equal(levelFrac(-90, -90, 6), 0);
  assert.equal(levelFrac(6, -90, 6), 1);
  assert.equal(levelFrac(-42, -90, 6), 0.5);
  assert.equal(levelFrac(-Infinity, -90, 6), 0);
  assert.equal(populated("5", 0), true);
  assert.equal(populated("5", 1), false);
  assert.equal(populated(String(2n ** 63n), 63), true, "all 64 bits");
  assert.deepEqual(smooth(undefined, [-10], true), [-10]);
  assert.deepEqual(smooth([-20], [-10], false), [-10]);
  const [v] = smooth([-20], [-10], true);
  assert.ok(Math.abs(v! - (-20 * 0.35 + -10 * 0.65)) < 1e-9);
});

test("WB6j: bars are the populated bands with a centre, per channel", () => {
  const v = {
    caps: { centres: [20, 40, 80, 0], maxBands: 4 },
    snapshot: { populated: String(0b1101), frames: [{ channel: 1, nBands: 4, silent: false, avgDb: [-10, -20, -30, -40], peakDb: [-5, -15, -25, -35] }] },
    ownership: { following: false, tap: 0, mask: 0, fftOrder: 0 },
  } as unknown as AnalyserView;
  assert.equal(barsOf(v, 0), null, "a channel not analysed");
  // Band 1 has no data, band 3 no centre.
  assert.deepEqual(barsOf(v, 1), [
    { hz: 20, db: -10, peak: -5 },
    { hz: 80, db: -30, peak: -25 },
  ]);
});

test("WB6j: following another console is said with the channels' names", () => {
  const names = { inputs: ["IN1 Left", "IN2 Right"], outputs: ["OUT1 Sub"] };
  assert.equal(followingText({ following: true, tap: 0, mask: 3, fftOrder: 10 }, names), "Another console is running the analyser: IN1 Left, IN2 Right.");
  assert.equal(followingText({ following: true, tap: 1, mask: 2, fftOrder: 10 }, names), "Another console is running the analyser: Out 2.");
  assert.equal(followingText({ following: true, tap: 1, mask: 0, fftOrder: 10 }, names), "Another console is running the analyser on the outputs.");
});

test("WB6j: the Spectrum preferences are WB6i's dspi-web.prefs, checked; the choice is tidied", () => {
  const st = new MemStore();
  assert.deepEqual(loadSpectrum(st), DEFAULT_SPECTRUM);
  st.setItem("dspi-web.prefs", "{broken");
  assert.deepEqual(loadSpectrum(st), DEFAULT_SPECTRUM);
  st.setItem("dspi-web.prefs", JSON.stringify({ graph: {}, spectrum: { floorDb: -120, ceilingDb: 12, fftOrder: 9, showPeakHold: false, graphOpacity: 9 } }));
  const p = loadSpectrum(st);
  assert.equal(p.floorDb, -120);
  assert.equal(p.ceilingDb, 12);
  assert.equal(p.fftOrder, 9);
  assert.equal(p.showPeakHold, false);
  assert.equal(p.graphOpacity, 1, "held to its range");
  assert.deepEqual(tidyChoice({ bars: true, tap: 7, mask: -1 }), { bars: true, graph: false, tap: 0, mask: 3 });
});

// ------------------------------------------------------------ signal generator

const caps: SiggenCapsInfo = {
  supported: true,
  outputChannels: 4,
  multitoneMax: 8,
  validMask: 0b0111,
  types: [
    { id: 0, tile: "Sine", name: "Sine", blurb: "", timing: 0, offered: true, params: [{ label: "Frequency", semantic: 1, unit: "Hz", min: 1, max: 30000, def: 1000 }, ...Array(3).fill({ label: "", semantic: 0, unit: "", min: 0, max: 0, def: 0 })] },
    {
      id: 4,
      tile: "Log Swp",
      name: "Log sweep",
      blurb: "",
      timing: 1,
      offered: true,
      params: [
        { label: "Start", semantic: 1, unit: "Hz", min: 1, max: 30000, def: 20 },
        { label: "End", semantic: 1, unit: "Hz", min: 1, max: 30000, def: 20000 },
        ...Array(2).fill({ label: "", semantic: 0, unit: "", min: 0, max: 0, def: 0 }),
      ],
    },
  ] as SiggenTypeInfo[],
};

test("WB6j: the generator's draft: a type's defaults, ranges held, the level to -120..0", () => {
  const d = initialDraft();
  assert.equal(d.levelDb, -20);
  assert.equal(d.channelMask, 3);
  const sweep = withType(d, caps.types[1]!);
  assert.deepEqual(sweep.p, [20, 20000, 0, 0]);
  assert.equal(sweep.durationMs, 5000, "a sweep gets a length");
  assert.equal(withParam(sweep, caps.types[1]!, 0, 99999).p[0], 30000);
  assert.equal(withParam(sweep, caps.types[1]!, 2, 5).p[2], 0, "an unused parameter is left alone");
  assert.equal(withLevel(d, 12).levelDb, 0);
  assert.equal(withLevel(d, -500).levelDb, -120);
  assert.equal(withLevel(d, NaN).levelDb, -20);
  assert.equal(playableMask(caps, 4), 0b0111, "the device's valid mask");
  assert.equal(playableMask(null, 2), 0b11);
});

test("WB6j: what stops Play, and the transport's words", () => {
  const d = initialDraft();
  assert.equal(startBlocker(false, caps, d, ""), "No device connected");
  assert.equal(startBlocker(true, { ...caps, supported: false }, d, ""), "Firmware has no signal generator");
  assert.equal(startBlocker(true, caps, d, "read only"), "read only");
  assert.equal(startBlocker(true, caps, { ...d, channelMask: 0 }, ""), "Select at least one output");
  assert.equal(startBlocker(true, caps, { ...withType(d, caps.types[1]!), durationMs: 0 }, ""), "Sweep length must be greater than 0");
  assert.equal(startBlocker(true, caps, d, ""), "");
  assert.equal(transportTitle(caps, d, null, ""), "Ready · Sine");
  const run = { state: 2, signalType: 4, activeChannel: 1, elapsedMs: 65000, cyclesDone: 0, stopReason: 0, currentFreq: 1500 };
  assert.equal(transportTitle(caps, d, run, ""), "Running · Log sweep");
  assert.equal(transportDetail(run, (o) => `OUT${o + 1}`), "1:05 · 1.50 kHz · OUT2");
  assert.equal(transportDetail({ ...run, state: 0 }, String), "");
  assert.equal(phaseText(4), "Fading out");
  assert.equal(FLAG_WALK, 4);
});

// ------------------------------------------------------------- stats, monitor

const slow = (over: Partial<StatsRead> = {}): StatsRead => ({
  counters: { clockHz: 307_200_000, coreMv: 1200, sampleRate: 48000, temperature: 3150, starvationTotal: 0, spdifOverruns: 0, spdifUnderruns: 0 },
  starvation: [],
  ring: { present: true, targetFrames: 96, targetUs: 2000, batchFrames: 48, underrunEpisodes: 0, refusedBlocks: 0, lowestFill: 40, worstInputGapUs: 1500, worstUsbGapUs: 900, overBudgetSteps: 0 },
  spdifRx: null,
  lg: null,
  adat: null,
  i2s: null,
  clockDiag: null,
  usbErrors: { total: 0, crc: 0, bitstuff: 0, rxOverflow: 0, rxTimeout: 0, dataSeq: 0 },
  ...over,
});

test("WB6j: Stats for nerbs' sections in the Console's words; absent ones left out", () => {
  const s = slowSections(slow());
  assert.deepEqual(
    s.map((x) => x.title),
    ["System Information", "Audio Output", "Output Ring Health"],
  );
  assert.deepEqual(s[0]!.rows.map((r) => r.text), ["307.2 MHz", "1.20 V", "48.0 kHz", "31.5 °C"]);
  assert.equal(formatTemperature(2 ** 32 - 500), "-5.0 °C");
  const more = slowSections(slow({ usbErrors: { total: 3, crc: 3, bitstuff: 0, rxOverflow: 0, rxTimeout: 0, dataSeq: 0 }, counters: { starvationTotal: 2 }, starvation: [1, 1, 0, 0] }));
  assert.ok(more.some((x) => x.title === "USB Errors"));
  assert.ok(more.some((x) => x.title === "S/PDIF DMA Starvation"));
  assert.equal(slowSections(slow({ ring: { present: false } })).some((x) => x.title === "Output Ring Health"), false);
});

test("WB6j: a failed read keeps the last values; a lower starvation total is a new base", () => {
  const a = slow();
  const b = slow({ ring: null, counters: { ...a.counters, clockHz: null } });
  const m = mergeSlow(a, b);
  assert.equal(m.ring, a.ring);
  assert.equal(m.counters.clockHz, 307_200_000);
  assert.equal(starvationDelta(null, 5), 0);
  assert.equal(starvationDelta(5, 8), 3);
  assert.equal(starvationDelta(8, 2), 0, "a reboot");
});

test("WB6j: the monitor keeps the newest lines, counts and copies them", () => {
  const line = (id: number) => ({ id, timeMs: Date.UTC(2026, 0, 1, 12, 0, 0, id), text: `line ${id}` });
  const lines = appendLines([line(1), line(2)], [line(3), line(4)], 3);
  assert.deepEqual(lines.map((l) => l.id), [2, 3, 4]);
  assert.equal(eventCount(1), "1 event");
  assert.equal(eventCount(3), "3 events");
  assert.match(copyText([line(7)]), /^\d\d:\d\d:\d\d\.007  line 7$/);
});

test("WB6j: cards per row: Auto or 1..3, kept, held to the cards there are", () => {
  const st = new MemStore();
  assert.equal(loadPerRow(st), 0);
  savePerRow(st, 2);
  assert.equal(loadPerRow(st), 2);
  savePerRow(st, 9);
  assert.equal(loadPerRow(st), 0);
  assert.equal(tidyPerRow(3), 3);
  assert.equal(cardColumns(3, 2), 2);
  assert.equal(cardColumns(1, 4), 1);
  assert.equal(perRowLabel(0), "Auto");
  assert.equal(perRowLabel(1), "1 card per row");
  assert.equal(perRowSummary(2), "Up to 2 cards per row.");
});

// -------------------------------------------------- on the real core (demo device)

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(access: "control" | "view") {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const calls: string[] = [];
  const client = {
    call: (m: never, p: never) => {
      calls.push(m);
      return core.call(m, p);
    },
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  return { session: new Session("s", "i", "Here", { kind: "demo" }, access, client), calls };
}

const until = async (what: () => boolean | Promise<boolean>, ms = 5000) => {
  const end = Date.now() + ms;
  while (!(await what())) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 20));
  }
};

test("WB6j: the analyser on the demo: a view starts it, the picture comes, the last view releases it", { skip }, async () => {
  const { session, calls } = await sessionOn("control");
  await session.open({ kind: "demo" });
  const a = new Analyser(session, new MemStore(), 30);
  a.setView("strip", { tap: 0, mask: 0b11 });
  await until(() => (a.view?.snapshot.frames.length ?? 0) > 0);
  assert.equal(a.view!.caps.supported, true);
  assert.ok(a.view!.caps.centres.length > 0);
  assert.equal(a.view!.ownership.following, false);
  assert.ok(barsOf(a.view!, 0)!.length > 0, "bars to draw");
  assert.ok(calls.includes("rtaStart"));
  a.setView("strip", null);
  assert.equal(a.view, null);
  await until(() => calls.includes("rtaStop"));
  a.stop();
});

test("WB6j: Identify and the generator on the demo, through the core; nothing plays read only", { skip }, async () => {
  const { session } = await sessionOn("control");
  await session.open({ kind: "demo" });
  const c = await session.core.call("siggenCaps", undefined);
  assert.equal(c.supported, true);
  await session.core.call("siggenIdentify", { output: 0 });
  const cfg = await session.core.call("siggenConfig", undefined);
  assert.deepEqual([cfg.signalType, cfg.channelMask, cfg.levelDb, cfg.repeat], [14, 1, -12, 2]);
  assert.notEqual((await session.core.call("siggenStatus", undefined)).state, 0);
  await session.core.call("siggenControl", { action: "stopNow" });
  assert.equal((await session.core.call("siggenStatus", undefined)).state, 0);

  const viewer = await sessionOn("view");
  await viewer.session.open({ kind: "demo" });
  await assert.rejects(viewer.session.core.call("siggenIdentify", { output: 0 }));
  assert.equal((await viewer.session.core.call("siggenStatus", undefined)).state, 0);
});

test("WB6j: Stats for nerbs and the monitor read the demo", { skip }, async () => {
  const { session } = await sessionOn("control");
  await session.open({ kind: "demo" });
  const s = await session.core.call("statsRead", undefined);
  assert.ok(slowSections(s).some((x) => x.title === "System Information"));
  const t = await session.core.call("statsTick", undefined);
  assert.equal(typeof t.buffers.streaming, "boolean");
  await session.core.call("statsResetWatermarks", undefined);
  await session.core.call("siggenSet", { config: { ...initialDraft() } });
  await session.core.call("siggenControl", { action: "start" });
  await session.core.call("siggenControl", { action: "stopNow" });
  await until(async () => (await session.core.call("monitorRead", { afterId: 0 })).entries.some((e) => e.text.includes("SiggenState")));
});
