// WP WB6d unit tests: the System view's signal chain (src/screens/system/chain.ts): the
// blocks' on or off state from the device's state, the routing summary, where each block
// opens, the meters; and on the real core's demo device, the chain following writes.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { DspState, FilterParams, OutputLabel, SystemStatus } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { activePeq, activeXover, blockText, chainView, meterOf, meterPct, outputText } from "../../src/screens/system/chain";

const OFF: FilterParams = { filterType: 0, bypass: false, freq: 1000, q: 0.707, gain: 0, qp: 0.707 };
const BELL: FilterParams = { filterType: 1, bypass: false, freq: 1000, q: 1, gain: 3, qp: 0.707 };
const LR4LP: FilterParams = { filterType: 34, bypass: false, freq: 80, q: 0.707, gain: 0, qp: 0.707 };

/** Two inputs, three outputs: OUT1 from IN1 with EQ, crossover and limiter; OUT3 disabled. */
function state(over: Partial<DspState> = {}): DspState {
  const nIn = 2;
  const nOut = 3;
  const bands = (n: number, p: FilterParams[] = []) => Array.from({ length: n }, (_, i) => p[i] ?? OFF);
  return {
    platformId: 1,
    numChannels: nIn + nOut,
    numInputChannels: nIn,
    numOutputChannels: nOut,
    formatVersion: 32,
    fwMajor: 1,
    fwMinor: 1,
    fwPatch: 6,
    fwBeta: 0,
    preampDb: 0,
    bypass: false,
    masterVolumeDb: 0,
    userVolumeDb: -12.5,
    userMute: false,
    loudnessEnabled: true,
    loudnessOutputMask: 0b010,
    outputGainDb: [-3, 0, 1.5],
    outputMuted: [false, true, false],
    channelNames: ["Left", "Right", "Out A", "Out B", "Out C"],
    presetNames: [],
    activePresetSlot: 0,
    masterVolumeMode: 0,
    presetOccupied: 0,
    presetStartupMode: 0,
    presetDefaultSlot: 0,
    inputPreampDb: [-2, 0],
    outputEnabled: [true, true, false],
    inputSource: 0,
    i2sInputChannels: 2,
    filters: [bands(10), bands(10, [BELL, { ...BELL, bypass: true }]), bands(10, [BELL, BELL, LR4LP]), bands(10), bands(10)],
    xover: [[], [], bands(4, [LR4LP, { ...LR4LP, bypass: true }]), bands(4), bands(4)],
    maxBands: 10,
    outputDelayMs: [1.5, 0, 0],
    matrixRouting: [
      [true, true, false],
      [false, true, false],
    ],
    matrixGain: [
      [0, 0, 0],
      [0, 0, 0],
    ],
    matrixInvert: [
      [false, false, false],
      [false, false, false],
    ],
    limiterEnabled: [true, false, false],
    limiterLinkGroup: [0, 0, 0],
    limiterThresholdDb: [-3, 0, 0],
    limiterReleaseMs: [100, 100, 100],
    crossfeedEnabled: false,
    levellerEnabled: true,
    psybassEnabled: false,
    psybassOutputMask: 0,
    tube: [1, 0b001],
    subharm: [0, 0b111],
    upmix: [0],
    ...over,
  };
}

const LABELS: OutputLabel[] = [
  { output: 0, label: "Out 1 · L · Low", shortLabel: "L · Low", slotName: "I2S 1 L", userNamed: false, channel: "L", band: "Low", unused: false },
  { output: 1, label: "Out B", shortLabel: "Out B", slotName: "I2S 1 R", userNamed: true, channel: "L+R", band: "Full", unused: false },
  { output: 2, label: "Out C", shortLabel: "Out C", slotName: "PDM", userNamed: true, channel: "None", band: "Full", unused: true },
];

const block = (bs: { id: string }[], id: string) => bs.find((b) => b.id === id) as ReturnType<typeof chainView>["outputs"][number]["blocks"][number];

test("WB6d: active bands count only acting PEQ bands within the device's count, crossovers their own", () => {
  const s = state();
  assert.equal(activePeq(s, 0), 0);
  assert.equal(activePeq(s, 1), 1, "a bypassed band does not act");
  assert.equal(activePeq(s, 2), 2, "a crossover type in the PEQ list is not EQ");
  assert.equal(activePeq({ ...s, maxBands: 1 }, 2), 1, "bands beyond the device's count are not its");
  assert.equal(activeXover(s, 2), 1, "a bypassed crossover band does not act");
  assert.equal(activeXover(s, 0), 0);
});

