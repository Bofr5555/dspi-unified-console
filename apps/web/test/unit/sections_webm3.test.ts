// WP WEBM3 unit tests: Library › History's rows (src/screens/history/model.ts); the Installation
// tab's S/PDIF source, slots and groups (src/screens/system/installation/model.ts), with the real
// core (crates/dspi-wasm, when $DSPI_WASM_DIR has it) for the edits and the hub's validator; the
// parts kept per session (ui/shell/places.ts); the sections' Library pages.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { HistoryEntry } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { historyRows, sourceWords, timeWords, valueWords, whatWords } from "../../src/screens/history/model";
import type { SystemModel, Unit } from "../../src/screens/system/model";
import {
  InstallationForm,
  NEW_SOURCE_MASK,
  joinEdit,
  needsUpstream,
  roleEdits,
  roleOf,
  slotEdit,
  upstreamChoices,
  withMember,
  type InstallationCall,
} from "../../src/screens/system/installation/model";
import type { InstallationDraft } from "../../src/screens/system/installation/protocol";
import { screensFor } from "../../src/ui/screens";
import { placeOf } from "../../src/ui/shell/places";
import { sectionsFor } from "../../src/ui/shell/sections";
import type { Session } from "../../src/ui/session";

// ---------------------------------------------------------------- History

test("WEBM3: History's values, names, sources and times in words", () => {
  assert.equal(valueWords({ float: -12 }), "−12");
  assert.equal(valueWords({ float: 1.23456 }), "1.23");
  assert.equal(valueWords({ bool: true }), "On");
  assert.equal(valueWords({ text: "Bar" }), "“Bar”");
  assert.equal(valueWords("trigger"), "");
  assert.equal(valueWords(null), "");
  assert.equal(whatWords("Gain", [2, 0]), "Gain · 3 · 1");
  assert.equal(sourceWords("user"), "");
  assert.equal(sourceWords("undo"), "Undo");
  assert.equal(sourceWords("hub_scene"), "Hub scene");
  const at = new Date(2026, 10, 22, 9, 5, 7).getTime();
  assert.equal(timeWords(at, at + 1000), "09:05:07");
  assert.equal(timeWords(at, at + 2 * 86_400_000), "2026-11-22 09:05:07");
});

test("WEBM3: History's rows keep the core's order (newest first) and say before → after", () => {
  const e = (path: string, before: HistoryEntry["before"], after: HistoryEntry["after"], source = "user", undoable = true): HistoryEntry => ({
    path,
    indices: [1],
    before,
    after,
    source,
    atMs: 1000,
    undoable,
  });
  const rows = historyRows([e("vol.master", { float: 0 }, { float: -12 }, "undo"), e("preset.load", null, "trigger", "user", false)], (p) => (p === "vol.master" ? "Master volume" : p), 1000);
  assert.deepEqual(
    rows.map((r) => [r.what, r.values, r.source, r.undoable]),
    [
      ["Master volume · 2", "0 → −12", "Undo", true],
      ["preset.load · 2", "", "", false],
    ],
  );
  assert.notEqual(rows[0]!.key, rows[1]!.key);
});

// ---------------------------------------------------------------- the shell

test("WEBM3: Library has History between Snapshots and Files, not in Simple; a page's part is kept per session", () => {
  const lib = (mode: "simple" | "advanced") => sectionsFor(screensFor(mode, false)).find((s) => s.id === "library")!.pages.map((p) => p.id);
  assert.deepEqual(lib("advanced"), ["snapshots", "history", "files"]);
  assert.deepEqual(lib("simple"), ["snapshots"]);
  const a = {} as Session;
  const b = {} as Session;
  placeOf(a).parts.set("effects", "loudness");
  assert.equal(placeOf(a).parts.get("effects"), "loudness");
  assert.equal(placeOf(b).parts.size, 0);
});

// ---------------------------------------------------------------- Installation (RB C-2)

function site(): SystemModel {
  return {
    rooms: [{ id: "main", name: "Main room" }],
    zones: [],
    groups: [{ id: "fronts", name: "Fronts", units: ["A", "B"] }],
    sources: [{ id: "music", name: "Music", sender: "tx", channels: [0, 1] }],
    senders: [{ id: "tx", name: "Sender", serial: "S-TX" }],
    fanOuts: [],
    units: [
      { id: "A", name: "Left", serial: "S-A", room: "main", upstream: "tx" },
      { id: "B", name: "Right", serial: "S-B", room: "main", upstream: "tx" },
      { id: "S", name: "Source", serial: "S-S", room: "main", upstream: "", transport: "spdif", spdifSource: true, spdifTxMask: 0b0001 },
      { id: "F", name: "Follower", serial: "S-F", room: "main", upstream: "S", transport: "spdif" },
    ],
    scenes: [],
  };
}

