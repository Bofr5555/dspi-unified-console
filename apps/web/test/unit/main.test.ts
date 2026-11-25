// WP WB6a unit tests: the main window's arithmetic (the Console's tapers, slot
// labels, the channel layout, the prompt's summary), and on the real core the
// session's write gate and the preset calls.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import {
  MASTER_STEPS,
  USER_STEPS,
  changeSummary,
  inputRows,
  liveInputs,
  masterToPos,
  meterHidden,
  meterNames,
  outputRows,
  posToMaster,
  posToUser,
  presetLabel,
  userToPos,
  volumeText,
} from "../../src/screens/main/model";
import type { DspState, OutputLabel } from "../../src/core/protocol";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { tapNames } from "../../src/screens/analysis/SpectrumStrip";

test("WB6a: the user taper is the Console's square root over -60..0 dB", () => {
  assert.equal(userToPos(0), USER_STEPS);
  assert.equal(userToPos(-60), 0);
  assert.equal(userToPos(-80), 0);
  // -45 dB is a quarter of the way in dB, a sixteenth of the travel.
  assert.equal(posToUser(USER_STEPS / 16), -45);
  assert.equal(posToUser(USER_STEPS / 4), -30);
  for (const db of [-42.3, -12.5, -0.1]) assert.ok(Math.abs(posToUser(userToPos(db)) - db) <= 0.3, `${db}`);
});

test("WB6a: the master taper is the firmware's steps: 0.1 dB to -10, 0.5 to -40, 1 dB to -128", () => {
  assert.equal(masterToPos(0), MASTER_STEPS);
  assert.equal(masterToPos(-10), MASTER_STEPS - 100);
  assert.equal(masterToPos(-40), MASTER_STEPS - 160);
  assert.equal(masterToPos(-128), 0);
  for (let pos = 0; pos <= MASTER_STEPS; pos++) assert.equal(masterToPos(posToMaster(pos)), pos);
  assert.equal(posToMaster(MASTER_STEPS - 1), -0.1);
  assert.equal(posToMaster(MASTER_STEPS - 101), -10.5);
  assert.equal(volumeText(-128, true), "−∞");
  assert.equal(volumeText(-128, false), "-128.0 dB");
  assert.equal(volumeText(-12.5, false), "-12.5 dB");
});

const state = {
  numInputChannels: 2,
  numOutputChannels: 4,
  channelNames: ["USB 1", "USB 2", "Out A", "Out B", "Out C", "Out D"],
  outputEnabled: [true, false, true, true],
  presetNames: ["Living room", "", "", ""],
  presetOccupied: 0b0101,
} as unknown as DspState;

test("WB6a: slots are labelled as the Console's picker: name, default name, Empty", () => {
  assert.equal(presetLabel(state, 0), "1: Living room");
  assert.equal(presetLabel(state, 1), "2: Empty");
  assert.equal(presetLabel(state, 2), "3: Preset 3");
});

test("WB6a: the live inputs are the Console's: by source, USB's from the status, 2 to the device's count", () => {
  const s = (inputSource: number, i2sInputChannels = 6) => ({ inputSource, i2sInputChannels, numInputChannels: 8 });
  assert.equal(liveInputs(s(0), null), 2);
  assert.equal(liveInputs(s(0), 6), 6);
  assert.equal(liveInputs(s(0), 12), 8);
  assert.equal(liveInputs(s(0), 0), 2);
  assert.equal(liveInputs(s(1), 8), 2);
  assert.equal(liveInputs(s(5), 8), 2);
  assert.equal(liveInputs(s(2), 8), 6);
  assert.equal(liveInputs(s(3), 2), 8);
});

test("WB6a: inputs are the first wire channels; only enabled outputs are listed, by their own index", () => {
  assert.deepEqual(
    inputRows(state, 2).map((r) => [r.wire, r.index, r.descriptor, r.name]),
    [
      [0, 0, "IN1", "USB 1"],
      [1, 1, "IN2", "USB 2"],
    ],
  );
  assert.deepEqual(
    outputRows(state).map((r) => [r.wire, r.index, r.descriptor, r.name]),
    [
      [2, 0, "OUT1", "Out A"],
      [4, 2, "OUT3", "Out C"],
      [5, 3, "OUT4", "Out D"],
    ],
  );
});

test("WB6a: the unsaved-changes prompt lists at most 15 lines, the last says how many more", () => {
  const few = ["a", "b"];
  assert.deepEqual(changeSummary(few), few);
  const many = Array.from({ length: 20 }, (_, i) => `change ${i}`);
  const s = changeSummary(many);
  assert.equal(s.length, 15);
  assert.equal(s[13], "change 13");
  assert.equal(s[14], "…and 6 more changes");
});

// -- The session's gate and the preset calls, on the real core (as wasm.test.ts) --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