test("WB6d: inputs: name, trim, EQ block opening the EQ screen at that input", () => {
  const c = chainView(state(), LABELS, 2);
  assert.deepEqual(
    c.inputs.map((i) => [i.descriptor, i.name, i.trimDb]),
    [
      ["IN1", "Left", -2],
      ["IN2", "Right", 0],
    ],
  );
  const eq = block(c.inputs[1]!.blocks, "eq");
  assert.equal(eq.on, true);
  assert.equal(eq.detail, "1 band");
  assert.deepEqual(eq.open, { screen: "eq", wire: 1, tab: "peq" });
  assert.equal(block(c.inputs[0]!.blocks, "eq").on, false);
  assert.equal(chainView(state(), LABELS, 1).inputs.length, 1, "only the live inputs");
});

test("WB6d: outputs: routing summary, blocks on or off, level, and where each opens", () => {
  const c = chainView(state(), LABELS, 2);
  const [o1, o2, o3] = c.outputs;
  assert.equal(o1!.from, "L");
  assert.deepEqual(o1!.routedFrom, ["IN1"]);
  assert.deepEqual(o2!.routedFrom, ["IN1", "IN2"]);
  assert.equal(o1!.name, "Out 1 · L · Low");
  assert.equal(block(o1!.blocks, "eq").detail, "2 bands");
  assert.deepEqual(block(o1!.blocks, "xo"), { id: "xo", label: "Crossover", on: true, detail: "Low", open: { screen: "eq", wire: 2, tab: "xo" } });
  assert.equal(block(o1!.blocks, "limiter").on, true);
  assert.equal(block(o1!.blocks, "limiter").detail, "-3.0 dBFS");
  assert.equal(block(o2!.blocks, "limiter").on, false);
  assert.equal(block(o2!.blocks, "eq").on, false);
  assert.deepEqual([o1!.gainDb, o1!.muted, o1!.delayMs], [-3, false, 1.5]);
  assert.equal(o2!.muted, true);
  assert.deepEqual(outputText(o1!), { gain: "-3.0 dB", delay: "1.5 ms" });
  assert.deepEqual(o1!.open, { screen: "eq", wire: 2, tab: undefined });
  // A disabled output is not on the EQ screen: it and its blocks open the Matrix.
  assert.equal(o3!.enabled, false);
  assert.deepEqual(o3!.open, { screen: "matrix" });
  assert.deepEqual(block(o3!.blocks, "eq").open, { screen: "matrix" });
  // -- WP WEB7 -- the core's `unused` (nothing routed): the chain view leaves it out
  assert.deepEqual(c.outputs.map((o) => o.unused), [false, false, true]);
  assert.deepEqual(chainView(state(), undefined, 2).outputs.map((o) => o.unused), [false, false, false], "no labels, nothing known unused");
});

test("WB6d: effects: per-output ones only while on, by their output mask; device-wide ones listed with their state", () => {
  const c = chainView(state(), LABELS, 2);
  const [o1, o2] = c.outputs;
  // Loudness on, mask OUT2; tube on, mask OUT1; subharmonic and psychoacoustic bass off: not per output.
  assert.equal(block(o1!.blocks, "loudness").on, false);
  assert.equal(block(o2!.blocks, "loudness").on, true);
  assert.equal(block(o1!.blocks, "tube").on, true);
  assert.equal(block(o2!.blocks, "tube").on, false);
  assert.equal(block(o1!.blocks, "subharm"), undefined);
  assert.equal(block(o1!.blocks, "psybass"), undefined);
  assert.deepEqual(
    c.deviceEffects.map((b) => [b.id, b.on, b.detail]),
    [
      ["crossfeed", false, ""],
      ["leveller", true, ""],
      ["upmix", false, ""],
      ["loudness", true, "OUT2"],
      ["psybass", false, ""],
      ["tube", true, "OUT1"],
      ["subharm", false, ""],
    ],
  );
  // -- fixup b4fix N3 -- each opens the Effects screen at itself
  assert.ok(c.deviceEffects.every((b) => b.open?.screen === "effects" && b.open.effect === b.id), "the effects open their screen");
  // A firmware without an effect has no field for it: not listed.
  const bare = chainView(state({ tube: [], subharm: undefined as unknown as number[], crossfeedEnabled: undefined }), LABELS, 2);
  assert.deepEqual(
    bare.deviceEffects.map((b) => b.id),
    ["leveller", "upmix", "loudness", "psybass"],
  );
});

