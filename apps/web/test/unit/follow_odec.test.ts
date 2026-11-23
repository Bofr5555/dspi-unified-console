// WP ODEC unit tests: the screens follow an AI agent (src/ui/follow.ts): the screen a change is on,
// the person's gate, the rate limit, and the test-signal grant in the agent's strip.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { DspState } from "../../src/core/protocol";
import { FOLLOW_MIN_MS, FollowGate, agentOn, changedScreen, interacting, splitGrant } from "../../src/ui/follow";

const band = (gain = 0) => ({ filterType: 1, bypass: false, freq: 1000, q: 0.7, gain, qp: 0.7 });
const base = (): DspState =>
  ({
    numInputChannels: 2,
    numOutputChannels: 2,
    filters: [[band()], [band()]],
    xover: [[band()], [band()]],
    matrixRouting: [[true, false]],
    matrixGain: [[0, 0]],
    matrixInvert: [[false, false]],
    outputDelayMs: [0, 0],
    outputGainDb: [0, 0],
    outputMuted: [false, false],
    outputEnabled: [true, true],
    inputPreampDb: [0, 0],
    masterVolumeDb: -10,
    userVolumeDb: -20,
    userMute: false,
    loudnessEnabled: false,
  }) as unknown as DspState;

test("wp_odec a change from elsewhere maps to the screen it is on", () => {
  const a = base();
  const with_ = (f: (s: DspState) => void) => {
    const b = structuredClone(a);
    f(b);
    return changedScreen(a, b);
  };
  assert.equal(with_((s) => (s.filters[1]![0] = band(3))), "eq");
  assert.equal(with_((s) => (s.xover[0]![0] = band(1))), "xover");
  assert.equal(with_((s) => (s.matrixRouting[0]![1] = true)), "matrix");
  assert.equal(with_((s) => (s.outputGainDb[1] = -3)), "delays");
  assert.equal(with_((s) => (s.outputMuted[0] = true)), "delays");
  assert.equal(with_((s) => (s.inputPreampDb[0] = 2)), "inputs");
  assert.equal(with_((s) => (s.masterVolumeDb = -5)), "main");
  assert.equal(with_((s) => (s.loudnessEnabled = true)), "effects");
  // The filters first when a change is both; nothing changed, nothing to follow
  assert.equal(with_((s) => ((s.filters[0]![0] = band(2)), (s.outputGainDb[0] = 1))), "eq");
  assert.equal(changedScreen(a, structuredClone(a)), null);
  assert.equal(changedScreen(null, a), null);
});

test("wp_odec only a device a presence row names an agent on", () => {
  const row = { kind: "agent", label: "Claude", by: "Mac", serial: "S-A", what: "", ttlS: 20 };
  assert.equal(agentOn([row], "S-A"), true);
  assert.equal(agentOn([row], "S-B"), false);
  assert.equal(agentOn([{ ...row, kind: "person" }], "S-A"), false);
  assert.equal(agentOn([row], ""), false);
});

test("wp_odec never while the person interacts: a pointer down, typing, a dialog", () => {
  const doc = (tag: string | null, dialog = false, editable = false) => ({
    activeElement: tag ? ({ tagName: tag, isContentEditable: editable } as unknown as Element) : null,
    querySelector: (() => (dialog ? ({} as Element) : null)) as unknown as Document["querySelector"],
  });
  assert.equal(interacting(doc("BODY"), false), false);
  assert.equal(interacting(doc("BUTTON"), false), false);
  assert.equal(interacting(doc("BODY"), true), true);
  for (const t of ["INPUT", "TEXTAREA", "SELECT"]) assert.equal(interacting(doc(t), false), true, t);
  assert.equal(interacting(doc("DIV", false, true), false), true);
  assert.equal(interacting(doc("BODY", true), false), true);
  assert.equal(interacting(null, false), false);
});

test("wp_odec at most one switch per three seconds, never twice to the same place, not to where the person is", () => {
  const g = new FollowGate();
  const eq = { serial: "S-A", screen: "eq" };
  const mx = { serial: "S-A", screen: "matrix" };
  assert.equal(g.take(eq, 0, true, null), false, "busy");
  assert.equal(g.take(eq, 0, false, { serial: "S-A", screen: "eq" }), false, "already there");
  assert.equal(g.take(eq, 0, false, null), true);
  assert.equal(g.take(mx, FOLLOW_MIN_MS - 1, false, null), false, "too soon");
  assert.equal(g.take(eq, FOLLOW_MIN_MS * 2, false, null), false, "the same place again");
  assert.equal(g.take(mx, FOLLOW_MIN_MS, false, null), true);
  assert.equal(g.take({ serial: "S-B", screen: "matrix" }, FOLLOW_MIN_MS * 2, false, null), true, "another device");
});

test("wp_odec the strip shows the test-signal grant apart from the work", () => {
  assert.deepEqual(splitGrant("Measuring: Out 1 · test signals allowed, 9 min left"), { work: "Measuring: Out 1", grant: "test signals allowed, 9 min left" });
  assert.deepEqual(splitGrant("Changing the gain: OUT1"), { work: "Changing the gain: OUT1", grant: "" });
  assert.deepEqual(splitGrant(" · Test signals allowed, 1 min left"), { work: "", grant: "Test signals allowed, 1 min left" });
});
