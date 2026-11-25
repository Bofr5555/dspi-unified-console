// WP UWN unit tests (UXP A15, A16, A17, C7): the web's names, recovery and crossover words.

import { test } from "node:test";
import assert from "node:assert/strict";
import { bandTypeName } from "../../src/screens/eq/BandList";
import { T, setTypeNames } from "../../src/screens/eq/model";
import { saveQuestion } from "../../src/ui/screens";
import { connectionWord } from "../../src/screens/system/StatusRail";

test("UWN A17: the band table says the inspector's shapes and slopes, not the core's Peaking / Low Cut (12dB)", () => {
  setTypeNames([
    { code: 0, name: "Off" },
    { code: 1, name: "Peaking" },
    { code: 5, name: "Low Cut (12dB)" },
    { code: 11, name: "Linkwitz Transform" },
  ]);
  assert.equal(bandTypeName(T.peaking), "Bell");
  assert.equal(bandTypeName(T.highPass), "Low Cut 12 dB/oct");
  assert.equal(bandTypeName(T.highPass1), "Low Cut 6 dB/oct");
  assert.equal(bandTypeName(T.lowPass), "High Cut 12 dB/oct");
  assert.equal(bandTypeName(T.lowShelf1), "Low Shelf 6 dB/oct");
  assert.equal(bandTypeName(T.notch), "Notch");
  assert.equal(bandTypeName(T.allPass), "All Pass 2nd Order");
  assert.equal(bandTypeName(T.allPass1), "All Pass 1st Order");
  // Off and the Linkwitz Transform keep the core's names
  assert.equal(bandTypeName(T.flat), "Off");
  assert.equal(bandTypeName(T.linkwitz), "Linkwitz Transform");
});

test("UWN A17: Save to Device names the preset it writes, not a slot number", () => {
  const s = { activePresetSlot: 0, presetNames: ["Demo", ""], presetOccupied: 0b1 };
  assert.equal(saveQuestion(s), "Save the current settings to preset 1: Demo?");
  assert.equal(saveQuestion({ ...s, activePresetSlot: 1 }), "Save the current settings to preset 2, empty until now?");
  assert.equal(saveQuestion({ ...s, presetNames: ["  "] }), "Save the current settings to preset 1: Preset 1?");
});

test("UWN A16: the status rail's words and dot come from one state: a link that is away reads Reconnecting", () => {
  const bound = { phase: "bound" as const };
  assert.equal(connectionWord({ status: "open", link: null }), "Connected");
  assert.equal(connectionWord({ status: "open", link: bound }), "Connected");
  assert.equal(connectionWord({ status: "open", link: { phase: "reconnecting" } }), "Reconnecting");
  assert.equal(connectionWord({ status: "choosing", link: { phase: "choose_device" } }), "Connecting");
  assert.equal(connectionWord({ status: "failed", link: null }), "Not connected");
});
