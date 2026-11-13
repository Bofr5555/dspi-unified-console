// WP WB8 unit tests: the undo keys, the session's undo and redo (the core's
// note naming another writer, the changes stepped over, read only), the drag
// mark's counting, and the event log's rows, filter and text.

import { test } from "node:test";
import assert from "node:assert/strict";
import { CoreCallError, type EventRecord, type HistoryMove } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { appleKeys, isTextField, undoKey } from "../../src/ui/undo";
import { alarmRow, kindWord, merged, recordRow, rowsText, shownRows, timeText, unitsOf, KEEP } from "../../src/screens/events/model";
import { browserFacts, fromBase64 } from "../../src/screens/events/EventsScreen";
import type { Alarm } from "../../src/screens/system/model";

const key = (k: string, mods: { ctrl?: boolean; meta?: boolean; shift?: boolean; alt?: boolean } = {}) => ({
  key: k,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
});

test("WB8: Ctrl+Z undoes, Ctrl+Shift+Z and Ctrl+Y redo; Cmd on a Mac", () => {
  assert.equal(undoKey(key("z", { ctrl: true }), false), "undo");
  assert.equal(undoKey(key("Z", { ctrl: true, shift: true }), false), "redo");
  assert.equal(undoKey(key("y", { ctrl: true }), false), "redo");
  assert.equal(undoKey(key("z"), false), null);
  assert.equal(undoKey(key("z", { meta: true }), false), null, "Cmd is not Ctrl off a Mac");
  assert.equal(undoKey(key("z", { ctrl: true, alt: true }), false), null);
  assert.equal(undoKey(key("z", { meta: true }), true), "undo");
  assert.equal(undoKey(key("z", { meta: true, shift: true }), true), "redo");
  assert.equal(undoKey(key("z", { ctrl: true }), true), null);
  assert.equal(undoKey(key("x", { ctrl: true }), false), null);
  assert.equal(appleKeys("MacIntel"), true);
  assert.equal(appleKeys("macOS"), true);
  assert.equal(appleKeys("Linux x86_64"), false);
  assert.equal(appleKeys("Windows"), false);
});

test("WB8: a text field keeps its own undo; a slider, a button and a checkbox do not", () => {
  const el = (tagName: string, type?: string, editable = false) => ({ tagName, type, isContentEditable: editable }) as unknown as Element;
  assert.equal(isTextField(el("INPUT", "text")), true);
  assert.equal(isTextField(el("INPUT", "")), true);
  assert.equal(isTextField(el("INPUT", "number")), true);
  assert.equal(isTextField(el("TEXTAREA")), true);
  assert.equal(isTextField(el("DIV", undefined, true)), true);
  assert.equal(isTextField(el("INPUT", "range")), false);
  assert.equal(isTextField(el("INPUT", "checkbox")), false);
  assert.equal(isTextField(el("BUTTON")), false);
  assert.equal(isTextField(el("SELECT")), false);
  assert.equal(isTextField(null), false);
});

// -- The session against a scripted core --

