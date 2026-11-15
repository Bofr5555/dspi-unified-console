// WP WB6l unit tests: the control-surface editor's records (which a page lists,
// what is changed, what Apply and Remove send), the shortcut table (which key
// opens what, the platform's words), and on the real core: the editor's desk
// reading the demo device, editing a row through the core's rules, applying,
// saving and reverting, a remote button learned, the quick strip's switches by
// their registry paths, and nothing written while read only.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CoreClient } from "../../src/core/client";
import type { CsRecord } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { CsDesk } from "../../src/screens/surfaces/desk";
import {
  TY,
  addableTypes,
  applyOps,
  codeText,
  delayText,
  inUse,
  isDirty,
  keyOf,
  keysFor,
  pagesOf,
  removeOps,
  same,
  syncRecords,
} from "../../src/screens/surfaces/model";
import { SHORTCUTS, keyText, listed, shortcutFor, toolRow } from "../../src/screens/shortcuts/table";

// -- the model, on a made-up snapshot --

function snapOf(p: Partial<CsRecord> = {}): CsRecord {
  const empty = { type: 0 };
  return {
    supported: true,
    capsVersion: 20,
    maxBindings: 4,
    types: Array.from({ length: 11 }, () => ({ actions: 0, pins: 1, pinClass: 0 })),
    maxIr: 2,
    maxGroups: 2,
    maxMacros: 2,
    maxSteps: 4,
    status: { dirty: false, activeMask: 0b0011, slotStatus: [0, 0, 0, 0] },
    bindings: [{ type: TY.Button, gpio0: 16 }, { type: TY.AuxOut, gpio0: 27 }, empty, empty],
    names: ["Mute", "", "", ""],
    ir: [
      { protocol: 1, code: 0x00ff00ff, noun: 3 },
      { protocol: 0, code: 0, noun: 0 },
    ],
    groups: [
      { targetKind: 2, memberMask: 3, name: "Mains" },
      { targetKind: 0, memberMask: 0, name: "" },
    ],
    macros: [
      { name: "", steps: [] },
      { name: "Night", steps: [{ noun: 1 }] },
    ],
    ext: { macroRunning: 0xff },
    ...p,
  };
}

test("WB6L records: synced from the snapshot; a draft being edited stays, one untouched follows the device", () => {
  const snap = snapOf();
  let recs = syncRecords(snap, new Map());
  assert.equal(recs.size, 4 + 2 + 2 + 2);
  assert.deepEqual(keysFor("controls", snap, recs), ["binding/0"], "aux outputs are on their own page");
  assert.deepEqual(keysFor("aux", snap, recs), ["binding/1"]);
  assert.deepEqual(keysFor("remotes", snap, recs), ["ir/0"]);
  assert.deepEqual(keysFor("groups", snap, recs), ["group/0"]);
  assert.deepEqual(keysFor("macros", snap, recs), ["macro/1"]);
  // An edit to binding 0, then the device changes binding 0 and 2.
  const r0 = recs.get("binding/0")!;
  recs.set("binding/0", { ...r0, draft: { ...r0.draft, gpio0: 17 } });
  const next = snapOf({ bindings: [{ type: TY.Button, gpio0: 18 }, { type: TY.AuxOut, gpio0: 27 }, { type: TY.Led, gpio0: 5 }, { type: 0 }] });
  recs = syncRecords(next, recs);
  assert.equal(recs.get("binding/0")!.draft.gpio0, 17, "the edit stays");
  assert.equal(recs.get("binding/0")!.live.gpio0, 18);
  assert.equal(recs.get("binding/2")!.draft.type, TY.Led, "untouched follows");
  assert.equal(isDirty(recs.get("binding/0")!, recs), true);
  assert.equal(isDirty(recs.get("binding/2")!, recs), false);
});

