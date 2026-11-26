// WP ROLEW unit tests: the speaker layout on the web (MPLAN 2.2). The page's copy per serial
// (checked, a refused storage said in words), the banner's three cases, the rows' edits, the
// Measure rows pre-filled from the layout, and the refusal matched by its key's prefix (review
// RA); then on the real core (the demo): the proposal shown, Confirm kept, the copy set again on a
// fresh core (the page reopened) and by the session's open, an imported layout unconfirmed with
// its banner (review RA F1), a copy that cannot be set again said, the Measure write-back keeping
// the layout's confirmation, and a measurement from an unconfirmed layout refused in the notice's
// words.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  KEEP_REFUSED,
  LAYOUT_UNCONFIRMED,
  NO_VIEW,
  bannerOf,
  baseOf,
  isLayoutUnconfirmed,
  keepLayout,
  loadKept,
  loadLayout,
  saveKept,
  stampWords,
  withRow,
  withoutRow,
  type LayoutCaller,
  type LayoutView,
  type SpeakerLayoutRecord,
} from "../../src/screens/settings/speakers/model";
import { initialDraft, layoutKey, setupOf, withLayout } from "../../src/screens/measure/model";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { CoreCallError, IMPORT_BY_BLOCK, IMPORT_CROSSOVER } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { loadNotices, noticeOf, resetNotices } from "../../src/ui/texts";