function scripted(answer: (method: string, params: unknown) => unknown, access: "control" | "view" = "control") {
  const calls: { method: string; params: unknown }[] = [];
  const core = {
    call: async (method: string, params: unknown) => {
      calls.push({ method, params });
      return answer(method, params);
    },
    subscribe: () => () => undefined,
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const s = new Session("s", "i", "Here", { kind: "demo" }, access, core);
  return { s, calls };
}

const snapshot = (readOnly = false) => ({ device: { name: "DSPi", serial: "S1", readOnly, readOnlyReason: readOnly ? "old firmware" : "" }, state: {} });
const entry = (path: string) => ({ path, indices: [], before: null, after: "trigger" as const, source: "user", atMs: 0, undoable: false });

async function opened(answer: (method: string, params: unknown) => unknown, access: "control" | "view" = "control") {
  const r = scripted((m, p) => {
    if (m === "open" || m === "state") return snapshot();
    if (m === "presetChanges") return { dirty: false, changes: [] };
    return answer(m, p);
  }, access);
  await r.s.open({ kind: "demo" });
  return r;
}

const settle = () => new Promise((r) => setTimeout(r, 0));

test("WB8: the session follows what undo and redo can do after every snapshot", async () => {
  let can = { canUndo: true, canRedo: false };
  const { s } = await opened((m) => (m === "canUndo" ? can : null));
  await settle();
  assert.deepEqual(s.state.history, { canUndo: true, canRedo: false });
  can = { canUndo: false, canRedo: true };
  await s.write("vol.user", { float: -3 });
  await settle();
  assert.deepEqual(s.state.history, { canUndo: false, canRedo: true });
});

test("WB8: an undo another writer stepped in on says who, in the core's words", async () => {
  const note: HistoryMove = { entry: null, skipped: [entry("vol.master")], note: "Master volume was changed by another console (Stage) since; it was left as they set it" };
  const { s, calls } = await opened((m) => (m === "undo" ? note : m === "canUndo" ? { canUndo: false, canRedo: false } : null));
  await s.undo();
  assert.deepEqual(s.state.notice, { text: note.note, tone: "warn" });
  assert.ok(calls.some((c) => c.method === "undo"));
  // A read-back follows the step.
  assert.equal(calls.filter((c) => c.method === "state").length >= 1, true);
  s.dismissNotice();
  assert.equal(s.state.notice, null);
});

test("WB8: changes stepped over are named by the registry's labels", async () => {
  const labels: Record<string, string> = { "preset.load": "Load preset", "out.enable": "Output on" };
  const { s } = await opened((m, p) => {
    if (m === "redo") return { entry: entry("vol.user"), skipped: [entry("preset.load"), entry("eq.band"), entry("out.enable")], note: null };
    if (m === "describe") {
      const path = (p as { path: string }).path;
      if (labels[path]) return { label: labels[path] };
      throw new CoreCallError("unknown_param", path);
    }
    return { canUndo: false, canRedo: false };
  });
  await s.redo();
  assert.deepEqual(s.state.notice, { text: "Stepped over changes that cannot be undone: Load preset; Filter band; Output on", tone: "info" });
});

test("WB8: a failed step says why and nothing else", async () => {
  const { s } = await opened((m) => {
    if (m === "undo") throw new CoreCallError("not_open", "no device");
    return { canUndo: true, canRedo: false };
  });
  await s.undo();
  assert.equal(s.state.error, "No device");
  assert.equal(s.state.notice, null);
});

test("WB8: a viewer's session sends no undo, redo or drag mark", async () => {
  const { s, calls } = await opened(() => ({ canUndo: true, canRedo: true }), "view");
  await settle();
  assert.deepEqual(s.state.history, { canUndo: false, canRedo: false });
  await s.undo();
  await s.redo();
  s.setDragging(true);
  s.setDragging(false);
  assert.deepEqual(calls.filter((c) => ["undo", "redo", "setDragging", "canUndo"].includes(c.method)), []);
});

test("WB8: the drag mark is counted: one start, one end, however many pointers", async () => {
  const { s, calls } = await opened(() => null);
  s.setDragging(true);
  s.setDragging(true);
  s.setDragging(false);
  s.setDragging(false);
  s.setDragging(false);
  const marks = calls.filter((c) => c.method === "setDragging").map((c) => (c.params as { dragging: boolean }).dragging);
  assert.deepEqual(marks, [true, false]);
});

// -- The event log's rows --

const rec = (id: number, t: number, kind: EventRecord["kind"], text: string, serial = "S1"): EventRecord => ({ v: 1, id, t, kind, text, serial });

test("WB8: event rows carry the time, the unit's name and the core's text; colour only for a clip or a reset", () => {
  const name = (s: string) => (s === "S1" ? "Stage left" : s);
  const clip = recordRow(rec(3, 1000, "clip", "IN1 clipped"), name);
  assert.deepEqual(
    { t: clip.t, unit: clip.unit, kind: clip.kind, text: clip.text, tone: clip.tone },
    { t: 1000, unit: "Stage left", kind: "Clip", text: "IN1 clipped", tone: "bad" },
  );
  assert.equal(recordRow(rec(4, 1, "reset", "Watchdog"), name).tone, "warn");
  assert.equal(recordRow(rec(5, 1, "audio_link", "Locked"), name).tone, "");
  assert.equal(kindWord("audio_link"), "Audio link");
  assert.equal(kindWord("something new"), "Event");
});

test("WB8: an alarm row names its subject and its severity's tone", () => {
  const model = { units: [{ id: "unit:A", name: "Bar" }], fanOuts: [], senders: [] } as never;
  const a: Alarm = { id: 7, code: "link_loss", subject: "unit:A", severity: "critical", message: "Audio link lost", state: "acknowledged", raisedAt: 2, count: 3 };
  const r = alarmRow(a, model);
  assert.equal(r.t, 2000);
  assert.equal(r.unit, "Bar");
  assert.equal(r.text, "Audio link lost (3×) (acknowledged)");
  assert.equal(r.tone, "bad");
  assert.equal(alarmRow({ ...a, severity: "warning", state: "raised", count: 1 }, null).tone, "warn");
});

test("WB8: rows filter by unit, newest first, and copy as one tab-separated line each", () => {
  const name = (s: string) => s;
  const rows = [recordRow(rec(1, Date.UTC(2026, 10, 13, 10, 0, 0), "connected", "Connected", "A"), name), recordRow(rec(2, Date.UTC(2026, 10, 13, 10, 0, 5), "clip", "OUT2 clipped", "B"), name)];
  assert.deepEqual(unitsOf(rows), [{ id: "A", name: "A" }, { id: "B", name: "B" }]);
  assert.deepEqual(shownRows(rows, "").map((r) => r.text), ["OUT2 clipped", "Connected"]);
  assert.deepEqual(shownRows(rows, "A").map((r) => r.text), ["Connected"]);
  const text = rowsText(shownRows(rows, ""));
  const lines = text.split("\n");
  assert.equal(lines.length, 2);
  assert.match(lines[0]!, /^2026-11-1\d \d\d:\d\d:05\tB\tClip\tOUT2 clipped$/);
});

test("WB8: the time is the clock today and the date before it otherwise", () => {
  const now = new Date(2026, 10, 13, 15, 0, 0);
  assert.equal(timeText(new Date(2026, 10, 13, 9, 5, 7).getTime(), now), "09:05:07");
  assert.equal(timeText(new Date(2026, 10, 12, 23, 59, 1).getTime(), now), "2026-11-12 23:59:01");
});

test("WB8: new records join the kept ones without repeats, the newest kept", () => {
  const a = [rec(1, 1, "connected", "a"), rec(2, 2, "clip", "b")];
  const b = merged(a, [rec(2, 2, "clip", "b"), rec(3, 3, "clip", "c"), rec(1, 4, "connected", "other unit", "S2")]);
  assert.deepEqual(b.map((r) => r.text), ["a", "b", "c", "other unit"]);
  const many = Array.from({ length: KEEP + 10 }, (_, i) => rec(i + 1, i, "clip", String(i)));
  const kept = merged([], many);
  assert.equal(kept.length, KEEP);
  assert.equal(kept[0]!.text, "10");
});

test("WB8: the bundle's browser file and the zip's bytes", () => {
  const facts = browserFacts({ userAgent: "UA/1", language: "sv-SE" }, true);
  assert.equal(facts, "User agent: UA/1\nLanguage: sv-SE\nJavaScript Promise Integration: yes\n");
  assert.match(browserFacts(null, false), /unknown[\s\S]*no\n$/);
  assert.deepEqual([...fromBase64("UEsDBA==")], [0x50, 0x4b, 0x03, 0x04]);
});
