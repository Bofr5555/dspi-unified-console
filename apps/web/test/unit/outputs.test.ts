// WP WB6c unit tests: the outputs slice's logic (crossover codes, the delay
// rule SL1's steps, value text, limiter groups, the Core 1 conflict), the EQ
// follow-ups (the Linkwitz Transform's types, retype and writes), and on the
// real core the crossover type, a crosspoint, the delay limit, a fractional
// delay kept exactly and a Linkwitz band written by eq.fp / eq.qp.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { FilterParams } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { T, availableTypes, bandLabel, bandWrites, dcBoost, retyped, setTypeNames, typeName } from "../../src/screens/eq/model";
import {
  compactGain,
  decodeXover,
  delayText,
  enableConflict,
  encodeXover,
  levelText,
  linkSummary,
  noNegZero,
  onGrid,
  outputSlotName,
  parseLevel,
  slopeText,
  stepGrid,
  stereoPairGroups,
  withFamily,
} from "../../src/screens/outputs/model";

test("WB6c: crossover codes decode to family, order and pass and back, for all 32 types", () => {
  for (let t = 32; t <= 63; t++) {
    const x = decodeXover(t);
    assert.ok(x.family >= 1 && x.family <= 3, `${t}`);
    assert.equal(encodeXover(x), t, `${t}`);
  }
  assert.deepEqual(decodeXover(35), { family: 1, order: 4, highPass: true });
  assert.deepEqual(decodeXover(44), { family: 2, order: 3, highPass: false });
  assert.deepEqual(decodeXover(63), { family: 3, order: 8, highPass: true });
  assert.equal(decodeXover(0).family, 0);
  assert.equal(decodeXover(5).family, 0, "a PEQ code is no crossover");
  assert.equal(slopeText(4), "24 dB/oct");
});

test("WB6c: a family change keeps the order or takes the nearest, keeps the pass; from off LR 12 dB/oct low pass", () => {
  assert.equal(withFamily(0, 1), 32, "off to LR: LR2 LP");
  assert.equal(withFamily(45, 1), 33, "BW3 HP to LR: the nearest even order (2), high pass kept");
  assert.equal(withFamily(35, 2), 47, "LR4 HP to BW: BW4 HP");
  assert.equal(withFamily(40, 3), 56, "BW1 LP to Bessel: BES2 LP");
  assert.equal(withFamily(39, 0), 0);
});

test("WB6c SL1: a drag lands on whole ms (no -0); the wheel and keys step to the next whole ms from a fraction", () => {
  assert.equal(onGrid(12.6, 1), 13);
  assert.equal(onGrid(12.3456, 1), 12);
  assert.ok(Object.is(onGrid(-0.2, 1), 0), "never -0");
  assert.ok(Object.is(noNegZero(-0), 0));
  assert.equal(stepGrid(12.3, 1, 1, [0, 42]), 13);
  assert.equal(stepGrid(12.3, -1, 1, [0, 42]), 12);
  assert.equal(stepGrid(12, 1, 1, [0, 42]), 13);
  assert.equal(stepGrid(12, -1, 1, [0, 42]), 11);
  assert.equal(stepGrid(0.4, -1, 1, [0, 42]), 0);
  assert.ok(Object.is(stepGrid(0.5, -1, 1, [0, 42]), 0));
  assert.equal(stepGrid(41.5, 1, 1, [0, 42]), 42);
  assert.equal(stepGrid(42, 1, 1, [0, 42]), 42, "clamped");
  assert.equal(stepGrid(3.3, 1, 1, [0, 42], 5), 8, "Page Up: five");
  // The gain's 0.1 dB grid, without float noise.
  assert.equal(stepGrid(-3.04, 1, 0.1, [-60, 20]), -3);
  assert.equal(stepGrid(-3, -1, 0.1, [-60, 20]), -3.1);
  assert.equal(onGrid(0.30000000000000004, 0.1), 0.3);
});

test("WB6c SL1: delays from the device are shown as they are; levels and typed values", () => {
  assert.equal(delayText(12.3456), "12.3456 ms");
  assert.equal(delayText(12), "12.0 ms");
  assert.equal(delayText(0.5), "0.5 ms");
  assert.equal(delayText(-0), "0.0 ms");
  assert.equal(levelText(-3.04), "-3.0 dB");
  assert.equal(levelText(-0.01), "0.0 dB");
  assert.equal(compactGain(0), "0 dB");
  assert.equal(compactGain(3.5), "+3.5 dB");
  assert.equal(compactGain(-12.25), "-12.25 dB");
  assert.equal(parseLevel("12.3456 ms", "ms"), 12.3456);
  assert.equal(parseLevel("+2 dB", "db"), 2);
  assert.equal(parseLevel("−6", "db"), -6);
  assert.equal(parseLevel("abc", "ms"), null);
});