/** A Session over a core in this process, counting the calls that reach it. */
async function sessionOn(access: "control" | "view") {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const calls: string[] = [];
  const client = {
    call: (m: never, p: never) => {
      calls.push(m);
      return core.call(m, p);
    },
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  return { session: new Session("s", "i", "Here", { kind: "demo" }, access, client), calls };
}

test("WB6a: the demo opens with its stored presets read, and nothing unsaved", { skip }, async () => {
  const { session } = await sessionOn("control");
  await session.open({ kind: "demo" });
  const st = session.state.snapshot!.state;
  assert.equal(st.presetOccupied & 1, 1, "slot 1 holds what the demo runs");
  assert.equal(st.presetNames[0], "Demo");
  assert.deepEqual(await session.core.call("presetChanges", undefined), { dirty: false, changes: [] });
  assert.equal(await session.write("out.gain", { float: -6 }, [0]), true);
  assert.equal((await session.core.call("presetChanges", undefined)).dirty, true);
  assert.equal(await session.change("presetRevert", undefined), true);
  assert.equal(session.state.snapshot!.state.outputGainDb[0], 0);
});

test("WB6a: read-only firmware and a viewer write nothing: the session refuses before the core", { skip }, async () => {
  for (const [access, target, reason] of [
    ["control", { kind: "demo", readOnly: true }, "the connected firmware is read-only in this version"],
    ["view", { kind: "demo" }, "You can view this DSPi but not change it"],
  ] as const) {
    const { session, calls } = await sessionOn(access);
    await session.open(target);
    assert.equal(session.readOnlyReason, reason);
    const before = calls.length;
    assert.equal(await session.write("vol.user", { float: -10 }), false);
    assert.equal(await session.change("factoryReset", undefined), false);
    assert.equal(await session.change("presetCopy", { source: 0, dest: 1 }), false);
    assert.equal(calls.length, before, "no call reached the core");
    assert.equal(session.state.error, reason);
  }
});

// -- BUG1 --
test("BUG1: outputs show the core's labels, keep their stored name for a rename and their slot for a tooltip", () => {
  // The K340 (tests/fixtures/preset/k340-bug1.dspipreset): the core's labels for its four outputs
  const k340 = {
    numChannels: 6,
    numInputChannels: 2,
    numOutputChannels: 4,
    channelNames: ["USB 1", "USB 2", "I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"],
    outputEnabled: [true, true, true, true],
  } as unknown as DspState;
  const label = (output: number, l: string, slotName: string): OutputLabel => ({ output, label: l, shortLabel: "", slotName, userNamed: false, channel: "", band: "", unused: false });
  const labels = [label(0, "Out 1 · R · Low", "I2S 1 L"), label(1, "Out 2 · R · High", "I2S 1 R"), label(2, "Out 3 · L · Low", "I2S 2 L"), label(3, "Out 4 · L · High", "I2S 2 R")];
  assert.deepEqual(
    outputRows(k340, labels).map((r) => [r.descriptor, r.name, r.stored, r.detail]),
    [
      ["OUT1", "Out 1 · R · Low", "I2S 1 L", "I2S 1 L"],
      ["OUT2", "Out 2 · R · High", "I2S 1 R", "I2S 1 R"],
      ["OUT3", "Out 3 · L · Low", "I2S 2 L", "I2S 2 L"],
      ["OUT4", "Out 4 · L · High", "I2S 2 R", "I2S 2 R"],
    ],
  );
  assert.deepEqual(meterNames(k340, labels), ["USB 1", "USB 2", "Out 1 · R · Low", "Out 2 · R · High", "Out 3 · L · Low", "Out 4 · L · High"]);
  // Without the core's labels (a stub, an older host): the stored names
  assert.deepEqual(meterNames(k340), ["USB 1", "USB 2", "I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"]);
  assert.deepEqual(outputRows(k340).map((r) => r.name), ["I2S 1 L", "I2S 1 R", "I2S 2 L", "I2S 2 R"]);
});


// -- WP INTE -- (owner 2026-11-24) the meters and the Spectrum strip are views: an unused output leaves them
test("INTE: an output nothing is routed to has no meter and no Spectrum chip", () => {
  const s = {
    numChannels: 5,
    numInputChannels: 2,
    numOutputChannels: 3,
    channelNames: ["USB 1", "USB 2", "Out 1", "Out 2", "Out 3"],
    outputEnabled: [true, true, true],
  } as unknown as DspState;
  const label = (output: number, l: string, unused: boolean): OutputLabel => ({ output, label: l, shortLabel: "", slotName: `Out ${output + 1}`, userNamed: false, channel: "", band: "", unused });
  const labels = [label(0, "Out 1 · L", false), label(1, "Out 2 · R", false), label(2, "3 · None", true)];
  assert.deepEqual(meterHidden(s, labels), [false, false, false, false, true]);
  assert.deepEqual(meterHidden(s), [false, false, false, false, false]);
  assert.deepEqual(tapNames(s, labels).unused, [false, false, true]);
});
