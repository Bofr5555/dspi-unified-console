// WP WEB6 unit tests: the Effects pages' layout as Qt's (layout.ts), the masks' "Presets ▾" with
// Qt's words and masks, the "Off" ends, the status and harmonics words, the tube picture's geometry.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { DspState } from "../../src/core/protocol";
import { EFFECTS } from "../../src/screens/effects/model";
import { allOutputs, columnsOf, harmonicText, headroomText, maskPresets, offAt, placedPaths, subOnly, upmixStatusText } from "../../src/screens/effects/layout";
import { envelopePath, geometryOf, pushPull, shortTubeName, TUBE_GROUPS } from "../../src/screens/effects/tube";
import { CROSSFEED_PRESET_TEXTS, tubeCaption } from "../../src/screens/effects/texts";

const state = (o: Partial<DspState>): DspState => ({ numOutputChannels: 9, numInputChannels: 8, platformId: 1, ...o }) as DspState;

test("WEB6: every parameter of every effect has its place in Qt's columns", () => {
  for (const e of EFFECTS) {
    const views = e.split ? [false, true] : [false];
    const placed = new Set<string>();
    for (const adv of views) for (const p of placedPaths(columnsOf(e.id, adv))) placed.add(p);
    for (const p of e.params) if (p.path !== "sub.solo") assert.ok(placed.has(p.path), `${e.id}: ${p.path}`);
  }
});

test("WEB6: the columns follow Qt: graph and presets left, parameters and outputs right; Subharmonic's order", () => {
  const cf = columnsOf("crossfeed", false);
  assert.deepEqual(cf.left.map((b) => b.kind), ["graph", "section"]);
  assert.deepEqual(
    columnsOf("subharm", false).right.map((b) => (b.kind === "section" ? b.title : b.kind)),
    ["Selectivity", "Sub ceiling", "LF boost", "Outputs"],
  );
  const up = columnsOf("upmix", false);
  assert.equal(up.left[0]!.kind, "status");
  const centre = up.right[0]!;
  assert.ok(centre.kind === "section" && centre.paths.indexOf("up.presence") === 2, "Presence after Centre Width, as Qt");
  assert.equal(columnsOf("tube", false).left[0]!.kind, "picture");
  assert.equal(columnsOf("tube", true).left[0]!.kind, "graph");
});

test("WEB6: the masks' Presets are Qt's words and masks; the sub is the last output", () => {
  const s = state({});
  assert.equal(allOutputs(s), 0x1ff);
  assert.equal(subOnly(s), 0x100);
  assert.deepEqual(maskPresets("bass.mask", s), [
    { label: "All outputs", value: 0x1ff },
    { label: "Exclude sub (recommended)", value: 0xff },
    { label: "None", value: 0 },
  ]);
  assert.deepEqual(maskPresets("sub.mask", s)[0], { label: "Sub only (recommended)", value: 0x100 });
  assert.deepEqual(maskPresets("loud.mask", s)[1], { label: "Slot 1 only (Headphones)", value: 3 });
  assert.deepEqual(maskPresets("lev.masks", s)[1], { label: "Center only (Dialog boost)", value: 0x0404 });
  assert.deepEqual(maskPresets("cf.outputs", s)[0], { label: "All pairs", value: 0x0f });
  assert.deepEqual(maskPresets("cf.outputs", state({ platformId: 0 }))[0], { label: "All pairs", value: 0x03 });
  assert.deepEqual(maskPresets("tube.mask", s)[1], { label: "Exclude sub", value: 0xff });
  assert.deepEqual(maskPresets("cf.freq", s), []);
});

test("WEB6: Qt's words: Off at a stage's floor or top, the headroom, the harmonics, the upmixer's status", () => {
  assert.equal(offAt("sub.low"), "floor");
  assert.equal(offAt("sub.ceiling"), "top");
  assert.equal(offAt("sub.boost"), null);
  assert.equal(headroomText(0), "none");
  assert.equal(headroomText(null), "none");
  assert.equal(headroomText(3.46), "+3.5 dB");
  assert.equal(harmonicText(-120), "none");
  assert.equal(harmonicText(-39.04), "-39 dB");
  assert.equal(upmixStatusText(false, null), "Idle: upmixer disabled");
  assert.equal(upmixStatusText(true, { active: true, parkedReason: 0 }), "Active - processing audio");
  assert.equal(upmixStatusText(true, { active: false, parkedReason: 2 }), "Idle: input is not stereo");
  assert.equal(CROSSFEED_PRESET_TEXTS[2]!.name, "Jan Meier");
});

test("WEB6: the tube's shelf and picture: Qt's groups, short names, families, a closed envelope", () => {
  assert.deepEqual(TUBE_GROUPS.map((g) => [g.from, g.to]), [[1, 8], [9, 10], [11, 16]]);
  assert.equal(shortTubeName("12AX7 / ECC83"), "12AX7");
  assert.equal(shortTubeName("EL34"), "EL34");
  assert.ok(pushPull(12) && !pushPull(16) && !pushPull(1));
  assert.equal(tubeCaption("Push-pull power, tight", true, false, false), "Push-pull power, tight. Meant for use with the output stage, in Advanced.");
  assert.equal(tubeCaption(null, false, false, true), "Character controls as set, no tube row applied.");
  for (let f = 0; f < 10; f++) {
    const d = envelopePath(geometryOf(f));
    assert.match(d, /^M-?\d/);
    assert.match(d, /Z$/);
    assert.ok(!d.includes("NaN"), `family ${f}`);
  }
  assert.equal(geometryOf(99), geometryOf(0));
});
