// WP WEB7 unit tests: the Signal Generator's and Stats for nerbs' rules as Qt Modern's panels have
// them (SiggenController::stepOutput and the timing rows; StatsController::fillRows and its bands).

import { test } from "node:test";
import assert from "node:assert/strict";
import { initialDraft, withType } from "../../src/screens/analysis/siggen";
import { CHANNEL_ID, idleDetail, noOutputs, stepOutput, timingRows, withTiming } from "../../src/screens/analysis/siggen-page";
import { elapsed, fillRows, otherSections, spdifBand, withHistory } from "../../src/screens/analysis/stats-page";
import type { SiggenTypeInfo, StatsRead, StatsTickRecord } from "../../src/core/tools-protocol";

const type = (id: number, timing: number): SiggenTypeInfo => ({ id, tile: "", name: "", blurb: "", timing, params: [], offered: true });

test("WEB7: an output's chip steps off, on, inverted, off; None clears the inversions", () => {
  let d = { ...initialDraft(), channelMask: 0, invertMask: 0 };
  d = stepOutput(d, 2);
  assert.deepEqual([d.channelMask, d.invertMask], [4, 0]);
  d = stepOutput(d, 2);
  assert.deepEqual([d.channelMask, d.invertMask], [4, 4]);
  d = stepOutput(d, 2);
  assert.deepEqual([d.channelMask, d.invertMask], [0, 0]);
  assert.equal(stepOutput(d, 16), d, "past the 16 outputs: nothing");
  const n = noOutputs({ ...d, channelMask: 3, invertMask: 1 });
  assert.deepEqual([n.channelMask, n.invertMask], [0, 0]);
  assert.equal(idleDetail({ ...initialDraft(), invertMask: 2 }), "2 outputs (1 inverted) · peak -20.0 dBFS");
});

test("WEB7: the timing rows by the type's timing model, Sine's Duration included", () => {
  const d = initialDraft();
  assert.deepEqual(timingRows(type(0, 0), d).map((r) => [r.label, r.caption]), [["Duration", "0 = play until stopped"]]);
  assert.deepEqual(timingRows(type(0, 0), { ...d, flags: 4 }).map((r) => r.label), ["Dwell per channel", "Passes"]);
  assert.deepEqual(timingRows(type(4, 1), d).map((r) => r.label), ["Sweep length", "Repeat", "Gap between sweeps"]);
  assert.deepEqual(timingRows(type(7, 2), d).map((r) => r.caption), ["Pattern periods. 0 = repeat forever", ""]);
  assert.match(timingRows(type(CHANNEL_ID, 2), d)[0]!.caption, /Passes over the selected outputs/);
  assert.equal(withTiming(d, "duration", 2.5, false).durationMs, 2500);
  assert.equal(withTiming(d, "duration", 0, true).durationMs, 10, "a sweep is never 0 s long");
  assert.equal(withTiming(d, "repeat", 70000, false).repeat, 65535);
  assert.equal(withTiming(d, "gap", -5, false).gapMs, 0);
  // A continuous type chosen starts with no duration (play until stopped).
  assert.equal(withType({ ...d, durationMs: 4000 }, type(0, 0)).durationMs, 0);
});

const tick = (over: Partial<StatsTickRecord["buffers"]> = {}, fills: StatsTickRecord["fills"] = null): StatsTickRecord => ({
  buffers: {
    numSpdif: 2,
    pdmActive: false,
    streaming: true,
    sequence: 1,
    spdif: [
      { free: 0, prepared: 0, playing: 0, fillPct: 50, minPct: 40, maxPct: 60 },
      { free: 0, prepared: 0, playing: 0, fillPct: 10, minPct: 0, maxPct: 20 },
    ],
    pdm: { dmaFillPct: 10, dmaMinPct: 5, dmaMaxPct: 15, ringFillPct: 30, ringMinPct: 20, ringMaxPct: 40 },
    ...over,
  },
  fills,
});

test("WEB7: the fill bars as Qt's: a pair per S/PDIF or I2S slot, the core's fills first, the PDM pair while it plays", () => {
  const rows = fillRows(tick(), (pair) => pair === 1);
  assert.deepEqual(rows.map((r) => [r.title, r.fill, r.band]), [
    ["Out 1/2 (SPDIF)", 50, "good"],
    ["Out 3/4 (I2S)", 10, "warn"],
  ]);
  const core = fillRows(tick({}, [{ fillPct: 37.5, minPct: 38, maxPct: 38, band: "good" }]), () => false);
  assert.deepEqual([core[0]!.fill, core[0]!.min, core[0]!.band], [37.5, 38, "good"]);
  const pdm = fillRows(tick({ pdmActive: true }), () => false);
  assert.deepEqual(pdm.slice(2).map((r) => [r.title, r.band, r.dashed]), [
    ["PDM DMA", "good", false],
    ["PDM Ring", "warn", true],
  ]);
  assert.equal(spdifBand(0), "bad");
  assert.equal(spdifBand(100), "bad");
  assert.equal(spdifBand(80), "warn");
  // 15 s of history at most, a row gone dropped.
  let h = new Map<string, number[]>();
  for (let i = 0; i < 70; i++) h = withHistory(h, rows);
  assert.equal(h.get("out0")!.length, 60);
  assert.equal(withHistory(h, rows.slice(0, 1)).has("out1"), false);
  assert.equal(elapsed(65_000), "1m 05s");
  assert.equal(elapsed(3_723_000), "1h 02m 03s");
});

test("WEB7: the right column holds what the left and middle do not, LG Sound Sync in Qt's words", () => {
  const s: StatsRead = {
    counters: { clockHz: 1, starvationTotal: 1 },
    starvation: [1, 0],
    ring: { present: true, targetFrames: 192, targetUs: 4000 },
    spdifRx: null,
    lg: { enabled: true, present: false, volume: 255, muted: false },
    adat: null,
    i2s: null,
    clockDiag: null,
    usbErrors: { total: 2, crc: 2, bitstuff: 0, rxOverflow: 0, rxTimeout: 0, dataSeq: 0 },
  };
  const o = otherSections(s);
  assert.deepEqual(o.map((x) => x.title), ["LG Sound Sync", "USB Errors"]);
  assert.deepEqual(o[0]!.rows.map((r) => `${r.label} ${r.text}`), ["Enabled Yes", "Present No", "TV Volume -", "TV Mute Off"]);
});
