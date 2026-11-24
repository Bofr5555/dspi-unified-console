// WP WEB7 unit tests: Library › Presets' rows and the actions each row offers
// (src/screens/presets/model.ts): Copy to… only on the active row and only from it (the core's
// copy saves the LIVE settings into both slots: REVE B1), an unknown active slot (>= 10) with no
// active row and nothing that copies, a read-only session with every action off (REVE C3).

import { test } from "node:test";
import assert from "node:assert/strict";
import { copyTargets, knownActive, slotActions, slotLabel, slotRows, startupWords, type Slots } from "../../src/screens/presets/model";

const slots = (over: Partial<Slots> = {}): Slots => ({
  presetNames: ["Music", "", "Movie", "", "", "", "", "", "", ""],
  presetOccupied: 0b101, // slots 1 and 3 stored
  activePresetSlot: 0,
  presetStartupMode: 0,
  presetDefaultSlot: 0,
  ...over,
});
const free = { ro: false, free: true, dirty: false };

test("WEB7 presets: rows say the name, Empty quietly, Active (with unsaved changes) and Startup", () => {
  const rows = slotRows(slots(), true);
  assert.equal(rows.length, 10);
  assert.deepEqual([rows[0]!.title, rows[0]!.note, rows[0]!.active, rows[0]!.startup], ["Music", "Active, with unsaved changes · Startup", true, true]);
  assert.deepEqual([rows[1]!.title, rows[1]!.muted, rows[1]!.note], ["Empty", true, ""]);
  assert.deepEqual([rows[2]!.title, rows[2]!.muted, rows[2]!.active], ["Movie", false, false]);
  assert.equal(slotRows(slots(), false)[0]!.note, "Active · Startup");
  // An active slot with nothing stored is "Preset N", not "Empty" (Qt).
  assert.equal(slotRows(slots({ activePresetSlot: 4 }), false)[4]!.title, "Preset 5");
  assert.equal(startupWords(slots({ presetDefaultSlot: 2 })), "At power-on the device loads preset 3.");
  assert.equal(startupWords(slots({ presetStartupMode: 1 })), "At power-on the device loads the preset it was last on.");
});

test("WEB7 presets (REVE B1): Copy to… on the active row only, from the active slot; a stored row's Copy Here copies the active one", () => {
  const s = slots();
  assert.deepEqual(slotActions(s, 0, free).copyTo, { shown: true, enabled: true });
  // Movie (stored, not active) offers no Copy to…: its stored contents would be overwritten by the live settings.
  assert.equal(slotActions(s, 2, free).copyTo.shown, false);
  assert.equal(slotActions(s, 1, free).copyTo.shown, false);
  // Another slot's "Save Current Settings Here" is a copy of the active slot, named so.
  const here = slotActions(s, 2, free).saveHere;
  assert.deepEqual([here.shown, here.copy, here.text], [true, true, "Copy “1: Music” Here"]);
  assert.deepEqual([slotActions(s, 0, free).saveHere.copy, slotActions(s, 0, free).saveHere.text], [false, "Save Current Settings Here"]);
  assert.deepEqual(copyTargets(s), [1, 2, 3, 4, 5, 6, 7, 8, 9]);
  // Recall on stored rows that are not active; Save on the active one.
  assert.deepEqual([slotActions(s, 2, free).recall.shown, slotActions(s, 1, free).recall.shown, slotActions(s, 0, free).recall.shown], [true, false, false]);
  assert.deepEqual([slotActions(s, 0, free).save.shown, slotActions(s, 2, free).save.shown], [true, false]);
  assert.equal(slotActions(s, 2, free).clear.text, "Clear “3: Movie”…");
  assert.equal(slotActions(s, 1, free).clear.shown, false);
});

test("WEB7 presets (REVE B1): an unknown active slot (>= 10) has no active row and nothing copies", () => {
  for (const a of [10, 255, -1]) {
    const s = slots({ activePresetSlot: a });
    assert.equal(knownActive(s), null);
    assert.ok(slotRows(s, false).every((r) => !r.active && !r.note.startsWith("Active")));
    assert.deepEqual(copyTargets(s), []);
    for (let slot = 0; slot < 10; slot++) {
      const x = slotActions(s, slot, free);
      assert.equal(x.copyTo.shown, false, `slot ${slot}`);
      assert.equal(x.saveHere.shown || x.saveHere.enabled, false, `slot ${slot}`);
      assert.equal(x.save.shown, false);
    }
    assert.equal(slotLabel(s, 1), "2: Empty");
  }
});

test("WEB7 presets (REVE C3): a read-only session looks only; a running action holds everything", () => {
  for (const o of [{ ...free, ro: true }, { ...free, free: false }]) {
    for (let slot = 0; slot < 10; slot++) {
      const x = slotActions(slots(), slot, o);
      for (const [k, v] of Object.entries(x)) assert.equal((v as { enabled: boolean }).enabled, false, `slot ${slot} ${k}`);
    }
  }
});
