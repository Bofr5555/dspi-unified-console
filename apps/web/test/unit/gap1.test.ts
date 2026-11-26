// WP GAP1 unit tests: the web gaps. The EQ graph's phase (its axis, its ticks, the dotted line
// only while asked, the Graphing switches kept); the output polarity's words and the routes a
// change writes; the crossfeed pair map's words, chips and the K340 question; and on the real core
// (as main.test.ts): `curves` with the phase, an output inverted on every feeding route in one undo
// step, and the pair map written and read back.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { draw, phaseExtent, phaseTicks, type Scene } from "../../src/screens/eq/draw";
import { DEFAULT_PREFS, tidyPrefs } from "../../src/screens/settings/model";
import { flipTo, polarityLine, polarityOf, polarityRoutes, polarityText, setOutputPolarity } from "../../src/screens/outputs/polarity";
import { choiceTitle, offersK340Mask, pairChipText } from "../../src/screens/effects/pairmap";
import { maskBits } from "../../src/screens/effects/model";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import type { CrossfeedPairMapView, DspState } from "../../src/core/protocol";
import { Session } from "../../src/ui/session";

// ---------------------------------------------------------------- the phase

test("GAP1 phase: the axis follows the dB zoom, Qt's ticks", () => {
  assert.equal(phaseExtent(25, -25), 180);
  assert.equal(phaseExtent(50, -50), 360);
  assert.equal(phaseExtent(5, -5), 36);
  const t = phaseTicks(180);
  assert.deepEqual(
    t.map((x) => x.text),
    ["+180°", "+90°", "0°", "−90°", "−180°"],
  );
  assert.deepEqual(
    t.map((x) => x.at),
    [0, 0.25, 0.5, 0.75, 1],
  );
});

/** A canvas context that keeps what was asked of it. */
function recorder() {
  const calls: { name: string; args: unknown[] }[] = [];
  const props: Record<string, unknown> = {};
  const ctx = new Proxy(props, {
    get(target, key: string) {
      if (key in target) return target[key];
      return (...args: unknown[]) => {
        calls.push({ name: key, args });
        return key === "measureText" ? { width: String(args[0]).length * 6 } : undefined;
      };
    },
    set(target, key: string, v) {
      target[key] = v;
      return true;
    },
  }) as unknown as CanvasRenderingContext2D;
  return { ctx, calls };
}

function scene(phase: Scene["phase"]): Scene {
  const freqs = Array.from({ length: 201 }, (_, i) => 10 * Math.pow(2000, i / 200));
  return {
    g: { width: 800, height: 300, minFreq: 20, maxFreq: 20000, dbTop: 25, dbBottom: -25 },
    freqs,
    others: [],
    main: { curve: freqs.map(() => 0), colour: "#4a90d9", offset: 0 },
    phase,
    bands: [],
    marquee: null,
    coarse: false,
    theme: { bg: "#000", grid: "#111", zero: "#222", text: "#ccc", ring: "#fff" },
  };
}

test("GAP1 phase: drawn dotted on its own axis with its scale only while asked", () => {
  const off = recorder();
  draw(off.ctx, scene(null));
  assert.ok(!off.calls.some((c) => c.name === "fillText" && String(c.args[0]).endsWith("°")), "no phase scale without the phase");
  assert.ok(!off.calls.some((c) => c.name === "setLineDash" && (c.args[0] as number[])[0] === 0.1));

  const on = recorder();
  const curve = Array.from({ length: 201 }, (_, i) => -i);
  draw(on.ctx, scene({ curve, extent: 180 }));
  assert.ok(on.calls.some((c) => c.name === "setLineDash" && (c.args[0] as number[])[0] === 0.1), "dotted");
  const scale = on.calls.filter((c) => c.name === "fillText" && String(c.args[0]).endsWith("°")).map((c) => c.args[0]);
  // The top tick is left out: the graph's chips sit in that corner
  assert.deepEqual(scale, ["+90°", "0°", "−90°", "−180°"]);
  // The line's points: 0° in the middle, -180° at the bottom
  const lines = on.calls.filter((c) => c.name === "lineTo" || c.name === "moveTo");
  const ys = lines.map((c) => c.args[1] as number);
  assert.ok(ys.includes(150), "0° at mid height");
  assert.ok(ys.includes(300), "-180° at the bottom");
});

test("GAP1 phase: the Graphing switches, off by default and kept", () => {
  assert.equal(DEFAULT_PREFS.graph.showPhase, false);
  assert.equal(DEFAULT_PREFS.graph.phaseUnwrapped, false);
  const p = tidyPrefs({ graph: { showPhase: true, phaseUnwrapped: "yes" } });
  assert.equal(p.graph.showPhase, true);
  assert.equal(p.graph.phaseUnwrapped, false);
  assert.equal(tidyPrefs({ graph: { phaseUnwrapped: true } }).graph.phaseUnwrapped, true);
});

// ---------------------------------------------------------------- output polarity