test("WEBM3: an S/PDIF source is a role of its own, fed by nothing; the follower is fed by a source", () => {
  const m = site();
  assert.deepEqual(m.units.map(roleOf), ["receiver", "receiver", "spdif_source", "spdif_follower"]);
  assert.equal(needsUpstream("spdif_source"), false);
  assert.deepEqual(upstreamChoices(m, "spdif_source"), []);
  assert.deepEqual(
    upstreamChoices(m, "spdif_follower", "F").map((c) => c.id),
    ["S"],
  );
  // Joining as a source: the core's follower, nothing upstream (the mark follows on the id the core made).
  assert.deepEqual(joinEdit("S-X", { name: "", room: "", role: "spdif_source", upstream: "tx" }), { op: "addUnit", serial: "S-X", room: null, role: "spdif_follower", upstream: null });
});

test("WEBM3: a role change to and from an S/PDIF source is one step with its mark", () => {
  const [a, , s] = site().units as [Unit, Unit, Unit, Unit];
  assert.deepEqual(roleEdits(a, "receiver"), []);
  assert.deepEqual(roleEdits(a, "spdif_source"), [
    { op: "setUnitRole", id: "A", role: "spdif_follower" },
    { op: "setSpdifSource", id: "A", txMask: NEW_SOURCE_MASK },
  ]);
  assert.deepEqual(roleEdits({ ...a, spdifTxMask: 0b0110 }, "spdif_source").at(-1), { op: "setSpdifSource", id: "A", txMask: 0b0110 }, "the slots it had");
  assert.deepEqual(roleEdits(s, "receiver"), [
    { op: "setSpdifSource", id: "S", txMask: 0 },
    { op: "setUnitRole", id: "S", role: "receiver" },
  ]);
});

test("WEBM3: a source's slots: on and off, never none; only a source has them", () => {
  const [a, , s] = site().units as [Unit, Unit, Unit, Unit];
  assert.deepEqual(slotEdit(s, 3, true), { op: "setSpdifSource", id: "S", txMask: 0b1001 });
  assert.match(slotEdit(s, 0, false) as string, /one pair of outputs at least/);
  assert.deepEqual(slotEdit({ ...s, spdifTxMask: 0b1001 }, 0, false), { op: "setSpdifSource", id: "S", txMask: 0b1000 });
  assert.equal(typeof slotEdit(a, 0, true), "string");
  assert.equal(typeof slotEdit(s, 4, true), "string");
  assert.deepEqual(withMember(["A"], "B", true), ["A", "B"]);
  assert.deepEqual(withMember(["A", "B"], "B", true), ["A", "B"]);
  assert.deepEqual(withMember(["A", "B"], "A", false), ["B"]);
});

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

test("WEBM3: the real core takes a joined S/PDIF source in one step, a group's units, a new group and a zone's source", { skip }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const call: InstallationCall = (m, p) => core.call(m, p as never) as never;
  const m = site();
  const draft: InstallationDraft = { revision: 2, base: m, model: m };
  const form = new InstallationForm(async (method, params) => {
    if (method === "installationView") return { revision: 2, role: "admin", mayEdit: true, draft, unseen: [], inventory: true } as never;
    return call(method, params);
  });
  await form.load();
  // Joined as a source: one step (one edit list), marked on the id the core made from its name.
  const ok = await form.editThen(joinEdit("S-X", { name: "Extra", room: "main", role: "spdif_source", upstream: "" }), (next) => {
    const id = next.model.units.find((u) => u.serial === "S-X")!.id;
    return [{ op: "setSpdifSource", id, txMask: NEW_SOURCE_MASK }];
  });
  assert.ok(ok, form.state.error);
  const x = form.state.draft!.model.units.find((u) => u.serial === "S-X")!;
  assert.equal(roleOf(x), "spdif_source");
  assert.equal(x.spdifTxMask, NEW_SOURCE_MASK);
  assert.equal(form.state.edits.length, 2);
  // A source's slot, a group's units, a new group, a zone playing Music.
  assert.ok(await form.edit(slotEdit(m.units[2]!, 2, true) as never), form.state.error);
  assert.ok(await form.edit({ op: "setGroupUnits", id: "fronts", units: ["A"] }), form.state.error);
  assert.ok(await form.edit({ op: "addGroup", name: "All", units: ["A", "B", x.id] }), form.state.error);
  assert.ok(await form.edit({ op: "addZone", name: "Everywhere", rooms: ["main"], source: "music" }), form.state.error);
  const model = form.state.draft!.model;
  assert.equal(model.units.find((u) => u.id === "S")!.spdifTxMask, 0b0101);
  assert.deepEqual(model.groups.find((g) => g.id === "fronts")!.units, ["A"]);
  assert.deepEqual(model.groups.find((g) => g.name === "All")!.units, ["A", "B", x.id]);
  assert.equal(model.zones.find((z) => z.name === "Everywhere")!.source, "music");
  assert.deepEqual(form.state.issues, [], JSON.stringify(form.state.issues));
  assert.ok(form.savable);
  // A refused first edit of a pair: nothing made, the reason said.
  assert.equal(await form.editThen({ op: "setUnitRoom", id: "ghost", room: null }, () => []), false);
  assert.ok(form.state.error);
  assert.equal(form.state.edits.length, 6);
});
