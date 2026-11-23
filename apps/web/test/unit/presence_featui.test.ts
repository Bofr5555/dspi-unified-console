// WP FEATUI unit tests: the web's AI agent strip (src/ui/presence.ts): an entry in words, the hub's
// texts shown without control and format characters.

import { test } from "node:test";
import assert from "node:assert/strict";
import { cleanPresence, presenceText } from "../../src/ui/presence";

const row = (over: Partial<Parameters<typeof presenceText>[0]> = {}) => ({
  kind: "agent",
  label: "Claude",
  by: "Studio Mac",
  serial: "S-A",
  what: "Changing the gain: OUT1",
  ttlS: 20,
  ...over,
});

test("wp_featui an agent's entry in words: who, through which console, on which device, what", () => {
  const name = (s: string) => (s === "S-B" ? "Bar" : s);
  assert.equal(presenceText(row(), name, "S-A"), "Claude, through Studio Mac, is working on this device: Changing the gain: OUT1");
  assert.equal(presenceText(row({ serial: "S-B" }), name, "S-A"), "Claude, through Studio Mac, is working on Bar: Changing the gain: OUT1");
  assert.equal(presenceText(row({ serial: "", what: "", by: "" }), name, "S-A"), "Claude is working.");
  assert.equal(presenceText(row({ label: "‮" }), name, ""), "An AI agent, through Studio Mac, is working on S-A: Changing the gain: OUT1");
});

test("wp_featui the hub's texts lose control and format characters", () => {
  assert.equal(cleanPresence("Cl‮aude​\n"), "Claude");
  assert.equal(cleanPresence("Källare ✓"), "Källare ✓");
});
