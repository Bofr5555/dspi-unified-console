// WP WEB6 unit tests: the Spectrum page's rules as Qt's SpectrumPanel and RtaController say them:
// the Curves / Bars / Both choice kept with the analyser's choice, the status line's parts, the bass
// load, the low-band and range notes, the marks.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { AnalyserView } from "../../src/core/tools-protocol";
import { bassText, dbTicks, hzText, lowBandText, rangeText, statusParts, tidyChoice } from "../../src/screens/analysis/analyser";

function view(snap: Partial<AnalyserView["snapshot"]> = {}, caps: Partial<AnalyserView["caps"]> = {}): AnalyserView {
  return {
    caps: { supported: true, inputChannels: 2, outputChannels: 9, orderMin: 8, orderMax: 12, orderDefault: 10, bassBands: 8, maxBands: 40, dynamicRangeDb: 120, centres: Array.from({ length: 40 }, (_, i) => 20 * 2 ** (i / 4)), bassDynamicRangeDb: 70, ...caps },
    snapshot: { version: 1, tap: 1, count: 1, frames: [], populated: "0", running: true, liveMask: 1, framesPerS: 46, sampleRate: 48000, fftOrder: 10, firstBand: 0, configRejected: false, liveCount: 1, refreshIntervalS: 0.022, lastFrameUs: 1900, busyUsPerS: 66000, bassBusyUsPerS: 1200, ...snap },
    ownership: { following: false, tap: 1, mask: 1, fftOrder: 10 },
  };
}

test("WEB6: the view choice is kept and checked", () => {
  assert.equal(tidyChoice({}).view, "curves");
  assert.equal(tidyChoice({ view: "bars" }).view, "bars");
  assert.equal(tidyChoice({ view: "both" }).view, "both");
  assert.equal(tidyChoice({ view: "pie" }).view, "curves");
});

test("WEB6: the status line says what Qt's says", () => {
  const v = view();
  assert.deepEqual(statusParts(v.snapshot), ["1 channel, each refreshed every 22 ms", "46 frames/s", "transform 1900 µs", "main loop 6.6%"]);
  assert.deepEqual(statusParts(view({ liveCount: 3, refreshIntervalS: 0.066 }).snapshot)[0], "3 channels, each refreshed every 66 ms");
  assert.deepEqual(statusParts(view({ running: false, framesPerS: 0, lastFrameUs: 0, busyUsPerS: 0 }).snapshot), ["idle"]);
  // An older core without the telemetry: the live mask counts, nothing is made up
  assert.deepEqual(statusParts(view({ liveCount: undefined, refreshIntervalS: undefined, lastFrameUs: undefined, busyUsPerS: undefined, liveMask: 0b101 }).snapshot), ["2 channels, each refreshed every 0 ms", "46 frames/s"]);
  assert.equal(bassText(v.snapshot), "bass 0.12%");
  assert.equal(bassText(view({ bassBusyUsPerS: 0xffff }).snapshot), "bass ≥6.55%");
  assert.equal(bassText(view({ bassBusyUsPerS: 0 }).snapshot), "");
  assert.equal(rangeText(v), "120/70 dB range");
  assert.equal(rangeText(view({}, { dynamicRangeDb: 0 })), "");
});

test("WEB6: the low bands' note only below the largest transform, with a first band", () => {
  assert.equal(lowBandText(view({ firstBand: 4 }), 10), "shaded bands below 40 Hz need a larger transform size in Settings");
  assert.equal(lowBandText(view({ firstBand: 4 }), 12), "", "the largest size: nothing to suggest");
  assert.equal(lowBandText(view({ firstBand: 0 }), 10), "");
  assert.equal(lowBandText(view({ firstBand: 0xff }), 10), "");
  assert.equal(lowBandText(view({ firstBand: 4, running: false }), 10), "");
});

test("WEB6: the marks", () => {
  assert.deepEqual(dbTicks(-90, 6), [0, -12, -24, -36, -48, -60, -72, -84]);
  assert.deepEqual(dbTicks(-60, 24), [24, 12, 0, -12, -24, -36, -48, -60]);
  assert.deepEqual([20, 500, 1000, 20000].map(hzText), ["20", "500", "1k", "20k"]);
});
