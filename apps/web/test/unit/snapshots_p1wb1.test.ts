// WP P1WB1 unit tests: the keys from Qt's table, the desk's small rules, and on the real core
// (as main.test.ts) snapshots, A/B, safe apply with rollback, the import preview and import,
// and an offline document.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { consoleKey } from "../../src/ui/keys";
import {
  SnapshotDesk,
  docMeta,
  fileNameFor,
  importOptions,
  mayToggle,
  secondsLeft,
  type DeskState,
} from "../../src/screens/snapshots/model";
import {
  IMPORT_ALL_BLOCKS,
  IMPORT_BY_BLOCK,
  IMPORT_EQ,
  IMPORT_VOLUMES,
  type GuardedApplyRecord,
} from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";

const key = (k: string, o: { ctrl?: boolean; meta?: boolean; shift?: boolean; alt?: boolean; code?: string } = {}) => ({
  key: k,
  code: o.code ?? "",
  ctrlKey: !!o.ctrl,
  metaKey: !!o.meta,
  shiftKey: !!o.shift,
  altKey: !!o.alt,
});

test("P1WB1: the keys are Qt's table's, Ctrl (Cmd on a Mac), none bare or with Alt", () => {
  assert.deepEqual(consoleKey(key("1", { ctrl: true }), false), { action: "preset", slot: 0 });
  assert.deepEqual(consoleKey(key("9", { ctrl: true }), false), { action: "preset", slot: 8 });
  assert.deepEqual(consoleKey(key("&", { ctrl: true, code: "Digit1" }), false), { action: "preset", slot: 0 }, "a layout that shifts digits");
  assert.equal(consoleKey(key("0", { ctrl: true }), false), null);
  assert.deepEqual(consoleKey(key("s", { ctrl: true }), false), { action: "commit" });
  assert.deepEqual(consoleKey(key("R", { ctrl: true, shift: true }), false), { action: "revert" });
  assert.equal(consoleKey(key("r", { ctrl: true }), false), null, "Ctrl+R stays the browser's reload");
  assert.deepEqual(consoleKey(key("m", { ctrl: true }), false), { action: "mute" });
  assert.deepEqual(consoleKey(key("b", { ctrl: true }), false), { action: "bypass" });
  assert.deepEqual(consoleKey(key("k", { ctrl: true }), false), { action: "ab" });
  assert.equal(consoleKey(key("k"), false), null);
  assert.equal(consoleKey(key("k", { ctrl: true, alt: true }), false), null);
  assert.equal(consoleKey(key("z", { ctrl: true }), false), null, "undo is undo.tsx's");
  // A Mac: Cmd, and Ctrl is not it.
  assert.deepEqual(consoleKey(key("k", { meta: true }), true), { action: "ab" });
  assert.equal(consoleKey(key("k", { ctrl: true }), true), null);
  assert.equal(consoleKey(key("k", { ctrl: true, meta: true }), false), null);
});

test("P1WB1: names, files, the countdown and when the toggle is on", () => {
  assert.deepEqual(docMeta(JSON.stringify({ meta: { name: "Kväll", note: "n", kind: "snapshot", savedUtc: "2026-11-14T10:00:00Z" } }), "x.dspipreset"), {
    name: "Kväll",
    note: "n",
    kind: "snapshot",
    savedUtc: "2026-11-14T10:00:00Z",
  });
  assert.equal(docMeta(JSON.stringify({ meta: { name: "S", kind: "snapshot" }, snapshot: { note: "own" } }), "").note, "own");
  assert.deepEqual(docMeta("not json", "dir/hall.dspipreset"), { name: "hall", note: "", kind: "preset", savedUtc: "" });
  assert.equal(fileNameFor('a/b:c*?"<>|'), "a_b_c_.dspipreset");
  assert.equal(fileNameFor(" .. "), "snapshot.dspipreset");
  assert.equal(fileNameFor("Kväll 🎶"), "Kväll 🎶.dspipreset");
  assert.equal(importOptions(IMPORT_VOLUMES), IMPORT_BY_BLOCK | IMPORT_VOLUMES);
  assert.equal(importOptions(0xffff), IMPORT_BY_BLOCK | IMPORT_ALL_BLOCKS);
  const g = (phase: GuardedApplyRecord["phase"], remainingMs: number) => ({ phase, remainingMs }) as GuardedApplyRecord;
  assert.equal(secondsLeft(g("waiting", 29_001)), 30);
  assert.equal(secondsLeft(g("waiting", -5)), 0);
  assert.equal(secondsLeft(g("kept", 10_000)), 0);
  assert.equal(secondsLeft(null), 0);
  const st = (o: Partial<DeskState>) =>
    ({ ab: { active: true, side: "a", aName: "A", bName: "B", recall: null }, guard: null, busy: "", ...o }) as DeskState;
  assert.equal(mayToggle(st({}), false), true);
  assert.equal(mayToggle(st({}), true), false, "read only");
  assert.equal(mayToggle(st({ busy: "Recall" }), false), false);
  assert.equal(mayToggle(st({ guard: g("waiting", 1000) }), false), false, "off while a safe apply waits");
  assert.equal(mayToggle(st({ guard: g("reverted", 0) }), false), true);
  assert.equal(mayToggle(st({ ab: { active: false, side: null, aName: "", bName: "", recall: null } }), false), false);
});

