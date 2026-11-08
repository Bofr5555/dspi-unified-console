// WP WB6b unit tests: the EQ screen's logic (types, shapes, dots, drags, keys,
// value text), and on the real core the curves, a band's writes with the
// read-back, the session's unsaved state and Save's naming of an unnamed slot.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { FilterParams } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { UnsavedWatch } from "../../src/ui/unsaved";
import {
  DEFAULT_LIMITS,
  T,
  availableTypes,
  bandLabel,
  bandWrites,
  curveAt,
  dbAt,
  dragResult,
  freqAt,
  freqText,
  gainText,
  newBand,
  nudge,
  parseFreq,
  parseNote,
  parseNumber,
  pinch,
  retyped,
  roleDb,
  roleOf,
  shapeOf,
  usesGain,
  usesQ,
  wheel,
  x,
  y,
  type Geometry,
} from "../../src/screens/eq/model";

const g: Geometry = { width: 600, height: 300, minFreq: 20, maxFreq: 20000, dbTop: 25, dbBottom: -25 };
const bell = (freq: number, gain: number, q = 1): FilterParams => ({ filterType: T.peaking, bypass: false, freq, q, gain, qp: 0.707 });

test("WB6b: the geometry maps log frequency across and dB down, both ways", () => {
  assert.equal(x(g, 20), 0);
  assert.ok(Math.abs(x(g, 20000) - 600) < 1e-9);
  assert.ok(Math.abs(freqAt(g, x(g, 1234)) - 1234) < 1e-6);
  assert.equal(y(g, 25), 0);
  assert.equal(y(g, 0), 150);
  assert.ok(Math.abs(dbAt(g, y(g, -7.5)) + 7.5) < 1e-9);
});

test("WB6b: a curve between the core's grid points is read in log frequency", () => {
  const freqs = [10, 100, 1000];
  assert.equal(curveAt(freqs, [0, 10, 20], 100), 10);
  assert.ok(Math.abs(curveAt(freqs, [0, 10, 20], Math.sqrt(10 * 100)) - 5) < 1e-9);
  assert.equal(curveAt(freqs, [0, 10, 20], 5), 0);
  assert.equal(curveAt(freqs, [0, 10, 20], 5000), 20);
});

test("WB6b: types offered follow the firmware, as the Console's picker; the Linkwitz Transform is not offered", () => {
  const v32 = availableTypes({ formatVersion: 32, fwMajor: 1, fwMinor: 1, fwPatch: 6 });
  assert.deepEqual(v32, [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13]);
  const old = availableTypes({ formatVersion: 12, fwMajor: 1, fwMinor: 1, fwPatch: 3 });
  assert.deepEqual(old, [0, 1, 2, 3, 4, 5]);
});

test("WB6b: shapes and slopes map to the firmware's types and back; a retype keeps frequency and fills Q", () => {
  assert.equal(shapeOf(T.highPass1)!.shape.id, "lowCut");
  assert.equal(shapeOf(T.highPass1)!.order, 1);
  assert.equal(shapeOf(T.lowPass)!.shape.id, "highCut");
  assert.equal(shapeOf(T.linkwitz), null);
  const shelf = retyped({ ...bell(500, 4), filterType: T.lowShelf1 }, T.lowShelf);
  assert.equal(shelf.q, 0.707, "a 1st-order shelf has no Q; the 2nd-order one starts at 0.707");
  assert.equal(shelf.freq, 500);
  assert.equal(shelf.gain, 4);
  const notch = retyped(bell(500, 4), T.notch);
  assert.equal(notch.gain, 0);
  assert.equal(notch.q, 1);
});

test("WB6b: dots sit where the Console puts them: on gain by the core's unit, cuts at 20 log Q, others fixed", () => {
  const p = bell(1000, 6);
  assert.equal(roleDb(roleOf(p, { curve: [], at: 6, unit: 1, usesGain: true, usesQ: true }), p), 6);
  const shelf = { ...p, filterType: T.lowShelf };
  assert.equal(roleDb(roleOf(shelf, { curve: [], at: 3, unit: 0.5, usesGain: true, usesQ: true }), shelf), 3);
  const cut = { ...p, filterType: T.lowPass, q: 2 };
  assert.ok(Math.abs(roleDb(roleOf(cut, undefined), cut) - 6.0206) < 1e-3);
  const notch = { ...p, filterType: T.notch };
  assert.equal(roleDb(roleOf(notch, undefined), notch), 0);
  assert.equal(roleOf({ ...p, filterType: T.linkwitz }, undefined).kind, "locked");
});

