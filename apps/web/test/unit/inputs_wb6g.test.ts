// WP WB6g unit tests: the input pages' model (the pair link kept per device,
// the mismatch and its question, the sources' titles and names) and, on the
// real core, a linked pair's trim as one core call and one undo step, linking
// with the side to keep as one undo step, a linked band edit reaching both
// sides, and the source names read from the core's cache.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CoreClient } from "../../src/core/client";
import type { DspState, FilterParams } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import {
  DEFAULT_LINKS,
  LINKS_KEY,
  builtInTitle,
  cleanName,
  inputLinks,
  linkPair,
  loadLinks,
  mismatchQuestion,
  pairAvailable,
  pairLabel,
  pairMismatch,
  saveLinks,
  sourceOptions,
  sourceTitle,
  writeTrim,
} from "../../src/screens/inputs/model";

const band = (p: Partial<FilterParams> = {}): FilterParams => ({ filterType: 0, bypass: false, freq: 1000, q: 0.707, gain: 0, qp: 0.707, ...p });

function state(p: Partial<DspState> = {}): DspState {
  return {
    numInputChannels: 2,
    maxBands: 10,
    filters: [Array.from({ length: 10 }, () => band()), Array.from({ length: 10 }, () => band())],
    inputPreampDb: [0, 0],
    inputSource: 0,
    i2sInputChannels: 2,
    spdifInputsEnabled: 1,
    adatInputEnabled: false,
    adatInputPin: 0xff,
    ...p,
  } as unknown as DspState;
}

test("WB6G links: kept per serial, pair 1/2 linked by default, bad storage ignored", () => {
  const mem = new Map<string, string>();
  const store = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v) };
  assert.equal(loadLinks(store, "A"), DEFAULT_LINKS);
  saveLinks(store, "A", 0b1010);
  saveLinks(store, "B", 0);
  assert.equal(loadLinks(store, "A"), 0b1010);
  assert.equal(loadLinks(store, "B"), 0);
  assert.equal(loadLinks(store, "C"), 1);
  saveLinks(store, "A", 0xff);
  assert.equal(loadLinks(store, "A"), 0xf, "four pairs at most");
  mem.set(LINKS_KEY, "{nonsense");
  assert.equal(loadLinks(store, "A"), 1);
  assert.equal(loadLinks(null, "A"), 1);
  saveLinks(null, "A", 3); // no storage: nothing thrown
});

test("WB6G pairs: available only with both halves; labels; what differs", () => {
  assert.equal(pairAvailable(state(), 0), true);
  assert.equal(pairAvailable(state(), 1), true);
  assert.equal(pairAvailable(state(), 2), false);
  assert.equal(pairAvailable(state({ numInputChannels: 3 }), 2), false, "3 has no partner");
  assert.equal(pairLabel(0), "1/2");
  assert.equal(pairLabel(5), "5/6");
  assert.deepEqual(pairMismatch(state(), 0), { bands: false, preamp: false });
  assert.deepEqual(pairMismatch(state({ inputPreampDb: [0, 0.04] }), 1), { bands: false, preamp: false }, "finer than 0.1 dB is noise");
  assert.deepEqual(pairMismatch(state({ inputPreampDb: [-3, 0] }), 0), { bands: false, preamp: true });
  const s = state();
  s.filters[1]![4] = band({ filterType: 1, gain: 3 });
  assert.deepEqual(pairMismatch(s, 0), { bands: true, preamp: false });
  // The Transform's qp counts only for the Transform.
  const t = state();
  t.filters[0]![0] = band({ qp: 0.5 });
  assert.equal(pairMismatch(t, 0).bands, false);
  t.filters[0]![0] = band({ filterType: 13, qp: 0.5 });
  t.filters[1]![0] = band({ filterType: 13 });
  assert.equal(pairMismatch(t, 0).bands, true);
});

test("WB6G the question: the Console's words, the page's input first, no device nouns", () => {
  const q = mismatchQuestion(1, { bands: true, preamp: true });
  assert.equal(q.title, "Inputs 2 and 1 don't match");
  assert.match(q.text, /^These inputs have different filters and input trims\. Linking mirrors future edits/);
  assert.equal(q.keepThis, "Keep IN2");
  assert.equal(q.keepOther, "Keep IN1");
  assert.match(mismatchQuestion(0, { bands: true, preamp: false }).text, /have different filters\./);
  assert.match(mismatchQuestion(0, { bands: false, preamp: true }).text, /have different input trims\./);
  // Owner rule: no host-device nouns in the slice's words.
  for (const f of ["src/screens/inputs/model.ts", "src/screens/inputs/InputsScreen.tsx"]) {
    const text = readFileSync(f, "utf8");
    assert.doesNotMatch(text, /\b(phone|computer|tablet|laptop|this Mac|your PC)\b/i, f);
  }
});