test("P0U: a limiter threshold is a full-scale level, dBFS, and takes a typed dB too", () => {
  assert.equal(levelText(-6, "dBFS"), "-6.0 dBFS");
  assert.equal(parseLevel("-6 dBFS", "dbfs"), -6);
  assert.equal(parseLevel("-6 dB", "dbfs"), -6);
  assert.equal(parseLevel("-6", "dbfs"), -6);
});

test("WB6c: limiter link groups and summaries, in the Console's words", () => {
  assert.deepEqual(stereoPairGroups({ numOutputChannels: 9 }), [1, 1, 2, 2, 3, 3, 4, 4, 0]);
  assert.deepEqual(stereoPairGroups({ numOutputChannels: 5 }), [1, 1, 2, 2, 0]);
  assert.equal(linkSummary(0, []), "Not linked.");
  assert.equal(linkSummary(2, []), "No other outputs in group 2.");
  assert.equal(linkSummary(1, ["A"]), "Linked with A.");
  assert.equal(linkSummary(1, ["A", "B", "C"]), "Linked with A, B and C.");
});

test("WB6c: enabling an output that shares Core 1 with PDM is a conflict, either way", () => {
  const s = (enabled: boolean[], platformId = 1) => ({ platformId, numOutputChannels: enabled.length, outputEnabled: enabled });
  const all8 = [true, true, true, false, false, false, false, false, false];
  assert.equal(enableConflict(s(all8), 8), "pdm");
  assert.equal(enableConflict(s([true, true, false, false, false, false, false, false, true]), 2), "worker");
  assert.equal(enableConflict(s([true, false, false, false, false, false, false, false, true]), 1), null, "outputs 1-2 never conflict");
  assert.equal(enableConflict(s([true, true, false, false, false], 0), 4), null);
  assert.equal(enableConflict(s([true, true, true, false, false], 0), 4), "pdm", "RP2040: outputs 3-4");
  assert.equal(outputSlotName({ numOutputChannels: 9, outputTypes: [0, 1, 0, 0, 0] }, 3), "I2S 2 R");
  assert.equal(outputSlotName({ numOutputChannels: 9 }, 8), "PDM");
});

test("WB6c M49: the Linkwitz Transform is offered on outputs from wire V22; entering it seeds a neutral one; its writes are eq.fp and eq.qp", () => {
  const fw = { formatVersion: 32, fwMajor: 1, fwMinor: 1, fwPatch: 6 };
  assert.ok(availableTypes(fw, true).includes(T.linkwitz));
  assert.ok(!availableTypes(fw, false).includes(T.linkwitz), "not on inputs");
  assert.ok(!availableTypes({ ...fw, formatVersion: 21 }, true).includes(T.linkwitz));
  const bell: FilterParams = { filterType: T.peaking, bypass: false, freq: 40, q: 0.9, gain: 6, qp: 0.707 };
  const lt = retyped(bell, T.linkwitz);
  assert.deepEqual([lt.freq, lt.q, lt.gain, lt.qp], [40, 0.9, 40, 0.9], "fp = f0, Qp = Q0 (the core's registry::retyped)");
  assert.equal(retyped({ ...bell, freq: 5 }, T.linkwitz).freq, 10, "f0 at least 10 Hz");
  assert.equal(retyped(lt, T.peaking).gain, 0, "leaving it: no 40 dB gain");
  const w = bandWrites(4, 2, bell, { ...lt, gain: 25, qp: 0.5 });
  assert.deepEqual(
    w.map((x) => x.path),
    ["eq.type", "eq.fp", "eq.qp"],
  );
  assert.ok(w.every((x) => x.path !== "eq.gain"));
  assert.equal(Math.round(dcBoost(40, 20) * 10) / 10, 12);
  assert.equal(dcBoost(40, 0), 0);
  setTypeNames([{ code: 11, name: "Linkwitz Transform" }]);
  assert.equal(bandLabel(0, { ...lt, gain: 25, qp: 0.5 }), "Band 1: Linkwitz Transform, 40.00 Hz, Q0 0.900, fp 25.00 Hz, Qp 0.500");
  assert.equal(typeName(11), "Linkwitz Transform");
});

// -- On the real core (as eq.test.ts) --

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