test("WB6d: no limiter block before wire format 32; bypass and volume carried", () => {
  const c = chainView(state({ formatVersion: 31, bypass: true, userMute: true }), LABELS, 2);
  assert.equal(block(c.outputs[0]!.blocks, "limiter"), undefined);
  assert.equal(c.bypass, true);
  assert.equal(c.userMute, true);
  assert.equal(c.userVolumeDb, -12.5);
});

test("WB6d: without the core's labels an output is named by its stored name and its routed inputs", () => {
  const c = chainView(state(), undefined, 2);
  assert.equal(c.outputs[0]!.name, "Out A");
  assert.equal(c.outputs[1]!.from, "IN1+IN2");
  assert.equal(c.outputs[2]!.from, "None");
  assert.equal(block(c.outputs[0]!.blocks, "xo").detail, "1 band");
});

test("WB6d: meters: a wire channel's level and clip marker; block text", () => {
  const m = { peaks: [1, 0.5, 0], clipHold: 0b100, stale: false } as unknown as SystemStatus;
  assert.deepEqual(meterOf(m, 0), { db: 0, clip: false });
  assert.equal(Math.round(meterOf(m, 1).db), -6);
  assert.deepEqual(meterOf(m, 2), { db: -90, clip: true });
  assert.deepEqual(meterOf(m, 9), { db: -90, clip: false });
  assert.deepEqual(meterOf(null, 0), { db: -90, clip: false });
  assert.equal(meterPct(0), 100);
  assert.equal(meterPct(-90), 0);
  assert.equal(meterPct(-200), 0);
  assert.equal(meterPct(-45), 50);
  assert.equal(blockText({ id: "eq", label: "EQ", on: true, detail: "2 bands", open: null }), "EQ: on, 2 bands");
  assert.equal(blockText({ id: "eq", label: "EQ", on: false, detail: "", open: null }), "EQ: off");
});

// -- On the real core's demo device --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demoSession() {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const client = {
    call: (m: never, p: never) => core.call(m, p),
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo" });
  return session;
}

test("WB6d: on the demo device the chain has every output and follows writes (EQ, limiter, loudness, mute)", { skip }, async () => {
  const session = await demoSession();
  const chainOf = () => {
    const snap = session.state.snapshot!;
    return chainView(snap.state, snap.outputLabels, snap.state.numInputChannels);
  };
  let c = chainOf();
  const s = session.state.snapshot!.state;
  assert.equal(c.outputs.length, s.numOutputChannels);
  assert.equal(c.inputs.length, s.numInputChannels);
  assert.ok(c.deviceEffects.some((b) => b.id === "loudness"), "the demo has loudness");
  const o1 = s.numInputChannels;
  // A bell on OUT1's first free band.
  const free = s.filters[o1]!.slice(0, s.maxBands || 10).findIndex((p) => p.filterType === 0);
  assert.ok(free >= 0);
  const before = activePeq(s, o1);
  assert.ok(await session.change("filter", { channel: o1, band: free, ...BELL }));
  assert.ok(await session.write("limit.on", { bool: !s.limiterEnabled[0] }, [0]));
  assert.ok(await session.write("out.mute", { bool: !s.outputMuted[0] }, [0]));
  assert.ok(await session.write("loud.on", { bool: true }));
  assert.ok(await session.write("loud.mask", { mask: 1 }));
  c = chainOf();
  const s2 = session.state.snapshot!.state;
  assert.equal(activePeq(s2, o1), before + 1);
  assert.equal(block(c.outputs[0]!.blocks, "eq").on, true);
  assert.equal(block(c.outputs[0]!.blocks, "limiter").on, !s.limiterEnabled[0]);
  assert.equal(c.outputs[0]!.muted, !s.outputMuted[0]);
  assert.equal(block(c.outputs[0]!.blocks, "loudness").on, true);
  assert.equal(block(c.outputs[1]!.blocks, "loudness").on, false);
  assert.equal(c.outputs[0]!.from, session.state.snapshot!.outputLabels![0]!.channel);
});