test("WB6b: a drag moves the grabbed dot with the pointer and the others by the same ratio and step", () => {
  const start = new Map([
    [0, bell(1000, 0)],
    [1, bell(2000, 3)],
  ]);
  const role = (_b: number, q: FilterParams) => roleOf(q, { curve: [], at: 0, unit: 1, usesGain: true, usesQ: true });
  // One octave right, 30 px up = 5 dB.
  const dx = x(g, 2000) - x(g, 1000);
  const out = dragResult({ grabbed: 0, start, qMode: false }, dx, -30, g, DEFAULT_LIMITS, role);
  assert.equal(out.get(0)!.freq, 2000);
  assert.equal(out.get(0)!.gain, 5);
  assert.equal(out.get(1)!.freq, 4000);
  assert.equal(out.get(1)!.gain, 8);
  // Gain stops at the registry's limit.
  assert.equal(dragResult({ grabbed: 0, start, qMode: false }, 0, -1000, g, DEFAULT_LIMITS, role).get(0)!.gain, 24);
  // Ctrl-drag is Q: 60 px up doubles it.
  assert.equal(dragResult({ grabbed: 0, start, qMode: true }, 0, -60, g, DEFAULT_LIMITS, role).get(0)!.q, 2);
});

test("WB6b: keys, wheel and pinch step frequency, gain and Q as the Console's keys do", () => {
  const p = bell(1000, 0);
  assert.ok(Math.abs(nudge(p, "ArrowRight", false, false, g, DEFAULT_LIMITS)!.freq - 1059) < 1);
  assert.equal(nudge(p, "ArrowUp", false, false, g, DEFAULT_LIMITS)!.gain, 0.5);
  assert.equal(nudge(p, "ArrowDown", false, true, g, DEFAULT_LIMITS)!.gain, -0.1);
  assert.equal(nudge(p, "ArrowUp", true, false, g, DEFAULT_LIMITS)!.q, 1.072);
  // A cut has no gain: up and down are its Q.
  assert.equal(nudge({ ...p, filterType: T.highPass, q: 1 }, "ArrowUp", false, false, g, DEFAULT_LIMITS)!.q, 1.072);
  assert.equal(nudge({ ...p, filterType: T.highPass1 }, "ArrowUp", false, false, g, DEFAULT_LIMITS), null);
  assert.equal(wheel(p, 100, false, DEFAULT_LIMITS)!.q, 2);
  assert.equal(wheel(p, 20, true, DEFAULT_LIMITS)!.gain, 1);
  assert.equal(pinch(p, 2, DEFAULT_LIMITS)!.q, 0.5, "fingers apart: wider");
});

test("WB6b: a double click makes a bell at the pointer; pulled from the curve's ends, a shelf", () => {
  const b = newBand(g, x(g, 500), y(g, 6), false, DEFAULT_LIMITS, [1, 2, 3]);
  assert.equal(b.filterType, T.peaking);
  assert.ok(Math.abs(b.freq - 500) < 0.01);
  assert.equal(b.gain, 6);
  assert.equal(newBand(g, 10, 0, true, DEFAULT_LIMITS, [1, 2, 3]).filterType, T.lowShelf);
  assert.equal(newBand(g, 595, 0, true, DEFAULT_LIMITS, [1, 2, 3]).filterType, T.highShelf);
});

test("WB6b: a band's writes are the fields that differ, by registry path, type first", () => {
  const now = bell(1000, 0);
  assert.deepEqual(bandWrites(4, 2, now, now), []);
  assert.deepEqual(
    bandWrites(4, 2, now, { ...now, filterType: T.lowShelf, gain: 3, bypass: true }).map((w) => w.path),
    ["eq.type", "eq.gain", "eq.bypass"],
  );
  assert.deepEqual(bandWrites(4, 2, now, { ...now, freq: 50 })[0], { path: "eq.freq", indices: [4, 2], value: { float: 50 } });
});

