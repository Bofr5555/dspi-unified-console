// WP WB6h unit tests: the effects' model (the gates by the probe's feature
// names, the shown parameters, the masks and their packing, the values'
// text and parsing, the crossfeed writes under a named voicing), and on the
// real core: every effect's paths described, the probe's features on the
// demo, each effect switched and a parameter written and read back, the
// leveller's two masks in one path, and a crossfeed change under a named
// voicing as one undo step (P1XF).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CoreClient } from "../../src/core/client";
import type { DspState } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import {
  CROSSFEED_CUSTOM,
  EFFECTS,
  availableEffects,
  choiceOptions,
  crossfeedPairs,
  crossfeedWrites,
  effectDef,
  effectPaths,
  effectTarget,
  featureNames,
  isEffectId,
  isOn,
  maskBits,
  maskValue,
  paramValue,
  parseValue,
  shownParams,
  stepFor,
  toggled,
  valueText,
} from "../../src/screens/effects/model";

/** A mirror with every effect's fields, as the core's `DspState` carries them. */
function state(over: Partial<DspState> = {}): DspState {
  return {
    platformId: 1,
    numChannels: 9,
    numInputChannels: 2,
    numOutputChannels: 7,
    channelNames: ["In L", "In R"],
    crossfeedEnabled: false,
    crossfeedPreset: 0,
    crossfeedFreq: 700,
    crossfeedFeed: 4.5,
    crossfeedItd: true,
    crossfeedOutputPairMask: 1,
    loudnessEnabled: true,
    loudnessRefSpl: 83,
    loudnessIntensity: 100,
    loudnessOutputMask: 0xffff,
    levellerEnabled: false,
    levellerAmount: 50,
    levellerSpeed: 1,
    levellerMaxGainDb: 15,
    levellerLookahead: true,
    levellerGateDb: -96,
    levellerDetectorMask: 0x03,
    levellerApplyMask: 0x0f,
    psybassEnabled: false,
    psybassOutputMask: 0x3f,
    psybassCutoffHz: 80,
    psybassHarmonicsDb: 0,
    psybassDriveDb: 6,
    psybassCharacterPct: 50,
    psybassOriginalDb: 0,
    upmix: Array.from({ length: 14 }, () => 0),
    subharm: [1, 0x40, -60, -60, -60, 0, 0, 50, 100, 0, 0],
    subharmSolo: false,
    tube: [0, 0x3f, 1, 0, 10, 3, 40, 20, 1, 0, 8, 60, 100, 0],
    ...over,
  } as unknown as DspState;
}

test("WB6h: effects are gated by the probe's feature names, not by a version; the three every firmware has need none", () => {
  const s = state();
  assert.deepEqual(availableEffects(s, {}).map((e) => e.id), ["crossfeed", "loudness", "leveller"]);
  const all = Object.fromEntries(featureNames().map((n) => [n, true]));
  assert.deepEqual(availableEffects(s, all).map((e) => e.id), ["crossfeed", "loudness", "leveller", "psybass", "tube", "subharm", "upmix"]);
  assert.deepEqual(availableEffects(s, { tube_preamp: true }).map((e) => e.id), ["crossfeed", "loudness", "leveller", "tube"]);
  // A mirror without an effect's fields (an image without it) has no such effect, whatever the probe says.
  assert.deepEqual(availableEffects(state({ upmix: [], tube: undefined } as Partial<DspState>), all).map((e) => e.id), ["crossfeed", "loudness", "leveller", "psybass", "subharm"]);
  for (const n of ["psychoacoustic_bass", "upmixer", "subharmonic_synth", "tube_preamp", "loudness_output_mask", "crossfeed_output_mask", "leveller_masks"]) {
    assert.ok(featureNames().includes(n), n);
  }
  // Not a version in sight: formatVersion is not read.
  assert.deepEqual(availableEffects(state({ formatVersion: 1 }), all).length, 7);
});

