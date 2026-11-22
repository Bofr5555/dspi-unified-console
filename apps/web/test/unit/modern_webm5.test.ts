// WP WEBM5: review RC's web items. RC7: a menu row or palette entry shows a key only where the web
// binds it (Edit History's Ctrl+Shift+H is Firefox's Library). RC16: the History page keeps its
// rows while the journal it reads again is the same list.

import { test } from "node:test";
import assert from "node:assert/strict";
import { ACTIONS, actionShortcut } from "../../src/ui/menus/actions";
import { actionEntry } from "../../src/ui/menus/session-actions";
import { SHORTCUTS } from "../../src/screens/shortcuts/table";
import { sameHistory } from "../../src/screens/history/model";
import type { HistoryEntry } from "../../src/core/protocol";

test("WEBM5 (RC7): only the keys the web binds are shown; the presets' Ctrl+1…9 are", () => {
  assert.equal(actionShortcut("edit.history"), "", "Ctrl+Shift+H is not the web's");
  assert.equal(actionShortcut("edit.selectAll"), "");
  assert.equal(actionShortcut("file.quit"), "");
  assert.equal(actionShortcut("device.preset3"), "Ctrl+3");
  assert.equal(actionShortcut("channel.copy"), "Ctrl+C", "a row the same as a bound one");
  assert.equal(actionShortcut("edit.undo"), "Ctrl+Z");
  const entry = actionEntry("edit.history", { run: () => undefined }, null, () => "Edit");
  assert.equal(entry?.keys, "");
  assert.ok(!(entry?.keywords ?? []).includes("Ctrl+Shift+H"));
  // Every key shown is one the shortcut table lists, with the same keys
  const keys = new Map(SHORTCUTS.map((r) => [r.id, r.key]));
  for (const a of ACTIONS) {
    const shown = actionShortcut(a.id);
    if (!shown) continue;
    const listed = keys.get(a.id) ?? keys.get(a.of ?? "") ?? (/^device\.preset\d$/.test(a.id) ? "Ctrl+1…9" : undefined);
    assert.ok(listed, `${a.id}: ${shown} is listed`);
    if (listed !== "Ctrl+1…9") assert.equal(shown, listed, a.id);
  }
});

const entry = (atMs: number, path: string, more: Partial<HistoryEntry> = {}): HistoryEntry => ({
  path,
  indices: [0],
  before: { float: 0 },
  after: { float: 1 },
  source: "user",
  atMs,
  undoable: true,
  ...more,
});

test("WEBM5 (RC16): the same journal read again is the same list; a new change, an undo or a source is not", () => {
  const a = [entry(2, "eq.band"), entry(1, "master.volume")];
  assert.equal(sameHistory(a, a.map((e) => ({ ...e, indices: [...e.indices] }))), true);
  assert.equal(sameHistory(a, [entry(3, "eq.band"), ...a]), false);
  assert.equal(sameHistory(a, [entry(2, "eq.band", { indices: [1] }), a[1]!]), false);
  assert.equal(sameHistory(a, [entry(2, "eq.band", { undoable: false }), a[1]!]), false);
  assert.equal(sameHistory(a, [entry(2, "eq.band", { source: "undo" }), a[1]!]), false);
  assert.equal(sameHistory([], []), true);
});
