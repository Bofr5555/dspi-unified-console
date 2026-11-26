// WP WEB3 unit tests: the Overview's channel cards (DL6 on the web: which inputs get a card, the
// chips' words, the mini response's path) and the effects' Qt words (DL1 on the web: titles,
// captions, sections).

import { test } from "node:test";
import assert from "node:assert/strict";
import type { DspState, FilterParams } from "../../src/core/protocol";
import { bandsChip, dbShort, feedsChip, freqShort, inputCard, miniPath, outputCard, overviewCards, xoverChips } from "../../src/screens/overview/channel-cards";
import { groupBySection, maskLabel, maskSection, paramCaption, paramTitle } from "../../src/screens/effects/texts";

const flat = (): FilterParams => ({ filterType: 0, bypass: false, freq: 1000, q: 0.707, gain: 0, qp: 0.707 });
const peak = (): FilterParams => ({ filterType: 1, bypass: false, freq: 100, q: 1, gain: -3, qp: 0.707 });

function state(over: Partial<DspState> = {}): DspState {
  const ins = 4;
  const outs = 3;
  return {
    platformId: 1,
    numInputChannels: ins,
    numOutputChannels: outs,
    maxBands: 4,
    channelNames: ["L", "R", "C", "LFE"],
    filters: Array.from({ length: ins + outs }, () => [flat(), flat(), flat(), flat()]),
    xover: Array.from({ length: ins + outs }, () => []),
    matrixRouting: [
      [true, false, false],
      [false, false, false],
      [false, true, true],
      [false, false, false],
    ],
    matrixGain: Array.from({ length: ins }, () => [0, 0, 0]),
    matrixInvert: [
      [false, false, false],
      [false, false, false],
      [false, false, true],
      [false, false, false],
    ],
    inputPreampDb: [0, 0, 0, 0],
    outputEnabled: [true, true, false],
    outputGainDb: [0, -5, 0],
    outputDelayMs: [0, 1.25, 0],
    outputMuted: [false, true, false],
    ...over,
  } as unknown as DspState;
}

test("WEB3 DL6: inputs in use get a card (routed, bands or a trim), the rest are named on one line; outputs that are on", () => {
  const s = state();
  s.filters[3]![0] = peak();
  const c = overviewCards(s, undefined, 4);
  assert.deepEqual(c.inputs.map((x) => x.descriptor), ["IN1", "IN3", "IN4"]);
  assert.deepEqual(c.idle, ["R"]);
  assert.deepEqual(c.outputs.map((x) => x.descriptor), ["OUT1", "OUT2"]);
  // Only the live inputs are looked at.
  assert.deepEqual(overviewCards(s, undefined, 2).inputs.map((x) => x.descriptor), ["IN1"]);
});

// -- WP REVEFIX --
test("REVEFIX: an output the core calls unused has no card (owner 2026-11-24: not shown where nothing is edited)", () => {
  const s = state({ outputEnabled: [true, true, true] } as Partial<DspState>);
  const label = (o: number, unused: boolean) => ({ output: o, label: unused ? `${o + 1} · None` : `Out ${o + 1}`, shortLabel: "", slotName: "", userNamed: false, channel: unused ? "None" : "L", band: "Full", unused, hidden: unused });
  const labels = [label(0, false), label(1, true), label(2, false)];
  assert.deepEqual(overviewCards(s, labels, 4).outputs.map((x) => x.descriptor), ["OUT1", "OUT3"]);
});