test("WB6h: on or off from the mirror (a table's switch at 0.5); the masks' own features and conditions; the tube's Basic view", () => {
  const s = state();
  assert.equal(isOn(effectDef("loudness"), s), true);
  assert.equal(isOn(effectDef("subharm"), s), true, "subharm[0] = 1");
  assert.equal(isOn(effectDef("tube"), s), false);
  const all = Object.fromEntries(featureNames().map((n) => [n, true]));
  const paths = (id: Parameters<typeof effectDef>[0], f: Record<string, boolean>, adv = true, st = s) => shownParams(effectDef(id), st, f, adv).map((p) => `${p.path}${p.mask ? `:${p.mask}` : ""}`);
  assert.ok(!paths("loudness", {}).includes("loud.mask:outputs"), "no mask without loudness_output_mask");
  assert.ok(paths("loudness", all).includes("loud.mask:outputs"));
  assert.ok(!paths("crossfeed", {}).includes("cf.outputs:pairs"));
  // The leveller's channels: their feature and more than two inputs.
  assert.ok(!paths("leveller", all).some((p) => p.startsWith("lev.masks")), "two inputs");
  assert.deepEqual(paths("leveller", all, true, state({ numInputChannels: 8 })).filter((p) => p.startsWith("lev.masks")), ["lev.masks:detector", "lev.masks:apply"]);
  // M38: Basic has the type, drive, mix and the outputs; Advanced all fourteen.
  assert.deepEqual(paths("tube", all, false), ["tube.mask:outputs", "tube.type", "tube.drive", "tube.mix"]);
  assert.equal(paths("tube", all, true).length, 13);
  assert.ok(paths("subharm", all).includes("sub.solo"));
});

test("WB6h: masks: outputs, crossfeed's pairs by platform, the leveller's halves packed in one value", () => {
  const s = state({ numInputChannels: 8 });
  const name = (o: number) => `Name ${o}`;
  assert.deepEqual(maskBits("outputs", s, name).map((b) => b.short), ["OUT1", "OUT2", "OUT3", "OUT4", "OUT5", "OUT6", "OUT7"]);
  assert.equal(crossfeedPairs({ platformId: 0 }), 2);
  assert.equal(crossfeedPairs({ platformId: 1 }), 4);
  assert.deepEqual(maskBits("pairs", s, name)[1], { bit: 1, short: "OUT3+4", long: "Name 2 and Name 3" });
  assert.equal(maskBits("detector", s, name).length, 8);
  assert.equal(toggled(0b101, 1), 0b111);
  assert.equal(toggled(0x8000, 15), 0);
  // lev.masks: the detector in the low byte, apply in the high one; a change to one keeps the other.
  assert.equal(maskValue("detector", s, 0x07), 0x0f07);
  assert.equal(maskValue("apply", s, 0xf0), 0xf003);
  assert.equal(maskValue("outputs", s, 0x41), 0x41);
});

test("WB6h: values: steps from the unit, text with its unit (never -0), typed values with or without it", () => {
  assert.equal(stepFor({ unit: "Hz", min: 500, max: 2000 }), 10);
  assert.equal(stepFor({ unit: "Hz", min: 30, max: 300 }), 1);
  assert.equal(stepFor({ unit: "%", min: 0, max: 200 }), 1);
  assert.equal(stepFor({ unit: "dB", min: -24, max: 12 }), 0.1);
  assert.equal(stepFor({ unit: "", min: 0.5, max: 2 }), 0.01);
  assert.equal(valueText(700, { unit: "Hz" }, 10), "700 Hz");
  assert.equal(valueText(-0.04, { unit: "dB" }, 0.1), "0.0 dB");
  assert.equal(valueText(83, { unit: "dB SPL" }, 0.1), "83.0 dB SPL");
  assert.equal(valueText(12.5, { unit: "" }, 0.1), "12.5");
  assert.equal(parseValue("650", { unit: "Hz" }), 650);
  assert.equal(parseValue("650 hz", { unit: "Hz" }), 650);
  assert.equal(parseValue("85 dB SPL", { unit: "dB SPL" }), 85);
  assert.equal(parseValue("-6 dB", { unit: "dBFS" }), -6);
  assert.equal(parseValue("+2", { unit: "dB" }), 2);
  assert.equal(parseValue("loud", { unit: "dB" }), null);
  assert.deepEqual(paramValue({ kind: "bool" }, true), { bool: true });
  assert.deepEqual(paramValue({ kind: "bool" }, 0.7), { bool: true });
  assert.deepEqual(paramValue({ kind: "choice" }, 2), { choice: 2 });
  assert.deepEqual(paramValue({ kind: "int" }, 4.6), { int: 5 });
  assert.deepEqual(paramValue({ kind: "mask" }, 0x8001), { mask: 0x8001 });
  assert.deepEqual(paramValue({ kind: "float" }, 1.5), { float: 1.5 });
  const info = { path: "tube.type", label: "Tube", plain: "", writable: true, readable: true, indices: 0, routed: true, kind: "int" as const, min: 0, max: 2 };
  assert.deepEqual(choiceOptions(info, undefined).map((o) => o.label), ["0", "1", "2"]);
  assert.deepEqual(choiceOptions(info, [{ value: 0, label: "Custom", detail: "" }]).map((o) => o.label), ["Custom"]);
});

