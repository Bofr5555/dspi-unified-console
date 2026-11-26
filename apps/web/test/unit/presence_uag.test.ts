// WP UAG unit tests (UXP A7): the web's AI agent strip names another device of the hub as its
// inventory does, not by its serial, and says where Pause and Stop are.

import { test } from "node:test";
import assert from "node:assert/strict";
import { nameFrom, presenceText, stopHint } from "../../src/ui/presence";

const row = { kind: "agent", label: "Claude", by: "Studio Mac", serial: "E66-1", what: "Changing the gain", ttlS: 20 };

test("wp_uag another device of the hub by its inventory's name, else its serial", () => {
  const name = nameFrom(new Map([["E66-1", "Main left"], ["E66-2", "‮"]]));
  assert.equal(presenceText(row, name, "S-A"), "Claude, through Studio Mac, is working on Main left: Changing the gain");
  assert.equal(name("E66-2"), "E66-2");
  assert.equal(name("E66-3"), "E66-3");
});

test("wp_uag the strip says where Pause and Stop are, and what Mute here does", () => {
  assert.equal(stopHint("Studio Mac"), "Pause and Stop are in the console on Studio Mac; Mute here silences this device.");
  assert.equal(stopHint(""), "Pause and Stop are in the agent's console; Mute here silences this device.");
});
