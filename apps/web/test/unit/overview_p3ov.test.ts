// WP P3OV unit tests: the Overview's model (PLAN-P3 U1). Rows from the
// session's mirror (routing sources, gain, delay, mute, limiter, disabled
// outputs, the core's labels), the meters from the core's feed (level, the
// held peak as its ballistics decay it, the clip hold), the gain reduction's
// text, the inline gain's steps; and on the real core: the rows follow the
// writes (the same writes the output card makes), the registry's units, the
// core's peak hold never below the level and falling once the peak is over,
// a read-only session's writes refused.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { DspState, OutputLabel, SystemStatus } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import {
  DEFAULT_UNITS,
  GR_FLOOR_DB,
  anyLimiter,
  gainStep,
  gainTyped,
  grText,
  meterChannels,
  meterPct,
  meterReading,
  overviewRows,
  routingText,
  valueTexts,
} from "../../src/screens/overview/model";

function state(over: Partial<DspState> = {}): DspState {
  const outs = 4;
  const ins = 2;
  return {
    platformId: 1,
    numChannels: ins + outs,
    numInputChannels: ins,
    numOutputChannels: outs,
    formatVersion: 32,
    channelNames: ["USB 1", "USB 2", "Out A", "Out B", "Out C", "Sub"],
    outputGainDb: [0, -3.04, 2, -60],
    outputMuted: [false, true, false, false],
    outputEnabled: [true, true, true, false],
    outputDelayMs: [0, 12.3456, 1, 0],
    matrixRouting: [
      [true, false, true, false],
      [false, true, true, false],
    ],
    matrixGain: [
      [0, 0, -6, 0],
      [0, 3.5, -6, 0],
    ],
    matrixInvert: [
      [false, false, false, false],
      [false, false, true, false],
    ],
    limiterEnabled: [true, false, true, false],
    limiterLinkGroup: [0, 0, 0, 0],
    limiterThresholdDb: [-1, -6, -3.25, 0],
    limiterReleaseMs: [100, 100, 100, 100],
    inputSource: 0,
    i2sInputChannels: 2,
    ...over,
  } as DspState;
}

function status(over: Partial<SystemStatus> = {}): SystemStatus {
  return { peaks: [], cpu0: 0, cpu1: 0, clipFlags: 0, activeInputChannels: 2, numChannels: 6, stale: false, ageMs: 0, peakHold: [], peakMax: [], clipHold: 0, ...over };
}

test("P3OV rows: every output from the mirror, enabled or not, with its sources, gain, delay, mute and limiter", () => {
  const labels: OutputLabel[] = [{ output: 0, label: "Front L", shortLabel: "L", slotName: "SPDIF 1 L", userNamed: true, channel: "L", band: "Full" } as OutputLabel];
  const rows = overviewRows(state(), labels);
  assert.equal(rows.length, 4, "the disabled output too");
  assert.deepEqual(
    rows.map((r) => [r.descriptor, r.wire, r.name, r.enabled]),
    [
      ["OUT1", 2, "Front L", true],
      ["OUT2", 3, "Out B", true],
      ["OUT3", 4, "Out C", true],
      ["OUT4", 5, "Sub", false],
    ],
  );
  assert.equal(rows[0]!.slot, "SPDIF 1 L");
  assert.equal(routingText(rows[0]!), "USB 1 0 dB");
  assert.equal(routingText(rows[1]!), "USB 2 +3.5 dB");
  assert.equal(routingText(rows[2]!), "USB 1 -6 dB, USB 2 -6 dB inv");
  assert.equal(routingText(rows[3]!), "None");
  assert.deepEqual(valueTexts(rows[1]!), { gain: "-3.0 dB", delay: "12.3456 ms", threshold: "Off" });
  assert.deepEqual(valueTexts(rows[2]!), { gain: "2.0 dB", delay: "1.0 ms", threshold: "-3.2 dBFS" });
  assert.equal(rows[1]!.muted, true);
  assert.equal(anyLimiter(rows), true);
  assert.equal(anyLimiter(overviewRows(state({ limiterEnabled: [false, false, false, false] }))), false);
});

