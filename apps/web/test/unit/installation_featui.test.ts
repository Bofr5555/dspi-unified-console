// WP FEATUI unit tests: FEAT1 on the web Installation tab (src/screens/system/installation/model.ts):
// channels and slots as typed, whose slots a zone's source owns, the notice of replaced hand-made
// slots, the zone's units, and a scene's actions in words.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { SystemModel } from "../../src/screens/system/model";
import {
  SILENT,
  actionText,
  numbersText,
  parseNumbers,
  parseSlots,
  programmeFeeds,
  programmeLimit,
  rerouted,
  sceneTargets,
  slotsOwner,
  slotsText,
  zoneUnits,
} from "../../src/screens/system/installation/model";

const s = (a: number, b: number) => [a, b, SILENT, SILENT, SILENT, SILENT, SILENT, SILENT];

function site(): SystemModel {
  return {
    rooms: [
      { id: "main", name: "Main room" },
      { id: "bar", name: "Bar" },
    ],
    zones: [{ id: "inside", name: "Inside", rooms: ["main"], source: "music" }],
    groups: [],
    sources: [
      { id: "music", name: "Music", sender: "tx", channels: [0, 1] },
      { id: "booth", name: "Booth", sender: "tx", channels: [8, 9] },
    ],
    senders: [{ id: "tx", name: "Sender", serial: "S-TX" }],
    fanOuts: [],
    units: [
      { id: "A", name: "Left", serial: "S-A", room: "main", upstream: "tx", slots: s(0, 1) },
      { id: "D", name: "Bar‮", serial: "S-D", room: "bar", upstream: "tx", slots: s(4, 5) },
      { id: "R", name: "Rear", serial: "S-R", room: "main", upstream: "", transport: "usb_direct" },
    ],
    scenes: [],
  };
}

test("wp_featui channels and inputs are typed from 0, ranges included", () => {
  assert.deepEqual(parseNumbers("8 9", 64), [8, 9]);
  assert.deepEqual(parseNumbers("8-11", 64), [8, 9, 10, 11]);
  assert.deepEqual(parseNumbers("0, 2;3", 8), [0, 2, 3]);
  for (const bad of ["", "64", "1-0", "a", "1--2", "-1", "8", "2.5"]) assert.equal(parseNumbers(bad, bad === "8" ? 8 : 64), null, bad);
  assert.equal(numbersText([8, 9]), "8 9");
});

test("wp_featui a unit's slots as typed: 8 entries, - for silent, nothing for its own", () => {
  assert.deepEqual(parseSlots("8 9"), s(8, 9));
  assert.deepEqual(parseSlots("- 3"), [SILENT, 3, SILENT, SILENT, SILENT, SILENT, SILENT, SILENT]);
  assert.deepEqual(parseSlots("  "), []);
  for (const bad of ["64", "x", "1 2 3 4 5 6 7 8 9", "-1"]) assert.equal(parseSlots(bad), null, bad);
  assert.equal(slotsText(s(8, 9)), "8 9 - - - - - -");
  assert.equal(slotsText(null), "");
});

test("wp_featui a zone's source owns its audio link units' slots, not a USB-direct one's", () => {
  const m = site();
  assert.deepEqual(slotsOwner(m, m.units[0]!), { zone: "Inside", source: "Music" });
  assert.equal(slotsOwner(m, m.units[1]!), null, "the bar is in no zone");
  assert.equal(slotsOwner(m, m.units[2]!), null, "USB-direct");
  assert.deepEqual(zoneUnits(m, "inside"), ["Left"]);
  assert.deepEqual(zoneUnits(m, "nope"), []);
});

test("wp_featui the notice: hand-made slots a zone's source replaces, names cleaned", () => {
  const base = site();
  const model = site();
  model.zones[0]!.source = "booth";
  model.zones[0]!.rooms = ["main", "bar"];
  model.units[0]!.slots = s(8, 9);
  model.units[1]!.slots = s(8, 9);
  const rows = rerouted({ revision: 1, base, model });
  assert.deepEqual(rows, [
    { id: "A", name: "Left", zone: "Inside", source: "Booth", handMade: true },
    { id: "D", name: "Bar", zone: "Inside", source: "Booth", handMade: true },
  ]);
  // A unit with no slots of its own before is said as following, not as replaced
  base.units[1]!.slots = null;
  assert.equal(rerouted({ revision: 1, base, model })[1]!.handMade, false);
  // Nothing moved: no notice
  assert.deepEqual(rerouted({ revision: 1, base: site(), model: site() }), []);
});

test("wp_featui scene actions in words, and what a programme source may feed", () => {
  const m = site();
  assert.equal(actionText(m, { target: "zone:inside", op: "source", source: "booth" }), "Switch Inside to source Booth");
  assert.equal(actionText(m, { target: "all", op: "mute" }), "Mute Everywhere");
  assert.equal(actionText(m, { target: "room:bar", op: "master_volume", value: -1250 }), "Bar: master volume -12.5 dB");
  assert.deepEqual(
    sceneTargets(m).map((t) => t.id),
    ["all", "zone:inside", "room:main", "room:bar"],
  );
  assert.deepEqual(programmeFeeds(m, "alink_sender"), [{ id: "tx", name: "Sender" }]);
  assert.equal(programmeFeeds(m, "computer_usb").length, 3);
  assert.equal(programmeLimit("alink_sender"), 64);
  assert.equal(programmeLimit("spdif"), 8);
});
