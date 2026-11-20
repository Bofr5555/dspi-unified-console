// WP REVERTW unit tests: a revert that was not fully successful (owner 2026-11-19) is said in the
// core's `revert.*` notice on every web path: the safe apply's outcome (a recall or an import with
// a way back), the auto-tune's stage, and the system apply's ended line. The notice table's words
// when it is read, the same English words before.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { GuardedApplyRecord, NoticeRecord } from "../../src/core/protocol";
import { guardOffered, guardOutcome } from "../../src/screens/snapshots/model";
import { stageWords, type TuneStateRecord } from "../../src/screens/measure/tune-model";
import { endedWords } from "../../src/screens/measure/system/model";
import { REVERT_INCOMPLETE_ACTION, REVERT_INCOMPLETE_TITLE, revertNotice } from "../../src/ui/texts";

const GUARD: GuardedApplyRecord = {
  id: 4,
  phase: "revert_incomplete",
  label: "Evening",
  hub: false,
  timeoutMs: 30_000,
  remainingMs: 0,
  reason: "reverted",
  message: "The settings from before were put back, but not all of them (see the notes).",
  changes: [],
  notes: ["Device name: not written (refused)"],
  revertNotice: "revert.incomplete",
};

const ROW: NoticeRecord = {
  key: "revert.incomplete",
  kind: "error",
  title: "The revert was not fully successful",
  reason: "Some of the settings from before were not put back.",
  action: "Check the settings, or restore a backup (a snapshot).",
  detail: "",
};
const row = (k: string) => (k === ROW.key ? ROW : null);

test("REVERTW: the safe apply's outcome leads with the notice, then what, then what to do", () => {
  const o = guardOutcome(GUARD, row);
  assert.equal(o.title, "The revert was not fully successful");
  assert.equal(o.tone, "error");
  assert.deepEqual(o.lines, [GUARD.message, ROW.action]);
  assert.deepEqual(o.notes, GUARD.notes);
  // Before the table is read: the same words.
  const f = guardOutcome({ ...GUARD, phase: "revert_failed", revertNotice: "revert.failed", message: "" }, () => null);
  assert.equal(f.title, REVERT_INCOMPLETE_TITLE);
  assert.deepEqual(f.lines, [REVERT_INCOMPLETE_ACTION]);
  // A clean revert keeps its label and its own words.
  const clean = guardOutcome({ ...GUARD, phase: "reverted", revertNotice: "", message: "Reverted: the settings from before are back.", notes: [] }, row);
  assert.equal(clean.title, "Evening");
  assert.equal(clean.tone, "warn");
  // A lost guard whose end the hub no longer knows offers no Revert; an ordinary lost one does.
  assert.equal(guardOffered({ ...GUARD, phase: "lost", revertNotice: "revert.unknown" }), false);
  assert.equal(guardOffered({ ...GUARD, phase: "lost", revertNotice: "" }), true);
  assert.equal(revertNotice(""), null);
});

test("REVERTW: the tune's and the system apply's revert not fully successful, in the notice's words", () => {
  const st = {
    stage: "revertIncomplete",
    running: false,
    run: 0,
    runs: 0,
    guard: { ...GUARD, phase: "not_written", revertNotice: "revert.not_written" },
    issue: { key: "revert.not_written", message: "nothing was written back" },
    snapshotBeforeId: "s1",
    snapshotAfterId: "",
  } as unknown as TuneStateRecord;
  assert.equal(stageWords(st), `${REVERT_INCOMPLETE_TITLE}. ${REVERT_INCOMPLETE_ACTION}`);
  assert.equal(
    endedWords({ stage: "revert_incomplete", message: "not reverted: the model changed", revertNotice: "revert.not_written" }),
    `${REVERT_INCOMPLETE_TITLE}. Not reverted: the model changed. ${REVERT_INCOMPLETE_ACTION}`,
  );
});