test("WB6c: the crossover's type by crossoverType, frequency and bypass by write, each read back", { skip }, async () => {
  const session = await demoSession();
  const wire = session.state.snapshot!.state.numInputChannels;
  assert.equal(await session.change("crossoverType", { channel: wire, band: 0, filterType: encodeXover({ family: 2, order: 3, highPass: true }), freq: 120 }), true);
  let x = session.state.snapshot!.state.xover[wire]![0]!;
  assert.deepEqual([x.filterType, x.freq], [45, 120]);
  assert.equal(await session.write("eq.freq", { float: 150.5 }, [wire, 20]), true);
  assert.equal(await session.write("eq.bypass", { bool: true }, [wire, 20]), true);
  x = session.state.snapshot!.state.xover[wire]![0]!;
  assert.deepEqual([x.filterType, x.freq, x.bypass], [45, 150.5, true]);
  // Refused in the core's words, the old band kept.
  assert.equal(await session.change("crossoverType", { channel: wire, band: 0, filterType: 1 }), false);
  assert.match(session.state.error, /not a crossover type/);
});

test("WB6c: a crosspoint's fields are written through mix and read back; the delay limit is the device's", { skip }, async () => {
  const session = await demoSession();
  assert.equal(await session.change("crosspoint", { input: 0, output: 3, enabled: true, gain: -6.04, invert: true }), true);
  const st = session.state.snapshot!.state;
  assert.equal(st.matrixRouting[0]![3], true);
  assert.equal(st.matrixInvert[0]![3], true);
  assert.ok(Math.abs(st.matrixGain[0]![3]! + 6) < 1e-4);
  const { maxDelayMs } = await session.core.call("outputLimits", undefined);
  assert.ok(maxDelayMs === 21 || maxDelayMs === 42, String(maxDelayMs));
  // SL1: a typed fractional delay is kept exactly.
  assert.equal(await session.write("out.delay", { float: 7.1234 }, [1]), true);
  assert.ok(Math.abs(session.state.snapshot!.state.outputDelayMs[1]! - 7.1234) < 1e-4);
  // Limiter: a write to one member of a group moves the group (the core's).
  if (session.state.snapshot!.state.formatVersion >= 32) {
    assert.equal(await session.writeAll([0, 1].map((o) => ({ path: "limit.link", value: { int: 1 }, indices: [o] }))), true);
    assert.equal(await session.write("limit.threshold", { float: -6 }, [0]), true);
    const l = session.state.snapshot!.state;
    assert.equal(l.limiterThresholdDb[1], -6, "the group's other member follows");
  }
});

test("WB6c M49: a Linkwitz band is written by eq.fp and eq.qp and reads back", { skip }, async () => {
  const session = await demoSession();
  const wire = session.state.snapshot!.state.numInputChannels;
  const now = session.state.snapshot!.state.filters[wire]![3]!;
  const lt = { ...retyped({ ...now, filterType: T.peaking, freq: 45, q: 0.8 }, T.linkwitz), gain: 22, qp: 0.6 };
  assert.equal(await session.writeAll(bandWrites(wire, 3, now, lt)), true, session.state.error);
  const kept = session.state.snapshot!.state.filters[wire]![3]!;
  assert.equal(kept.filterType, T.linkwitz);
  assert.equal(kept.freq, 45);
  assert.ok(Math.abs(kept.gain - 22) < 1e-3, `fp ${kept.gain}`);
  assert.ok(Math.abs(kept.qp - 0.6) < 0.01, `qp ${kept.qp}`);
});

test("WB6c WB6b-3: over a slow queue every curves answer is shown as it comes, stale requests are dropped, the latest always sent", async () => {
  const { LatestOnly } = await import("../../src/screens/eq/curves");
  const sent: number[] = [];
  const shown: number[] = [];
  const slow = (q: number) =>
    new Promise<number>((r) => {
      sent.push(q);
      setTimeout(() => r(q), 30);
    });
  const p = new LatestOnly<number, number>(slow, (a) => shown.push(a));
  // A drag: a request every 10 ms, three times faster than the queue answers.
  for (let i = 1; i <= 12; i++) {
    p.request(i);
    await new Promise((r) => setTimeout(r, 10));
  }
  await new Promise((r) => setTimeout(r, 120));
  assert.equal(sent[0], 1);
  assert.equal(sent[sent.length - 1], 12, "the latest is sent");
  assert.ok(sent.length < 12, `stale requests dropped (${sent.length} sent)`);
  assert.deepEqual(shown, sent, "every answer shown, in order");
  assert.ok(shown.filter((x) => x < 12).length >= 2, "the curve moved during the drag, not only at its end");
});