// -- On the real core (as main.test.ts) --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
const desks: SnapshotDesk[] = [];
after(() => {
  for (const d of desks) d.stop();
  for (const c of made) c.dispose();
});
const MAC = readFileSync(new URL("../../../../tests/fixtures/preset/mac-swift-v32.dspipreset", import.meta.url), "utf8");

async function deskOn(access: "control" | "view" = "control", pollMs = 20) {
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
  const session = new Session("s", "i", "Here", { kind: "demo" }, access, client);
  const desk = new SnapshotDesk(session, pollMs);
  desks.push(desk);
  return { session, desk, calls };
}

const gain0 = (s: Session) => s.state.snapshot!.state.outputGainDb[0];
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until(what: string, f: () => boolean, ms = 8000) {
  const end = Date.now() + ms;
  while (!f()) {
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await sleep(20);
  }
}

test("P1WB1: capture, change, recall: every value back, the history ended, said in the core's words", { skip }, async () => {
  const { session, desk } = await deskOn();
  await session.open({ kind: "demo" });
  await desk.start();
  assert.equal(desk.state.guardDefault, false, "on USB and the demo the rollback is the user's choice");
  const a = (await desk.capture("Kväll ✓", "före"))!;
  assert.equal(a.name, "Kväll ✓");
  assert.equal(a.kind, "snapshot");
  assert.equal(await session.write("out.gain", { float: -7 }, [0]), true);
  assert.equal(gain0(session), -7);
  assert.equal(await desk.recall(a.id), true);
  assert.equal(gain0(session), 0);
  assert.match(desk.state.outcome!.title, /Recalled “Kväll ✓”/);
  assert.deepEqual(await session.core.call("canUndo", undefined), { canUndo: false, canRedo: false }, "a recall ends the undo history");

  // A file the core cannot read is refused in its words, and not held.
  assert.equal(await desk.addFile("{", "broken.dspipreset"), null);
  assert.equal(desk.state.outcome!.tone, "error");
  assert.equal(desk.state.docs.length, 1);
  const f = (await desk.addFile(a.text, "copy.dspipreset"))!;
  assert.equal(f.origin, "file");
  desk.remove(f.id);
  assert.equal(desk.state.docs.length, 1);
});

test("P1WB1: A/B toggles between the device and a snapshot, and Keep ends it", { skip }, async () => {
  const { session, desk } = await deskOn();
  await session.open({ kind: "demo" });
  await desk.start();
  await session.write("out.gain", { float: -4 }, [0]);
  const b = (await desk.capture("Bright", ""))!;
  await session.write("out.gain", { float: 0 }, [0]);
  assert.equal(await desk.abStart(b.id, null), true);
  assert.equal(desk.state.ab.active, true);
  assert.equal(desk.state.ab.side, "a");
  assert.equal(await desk.abToggle(), true);
  assert.equal(desk.state.ab.side, "b");
  assert.equal(gain0(session), -4);
  const d = (await desk.abDiff())!;
  assert.ok(d.count >= 1 && d.rows.some((r) => r.block === "outputs"), JSON.stringify(d.rows.slice(0, 3)));
  assert.equal(await desk.abToggle(), true);
  assert.equal(gain0(session), 0);
  assert.equal(await desk.abKeep("b"), true);
  assert.equal(desk.state.ab.active, false);
  assert.equal(gain0(session), -4);
  assert.equal(desk.state.abDiff, null);
});

test("P1WB1: safe apply: Keep keeps; the toggle is off while it waits; Revert puts back and ends the compare", { skip }, async () => {
  const { session, desk, calls } = await deskOn();
  await session.open({ kind: "demo" });
  await desk.start();
  const a = (await desk.capture("Before", ""))!;
  await session.write("out.gain", { float: -9 }, [0]);
  const b = (await desk.capture("After", ""))!;

  // Keep.
  assert.equal(await desk.recall(a.id, true, 30), true);
  assert.equal(desk.state.guard!.phase, "waiting");
  assert.equal(gain0(session), 0);
  assert.equal(await desk.keep(), true);
  assert.equal(desk.state.guard!.phase, "kept");
  assert.equal(gain0(session), 0);

  // A compare, then a guarded recall: no toggle while it waits; Revert ends the compare.
  assert.equal(await desk.abStart(b.id, null), true);
  assert.equal(await desk.recall(b.id, true, 30), true);
  assert.equal(desk.state.guard!.phase, "waiting");
  assert.equal(gain0(session), -9);
  const before = calls.filter((c) => c === "abToggle").length;
  assert.equal(await desk.abToggle(), false);
  assert.equal(calls.filter((c) => c === "abToggle").length, before, "the page sent no toggle");
  assert.equal(await desk.revert(), true);
  assert.equal(desk.state.guard!.phase, "reverted");
  assert.equal(gain0(session), 0);
  assert.equal(desk.state.ab.active, false, "a revert ends the compare");
});

