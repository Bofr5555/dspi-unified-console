// WP UB7 unit tests (UXP B7, C3): one speaker table. A speaker's name sets the group (the same name
// joins, a new name gets its own), the groups shown by their names, the override undone by
// following the names again; the layout in one line for Measure and Auto-tune; the tune takes the
// speaker layout's roles; the glossary's role words and one word for the suggestion.

import { test } from "node:test";
import assert from "node:assert/strict";
import { ROLE_WORDS, SUGGEST, followNames, groupName, groupsFollowNames, layoutSummary, settleGroup, withRow, withSpeaker, type SpeakerLayoutRecord } from "../../src/screens/settings/speakers/model";
import { withSpeakerRoles, type TuneLayoutRecord } from "../../src/screens/measure/tune-model";

const twoWay = (): SpeakerLayoutRecord => ({
  serial: "S",
  confirmedByUser: true,
  updatedMs: 1,
  notes: [],
  outputs: [
    { output: 0, role: "low", speaker: "L", group: 0 },
    { output: 1, role: "high", speaker: "L", group: 0 },
    { output: 2, role: "low", speaker: "R", group: 1 },
    { output: 3, role: "high", speaker: "R", group: 1 },
  ],
});
const groupOf = (l: SpeakerLayoutRecord, o: number) => l.outputs.find((x) => x.output === o)!.group;
const label = (o: number) => `OUT${o + 1}`;

test("UB7: a speaker named like another output's joins its group, whatever the case", () => {
  const l = withSpeaker(twoWay(), 3, "l");
  assert.equal(groupOf(l, 3), 0);
  assert.ok(groupsFollowNames(l));
});

test("UB7: a new name leaves a group it shared for a group of its own", () => {
  const l = withSpeaker(twoWay(), 1, "Sub");
  assert.notEqual(groupOf(l, 1), 0);
  assert.notEqual(groupOf(l, 1), 1);
  assert.ok(groupsFollowNames(l));
  // The output alone in its group keeps it when renamed
  const alone = withSpeaker(l, 1, "Sub 2");
  assert.equal(groupOf(alone, 1), groupOf(l, 1));
});

test("UB7: an unnamed output keeps its group; an output taken in joins its name's", () => {
  const l = withSpeaker(twoWay(), 2, "");
  assert.equal(groupOf(l, 2), 1);
  const added = settleGroup(withRow(twoWay(), 4, { speaker: "R", group: 0 }), 4);
  assert.equal(groupOf(added, 4), 1);
});

test("UB7: groups by hand are seen, and following the names again undoes them", () => {
  const byHand = withRow(twoWay(), 2, { group: 0 });
  assert.equal(groupsFollowNames(byHand), false);
  assert.equal(groupName(byHand, 0, label), "L + R");
  const back = followNames(byHand);
  assert.ok(groupsFollowNames(back));
  assert.notEqual(groupOf(back, 2), groupOf(back, 0));
  assert.equal(groupOf(back, 2), groupOf(back, 3));
  const as = twoWay();
  assert.equal(followNames(as), as); // nothing to change: the same layout
});

test("UB7: measured together reads as the names, an unnamed output by its label", () => {
  assert.equal(groupName(twoWay(), 1, label), "L");
  const l = withRow(twoWay(), 4, { speaker: "", group: 5 });
  assert.equal(groupName(l, 4, label), "OUT5");
  assert.equal(groupName(l, 7, label), "");
});

test("UB7: the layout in one line, with whether it is confirmed", () => {
  assert.equal(layoutSummary(twoWay(), true), "L: woofer Out 1, tweeter Out 2 · R: woofer Out 3, tweeter Out 4 · confirmed");
  assert.ok(layoutSummary({ ...twoWay(), confirmedByUser: false }, true).endsWith(" · not confirmed"));
  assert.ok(layoutSummary(twoWay(), false).endsWith(" · suggested, not confirmed"));
  const sub = withRow(twoWay(), 4, { role: "sub", speaker: "", group: 2 });
  assert.ok(layoutSummary(sub, true).includes(" · subwoofer Out 5 · "));
  assert.equal(layoutSummary({ ...twoWay(), outputs: [] }, true), "No output is in the speaker layout yet.");
  assert.equal(layoutSummary(null, false), "");
});

test("UB7: the tune takes the speaker layout's roles, speakers and groups", () => {
  const tune: TuneLayoutRecord = {
    template: "",
    notes: [],
    outputs: [
      { output: 0, role: "full", speaker: "A", group: 3, name: "", feedGainDb: 0, feedInverted: false },
      { output: 1, role: "full", speaker: "A", group: 3, name: "", feedGainDb: 0, feedInverted: false },
      { output: 5, role: "full", speaker: "", group: 0, name: "", feedGainDb: 0, feedInverted: false },
    ],
  };
  const next = withSpeakerRoles(tune, twoWay());
  assert.deepEqual(
    next.outputs.map((o) => [o.output, o.role, o.speaker, o.group]),
    [
      [0, "low", "L", 0],
      [1, "high", "L", 0],
      [5, "full", "", 0],
    ],
  );
  assert.equal(withSpeakerRoles(next, twoWay()), next);
});

test("UB7: the glossary's role words and one word for the suggestion", () => {
  assert.deepEqual(Object.values(ROLE_WORDS), ["Full range", "Subwoofer", "Woofer", "Mid driver", "Tweeter"]);
  assert.equal(SUGGEST, "Suggest from Routing and Filters");
});
