// WP AUSB1 unit tests: USB-direct units in the System view's logic (their lock in
// words, the statistics line, the topology beside the audio link's tree, the mode
// edit, the alignment's words), and on the real core the USB-direct demo's calls.

import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import {
  NO_USB_DIRECT,
  endWord,
  isUsbDirect,
  pathWord,
  roomsView,
  senderChannels,
  topology,
  usbStateWord,
  usbStats,
  withUsbMode,
  writeWhat,
  type SystemModel,
  type SystemStatus,
  type UsbSeen,
} from "../../src/screens/system/model";

function seen(over: Partial<UsbSeen> = {}): UsbSeen {
  return {
    mode: "phase_lock", d: 192, autoReanchor: true, state: "phase_locked", phaseState: 2, rateCode: 1,
    hostPpm: -11.5, vcxoPpm: -11.46, phaseError: 0.08, busFrame: 900, sample: 48 * 6000, dMin: 170, dMax: 399,
    fill: 127, stream: true, holdover: false, streamStartBusFrame: 10, streamShift: 0, gapCount: 1, anchors: 1,
    reanchors: 0, missedSamples: 96, lateSamples: 0,
    diag: { lastCause: "sof_lost", ends: 2, holdovers: 3, rides: 3, suspends: 0, busResets: 1, unplugs: 0, longestRideUs: 300000 },
    ...over,
  };
}

function model(): SystemModel {
  return {
    rooms: [{ id: "main", name: "Main room" }],
    zones: [], groups: [], sources: [{ id: "music", name: "Music", sender: "tx", channels: [0, 1] }],
    senders: [{ id: "tx", name: "Sender", serial: "TX" }],
    fanOuts: [],
    units: [
      { id: "A", name: "Main left", serial: "A", room: "main", upstream: "tx", slots: [0, 1, 255, 255, 255, 255, 255, 255] },
      { id: "R", name: "Rear L/R", serial: "R", room: "main", upstream: "", transport: "usb_direct", usbMode: "phase_lock" },
      { id: "H", name: "Height", serial: "H", room: "main", upstream: "", transport: "usb_direct" },
    ],
    scenes: [],
  };
}

function status(): SystemStatus {
  return {
    revision: 1,
    observed: {
      TX: { online: true, role: "sender", alink: "present" },
      A: { online: true, role: "receiver", state: "locked", alink: "present" },
      R: { online: true, usbDirect: "present", usb: seen() },
      H: { online: true, usbDirect: "none" },
    },
    alignment: { dFrames: 260, dAuto: true, rateHz: 48000, latencyMaxFrames: 0, units: [], issues: [] },
    alarms: [],
  };
}

test("AUSB1: a USB-direct unit says its USB-direct state and statistics, never the audio link's", () => {
  const [room] = roomsView(model(), status());
  const r = room!.units.find((u) => u.id === "R")!;
  assert.equal(r.lock, "Phase-locked");
  assert.equal(r.usb?.stats, "host -11.5 ppm · VCXO -11.5 ppm · phase error +0.08 samples · gaps 1, missed 96, late 0 samples");
  assert.deepEqual(r.slots, []);
  const h = room!.units.find((u) => u.id === "H")!;
  assert.equal(h.lock, NO_USB_DIRECT);
  assert.equal(room!.units.find((u) => u.id === "A")!.usb, undefined);
  // Not phase-locked: no phase error in the line.
  assert.ok(!usbStats(seen({ state: "rate_locked", phaseState: 0 })).includes("phase error"));
});

test("AUSB1: in the topology USB-direct units stand beside the audio link's tree; they take no channels", () => {
  const roots = topology(model(), status());
  const usb = roots.filter((n) => n.kind === "usb_direct").map((n) => n.name);
  assert.deepEqual(usb, ["Rear L/R", "Height"]);
  assert.equal(roots.find((n) => n.id === "tx")!.children.map((c) => c.id).join(), "A");
  assert.equal(roots.find((n) => n.id === "R")!.slots, null);
  assert.deepEqual(senderChannels(model()).slice(0, 2), [0, 1]);
});

test("AUSB1: the mode edit sets or clears usbMode; words", () => {
  const m = model();
  const set = withUsbMode(m, "H", "rate_lock");
  assert.equal(set.units.find((u) => u.id === "H")!.usbMode, "rate_lock");
  const cleared = withUsbMode(set, "R", null);
  assert.ok(!("usbMode" in cleared.units.find((u) => u.id === "R")!));
  assert.equal(m.units.find((u) => u.id === "H")!.usbMode, undefined, "the model is not changed in place");
  assert.ok(isUsbDirect(m.units[1]!) && !isUsbDirect(m.units[0]!));
  assert.equal(usbStateWord("rejected"), "Host out of range");
  assert.equal(usbStateWord("no_sof"), "No USB time base");
  assert.equal(endWord("reanchor_output_reset"), "re-anchor (output reset)");
  assert.equal(endWord(null), "none");
  assert.equal(writeWhat("usb_direct"), "USB-direct mode and delay");
  assert.equal(pathWord("usb_frame"), "at a USB frame");
});

// The core under Node, when the build has made it (apps/web/tools/ci.sh).
const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";

test("AUSB1: on the real core, the USB-direct demo answers its calls; the plain demo has none", { skip }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  await core.call("open", { kind: "demo", usbDirect: true });
  assert.equal(await core.call("usbDirectSupported", undefined), true);
  const s = await core.call("alinkUsbStatus", undefined);
  assert.deepEqual([s.state, s.dMin, s.dMax], ["phase_locked", 170, 399]);
  const c = await core.call("alinkUsb", undefined);
  assert.deepEqual([c.modeName, c.d], ["phase_lock", 192]);
  await core.call("open", { kind: "demo" });
  assert.equal(await core.call("usbDirectSupported", undefined), false);
  await assert.rejects(core.call("alinkUsbStatus", undefined), (e: { code?: string }) => e.code === "unsupported");
  core.dispose();
});
