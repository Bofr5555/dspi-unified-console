// WP WEB7 unit tests: the Measure pages as Qt Modern's: the preflight's line says its title once
// (REVE D4), "Propose from the Device" chooses the outputs the core proposes (Qt's layoutInto).

import { test } from "node:test";
import assert from "node:assert/strict";
import { displayName, initialDraft, issueText, withProposal } from "../../src/screens/measure/model";
import { dbLabels, dbTicks } from "../../src/screens/measure/graph";

const name = (o: number) => `Out ${o + 1}`;

test("WEB7: a preflight line whose message starts with its title says the title once (REVE D4)", () => {
  const title = (k: string) => (k === "measure.uncalibrated" ? "No microphone calibration" : null);
  assert.equal(
    issueText({ key: "measure.uncalibrated", message: "no microphone calibration: levels are relative dB" }, title, name),
    "No microphone calibration: levels are relative dB",
  );
  // A message that does not repeat it keeps the title in front.
  assert.equal(issueText({ key: "measure.uncalibrated", message: "levels are relative" }, title, name), "No microphone calibration. Levels are relative");
});

test("WEB7: Propose from the Device chooses the proposed outputs with their ways, the rest left out; none proposed, the first two", () => {
  const d = { ...initialDraft([0, 1, 2, 3]), placement: "listeningPosition" as const };
  const p = withProposal(d, {
    outputs: [
      { output: 2, role: "high", speaker: "L", group: 1 },
      { output: 3, role: "low", speaker: "L", group: 20 },
    ],
  });
  assert.deepEqual(
    p.outputs.map((o) => [o.output, o.on, o.role, o.speaker, o.group]),
    [
      [0, false, "full", "", 0],
      [1, false, "full", "", 0],
      [2, true, "high", "L", 1],
      [3, true, "low", "L", 15],
    ],
  );
  assert.equal(p.placement, "listeningPosition", "the rest of the setup is kept");
  const none = withProposal({ ...d, outputs: d.outputs.map((o) => ({ ...o, on: false })) }, { outputs: [] });
  assert.deepEqual(none.outputs.map((o) => o.on), [true, true, false, false]);
});

test("WEB7: the measured graph's dB grid steps by 5 or 10 dB and no label sits on another or on the frequencies' row (REVE D4)", () => {
  assert.deepEqual(dbTicks(-31, 7), [-30, -25, -20, -15, -10, -5, 0, 5]);
  assert.deepEqual(dbTicks(-60, 6), [-60, -50, -40, -30, -20, -10, 0]);
  assert.deepEqual(dbTicks(5, 5), []);
  // 300 px high, 0 at the top: 66 dB over 300 px, so 10 dB is 45 px; the bottom row (above 276) keeps no label.
  const y = (v: number) => 300 - ((v + 60) / 66) * 300;
  const labels = dbLabels(dbTicks(-60, 6), y, 15, 276);
  assert.ok(labels.every(([, ly]) => ly >= 15 && ly <= 276));
  assert.ok(!labels.some(([v]) => v === -60), "the bottom tick's label would collide with the Hz labels");
  for (let i = 1; i < labels.length; i++) assert.ok(labels[i]![1] - labels[i - 1]![1] <= -12 || labels[i - 1]![1] - labels[i]![1] >= 12);
  // Tight ticks: every other label is dropped rather than drawn over its neighbour.
  const tight = dbLabels([0, 1, 2, 3], (v) => 100 - v * 5, 0, 300);
  assert.deepEqual(tight.map(([v]) => v), [0, 3]);
});

test("WEB7: a name with the core's ISO time reads in local words; other names as they are (REVE F17)", () => {
  const f = (d: Date) => `at ${d.toISOString().slice(0, 16)}`;
  assert.equal(displayName("Measurement 2026-11-24T21:32:00Z", f), "Measurement at 2026-11-24T21:32");
  assert.equal(displayName("2026-11-24T21:32:05.123Z", f), "at 2026-11-24T21:32");
  assert.equal(displayName("Unreliable copy", f), "Unreliable copy");
  assert.equal(displayName("Room 2026-13-45T99:99Z", f), "Room 2026-13-45T99:99Z", "not a time: kept");
  assert.doesNotMatch(displayName("Measurement 2026-11-24T21:32:00Z"), /T21:32:00Z/);
});