test("WB6G sources: the menu's order, titles numbered with a second receiver, names clean", () => {
  assert.deepEqual(sourceOptions(state()), [0, 1, 2]);
  assert.deepEqual(sourceOptions(state({ spdifInputsEnabled: 0b0101, adatInputEnabled: true, adatInputPin: 12 } as Partial<DspState>)), [0, 1, 5, 2, 3]);
  assert.deepEqual(sourceOptions(state({ i2sInputChannels: 0, adatInputEnabled: true } as Partial<DspState>)), [0, 1], "ADAT without a pin is not offered");
  assert.equal(builtInTitle(state(), 1), "S/PDIF");
  assert.equal(builtInTitle(state({ spdifInputsEnabled: 3 } as Partial<DspState>), 1), "S/PDIF 1");
  assert.equal(builtInTitle(state(), 4), "S/PDIF 2");
  assert.equal(builtInTitle(state(), 6), "S/PDIF 4");
  assert.equal(builtInTitle(state(), 3), "ADAT");
  assert.equal(sourceTitle(state(), 0, ["Desk", ""]), "Desk");
  assert.equal(sourceTitle(state(), 1, ["Desk", "  "]), "S/PDIF");
  assert.equal(cleanName("  Plattenspieler  "), "Plattenspieler");
  assert.equal(cleanName("Café\u0007 TV"), "Caf TV", "printable ASCII only");
  assert.equal(cleanName("x".repeat(40)).length, 31);
});

// -- On the real core (as outputs.test.ts) --

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

const pre = (s: Session) => s.state.snapshot!.state.inputPreampDb;

test("WB6G M48: a linked pair's trim is both sides and one undo step; unlinked, one side", { skip }, async () => {
  const session = await demoSession();
  const links = inputLinks(session);
  assert.equal(links.isLinked(0), true, "1/2 linked by default");
  assert.equal(links.partner(0), 1);
  const before = pre(session).slice(0, 2);
  assert.equal(await writeTrim(session, 0, -6), true, session.state.error);
  assert.deepEqual(pre(session).slice(0, 2), [-6, -6]);
  await session.undo();
  assert.deepEqual(pre(session).slice(0, 2), before, "one step puts both back");
  links.set(1, false);
  assert.equal(links.partner(0), -1);
  assert.equal(await writeTrim(session, 1, -2.5), true);
  assert.deepEqual(pre(session).slice(0, 2), [before[0], -2.5]);
});

test("WB6G M48: linking a pair that differs keeps the chosen side, one undo step; a linked band reaches both sides", { skip }, async () => {
  const session = await demoSession();
  const links = inputLinks(session);
  links.set(0, false);
  const was = { band: session.state.snapshot!.state.filters[0]![2]!, db: pre(session)[0] };
  const bell = { filterType: 1, bypass: false, freq: 250, q: 1.2, gain: 4, qp: 0.707 };
  assert.equal(await session.change("filter", { channel: 1, band: 2, ...bell }), true);
  assert.equal(await session.write("pre", { float: -3 }, [1]), true);
  const s = session.state.snapshot!.state;
  assert.deepEqual(pairMismatch(s, 0), { bands: true, preamp: true });
  // Keep IN2: its band and trim go onto IN1.
  assert.equal(await linkPair(session, 0, 1), true, session.state.error);
  assert.equal(links.isLinked(0), true);
  const t = session.state.snapshot!.state;
  assert.equal(t.filters[0]![2]!.filterType, 1);
  assert.equal(t.filters[0]![2]!.freq, 250);
  assert.equal(t.inputPreampDb[0], -3);
  assert.deepEqual(pairMismatch(t, 0), { bands: false, preamp: false });
  // One undo step takes the copy back (IN1 as it was), IN2 untouched.
  await session.undo();
  const u = session.state.snapshot!.state;
  assert.deepEqual(u.filters[0]![2], was.band);
  assert.equal(u.inputPreampDb[0], was.db);
  assert.equal(u.inputPreampDb[1], -3);
  // A linked band edit: the core's pair call, both sides.
  assert.equal(await session.change("inputPairFilter", { input: 1, band: 5, ...bell, gain: -2 }), true);
  const v = session.state.snapshot!.state;
  assert.equal(v.filters[0]![5]!.gain, -2);
  assert.equal(v.filters[1]![5]!.gain, -2);
});

test("WB6G source names: the core's cache, renamed by in.name, cleared by an empty name", { skip }, async () => {
  const session = await demoSession();
  const r = await session.core.call("inputSourceNames", undefined);
  assert.equal(r.supported, true);
  assert.ok(r.names.length >= 7);
  assert.equal(await session.write("in.name", { text: "Turntable" }, [1]), true, session.state.error);
  assert.equal((await session.core.call("inputSourceNames", undefined)).names[1], "Turntable");
  assert.equal(sourceTitle(session.state.snapshot!.state, 1, (await session.core.call("inputSourceNames", undefined)).names), "Turntable");
  assert.equal(await session.write("in.name", { text: "" }, [1]), true);
  assert.equal((await session.core.call("inputSourceNames", undefined)).names[1], "");
});
