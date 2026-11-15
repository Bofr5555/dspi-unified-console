// WP WB6i unit tests: the Settings page's own rules (the console's preferences, kept and
// checked; the help links; default channel names from the core's slot names; the latency's
// words; the pins the state lists; the core-voltage texts; the manifest's gates), and on the
// real core (as main.test.ts): the device name and the latency through `write`, Reset
// Channel Names as one undo step, the core-voltage flow's view and its refusals (a voltage
// write only against the demo device), and the Settings screen in every mode, last.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  DEFAULT_PREFS,
  HELP_LINKS,
  NO_PIN,
  PrefsStore,
  dbSpan,
  defaultChannelNames,
  deviceNameValue,
  durationText,
  latencyCaption,
  latencyChoices,
  latencyMs,
  loadPrefs,
  nameWrites,
  operationIn,
  pinRows,
  pinText,
  pinValue,
  readoutText,
  resetChannelNames,
  savePrefs,
  screenIn,
  soakMinutes,
  tidyPrefs,
  voltsText,
} from "../../src/screens/settings/model";
import { screensFor } from "../../src/ui/screens";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import type { DspState, OutputLabel, OutputLatencyRecord } from "../../src/core/protocol";
import { Session } from "../../src/ui/session";
import manifest from "../../../../assets/modes/modes.json";