test("WB6b: values read and type as on the Console: 2k, note names, units", () => {
  assert.equal(freqText(1000), "1.000 kHz");
  assert.equal(freqText(12345), "12.35 kHz");
  assert.equal(freqText(250), "250.0 Hz");
  assert.equal(gainText(-0.001), "+0.00 dB");
  assert.equal(gainText(3), "+3.00 dB");
  assert.equal(parseFreq("2k"), 2000);
  assert.equal(parseFreq("2 kHz"), 2000);
  assert.equal(parseFreq("440"), 440);
  assert.equal(parseFreq("A4"), 440);
  assert.ok(Math.abs(parseNote("c4")! - 261.626) < 0.01);
  assert.ok(Math.abs(parseNote("a4+100")! - 466.16) < 0.01);
  assert.equal(parseFreq("loud"), null);
  assert.equal(parseNumber("+3.5 dB", "db"), 3.5);
  assert.equal(parseNumber("q0.7", ""), 0.7);
  assert.equal(parseNumber("", ""), null);
  assert.equal(bandLabel(2, { ...bell(1000, 3), bypass: true }), "Band 3: Peaking, 1.000 kHz, +3.00 dB, Q 1.000, bypassed");
});

// -- On the real core (as wasm.test.ts) --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(access: "control" | "view") {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const calls: string[] = [];
  const client = {
    call: (m: never, p: never) => {
      calls.push(m);
      return core.call(m, p);
    },
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  return { session: new Session("s", "i", "Here", { kind: "demo" }, access, client), calls };
}

const settle = () => new Promise((r) => setTimeout(r, 50));

test("WB6b: the core answers curves and which types use Q and gain, as the page's table says", { skip }, async () => {
  const { session } = await sessionOn("control");
  const grid = await session.core.call("curveGrid", undefined);
  assert.equal(grid.freqs.length, 201);
  const bands = Array.from({ length: 14 }, (_, t) => ({ ...bell(1000, 3), filterType: t, gain: t === T.linkwitz ? 500 : 3 }));
  const r = await session.core.call("curves", { chains: [[bell(1000, 6)]], bands });
  assert.ok(Math.abs(curveAt(grid.freqs, r.chains[0]!, 1000) - 6) < 0.05);
  for (let t = 1; t < 14; t++) {
    assert.equal(r.bands[t]!.usesQ, usesQ(t), `usesQ ${t}`);
    assert.equal(r.bands[t]!.usesGain, usesGain(t), `usesGain ${t}`);
  }
});

test("WB6b: a band's writes go through the registry paths and the state reads back what the device kept", { skip }, async () => {
  const { session } = await sessionOn("control");
  await session.open({ kind: "demo" });
  const wire = session.state.snapshot!.state.numInputChannels; // OUT1
  const now = session.state.snapshot!.state.filters[wire]![0]!;
  const next = { ...now, filterType: T.peaking, freq: 250, q: 2, gain: -4.5, bypass: false };
  assert.equal(await session.writeAll(bandWrites(wire, 0, now, next)), true);
  const kept = session.state.snapshot!.state.filters[wire]![0]!;
  assert.deepEqual([kept.filterType, kept.freq, kept.q, kept.gain], [T.peaking, 250, 2, -4.5]);
  // A value the registry refuses stops there, and says why.
  assert.equal(await session.writeAll([{ path: "eq.gain", indices: [wire, 0], value: { float: 40 } }]), false);
  assert.match(session.state.error, /Gain must be between -24 and 24/);
  // The session follows what is unsaved, in the core's words, for every screen and the page.
  await settle();
  assert.ok(session.dirty);
  assert.ok(session.state.unsaved.length > 0);
});

test("WB6b: the console's unsaved state follows its sessions; the leave prompt is optional", { skip }, async () => {
  const { session } = await sessionOn("control");
  await session.open({ kind: "demo" });
  const watch = new UnsavedWatch(false);
  const seen: boolean[] = [];
  watch.onChange((u) => seen.push(u));
  watch.track([session]);
  assert.equal(watch.unsaved, false);
  await session.write("out.gain", { float: -3 }, [0]);
  await settle();
  assert.equal(watch.unsaved, true);
  await session.change("presetRevert", undefined);
  await settle();
  assert.equal(watch.unsaved, false);
  assert.deepEqual(seen, [true, false]);
});

test("WB6b: Save names an unnamed slot Preset N first, as the Console and Qt do (c6p WB6a-3)", { skip }, async () => {
  const { session, calls } = await sessionOn("control");
  await session.open({ kind: "demo" });
  assert.equal(session.state.snapshot!.state.presetNames[4] ?? "", "");
  calls.length = 0;
  assert.equal(await session.savePreset(4), true);
  assert.equal(session.state.snapshot!.state.presetNames[4], "Preset 5");
  assert.ok(session.state.snapshot!.state.presetOccupied & (1 << 4));
  // A named slot keeps its name.
  assert.equal(await session.savePreset(0), true);
  assert.equal(session.state.snapshot!.state.presetNames[0], "Demo");
});