test("P1WB1: a guarded import that is not kept reverts on the page's poll when its time runs out", { skip }, async () => {
  const { session, desk } = await deskOn("control", 20);
  await session.open({ kind: "demo" });
  await desk.start();
  const vol = session.state.snapshot!.state.masterVolumeDb;
  const all = await desk.preview(MAC, IMPORT_ALL_BLOCKS);
  assert.ok((all.blocks.volumes ?? 0) > 0, JSON.stringify(all.blocks));
  assert.equal(await desk.importDoc(MAC, "Mac", IMPORT_VOLUMES, 0xffff_ffff, true, 1), true);
  assert.equal(desk.state.guard!.phase, "waiting");
  assert.notEqual(session.state.snapshot!.state.masterVolumeDb, vol);
  await until("the revert", () => desk.state.guard?.phase === "reverted");
  assert.equal(desk.state.guard!.reason, "timeout");
  assert.equal(session.state.snapshot!.state.masterVolumeDb, vol);
  assert.equal(desk.state.outcome!.tone, "warn");
});

test("P1WB1: the preview shows what the import then changes, block by block; nothing chosen is refused", { skip }, async () => {
  const { session, desk } = await deskOn();
  await session.open({ kind: "demo" });
  await desk.start();
  const v = await desk.preview(MAC, IMPORT_VOLUMES);
  assert.ok(v.count > 0 && v.rows.every((r) => r.block === "volumes"), JSON.stringify(v.rows));
  const eq1 = await desk.preview(MAC, IMPORT_EQ, 0b10);
  assert.ok(eq1.rows.every((r) => r.block === "eq"));
  assert.equal(await desk.importDoc(MAC, "Mac", IMPORT_VOLUMES, 0xffff_ffff, false), true);
  assert.equal(desk.state.outcome!.title, "Imported “Mac”");
  assert.equal((await desk.preview(MAC, IMPORT_VOLUMES)).count, 0, "the volumes are the document's now");
  assert.ok((await desk.preview(MAC, IMPORT_EQ)).count > 0, "the EQ was left alone");
  assert.equal(await desk.importDoc(MAC, "Mac", 0, 0, false), false);
  assert.equal(desk.state.outcome!.tone, "error");

  // Read only: nothing written, said why.
  const ro = await deskOn("view");
  await ro.session.open({ kind: "demo" });
  const n = ro.calls.length;
  assert.equal(await ro.desk.importDoc(MAC, "Mac", IMPORT_VOLUMES, 0xffff_ffff, false), false);
  assert.equal(await ro.desk.recall(1), false);
  assert.equal(ro.calls.length, n, "no call reached the core");
});

test("P1WB1: a template opens offline, says which file, saves, exports", { skip }, async () => {
  const { session, desk } = await deskOn();
  const list = await session.core.call("templateList", undefined);
  const k340 = list.find((t) => t.id === "builtin:k340")!;
  assert.ok(k340, JSON.stringify(list));
  const { text } = await session.core.call("templateText", { id: k340.id });
  await session.openDocument(text, "K340.dspipreset");
  assert.equal(session.state.status, "open");
  assert.equal(session.document!.fileName, "K340.dspipreset");
  const st = session.state.snapshot!.state;
  assert.deepEqual(st.outputEnabled.flatMap((on, o) => (on ? [o] : [])), [0, 1, 2, 3], "the K340: four outputs play");
  assert.deepEqual(await session.core.call("documentInfo", undefined), {
    offline: true,
    fileName: "K340.dspipreset",
    name: session.document!.name,
    kind: "template",
    platform: session.document!.platform,
  });
  await session.write("out.gain", { float: -3 }, [1]);
  const saved = (await desk.saveDocument())!;
  assert.equal(JSON.parse(saved).meta.kind, "template");
  const exported = (await desk.exportDoc("Hall"))!;
  assert.equal(JSON.parse(exported).meta.name, "Hall");
  const t = (await desk.saveTemplate("Hall"))!;
  assert.equal(JSON.parse(t).meta.kind, "template");

  const bad = await deskOn();
  await bad.session.openDocument("nope", "x.dspipreset");
  assert.equal(bad.session.state.status, "failed");
  assert.ok(bad.session.state.error);
});
