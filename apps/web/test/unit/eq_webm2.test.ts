// WP WEBM2 unit tests: the Equaliser's tabs in the shell (PEQ | Crossover | Delay & gain), an
// output's high and low pass over its four crossover bands, delay and distance, the arrival bars,
// the on-graph band label, and where the inspector goes (and, on the real core, a pass written
// the way the Crossover tab and the inspector write it).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { FilterParams } from "../../src/core/protocol";
import { SCREENS, screensFor, withDeviceScreen } from "../../src/ui/screens";
import { sectionsFor } from "../../src/ui/shell/sections";
import {
  arrivals,
  delayToMetres,
  distanceText,
  freeBand,
  metresToDelay,
  parseDistance,
  passChoices,
  passName,
  passes,
  setPass,
} from "../../src/screens/eq/align";
import { encodeXover } from "../../src/screens/outputs/model";
import { bandTag, OFF_BAND, T } from "../../src/screens/eq/model";
import { bandInInspector, BESIDE_MIN_PX, loadCollapsed, placement, saveCollapsed } from "../../src/ui/inspector/model";

const off: FilterParams = { filterType: 0, bypass: false, freq: 1000, q: 0.707, gain: 0, qp: 0.707 };
const xo = (filterType: number, freq: number, bypass = false): FilterParams => ({ ...off, filterType, freq, bypass });
const LR4_HP = encodeXover({ family: 1, order: 4, highPass: true });
const LR4_LP = encodeXover({ family: 1, order: 4, highPass: false });
const BW2_HP = encodeXover({ family: 2, order: 2, highPass: true });

test("WEBM2 tabs: the Equaliser is PEQ | Crossover | Delay & gain (then Inputs); Simple has PEQ and Inputs", () => {
  const eq = (m: "simple" | "advanced" | "installer") => sectionsFor(withDeviceScreen(screensFor(m, false))).find((s) => s.id === "equaliser")!;
  assert.deepEqual(
    eq("advanced").pages.map((p) => p.label),
    ["PEQ", "Crossover", "Delay & gain", "Inputs"],
  );
  assert.deepEqual(
    eq("installer").pages.map((p) => p.id),
    ["eq", "xover", "delays", "inputs"],
  );
  assert.deepEqual(
    eq("simple").pages.map((p) => p.id),
    ["eq", "inputs"],
    "setting up is not Simple's",
  );
  assert.equal(SCREENS.filter((s) => s.id === "xover" || s.id === "delays").length, 2);
});

test("WEBM2 passes: the first high and low pass that are on, whichever band holds them", () => {
  assert.deepEqual(passes([off, off, off, off]), { hp: null, lp: null });
  const bands = [xo(LR4_LP, 2400), xo(BW2_HP, 80, true), xo(LR4_HP, 90), off];
  const p = passes(bands);
  assert.deepEqual(p.hp, { band: 1, filterType: BW2_HP, freq: 80, bypass: true }, "the first high pass, bypassed or not");
  assert.deepEqual(p.lp, { band: 0, filterType: LR4_LP, freq: 2400, bypass: false });
  assert.equal(passName(p.hp), "BW2");
  assert.equal(passName(null), "Off");
  assert.equal(freeBand(bands), 3);
  assert.equal(freeBand([xo(LR4_LP, 1), xo(LR4_LP, 1), xo(LR4_LP, 1), xo(LR4_LP, 1)]), null);
});

test("WEBM2 passes: a new pass takes the first free band, a change keeps its band, Off clears it", () => {
  const bands = [xo(LR4_LP, 2400), off, off, off];
  assert.deepEqual(setPass(bands, true, LR4_HP), { band: 1, filterType: LR4_HP });
  assert.deepEqual(setPass(bands, false, encodeXover({ family: 2, order: 4, highPass: false })), {
    band: 0,
    filterType: encodeXover({ family: 2, order: 4, highPass: false }),
  });
  assert.deepEqual(setPass(bands, false, 0), { band: 0, filterType: 0 });
  assert.equal(setPass(bands, false, LR4_LP), null, "no change");
  assert.equal(setPass(bands, true, 0), null, "no high pass to clear");
  const full = [xo(LR4_LP, 1), xo(LR4_LP, 2), xo(LR4_LP, 3), xo(LR4_LP, 4)];
  assert.equal(setPass(full, true, LR4_HP), null, "no band free");
});