test("P3OV rows: firmware without the limiter has no limiter column value; short or odd arrays read as defaults", () => {
  const rows = overviewRows(state({ formatVersion: 31 }));
  assert.ok(rows.every((r) => r.limiter === null));
  assert.equal(valueTexts(rows[0]!).threshold, "—");
  assert.equal(grText(rows[0]!, [3]), "—");
  // A mirror missing fields (an older core, a document): no throw, zeros and nothing routed.
  const bare = overviewRows(state({ outputGainDb: [], outputDelayMs: [], matrixRouting: [], matrixGain: [], matrixInvert: [], outputMuted: [], outputEnabled: [] }));
  assert.equal(bare.length, 4);
  assert.ok(bare.every((r) => r.gainDb === 0 && r.delayMs === 0 && !r.muted && r.enabled && r.sources.length === 0));
  // A nonsense count is bounded.
  assert.equal(overviewRows(state({ numOutputChannels: 1e9 })).length, 64);
});

test("P3OV meters: the live inputs and every output, in wire order", () => {
  const ch = meterChannels(state(), undefined, 2);
  assert.deepEqual(
    ch.map((c) => [c.wire, c.descriptor, c.kind]),
    [
      [0, "IN1", "input"],
      [1, "IN2", "input"],
      [2, "OUT1", "output"],
      [3, "OUT2", "output"],
      [4, "OUT3", "output"],
      [5, "OUT4", "output"],
    ],
  );
});

test("P3OV meters: level, held peak and clip from the core's feed; the hold falls as the core's ballistics let it", () => {
  assert.deepEqual(meterReading(null, 0), { db: -90, holdDb: -90, clip: false });
  // A peak of 0.5 (-6 dB) held at 1.0 (0 dB); channel 2's clip hold lit.
  const m = status({ peaks: [0.5, 0, 0.1], peakHold: [1, 0, 0.05], clipHold: 0b100 });
  const a = meterReading(m, 0);
  assert.ok(Math.abs(a.db + 6.02) < 0.01, String(a.db));
  assert.equal(a.holdDb, 0);
  assert.equal(a.clip, false);
  assert.equal(meterReading(m, 1).db, -90, "silence is the floor");
  const c = meterReading(m, 2);
  assert.equal(c.clip, true);
  assert.equal(c.holdDb, c.db, "the hold is never below the level");
  // The core decays the hold after a peak: successive readings with a steady level.
  const seq = [1, 0.8, 0.6, 0.5, 0.5].map((h) => meterReading(status({ peaks: [0.5], peakHold: [h] }), 0).holdDb);
  for (let i = 1; i < seq.length; i++) assert.ok(seq[i]! <= seq[i - 1]!, `falls: ${seq.join(", ")}`);
  assert.equal(seq.at(-1), meterReading(status({ peaks: [0.5] }), 0).db, "the fall ends at the level");
  // A core without ballistics (no peakHold): the marker is the level.
  assert.equal(meterReading(status({ peaks: [0.25] }), 0).holdDb, meterReading(status({ peaks: [0.25] }), 0).db);
  // The top bit of a 32-bit mask.
  assert.equal(meterReading(status({ clipHold: 0x8000_0000 | 0 }), 31).clip, true);
  assert.equal(meterPct(-90), 0);
  assert.equal(meterPct(0), 100);
  assert.equal(meterPct(-45), 50);
  assert.equal(meterPct(12), 100);
});

test("P3OV gain reduction: positive dB from limiterMeter shown as a reduction; none, the floor and no limiter", () => {
  const rows = overviewRows(state());
  assert.equal(grText(rows[0]!, [3.24]), "-3.2 dB");
  assert.equal(grText(rows[0]!, [0]), "0 dB");
  assert.equal(grText(rows[0]!, [GR_FLOOR_DB]), "—", "the display floor is no reading");
  assert.equal(grText(rows[0]!, null), "—");
  assert.equal(grText(rows[0]!, []), "—");
  assert.equal(grText(rows[1]!, [0, 5]), "—", "its limiter is off");
});

test("P3OV inline gain: half-dB steps from a fraction, inside the registry's range; typed values clamped", () => {
  const r: [number, number] = [-60, 10];
  assert.equal(gainStep(-3.04, 1, r), -3);
  assert.equal(gainStep(-3.04, -1, r), -3.5);
  assert.equal(gainStep(-3, 1, r), -2.5);
  assert.equal(gainStep(10, 1, r), 10);
  assert.equal(gainStep(-60, -1, r), -60);
  assert.ok(Object.is(gainStep(-0.5, 1, r), 0), "never -0");
  assert.equal(gainTyped(99, r), 10);
  assert.equal(gainTyped(-99, r), -60);
  assert.equal(gainTyped(-7.25, r), -7.25);
  assert.equal(DEFAULT_UNITS.gain.unit, "dB");
});