test("WB6L Apply sends the binding and its name; a receiver's Apply sends its changed remote buttons too", () => {
  const snap = snapOf({ bindings: [{ type: TY.Ir, gpio0: 15 }, { type: 0 }, { type: 0 }, { type: 0 }] });
  const recs = syncRecords(snap, new Map());
  const rx = recs.get("binding/0")!;
  recs.set(rx.key, { ...rx, nameEdit: "Remote" });
  const ir1 = recs.get("ir/1")!;
  recs.set(ir1.key, { ...ir1, draft: { ...ir1.draft, protocol: 1, code: 5 } });
  const { ops, irKeys } = applyOps(recs.get("binding/0")!, recs);
  assert.deepEqual(
    ops.map((o) => o.op),
    ["name", "ir"],
  );
  assert.deepEqual(irKeys, ["ir/1"]);
  // A remote button with no code learned has nothing to apply.
  const ir0 = { ...recs.get("ir/0")!, live: { protocol: 0, code: 0 }, draft: { protocol: 0, code: 0, noun: 4 } };
  recs.set(ir0.key, ir0);
  assert.equal(isDirty(ir0, recs), false);
});

test("WB6L Remove clears what the device holds and only that", () => {
  const snap = snapOf();
  const recs = syncRecords(snap, new Map());
  assert.deepEqual(removeOps(recs.get("binding/0")!, snap), [
    { op: "binding", index: 0, binding: {} },
    { op: "name", index: 0, name: "" },
  ]);
  assert.deepEqual(removeOps(recs.get("group/0")!, snap), [{ op: "group", index: 0, group: {} }]);
  const drafted = { ...recs.get("group/1")!, draft: { targetKind: 2, memberMask: 0, name: "Group 2" } };
  assert.equal(inUse(drafted), true);
  assert.deepEqual(removeOps(drafted, snap), [], "a draft never applied is only dropped");
});

test("WB6L Add offers one IR receiver and one display, aux types on the aux page; pages by what the firmware has", () => {
  const snap = snapOf({ bindings: [{ type: TY.Ir }, { type: TY.Display }, { type: 0 }, { type: 0 }] });
  const recs = syncRecords(snap, new Map());
  const controls = addableTypes("controls", snap, recs).map((t) => t.type);
  assert.ok(!controls.includes(TY.Ir) && !controls.includes(TY.Display));
  assert.ok(controls.includes(TY.Button) && !controls.includes(TY.AuxOut));
  assert.deepEqual(
    addableTypes("aux", snap, recs).map((t) => t.type),
    [TY.AuxOut, TY.AuxPwm],
  );
  assert.deepEqual(pagesOf(snap), ["controls", "groups", "macros", "aux"]);
  assert.deepEqual(pagesOf(snapOf({ maxGroups: 0, groups: [], capsVersion: 12 })), ["controls", "macros"]);
  assert.deepEqual(pagesOf({ supported: false }), ["controls"]);
});

test("WB6L words: a learned code, a delay, key-order-blind equality", () => {
  assert.equal(codeText({ protocol: 1, code: 0x00ff00ff }), "NEC 0x00FF00FF");
  assert.equal(codeText({ protocol: 0 }), "Not learned");
  assert.equal(delayText(2.5), "2.5 s");
  assert.equal(delayText(90), "1.5 min");
  assert.equal(same({ a: 1, b: [1, { c: 2 }] }, { b: [1, { c: 2 }], a: 1 }), true);
  assert.equal(same({ a: 1 }, { a: 1, b: 0 }), false);
});

// -- the shortcut table --

const ev = (key: string, mods: { ctrl?: boolean; meta?: boolean; shift?: boolean; alt?: boolean } = {}, code = "") => ({
  key,
  code,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
});

