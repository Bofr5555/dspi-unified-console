// WP WEB7 unit test: the status rail's alarm word (REVE D5).

import { test } from "node:test";
import assert from "node:assert/strict";
import { railAlarmWord } from "../../src/screens/system/model";

test("WEB7: the rail never says \"No hub alarms\" while the hub's banner counts some (REVE D5)", () => {
  // No installation: the hub's own device alarms, as the banner counts them
  assert.equal(railAlarmWord(false, 0, 2), "2 hub alarms");
  assert.equal(railAlarmWord(false, 0, 1), "1 hub alarm");
  assert.equal(railAlarmWord(false, 0, 0), "No hub alarms");
  assert.equal(railAlarmWord(false, 0, null), "No hub alarms");
  // An installation: its own alarms, as before
  assert.equal(railAlarmWord(true, 0, 5), "No alarms");
  assert.equal(railAlarmWord(true, 3, 0), "3 alarms");
  assert.equal(railAlarmWord(true, 1, null), "1 alarm");
});