test("GAP1 polarity: Qt's words, the click's choice, the routes a change writes", () => {
  const snap = { outputPolarity: ["normal", "inverted", "mixed", "none"] as const };
  assert.equal(polarityOf({ outputPolarity: [...snap.outputPolarity] }, 1), "inverted");
  assert.equal(polarityOf({ outputPolarity: [...snap.outputPolarity] }, 9), "none");
  assert.equal(polarityOf(null, 0), "none");
  assert.deepEqual(
    snap.outputPolarity.map(polarityText),
    ["Normal", "Inverted", "Mixed", "–"],
  );
  assert.deepEqual(
    snap.outputPolarity.map(polarityLine),
    ["Polarity normal", "Polarity inverted", "Polarity mixed", "Polarity normal"],
  );
  assert.deepEqual(
    snap.outputPolarity.map(flipTo),
    [true, false, false, false],
  );
  const s = {
    numInputChannels: 3,
    matrixRouting: [
      [true, true],
      [true, false],
      [false, true],
      [true, true],
    ],
    matrixInvert: [
      [false, true],
      [true, true],
      [true, false],
      [false, false],
    ],
  };
  // Out 1 inverted: in 1 changes (in 2 already is; in 3 is not routed; in 4 is past the live inputs)
  assert.deepEqual(polarityRoutes(s, 0, true), [0]);
  assert.deepEqual(polarityRoutes(s, 0, false), [1]);
  assert.deepEqual(polarityRoutes(s, 1, false), [0]);
  assert.deepEqual(polarityRoutes(s, 1, true), [2]);
});

// ---------------------------------------------------------------- crossfeed pairs

test("GAP1 crossfeed pairs: Qt's menu words, the map's chips and tips, the K340 question", () => {
  assert.equal(choiceTitle("default"), "Default (1/2, 3/4, …)");
  assert.equal(choiceTitle("k340"), "One DAC per ear (K340)");
  assert.equal(choiceTitle("custom"), "Custom");
  assert.equal(pairChipText(0, { left: 2, right: 0 }), "1: Out 3 / Out 1");
  const s = { platformId: 1, numOutputChannels: 9 } as unknown as DspState;
  const name = (o: number) => `Name ${o}`;
  const k340 = [
    { left: 2, right: 0 },
    { left: 3, right: 1 },
    { left: 4, right: 5 },
    { left: 6, right: 7 },
  ];
  assert.deepEqual(maskBits("pairs", s, name, k340)[0], { bit: 0, short: "OUT3+1", long: "Name 2 and Name 0" });
  assert.deepEqual(maskBits("pairs", s, name)[0], { bit: 0, short: "OUT1+2", long: "Name 0 and Name 1" });
  const view: CrossfeedPairMapView = { supported: true, pairCount: 4, pairs: k340, choice: "k340", k340Mask: 3 };
  assert.equal(offersK340Mask(view, 0x01), true, "the factory mask leaves the tweeters out");
  assert.equal(offersK340Mask(view, 0x03), false);
  assert.equal(offersK340Mask({ ...view, choice: "custom" }, 0x01), false);
});

// ---------------------------------------------------------------- on the real core

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(): Promise<Session> {
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

test("GAP1 curves: the edited channel's phase when asked, Qt's numbers", { skip }, async () => {
  const s = await sessionOn();
  const band = { filterType: 34, bypass: false, freq: 1000, q: 0.707, gain: 0, qp: 0 };
  const plain = await s.core.call("curves", { chains: [[band]] });
  assert.equal(plain.phase, undefined);
  const r = await s.core.call("curves", { chains: [[band]], phase: true, unwrap: true });
  assert.equal(r.phase?.length, 201);
  // An LR4 low pass: about -90° per order-pair at its corner, -180° there
  const grid = (await s.core.call("curveGrid", undefined)).freqs;
  const at = grid.findIndex((f) => f >= 1000);
  assert.ok(Math.abs(r.phase![at]! + 180) < 10, `${r.phase![at]}`);
});

test("GAP1 polarity on the core: every feeding route, one undo step, the snapshot's word", { skip }, async () => {
  const s = await sessionOn();
  const snap = () => s.state.snapshot!;
  assert.equal(polarityOf(snap(), 0), "normal");
  // A second route to Out 1, then the output inverted: both routes
  assert.equal(await s.change("crosspoint", { input: 1, output: 0, enabled: true, gain: -6, invert: false }), true);
  assert.equal(await setOutputPolarity(s, 0, true), true);
  assert.equal(polarityOf(snap(), 0), "inverted");
  assert.equal(snap().state.matrixInvert[0]![0], true);
  assert.equal(snap().state.matrixInvert[1]![0], true);
  assert.equal(snap().state.matrixGain[1]![0], -6, "the gain kept");
  await s.undo();
  assert.equal(polarityOf(snap(), 0), "normal", "one undo step");
  // Nothing feeds the last output: nothing to write
  const last = snap().state.numOutputChannels - 1;
  assert.equal(polarityOf(snap(), last), "none");
  assert.equal(await setOutputPolarity(s, last, true), true);
  assert.equal(polarityOf(snap(), last), "none");
});

test("GAP1 crossfeed pair map on the core: the choice written, read back, the snapshot's map", { skip }, async () => {
  const s = await sessionOn();
  const v = await s.core.call("crossfeedPairMap", undefined);
  assert.equal(v.supported, true);
  assert.equal(v.choice, "default");
  assert.equal(await s.change("setCrossfeedPairMap", { choice: "k340" }), true);
  assert.deepEqual(s.state.snapshot!.state.crossfeedPairMap, [0x02, 0x13, 0, 0]);
  assert.equal((await s.core.call("crossfeedPairMap", undefined)).choice, "k340");
  const pairs = v.pairs.map((p, i) => (i === 0 ? { left: 0, right: 0 } : p));
  assert.equal(await s.core.call("crossfeedPairsValid", { pairs }), false);
  assert.equal(await s.change("setCrossfeedPairMap", { pairs }), false, "refused");
  assert.deepEqual(s.state.snapshot!.state.crossfeedPairMap, [0x02, 0x13, 0, 0]);
});