test("WB6L M24: the Tools keys open their screens, Cmd on a Mac, by the key or its place", () => {
  assert.equal(shortcutFor(ev("M", { ctrl: true, shift: true }), false)?.open?.screen, "matrix");
  assert.equal(shortcutFor(ev("m", { meta: true, shift: true }), true)?.open?.screen, "matrix");
  assert.equal(shortcutFor(ev("m", { ctrl: true, shift: true }), true), null, "Ctrl is not Cmd on a Mac");
  assert.equal(shortcutFor(ev("m", { ctrl: true }), false), null, "Ctrl+M is Mute (ui/keys.ts)");
  assert.equal(shortcutFor(ev("M", { ctrl: true, shift: true, alt: true }), false), null);
  assert.deepEqual(shortcutFor(ev("X", { ctrl: true, shift: true }), false)?.open, { screen: "effects", at: { effect: "crossfeed" } });
  // A layout that gives another letter: the key's place decides.
  assert.equal(shortcutFor(ev("Ж", { ctrl: true, shift: true }, "KeyL"), false)?.id, "tools.loudness");
  assert.equal(shortcutFor(ev(",", { ctrl: true }), false)?.open?.screen, "settings");
  assert.equal(shortcutFor(ev("o", { ctrl: true }), false)?.open?.screen, "files");
  assert.equal(shortcutFor(ev("?", { shift: true }), false)?.help, true);
  assert.equal(shortcutFor(ev("z", { ctrl: true }), false), null, "undo is undo.tsx's");
  assert.equal(shortcutFor(ev("s", { ctrl: true }), false), null, "Commit is keys.ts's");
  // Every key the table acts on is one key, not two rows'.
  const acted = SHORTCUTS.filter((r) => r.open || r.help).map((r) => r.key);
  assert.equal(new Set(acted).size, acted.length);
});

test("WB6L M24: the list shows a console's own rows; keys in the platform's words", () => {
  const groups = listed(new Set(["main", "eq", "matrix"]));
  const ids = groups.flatMap((g) => g.rows.map((r) => r.id));
  assert.ok(ids.includes("tools.matrix") && ids.includes("device.commit") && ids.includes("graph.move"));
  assert.ok(!ids.includes("tools.crossfeed"), "no effects screen, no row");
  assert.ok(!listed(new Set(["main"])).some((g) => g.menu === "EQ graph"));
  assert.equal(keyText("Ctrl+Shift+M", false), "Ctrl+Shift+M");
  assert.equal(keyText("Ctrl+Shift+M", true), "⇧⌘M");
  assert.equal(toolRow("psybass")?.key, "Ctrl+Shift+P");
});

// -- on the real core --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
const desks: CsDesk[] = [];
after(() => {
  for (const d of desks) d.dispose();
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

async function deskOf(session: Session) {
  const d = new CsDesk(session);
  desks.push(d);
  await d.fetch();
  assert.equal(d.loaded, true, d.loadError);
  return d;
}

const until = async (ok: () => boolean, ms = 3000) => {
  const end = Date.now() + ms;
  while (!ok()) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 20));
  }
};

test("WB6L CG20: the demo's controls, a row changed by the core's rules, applied, saved and reverted", { skip }, async () => {
  const session = await demoSession();
  const desk = await deskOf(session);
  const snap = desk.snap!;
  assert.ok(keysFor("controls", snap, desk.recs).length >= 4, "the demo's encoder, button, pot, receiver, display");
  const enc = desk.rec(keyOf("binding", 0))!;
  assert.equal(enc.description?.ok, true);
  // The encoder's noun, to another one the core offers.
  const ed = enc.description!;
  const noun = ed.ok ? ed.rows.find((r) => r.id === "noun") : null;
  assert.ok(noun?.options?.length);
  const other = noun!.options!.find((o) => o.value !== noun!.value && !o.disabled)!;
  await desk.edit(enc.key, "noun", other.value);
  assert.equal(desk.rec(enc.key)!.draft.noun, other.value);
  assert.equal(isDirty(desk.rec(enc.key)!, desk.recs), true);
  assert.equal(desk.rec(enc.key)!.live.noun, noun!.value, "nothing sent before Apply");
  await desk.apply(enc.key);
  assert.equal(desk.rec(enc.key)!.message, "", desk.rec(enc.key)!.message);
  assert.equal(desk.rec(enc.key)!.live.noun, other.value, "the device holds it");
  assert.equal(desk.snap!.status && (desk.snap!.status as CsRecord).dirty, true, "live, not saved");
  // Revert puts the stored one back.
  assert.equal(await desk.revert(), "");
  assert.equal(desk.rec(enc.key)!.live.noun, noun!.value);
  assert.equal((desk.snap!.status as CsRecord).dirty, false);
  // A new push button, applied at once; then saved to flash.
  const key = await desk.addControl(TY.Button);
  assert.ok(key);
  assert.equal(desk.rec(key!)!.live.type, TY.Button, desk.rec(key!)!.message);
  assert.equal(await desk.save(), "");
  assert.equal((desk.snap!.status as CsRecord).dirty, false);
  await desk.remove(key!);
  assert.equal(desk.rec(key!)!.live.type, TY.None);
});