test("WB6h P1XF: a frequency or feed change under a named voicing makes it Custom from the values shown; under Custom the value alone", () => {
  const named = state({ crossfeedPreset: 2, crossfeedFreq: 650, crossfeedFeed: 9.5 });
  assert.deepEqual(crossfeedWrites(named, "cf.freq", 800), [
    { path: "cf.preset", value: { choice: CROSSFEED_CUSTOM } },
    { path: "cf.feed", value: { float: 9.5 } },
    { path: "cf.freq", value: { float: 800 } },
  ]);
  assert.deepEqual(crossfeedWrites(named, "cf.feed", 3).map((w) => w.path), ["cf.preset", "cf.freq", "cf.feed"]);
  assert.deepEqual(crossfeedWrites(state({ crossfeedPreset: CROSSFEED_CUSTOM }), "cf.freq", 800), [{ path: "cf.freq", value: { float: 800 } }]);
});

test("WB6h: each effect is addressable by id", () => {
  for (const e of EFFECTS) assert.ok(isEffectId(e.id));
  assert.equal(isEffectId("nope"), false);
  assert.deepEqual(effectTarget("tube"), { screen: "effects", effect: "tube" });
  // The System view's block ids (WB6d) are these.
  assert.deepEqual(EFFECTS.map((e) => e.id).sort(), ["crossfeed", "leveller", "loudness", "psybass", "subharm", "tube", "upmix"]);
});

// -- On the real core --

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

test("WB6h core: every path described and writable; the demo has every effect's feature; the choices cover the registry's", { skip }, async () => {
  const session = await demoSession();
  for (const path of effectPaths()) {
    const d = await session.core.call("describe", { path });
    assert.equal(d.writable && d.routed, true, path);
  }
  const f = await session.core.call("features", { names: featureNames() });
  for (const n of featureNames()) assert.equal(f[n], true, n);
  const s = session.state.snapshot!.state;
  assert.equal(availableEffects(s, f).length, 7);
  const choices = await session.core.call("effectChoices", undefined);
  assert.equal(choices["cf.preset"]![2]!.label, "Meier");
  assert.equal(choices["tube.type"]![0]!.label, "Custom");
});