function memory() {
  const m = new Map<string, string>();
  return { map: m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

test("WB6I prefs: defaults, every field checked, kept in this page's storage; a refused storage throws nothing", () => {
  assert.deepEqual(tidyPrefs(null), DEFAULT_PREFS);
  assert.deepEqual(tidyPrefs("nonsense"), DEFAULT_PREFS);
  const t = tidyPrefs({ graph: { lineWidth: 9, dbRange: 3, dbCenter: 7.6, minFreq: 33, maxFreq: 10000, showGlow: "yes" }, spectrum: { fftOrder: 12, floorDb: -120 }, debug: true });
  assert.equal(t.graph.lineWidth, 4, "clamped");
  assert.equal(t.graph.dbRange, 10, "clamped");
  assert.equal(t.graph.dbCenter, 8, "whole dB");
  assert.equal(t.graph.minFreq, 20, "not a choice: the default");
  assert.equal(t.graph.maxFreq, 10000);
  assert.equal(t.graph.showGlow, false, "not a boolean: the default");
  assert.equal(t.spectrum.fftOrder, DEFAULT_PREFS.spectrum.fftOrder);
  assert.equal(t.spectrum.floorDb, -120);
  assert.equal(t.debug, true);

  const st = memory();
  savePrefs(st, t);
  assert.deepEqual(loadPrefs(st), t);
  st.setItem("dspi-web.prefs", "{broken");
  assert.deepEqual(loadPrefs(st), DEFAULT_PREFS);
  assert.deepEqual(loadPrefs(null), DEFAULT_PREFS);
  const refusing = { getItem: () => { throw new Error("denied"); }, setItem: () => { throw new Error("quota"); } };
  assert.deepEqual(loadPrefs(refusing), DEFAULT_PREFS);
  savePrefs(refusing, t);
});

test("WB6I prefs store: a change reaches every watcher at once and is kept; Restore Defaults per page", () => {
  const st = memory();
  const store = new PrefsStore(st);
  let seen = 0;
  const stop = store.watch(() => seen++);
  store.set({ graph: { showDbGrid: false, minFreq: 50 } });
  assert.equal(seen, 1);
  assert.equal(store.prefs.graph.showDbGrid, false);
  assert.equal(store.prefs.graph.showFreqGrid, true, "the rest kept");
  store.set({ spectrum: { avgMs: 300 }, debug: true });
  assert.equal(new PrefsStore(st).prefs.spectrum.avgMs, 300, "kept for the next page");
  assert.equal(new PrefsStore(st).prefs.debug, true);
  store.reset("graph");
  assert.deepEqual(store.prefs.graph, DEFAULT_PREFS.graph);
  assert.equal(store.prefs.spectrum.avgMs, 300, "the other page untouched");
  stop();
  store.set({ debug: false });
  assert.equal(seen, 3, "no call after stop");
});

test("WB6I graphing: the vertical span from range and centre; the readout as asked", () => {
  assert.deepEqual(dbSpan(DEFAULT_PREFS.graph), [-25, 25], "the graph as before");
  assert.deepEqual(dbSpan({ ...DEFAULT_PREFS.graph, dbRange: 40, dbCenter: -10 }), [-30, 10]);
  const g = DEFAULT_PREFS.graph;
  assert.equal(readoutText(g, 1250, -3.46), "1.25 kHz  -3.5 dB");
  assert.equal(readoutText(g, 63.4, 2), "63 Hz  +2.0 dB");
  assert.equal(readoutText({ ...g, showLevelReadout: false }, 12500, 0), "12.5 kHz");
  assert.equal(readoutText({ ...g, showFreqReadout: false, showLevelReadout: false }, 100, 0), "");
});

test("WB6I help links: this console's project and the firmware, both https", () => {
  assert.deepEqual(
    HELP_LINKS.map((l) => l.title),
    ["DSPi Unified Console on GitHub", "DSPi Firmware on GitHub"],
  );
  for (const l of HELP_LINKS) assert.match(l.url, /^https:\/\/github\.com\//);
});

const label = (output: number, slotName: string) => ({ output, slotName }) as OutputLabel;

test("WB6I names: the defaults from the core's slot names; only changed channels written; the device name's limit", () => {
  const s = { numInputChannels: 3, numOutputChannels: 3 };
  assert.deepEqual(defaultChannelNames(s, [label(0, "SPDIF 1 L"), label(1, "I2S 1 R"), label(2, "PDM")]), ["USB L", "USB R", "USB 3", "SPDIF 1 L", "I2S 1 R", "PDM"]);
  assert.equal(defaultChannelNames(s, undefined), null, "no labels yet: nothing guessed");
  assert.equal(defaultChannelNames(s, [label(0, "SPDIF 1 L")]), null);
  assert.deepEqual(nameWrites(["USB L", "Vocal"], ["USB L", "USB R"]), [{ path: "ch.name", indices: [1], value: { text: "USB R" } }]);
  assert.equal(deviceNameValue("  Stage left "), "Stage left");
  assert.equal(deviceNameValue("x".repeat(31)), "x".repeat(31));
  assert.equal(deviceNameValue("x".repeat(32)), null);
  assert.equal(deviceNameValue("å".repeat(16)), null, "bytes, not characters");
});

test("WB6I latency: the device's options, the default marked, the caption's notes", () => {
  const r: OutputLatencyRecord = {
    supported: true,
    requestedId: 5,
    effectiveId: 5,
    effectiveUs: 4000,
    pending: false,
    options: [
      { id: 1, streamOnly: true, available: false, latencyUs: 667, batchUs: 0, effectiveUs: 2000 },
      { id: 5, streamOnly: false, available: true, latencyUs: 4000, batchUs: 0, effectiveUs: 3000 },
    ],
  };
  assert.equal(latencyMs(667), "0.67 ms");
  assert.equal(latencyMs(4000), "4 ms");
  assert.deepEqual(latencyChoices(r), [
    { id: 1, text: "0.67 ms", available: false },
    { id: 5, text: "4 ms (default)", available: true },
  ]);
  assert.deepEqual(latencyChoices({ ...r, requestedId: 7 }).at(-1), { id: 7, text: "-", available: false }, "an unknown request kept");
  assert.match(latencyCaption(r), /runs as 3 ms/);
  assert.match(latencyCaption(r), /no effect upon USB Audio\.$/);
  assert.match(latencyCaption({ ...r, pending: true }), /still applying/);
});

test("WB6I pins: every pin the state lists, with the path that moves it; nothing judged", () => {
  const s = {
    numPinOutputs: 2,
    outputPins: [6, 7, 8],
    outputTypes: [0, 1],
    i2sBckPin: 14,
    i2sBckPinSlave: 16,
    i2sClockPinMode: 0,
    mckEnabled: true,
    mckPin: 21,
    adatOutEnabled: false,
    adatOutPin: 3,
    spdifRxPins: [11, 12],
    spdifInputsEnabled: 1,
    i2sRxPins: [18, 19],
    i2sInputChannels: 2,
    adatInputEnabled: true,
    adatInputPin: NO_PIN,
  } as unknown as DspState;
  const rows = pinRows(s);
  assert.deepEqual(
    rows.map((r) => [r.title, r.path, r.indices, r.pin]),
    [
      ["Output 1 (S/PDIF)", "out.pin", [0], 6],
      ["Output 2 (I2S)", "out.pin", [1], 7],
      ["I2S bit clock (word clock on the next pin)", "i2s.bck", [], 14],
      ["Master clock", "i2s.mck.pin", [], 21],
      ["S/PDIF input 1", "in.spdif.pin", [0], 11],
      ["I2S input pair 1", "in.i2s.pin", [0], 18],
      ["ADAT input", "in.adat.pin", [], NO_PIN],
    ],
  );
  assert.deepEqual(pinRows({} as DspState), [], "a state without pins: none");
  assert.equal(pinText(NO_PIN), "none");
  assert.equal(pinText(4), "GPIO 4");
  assert.deepEqual(pinValue(" 12 "), { int: 12 });
  assert.deepEqual(pinValue("99"), { int: 99 }, "the core judges the range");
  assert.equal(pinValue("1.5"), null);
  assert.equal(pinValue(""), null);
});

test("WB6I core voltage: volts, durations, the soak choices from the core's minimum", () => {
  assert.equal(voltsText(1200), "1.20 V");
  assert.equal(voltsText(0), "-");
  assert.equal(durationText(1800), "30 min 0 s");
  assert.equal(durationText(3 * 3600 + 120), "3 h 2 min");
  assert.equal(durationText(90000), "1 d 1 h");
  assert.deepEqual(soakMinutes(600, 1800), [10, 30, 60, 120, 240]);
  assert.deepEqual(soakMinutes(3700, 1800), [62, 120, 240], "nothing under the minimum");
});

test("WB6I gates: Core Voltage in the Installer mode, Pins with the hardware screen, from the core's manifest", () => {
  assert.equal(operationIn(manifest, "core_voltage", "installer"), true);
  assert.equal(operationIn(manifest, "core_voltage", "advanced"), false);
  assert.equal(operationIn(manifest, "core_voltage", "simple"), false);
  assert.equal(operationIn(null, "core_voltage", "installer"), false, "without the manifest: not shown");
  assert.equal(screenIn(manifest, "hardware", "advanced"), true);
  assert.equal(screenIn(manifest, "hardware", "simple"), false);
  for (const m of ["simple", "advanced", "installer"] as const) {
    assert.equal(screensFor(m, false).at(-1)?.id, "settings", m);
    assert.equal(screensFor(m, true).at(-1)?.id, "settings", m);
  }
});

// ---------------------------------------------------------------- on the real core

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(): Promise<Session> {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const client = {
    call: (m: never, p: never) => core.call(m, p),
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo" });
  return session;
}

test("WB6I device name and latency: written through the core, read back", { skip }, async () => {
  const s = await sessionOn();
  assert.equal((await s.core.call("deviceName", undefined)).supported, true);
  assert.equal(await s.write("dev.name", { text: "Stage left" }), true);
  assert.equal(s.state.snapshot?.device.name, "Stage left");
  const l = await s.core.call("outputLatency", undefined);
  assert.equal(l.supported, true);
  const other = l.options.find((o) => o.id !== l.requestedId)!;
  assert.equal(await s.write("out.latency", { choice: other.id }), true);
  assert.equal((await s.core.call("outputLatency", undefined)).requestedId, other.id);
  // A pin that is not a GPIO at all: the core refuses it in its own words.
  assert.equal(await s.write("out.pin", { int: 99 }, [0]), false);
  assert.notEqual(s.state.error, "");
});

test("WB6I Reset Channel Names: every channel to its default, one undo step", { skip }, async () => {
  const s = await sessionOn();
  const snap = s.state.snapshot!;
  const inputs = snap.state.numInputChannels;
  assert.equal(await s.write("ch.name", { text: "Vocal" }, [0]), true);
  assert.equal(await s.write("ch.name", { text: "Sub" }, [inputs]), true);
  assert.equal(await resetChannelNames(s), true);
  const names = s.state.snapshot!.state.channelNames;
  assert.equal(names[0], "USB L");
  assert.equal(names[inputs], snap.outputLabels?.find((x) => x.output === 0)?.slotName);
  await s.undo();
  const back = (await s.core.call("state", undefined)).state.channelNames;
  assert.equal(back[0], "Vocal", "one step restores both");
  assert.equal(back[inputs], "Sub");
  // Nothing to change: no write, still true.
  await s.redo();
  await s.refresh();
  assert.equal(await resetChannelNames(s), true);
});

test("WB6I core voltage on the demo device: the core's view, its word and token refusals", { skip }, async () => {
  const s = await sessionOn();
  const v = await s.core.call("vregPoll", undefined);
  assert.equal(v.available, true);
  assert.equal(v.confirmationWord, "OVERVOLT");
  assert.ok(v.status && v.status.millivolts > 0);
  const open = await s.core.call("vregOpen", undefined);
  assert.equal(open.phase, "review");
  assert.ok(open.choices.every((c) => c.millivolts <= 1250), "the GUI's cap is the core's");
  const high = open.choices.find((c) => c.needsWord);
  if (high) {
    await assert.rejects(s.core.call("vregTrial", { value: high.code }), (e: { code?: string }) => e.code === "overvolt_word");
    await assert.rejects(s.core.call("vregTrial", { value: high.code, typed: "overvolt" }), (e: { code?: string }) => e.code === "overvolt_word", "exactly the word");
  }
  await assert.rejects(s.core.call("vregSet", { value: 0 }), (e: { code?: string }) => e.code === "confirmation_required", "a save needs its token");
  const token = await s.token("vreg_save");
  assert.ok(token);
  const after = await s.core.call("vregSet", { value: 0, token, typed: open.resetNeedsWord ? "OVERVOLT" : undefined });
  assert.equal(after.lastError, null);
  await s.core.call("vregClose", undefined).catch(() => undefined);
});