test("WB6L CG20: a group needs members before Apply; a macro's steps; a remote button learned on the demo", { skip }, async () => {
  const session = await demoSession();
  const desk = await deskOf(session);
  const g = desk.addGroup()!;
  await until(() => desk.rec(g)!.description !== null);
  const gd = desk.rec(g)!.description!;
  const member = gd.ok ? gd.rows.find((r) => r.id.startsWith("member.")) : null;
  assert.ok(member, "the core offers members");
  await desk.edit(g, member!.id, true);
  await desk.apply(g);
  assert.equal(desk.rec(g)!.message, "");
  assert.notEqual((desk.rec(g)!.live.memberMask as number) ?? 0, 0);

  const m = desk.addMacro()!;
  await desk.addStep(m);
  await desk.addStep(m);
  assert.equal((desk.rec(m)!.draft.steps as unknown[]).length, 2);
  desk.removeStep(m, 1);
  await desk.apply(m);
  assert.equal(desk.rec(m)!.message, "");
  assert.equal((desk.rec(m)!.live.steps as unknown[]).length, 1);

  const ir = await desk.addRemoteButton();
  assert.ok(ir);
  const sub = desk.rec(ir!)!.slot;
  const learning = desk.startLearn(sub);
  await learning;
  assert.equal(desk.learning, sub);
  assert.equal(await session.core.call("csDemoLearn", { protocol: 1, code: 0x20df10ef }), true);
  await until(() => desk.learning < 0);
  assert.equal(desk.rec(ir!)!.draft.code, 0x20df10ef, desk.rec(ir!)!.message);
  assert.match(desk.rec(ir!)!.message, /Learned a NEC code/);
  const rx = keysFor("controls", desk.snap!, desk.recs).find((k) => desk.rec(k)!.live.type === TY.Ir)!;
  await desk.apply(rx);
  assert.equal(desk.rec(ir!)!.live.code, 0x20df10ef, desk.rec(rx)!.message);
});

test("WB6L read only: everything reads, nothing is written, and the core says why", { skip }, async () => {
  const session = await demoSession("view");
  const desk = await deskOf(session);
  assert.equal(desk.writable, false);
  const enc = desk.rec(keyOf("binding", 0))!;
  const before = enc.live;
  assert.equal(await desk.addControl(TY.Button), null);
  // Past the page: the core refuses before anything is sent.
  const r = await session.core.call("csApply", { op: { op: "binding", index: 0, binding: { ...before, gpio0: 20 } } });
  assert.equal(r.ok, false);
  await desk.fetch();
  assert.deepEqual(desk.rec(enc.key)!.live, before);
  // The strip's switch through write is refused too.
  const strip = await session.core.call("quickStrip", undefined);
  const cf = strip.find((x) => x.id === "crossfeed")!;
  assert.equal(await session.write(cf.path, { bool: !cf.on }), false);
});

test("WB6L M60: the quick strip names the registry paths the device has and follows a switch", { skip }, async () => {
  const session = await demoSession();
  const strip = await session.core.call("quickStrip", undefined);
  assert.deepEqual(
    strip.map((r) => [r.id, r.path]),
    [
      ["crossfeed", "cf.on"],
      ["loudness", "loud.on"],
      ["leveller", "lev.on"],
      ["psybass", "bass.on"],
      ["bypass", "bypass"],
    ],
  );
  assert.ok(strip.every((r) => r.present));
  const lev = strip.find((r) => r.id === "leveller")!;
  assert.equal(await session.write(lev.path, { bool: !lev.on }), true, session.state.error);
  const after = await session.core.call("quickStrip", undefined);
  assert.equal(after.find((r) => r.id === "leveller")!.on, !lev.on);
  assert.equal(session.state.snapshot!.state.levellerEnabled, !lev.on);
});