function memory() {
  const m = new Map<string, string>();
  return { map: m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

const layout = (outputs: [number, SpeakerLayoutRecord["outputs"][number]["role"], string, number][], confirmed = true, updatedMs = 1_700_000_000_000): SpeakerLayoutRecord => ({
  serial: "S1",
  outputs: outputs.map(([output, role, speaker, group]) => ({ output, role, speaker, group })),
  confirmedByUser: confirmed,
  updatedMs,
  notes: [],
});

test("ROLEW the page's copy: per serial, checked, a refused storage said in words", () => {
  const s = memory();
  assert.equal(loadKept(s, "S1"), null);
  assert.equal(saveKept(s, "S1", { layout: layout([[0, "low", "L", 0], [1, "high", "L", 0]]), imported: false }), "");
  assert.equal(saveKept(s, "S2", { layout: layout([[0, "full", "R", 1]], false), imported: true }), "");
  const k = loadKept(s, "S1")!;
  assert.deepEqual(k.layout.outputs.map((o) => o.role), ["low", "high"]);
  assert.equal(k.layout.confirmedByUser, true);
  assert.equal(k.imported, false);
  assert.equal(loadKept(s, "S2")!.imported, true, "each serial its own");
  assert.equal(loadKept(s, ""), null);
  // A copy that does not read is none, never a throw.
  s.map.set("dspi.speakerLayouts", JSON.stringify({ S1: { layout: { outputs: [{ output: 0, role: "tweeter" }] } }, S3: "x" }));
  assert.equal(loadKept(s, "S1"), null, "an unknown role");
  assert.equal(loadKept(s, "S3"), null);
  s.map.set("dspi.speakerLayouts", "{not json");
  assert.equal(loadKept(s, "S1"), null);
  // Storage refused (a private window, full): said, never thrown.
  const refusing = { getItem: () => null, setItem: () => { throw new Error("QuotaExceededError"); } };
  assert.equal(saveKept(refusing, "S1", { layout: layout([]), imported: false }), KEEP_REFUSED);
  assert.equal(saveKept(null, "S1", { layout: layout([]), imported: false }), KEEP_REFUSED);
});

test("ROLEW the banner: imported, not confirmed, none; nothing once confirmed", () => {
  const v = (stored: SpeakerLayoutRecord | null, imported = false): LayoutView => ({ ...NO_VIEW, stored, imported });
  assert.equal(bannerOf(v(layout([[0, "full", "L", 0]]))), null);
  assert.equal(bannerOf(v(layout([[0, "full", "L", 0]], false), true))!.kind, "imported");
  assert.match(bannerOf(v(layout([[0, "full", "L", 0]], false), true))!.text, /imported.*Settings › Speakers/is);
  assert.equal(bannerOf(v(layout([[0, "full", "L", 0]], false)))!.kind, "unconfirmed");
  assert.equal(bannerOf(v(null))!.kind, "none");
  assert.doesNotMatch(bannerOf(v(null))!.here, /Settings/, "on the page itself it does not send the person there");
  assert.equal(stampWords(null), "");
  assert.match(stampWords(layout([], true, Date.now())), /^Confirmed today at /);
});

test("ROLEW the rows: a role changed or added in output order, an output left out", () => {
  const l = layout([[0, "low", "L", 0], [2, "high", "L", 0]]);
  const a = withRow(l, 1, { role: "mid" });
  assert.deepEqual(a.outputs.map((o) => [o.output, o.role]), [[0, "low"], [1, "mid"], [2, "high"]]);
  assert.equal(withRow(a, 2, { speaker: "R" }).outputs[2]!.role, "high", "the rest of the row kept");
  assert.deepEqual(withoutRow(a, 0).outputs.map((o) => o.output), [1, 2]);
  // -- WP ADETW -- a row the person adds is theirs
  assert.deepEqual(withRow(null, 3, { role: "sub" }).outputs, [{ output: 3, role: "sub", speaker: "", group: 0, source: "user" }]);
  assert.equal(l.outputs.length, 2, "never changed in place");
});

test("ROLEW Measure: the rows pre-filled from the layout once per layout; the chosen outputs kept", () => {
  const d = initialDraft([0, 1, 2]);
  const l = layout([[0, "low", "L", 0], [1, "high", "L", 0]]);
  const p = withLayout(d, l);
  assert.deepEqual(p.outputs.map((o) => o.role), ["low", "high", "full"]);
  assert.deepEqual(p.outputs.map((o) => o.on), d.outputs.map((o) => o.on));
  assert.equal(p.layoutFrom, layoutKey(l));
  // The same layout again changes nothing (the person's edits stay); a newer one is applied.
  const edited = { ...p, outputs: p.outputs.map((o) => (o.output === 0 ? { ...o, role: "sub" as const } : o)) };
  assert.equal(withLayout(edited, l), edited);
  assert.equal(withLayout(edited, { ...l, updatedMs: l.updatedMs + 1 }).outputs[0]!.role, "low");
  assert.equal(withLayout(d, null), d);
  assert.equal(layoutKey({ updatedMs: 0 }), "proposal");
  // The setup sends the rows' roles (the core still judges the tweeter rule).
  assert.deepEqual(setupOf({ ...p, placement: "listeningPosition" }, 48_000).outputs.map((o) => o.role), ["low", "high"]);
});

test("ROLEW the refusal for want of a confirmed layout: its key's prefix, not the whole message", () => {
  assert.ok(isLayoutUnconfirmed(new Error("invalid argument: measure.layout_unconfirmed: the speaker layout is not confirmed")));
  assert.ok(isLayoutUnconfirmed(new Error("measure.layout_unconfirmed: other words, later")));
  assert.ok(isLayoutUnconfirmed("measure.layout_unconfirmed:"));
  assert.ok(!isLayoutUnconfirmed(new Error("measure.layout_unconfirmed")), "the key alone is not the refusal");
  assert.ok(!isLayoutUnconfirmed(new Error("invalid argument: see measure.layout_unconfirmed: x")));
  assert.ok(!isLayoutUnconfirmed(null));
  assert.equal(LAYOUT_UNCONFIRMED, "measure.layout_unconfirmed");
});

// ------------------------------------------------------------------ on the real core (demo device)

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demo(): Promise<{ core: WasmCore; serial: string }> {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const snap = await core.call("open", { kind: "demo", blank: true }); // -- WP UB12 -- a new board
  return { core, serial: snap.device.serial };
}

const caller = (core: WasmCore) => core as unknown as LayoutCaller;

test("ROLEW on the demo: the proposal shown, Confirm kept, the copy set again on a fresh core", { skip, timeout: 60_000 }, async () => {
  const store = memory();
  const a = await demo();
  assert.ok(a.serial, "the demo has a serial");
  const v0 = await loadLayout(caller(a.core), a.serial, store);
  assert.equal(v0.stored, null);
  assert.ok(v0.proposal && v0.proposal.outputs.length > 0, "a proposal from the filters");
  assert.equal(v0.proposal!.confirmedByUser, false);
  assert.equal(bannerOf(v0)!.kind, "none");
  assert.equal(store.map.size, 0, "a proposal is never kept by itself");
  const draft = withRow(baseOf(v0), v0.proposal!.outputs[0]!.output, { role: "sub", speaker: "Sub" });
  const v1 = await keepLayout(caller(a.core), a.serial, store, draft, true, v0);
  assert.equal(v1.stored!.confirmedByUser, true);
  assert.ok(v1.stored!.updatedMs > 0);
  assert.equal(bannerOf(v1), null);
  assert.equal(v1.problem, "");
  assert.equal(loadKept(store, a.serial)!.layout.outputs[0]!.role, "sub", "the page's copy");
  // The page reopened: a fresh core has none, and the copy is set again, still confirmed.
  const b = await demo();
  assert.equal(await b.core.call("speakerLayout" as never, undefined as never), null);
  const v2 = await loadLayout(caller(b.core), b.serial, store);
  assert.equal(v2.stored!.confirmedByUser, true);
  assert.equal(v2.stored!.outputs[0]!.role, "sub");
  assert.equal(v2.imported, false);
  assert.equal(bannerOf(v2), null);
  assert.equal(((await b.core.call("speakerLayout" as never, undefined as never)) as SpeakerLayoutRecord | null)!.confirmedByUser, true, "the core has it now");
});

test("ROLEW on the demo: an imported layout is unconfirmed with its banner until confirmed (review RA F1)", { skip, timeout: 60_000 }, async () => {
  const store = memory();
  const a = await demo();
  const v0 = await loadLayout(caller(a.core), a.serial, store);
  await keepLayout(caller(a.core), a.serial, store, withRow(baseOf(v0), 0, { role: "high" }), true, v0);
  const { text } = await a.core.call("configExport", { name: "Friend's", appVersion: "test" });
  assert.equal((JSON.parse(text) as { meta: { speakerLayout: { confirmedByUser: boolean } } }).meta.speakerLayout.confirmedByUser, true, "the file says confirmed");
  // Another browser (its own storage) with its own confirmed layout imports the file.
  const mine = memory();
  const b = await demo();
  const w0 = await loadLayout(caller(b.core), b.serial, mine);
  await keepLayout(caller(b.core), b.serial, mine, withRow(baseOf(w0), 0, { role: "full" }), true, w0);
  const token = await b.core.call("confirmToken", { op: "config_import" });
  const r = (await b.core.call("configImport", { text, options: IMPORT_BY_BLOCK | IMPORT_CROSSOVER, token })) as unknown as { speakerLayout?: boolean };
  assert.equal(r.speakerLayout, true, "the layout came with the crossover");
  const w1 = await loadLayout(caller(b.core), b.serial, mine);
  assert.equal(w1.stored!.confirmedByUser, false, "never confirmed by a file");
  assert.equal(w1.stored!.outputs[0]!.role, "high", "the file's roles, to check");
  assert.equal(w1.imported, true);
  assert.equal(bannerOf(w1)!.kind, "imported");
  // Read again (the page reloaded with the core still open): still the import's banner.
  assert.equal((await loadLayout(caller(b.core), b.serial, mine)).imported, true);
  // A Measure edit keeps it unconfirmed (and imported); Confirm ends both.
  const w2 = await keepLayout(caller(b.core), b.serial, mine, withRow(w1.stored, 1, { role: "low" }), !!w1.stored!.confirmedByUser, w1);
  assert.equal(w2.stored!.confirmedByUser, false);
  assert.equal(bannerOf(w2)!.kind, "imported");
  const w3 = await keepLayout(caller(b.core), b.serial, mine, w2.stored!, true, w2);
  assert.equal(bannerOf(w3), null);
  assert.equal(loadKept(mine, b.serial)!.imported, false);
});

test("ROLEW on the demo: a copy that cannot be set again, and a storage that refuses, are said", { skip, timeout: 60_000 }, async () => {
  const a = await demo();
  const store = memory();
  saveKept(store, a.serial, { layout: layout([[15, "high", "L", 0]]), imported: false });
  const v = await loadLayout(caller(a.core), a.serial, store);
  assert.equal(v.stored, null);
  assert.match(v.problem, /^Your saved speaker layout could not be set again: .*output 16/);
  assert.equal(bannerOf(v)!.kind, "none", "the suggestion is shown instead");
  const refusing = { getItem: () => null, setItem: () => { throw new Error("QuotaExceededError"); } };
  const k = await keepLayout(caller(a.core), a.serial, refusing, baseOf(v)!, true, v);
  assert.equal(k.stored!.confirmedByUser, true, "the core has it for this visit");
  assert.equal(k.problem, KEEP_REFUSED);
});

test("ROLEW on the demo: the session's open sets the page's copy again", { skip, timeout: 60_000 }, async () => {
  const g = globalThis as { localStorage?: unknown };
  const saved = g.localStorage;
  const store = memory();
  g.localStorage = store;
  try {
    const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
    made.push(core);
    const serial = (await core.call("open", { kind: "demo", blank: true })).device.serial; // -- WP UB12 -- a new board
    await core.call("close" as never, undefined as never);
    saveKept(store, serial, { layout: layout([[0, "mid", "C", 2]]), imported: false });
    const client = { call: (m: string, p: unknown) => core.call(m as never, p as never), subscribe: () => () => undefined, dispose: async () => undefined } as unknown as CoreClient;
    const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
    await session.open({ kind: "demo", blank: true }); // -- WP UB12 -- a new board
    assert.equal(session.state.status, "open");
    const l = (await core.call("speakerLayout" as never, undefined as never)) as SpeakerLayoutRecord | null;
    assert.equal(l?.confirmedByUser, true);
    assert.deepEqual(l?.outputs.map((o) => [o.output, o.role, o.speaker, o.group]), [[0, "mid", "C", 2]]);
  } finally {
    g.localStorage = saved;
  }
});

test("ROLEW on the demo: a measurement from an unconfirmed layout is refused in the notice's words", { skip, timeout: 60_000 }, async () => {
  resetNotices();
  const a = await demo();
  await loadNotices(a.core as unknown as CoreClient);
  const v = await loadLayout(caller(a.core), a.serial, memory());
  await keepLayout(caller(a.core), a.serial, memory(), baseOf(v)!, false, v);
  const setup = { ...setupOf({ ...withLayout(initialDraft([0, 1]), baseOf(v)), placement: "listeningPosition" }, 48_000), rolesFromLayout: true };
  let err: unknown = null;
  try {
    await a.core.call("measurePreflight" as never, { setup } as never);
  } catch (e) {
    err = e;
  }
  assert.ok(err instanceof CoreCallError, String(err));
  assert.equal(err.noticeKey, "invalid", "the core's key is the generic one");
  assert.ok(isLayoutUnconfirmed(err));
  const n = noticeOf(err);
  assert.equal(n?.key, LAYOUT_UNCONFIRMED, "matched by the prefix");
  assert.equal(n?.title, "Speaker layout not confirmed");
  assert.match(n!.detail, /Settings › Speakers/);
  assert.doesNotMatch(n!.detail, /ask the user/, "the agent's words are not the person's");
  // -- WP RCFIX -- (review RC, RC14) pink noise on a tweeter, in its notice's words too.
  let pink: unknown = null;
  try {
    await a.core.call("measurePreflight" as never, { setup: { ...setup, kind: "pink", outputs: [{ ...setup.outputs[0]!, role: "high" }], rolesFromLayout: false } } as never);
  } catch (e) {
    pink = e;
  }
  assert.match(String(pink), /measure\.tweeter_stimulus: /);
  const t = noticeOf(pink);
  assert.equal(t?.key, "measure.tweeter_stimulus");
  assert.match(t!.detail, /tweeter is never played below 1 kHz/);
  assert.doesNotMatch(t!.detail, /invalid argument|measure\./);
  // Another refusal keeps its own notice.
  assert.notEqual(noticeOf(new CoreCallError("invalid", "invalid argument: measure.tune.old: x", "invalid"))?.key, LAYOUT_UNCONFIRMED);
});
