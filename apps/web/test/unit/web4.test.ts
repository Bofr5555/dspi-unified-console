// WP WEB4 unit tests: Library › History in Qt's words (DL3a's HistoryController::describe, ported to
// src/screens/history/model.ts), and the signal generator's waveform tiles (src/screens/analysis/waves.ts).

import { test } from "node:test";
import assert from "node:assert/strict";
import type { HistoryEntry } from "../../src/core/protocol";
import { XOVER_BAND_BASE, describeEntry, historyRows, labelOf, valueText, type HistoryNames } from "../../src/screens/history/model";
import { wavePath } from "../../src/screens/analysis/waves";

const names: HistoryNames = {
  wire: (w) => (w < 2 ? `In ${w + 1}` : w === 3 ? "Sub" : `Out ${w - 1}`),
  output: (o) => (o === 1 ? "Sub" : `Out ${o + 1}`),
  input: (i) => `In ${i + 1}`,
};

const e = (path: string, indices: number[], before: HistoryEntry["before"], after: HistoryEntry["after"]): HistoryEntry => ({
  path,
  indices,
  before,
  after,
  source: "user",
  atMs: 1000,
  undoable: true,
});

const raw = (p: string) => `raw:${p}`;

test("WEB4: History names the channel, the band and the value with its unit, as Qt", () => {
  assert.deepEqual(describeEntry(e("eq.q", [2, 2], { float: 0.71 }, { float: 0.7 }), names, raw), { title: "Q, Out 1, band 3", detail: "0.71 → 0.70" });
  assert.deepEqual(describeEntry(e("eq.gain", [3, 0], { float: 0 }, { float: -1.5 }), names, raw), { title: "Gain, Sub, band 1", detail: "0.0 dB → −1.5 dB" });
  assert.deepEqual(describeEntry(e("eq.freq", [2, 1], { float: 1000 }, { float: 180 }), names, raw), { title: "Frequency, Out 1, band 2", detail: "1000 Hz → 180 Hz" });
  // A crossover band is a filter from XOVER_BAND_BASE on
  assert.equal(describeEntry(e("eq.freq", [3, XOVER_BAND_BASE], { float: 850 }, { float: 3000 }), names, raw).title, "Crossover frequency, Sub, filter 1");
  assert.equal(describeEntry(e("out.gain", [1], { float: 0 }, { float: -3 }), names, raw).title, "Output gain, Sub");
  assert.equal(describeEntry(e("mix", [0, 2], { float: 0 }, { float: -6 }), names, raw).title, "Crosspoint, In 1 to Out 3");
});

test("WEB4: History's presets by slot, the generator in words, effects' settings by effect", () => {
  assert.equal(describeEntry(e("preset.save", [0], null, "trigger"), names, raw).title, "Saved to preset 1");
  assert.equal(describeEntry(e("preset.load", [2], null, "trigger"), names, raw).title, "Preset 3 loaded");
  assert.deepEqual(describeEntry(e("sig.control", [], { choice: 0 }, { choice: 1 }), names, raw), { title: "Signal generator started", detail: "" });
  assert.equal(describeEntry(e("sig.control", [], { choice: 1 }, { choice: 2 }), names, raw).title, "Signal generator stopped at once");
  assert.equal(describeEntry(e("sig.control", [], { choice: 1 }, { choice: 0 }), names, raw).title, "Signal generator stopped");
  // -- WP INTE -- (SHOWFIX finding 5) the core's one row for a measurement, by its words
  const run = { ...e("sig.run", [], null, { int: 13 }), label: "Measurement: Centre woofer, Sub, 13 sweeps" };
  assert.deepEqual(describeEntry(run, names, raw), { title: "Measurement: Centre woofer, Sub, 13 sweeps", detail: "" });
  assert.equal(labelOf("loud.ref", raw), "Loudness › Reference level");
  assert.equal(labelOf("loud.enable", raw), "Loudness");
  assert.equal(labelOf("vol.master", raw), "Master volume");
  // A path the glossary does not name: the registry's label
  assert.equal(labelOf("something.new", raw), "raw:something.new");
  assert.equal(valueText({ float: -12 }, "vol.user"), "−12.0 dB");
  assert.equal(valueText({ bool: true }, "vol.mute"), "On");
});

test("WEB4: History's rows use the words when the names are known, and the old rows without", () => {
  const list = [e("vol.user", [], { float: 0 }, { float: -12 })];
  assert.deepEqual(historyRows(list, raw, 1000, names).map((r) => [r.what, r.values]), [["Volume", "0.0 dB → −12.0 dB"]]);
  assert.deepEqual(historyRows(list, raw, 1000).map((r) => [r.what, r.values]), [["raw:vol.user", "0 → −12"]]);
});

test("WEB4: every generator tile draws a path inside its 100 × 28 box", () => {
  for (const tile of ["Sine", "Square", "White", "Pink", "Log Swp", "Lin Swp", "Step Swp", "Impulse", "Clicks", "Polarity", "Burst", "2-Tone", "Multi", "ISP", "Chan ID", "Unknown"]) {
    const d = wavePath(tile);
    assert.match(d, /^M/, tile);
    for (const [, x, y] of d.matchAll(/[ML](-?[\d.]+),(-?[\d.]+)/g)) {
      assert.ok(Number(x) >= 0 && Number(x) <= 100, `${tile} x ${x}`);
      assert.ok(Number(y) >= 0 && Number(y) <= 28, `${tile} y ${y}`);
    }
  }
  assert.notEqual(wavePath("Sine"), wavePath("Square"));
});