test("WB6h core: each effect switched on and back, and a parameter written, through write, read back from the mirror", { skip }, async () => {
  const session = await demoSession();
  const f = await session.core.call("features", { names: featureNames() });
  for (const e of availableEffects(session.state.snapshot!.state, f)) {
    const info = await session.core.call("describe", { path: e.on.path });
    const was = isOn(e, session.state.snapshot!.state);
    assert.equal(await session.write(e.on.path, paramValue(info, !was)), true, e.id);
    assert.equal(isOn(e, session.state.snapshot!.state), !was, `${e.id} switched`);
    assert.equal(await session.write(e.on.path, paramValue(info, was)), true);
    assert.equal(isOn(e, session.state.snapshot!.state), was, `${e.id} back`);
  }
  const s0 = session.state.snapshot!.state;
  const set = async (path: string, value: number, read: (s: DspState) => unknown) => {
    const info = await session.core.call("describe", { path });
    assert.equal(await session.write(path, paramValue(info, value)), true, path);
    assert.equal(read(session.state.snapshot!.state), value, path);
  };
  await set("loud.ref", 77, (s) => s.loudnessRefSpl);
  await set("lev.amount", 33, (s) => s.levellerAmount);
  await set("bass.cutoff", 120, (s) => s.psybassCutoffHz);
  await set("up.presence", 2.5, (s) => (s.upmix as number[])[13]);
  await set("sub.boost", 3, (s) => (s.subharm as number[])[5]);
  await set("tube.drive", 6, (s) => (s.tube as number[])[3]);
  // A tube type loads its row (the core's): the bias follows.
  const bias = (s: DspState) => (s.tube as number[])[4];
  await set("tube.type", 2, (s) => (s.tube as number[])[2]);
  const b2 = bias(session.state.snapshot!.state);
  await set("tube.type", 1, (s) => (s.tube as number[])[2]);
  assert.notEqual(bias(session.state.snapshot!.state), b2, "the type's row was loaded");
  // The leveller's two masks in one write, each half kept.
  const st = state({ ...s0, levellerDetectorMask: s0.levellerDetectorMask, levellerApplyMask: s0.levellerApplyMask } as Partial<DspState>);
  assert.equal(await session.write("lev.masks", { mask: maskValue("detector", st, 0x01) }), true);
  const after1 = session.state.snapshot!.state;
  assert.equal(after1.levellerDetectorMask, 0x01);
  assert.equal(after1.levellerApplyMask, s0.levellerApplyMask);
  assert.equal(await session.write("lev.masks", { mask: maskValue("apply", after1, 0x02) }), true);
  assert.equal(session.state.snapshot!.state.levellerDetectorMask, 0x01);
  assert.equal(session.state.snapshot!.state.levellerApplyMask, 0x02);
  // An output mask.
  assert.equal(await session.write("bass.mask", { mask: toggled(Number(s0.psybassOutputMask), 0) }), true);
  assert.equal(session.state.snapshot!.state.psybassOutputMask, toggled(Number(s0.psybassOutputMask), 0));
});

test("WB6h core P1XF: under a named voicing the mirror shows its values; a change makes it Custom from them, one undo step", { skip }, async () => {
  const session = await demoSession();
  assert.equal(await session.write("cf.preset", { choice: 1 }), true);
  let s = session.state.snapshot!.state;
  assert.deepEqual([s.crossfeedPreset, s.crossfeedFreq, s.crossfeedFeed], [1, 700, 6], "Chu Moy's own values");
  const writes = crossfeedWrites(s, "cf.freq", 900);
  assert.equal(await session.group("Crossfeed, custom", () => session.writeAll(writes)), true);
  s = session.state.snapshot!.state;
  assert.deepEqual([s.crossfeedPreset, s.crossfeedFreq, s.crossfeedFeed], [CROSSFEED_CUSTOM, 900, 6], "Custom from Chu Moy's feed");
  // One step back: Chu Moy again, with its values.
  await session.undo();
  s = session.state.snapshot!.state;
  assert.deepEqual([s.crossfeedPreset, s.crossfeedFreq, s.crossfeedFeed], [1, 700, 6]);
});

test("WB6h core: a drag's writes are one undo step (setDragging)", { skip }, async () => {
  const session = await demoSession();
  const before = session.state.snapshot!.state.loudnessIntensity;
  session.setDragging(true);
  for (const v of [110, 120, 130]) assert.equal(await session.write("loud.intensity", { float: v }), true);
  session.setDragging(false);
  assert.equal(session.state.snapshot!.state.loudnessIntensity, 130);
  await session.undo();
  assert.equal(session.state.snapshot!.state.loudnessIntensity, before, "one step back is before the drag");
});