test("WEB3 DL6: the chips say what is set, in Qt's words", () => {
  const s = state();
  assert.deepEqual(inputCard(s, 2).chips.map((x) => x.text), ["No bands", "to Out 2 + Out 3"]);
  assert.deepEqual(inputCard(s, 1).chips, [{ text: "No bands", kind: "muted" }, { text: "To no output", kind: "muted" }]);
  s.inputPreampDb[0] = 2;
  assert.deepEqual(inputCard(s, 0).chips.map((x) => x.text), ["No bands", "to Out 1", "Preamp +2 dB"]);
  // An output: from its input, its delay, its gain, muted
  assert.deepEqual(outputCard(s, undefined, 1).chips.map((x) => x.text), ["No bands", "from C", "1.25 ms", "−5 dB", "Muted"]);
  // An inverted crosspoint is said in the warning's tone
  const o3 = outputCard(s, undefined, 2).chips;
  assert.deepEqual(o3[o3.length - 1], { text: "Inverted", kind: "warn" });
  assert.deepEqual(outputCard(s, undefined, 0).chips.slice(-1), [{ text: "from L" }]);
  assert.deepEqual(bandsChip(1), { text: "1 band" });
  assert.deepEqual(bandsChip(3), { text: "3 bands" });
  assert.deepEqual(feedsChip([0, 1, 2, 3]), { text: "to 4 outputs" });
  assert.equal(freqShort(400), "400 Hz");
  assert.equal(freqShort(3000), "3 kHz");
  assert.equal(freqShort(2500), "2.5 kHz");
  assert.equal(dbShort(-1.5), "−1.5 dB");
  assert.equal(dbShort(6), "+6 dB");
});

test("WEB3 DL6: a crossover's first high and low pass, family and slope: 'HP LR24 3 kHz'", () => {
  // config.h: LR4 low pass 34, LR4 high pass 35, BW2 high pass 43
  const lr4hp: FilterParams = { filterType: 35, bypass: false, freq: 3000, q: 0.707, gain: 0, qp: 0.707 };
  const lr4lp: FilterParams = { filterType: 34, bypass: false, freq: 400, q: 0.707, gain: 0, qp: 0.707 };
  assert.deepEqual(xoverChips([lr4lp, lr4hp]).map((x) => x.text), ["HP LR24 3 kHz", "LP LR24 400 Hz"]);
  assert.deepEqual(xoverChips([{ ...lr4hp, bypass: true }]), []);
  assert.deepEqual(xoverChips([{ ...lr4hp, filterType: 43, freq: 80 }]).map((x) => x.text), ["HP BW12 80 Hz"]);
});

test("WEB3 DL6: the mini response's path spans the box; nothing to draw is no path", () => {
  const p = miniPath([20, 1000, 20000], [0, 0, 0], 0);
  assert.equal(p.line, "M0.00 16.00L56.63 16.00L100.00 16.00");
  assert.ok(p.fill.endsWith("L100.00 48L0.00 48Z"));
  // Clamped to the box (+12..−24 dB), the offset added
  assert.equal(miniPath([20], [30], 0).line, "M0.00 0.00");
  assert.equal(miniPath([20], [-30], 0).line, "M0.00 48.00");
  assert.equal(miniPath([20], [0], -24).line, "M0.00 48.00");
  assert.deepEqual(miniPath([], [], 0), { line: "", fill: "" });
});

test("WEB3 DL1: Qt's titles and captions, the registry's where the table has none; sections in order", () => {
  assert.equal(paramTitle("lev.maxgain", "Maximum boost"), "Max Gain");
  assert.equal(paramTitle("cf.freq", "Cutoff frequency"), "Cutoff Frequency");
  assert.equal(paramTitle("no.such", "Its label"), "Its label");
  assert.equal(paramCaption("lev.lookahead", "plain"), "Adds 5ms latency. Improves transient handling.");
  assert.equal(paramCaption("no.such", "plain"), "plain");
  assert.equal(paramCaption("no.such", undefined), "");
  assert.equal(maskSection("pairs"), "Output Pairs");
  assert.equal(maskSection("detector"), "Channels");
  assert.equal(maskLabel("apply"), "Apply");
  assert.equal(maskLabel("outputs"), "Outputs");
  const groups = groupBySection(["a", "b", "c", "d"], (x) => (x === "a" ? undefined : x === "d" ? "Two" : "One"));
  assert.deepEqual(groups, [
    { section: "Parameters", items: ["a"] },
    { section: "One", items: ["b", "c"] },
    { section: "Two", items: ["d"] },
  ]);
});