// -- On the real core (as outputs.test.ts) --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demoSession(access: "control" | "view" = "control") {
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
  const session = new Session("s", "i", "Here", { kind: "demo" }, access, client);
  await session.open({ kind: "demo" });
  return session;
}

test("P3OV on the core: the rows follow the output card's writes (gain, mute, crosspoint, limiter), with the registry's units", { skip }, async () => {
  const session = await demoSession();
  const rows0 = overviewRows(session.state.snapshot!.state, session.state.snapshot!.outputLabels);
  assert.equal(rows0.length, session.state.snapshot!.state.numOutputChannels);
  assert.equal(await session.write("out.gain", { float: -7.5 }, [2]), true);
  assert.equal(await session.write("out.mute", { bool: true }, [3]), true);
  assert.equal(await session.change("crosspoint", { input: 1, output: 0, enabled: true, gain: -6, invert: true }), true);
  const s = session.state.snapshot!.state;
  const rows = overviewRows(s, session.state.snapshot!.outputLabels);
  assert.equal(valueTexts(rows[2]!).gain, "-7.5 dB");
  assert.equal(rows[3]!.muted, true);
  const inv = rows[0]!.sources.find((x) => x.descriptor === "IN2");
  assert.ok(inv && inv.invert && Math.abs(inv.gainDb + 6) < 1e-3, JSON.stringify(rows[0]!.sources));
  if (s.formatVersion >= 32) {
    assert.equal(await session.write("limit.on", { bool: true }, [1]), true);
    assert.equal(await session.write("limit.threshold", { float: -4 }, [1]), true);
    const l = overviewRows(session.state.snapshot!.state)[1]!;
    assert.deepEqual(l.limiter, { on: true, thresholdDb: -4 });
    const gr = await session.core.call("limiterMeter", undefined);
    assert.ok(Array.isArray(gr));
    assert.notEqual(grText(l, gr), "", "a reading or the dash");
  }
  // The registry's range and units for the inline gain and the headers.
  const g = await session.core.call("describe", { path: "out.gain" });
  assert.equal(typeof g.min, "number");
  assert.equal(typeof g.max, "number");
  const d = await session.core.call("describe", { path: "out.delay" });
  assert.ok(typeof d.unit === "string" || d.unit === undefined);
  // Past the registry's range the core refuses; the row keeps the old value.
  assert.equal(await session.write("out.gain", { float: g.max! + 50 }, [2]), false);
  assert.equal(overviewRows(session.state.snapshot!.state)[2]!.gainDb, -7.5);
});

test("P3OV on the core: the meter feed's hold is never below the level, for every live channel", { skip }, async () => {
  const session = await demoSession();
  await session.core.call("setMeterBallistics", { holdMs: 50, decayPerS: 2, clipHoldMs: 200 });
  const snap = session.state.snapshot!;
  const ch = meterChannels(snap.state, snap.outputLabels, 2);
  for (let k = 0; k < 3; k++) {
    const m = await session.core.call("meters", undefined);
    for (const c of ch) {
      const r = meterReading(m, c.wire);
      assert.ok(r.holdDb >= r.db, `${c.descriptor}: ${r.holdDb} < ${r.db}`);
    }
  }
  // A clip on IN1 lights its clip marker from the core's clip hold.
  await session.core.call("demo", { action: "clip", mask: 1 });
  let lit = false;
  for (let k = 0; k < 20 && !lit; k++) {
    lit = meterReading(await session.core.call("meters", undefined), 0).clip;
    if (!lit) await new Promise((r) => setTimeout(r, 50));
  }
  assert.equal(lit, true);
});

test("P3OV on the core: a read-only session's inline writes are refused, nothing changes", { skip }, async () => {
  const session = await demoSession("view");
  const before = overviewRows(session.state.snapshot!.state)[0]!;
  assert.equal(await session.write("out.gain", { float: -9 }, [0]), false);
  assert.equal(await session.write("out.mute", { bool: !before.muted }, [0]), false);
  const after = overviewRows(session.state.snapshot!.state)[0]!;
  assert.deepEqual([after.gainDb, after.muted], [before.gainDb, before.muted]);
});