test("WEBM2 passes: every family's slopes, high or low, and nothing else", () => {
  const hp = passChoices(true);
  assert.equal(hp[0]!.label, "Off");
  assert.equal(hp.length, 1 + 4 + 8 + 4);
  assert.ok(hp.some((c) => c.label === "LR4 24 dB/oct" && c.value === LR4_HP));
  assert.ok(hp.some((c) => c.label === "BW1 6 dB/oct"));
  assert.ok(
    hp.slice(1).every((c) => c.value >= 32 && c.value <= 63 && (c.value - 32) % 2 === 1),
    "odd: high pass",
  );
  assert.ok(
    passChoices(false)
      .slice(1)
      .every((c) => (c.value - 32) % 2 === 0),
    "even: low pass",
  );
});

test("WEBM2 delays: distance at 343 m/s both ways, typed with its unit, and the arrival bars", () => {
  assert.equal(delayToMetres(1), 0.343);
  assert.ok(Math.abs(metresToDelay(delayToMetres(2.5)) - 2.5) < 1e-12);
  assert.equal(distanceText(0.343), "34.3 cm");
  assert.equal(distanceText(1.715), "1.72 m");
  assert.equal(parseDistance("1.2"), 1.2);
  assert.equal(parseDistance("35 cm"), 0.35);
  assert.equal(parseDistance("343mm"), 0.343);
  assert.equal(parseDistance("1,5 m"), 1.5);
  for (const bad of ["", "-1", "abc", "1 km"]) assert.equal(parseDistance(bad), null, bad);
  assert.deepEqual(arrivals([0, 1, 2, 4]), [0, 0.25, 0.5, 1]);
  assert.deepEqual(arrivals([0, 0]), [0, 0], "no delay anywhere");
  assert.deepEqual(arrivals([]), []);
});

test("WEBM2 graph label: the chosen band in short, live values", () => {
  assert.equal(bandTag(2, { ...OFF_BAND, filterType: T.peaking, freq: 1200, gain: 3, q: 1.41 }), "3 · Bell · 1.20kHz · +3.0 dB · Q 1.410");
  assert.equal(bandTag(0, { ...OFF_BAND, filterType: T.highPass1, freq: 80, bypass: true }), "1 · Low Cut · 80.0Hz · off");
});

test("WEBM2 inspector: beside a wide page, under a narrow one, hidden when the viewer hid it; the band bar on touch", () => {
  assert.equal(placement(BESIDE_MIN_PX, false), "beside");
  assert.equal(placement(BESIDE_MIN_PX - 1, false), "under");
  assert.equal(placement(2000, true), "hidden");
  assert.equal(bandInInspector("beside", false), true);
  assert.equal(bandInInspector("beside", true), false, "a finger keeps the band bar under the graph");
  assert.equal(bandInInspector("under", false), false);
  const kept = new Map<string, string>();
  const store = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  assert.equal(loadCollapsed(store), false);
  saveCollapsed(store, true);
  assert.equal(loadCollapsed(store), true);
  kept.set("dspi-web.inspector", "{not json");
  assert.equal(loadCollapsed(store), false);
  const broken = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  assert.equal(loadCollapsed(broken), false);
  assert.doesNotThrow(() => saveCollapsed(broken, true));
  assert.equal(loadCollapsed(null), false);
});

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

test("WEBM2 on the demo: a high pass set, moved and cleared the way the Crossover tab writes it", { skip, timeout: 60_000 }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const snap = await core.call("open", { kind: "demo" });
  const s = snap.state;
  assert.ok(Array.isArray(s.xover) && s.xover.length > 0, "the demo has crossovers");
  const wire = s.numInputChannels;
  const bands = () => core.call("state", undefined).then((x) => x.state.xover![wire]!);
  // Clear every band first: the demo may come with a crossover.
  for (let b = 0; b < (await bands()).length; b++) await core.call("crossoverType", { channel: wire, band: b, filterType: 0, freq: 1000 });
  const add = setPass(await bands(), true, LR4_HP)!;
  assert.equal(add.band, 0);
  await core.call("crossoverType", { channel: wire, band: add.band, filterType: add.filterType });
  await core.call("write", { path: "eq.freq", value: { float: 85 }, indices: [wire, 20 + add.band] });
  let p = passes(await bands());
  assert.equal(p.hp?.filterType, LR4_HP);
  assert.ok(Math.abs(p.hp!.freq - 85) < 0.5, `freq ${p.hp?.freq}`);
  assert.equal(p.lp, null);
  const change = setPass(await bands(), true, BW2_HP)!;
  assert.equal(change.band, 0, "kept in its band");
  await core.call("crossoverType", { channel: wire, band: change.band, filterType: change.filterType });
  p = passes(await bands());
  assert.equal(p.hp?.filterType, BW2_HP);
  const clear = setPass(await bands(), true, 0)!;
  await core.call("crossoverType", { channel: wire, band: clear.band, filterType: 0, freq: 1000 });
  assert.deepEqual(passes(await bands()), { hp: null, lp: null });
});
