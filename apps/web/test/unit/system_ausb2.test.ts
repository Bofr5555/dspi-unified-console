// WP AUSB2 unit tests: a USB-direct unit's lock-error action in the installation
// the Installer edits, and a unit in lock error as the System view says it.

import { test } from "node:test";
import assert from "node:assert/strict";
import { lockErrorState, usbLock, usbStats, withOnLockError, type Observed, type SystemModel, type UsbSeen } from "../../src/screens/system/model";

function model(): SystemModel {
  return {
    rooms: [{ id: "main", name: "Main room" }],
    zones: [], groups: [], sources: [], senders: [], fanOuts: [],
    units: [{ id: "R", name: "Rear L/R", serial: "R", room: "main", upstream: "", transport: "usb_direct", usbMode: "phase_lock" }],
    scenes: [],
  };
}

function seen(over: Partial<UsbSeen>): UsbSeen {
  return {
    mode: "phase_lock", d: 192, autoReanchor: true, state: "phase_locked", phaseState: 2, rateCode: 1,
    hostPpm: -11.5, vcxoPpm: -11.5, phaseError: 0.05, busFrame: 0, sample: 0, dMin: 170, dMax: 399, fill: 0,
    stream: true, holdover: false, streamShift: 0, gapCount: 0, anchors: 1, reanchors: 0, missedSamples: 0, lateSamples: 0,
    diag: { ends: 0, holdovers: 0, rides: 0, suspends: 0, busResets: 0, unplugs: 0, longestRideUs: 0, longestRideMs: 0, lockErrors: 0, badSofReadings: 0, version: 2 },
    onLockError: "mute", lockError: false,
    ...over,
  };
}

test("AUSB2: the lock-error action is set, and mute (the default) is not written", () => {
  const play = withOnLockError(model(), "R", "play_unaligned");
  assert.equal(play.units[0]!.onLockError, "play_unaligned");
  const mute = withOnLockError(play, "R", "mute");
  assert.equal("onLockError" in mute.units[0]!, false);
  assert.equal(mute.units[0]!.usbMode, "phase_lock", "the mode stays");
});

test("AUSB2: a unit in lock error says what it does and why", () => {
  const o: Observed = { online: true, usbDirect: "present", usb: seen({ state: "rejected", phaseState: 0, lockError: true, lockErrorCause: "host_rejected" }) };
  assert.equal(usbLock(o), "Can't lock: muted");
  assert.equal(lockErrorState(seen({ onLockError: "play_unaligned" })), "Can't lock: playing unaligned");
  const line = usbStats(o.usb!);
  assert.match(line, /lock errors 0 · can't lock: the host's rate is out of range$/);
  assert.equal(usbLock({ ...o, usb: seen({}) }), "Phase-locked");
});

test("AUSB2: DIAG layout 2's counters are in the stats line, layout 1 says none", () => {
  const d2 = seen({ diag: { ends: 1, holdovers: 0, rides: 0, suspends: 0, busResets: 0, unplugs: 0, longestRideUs: 300000, longestRideMs: 300, lockErrors: 2, badSofReadings: 3, version: 2 } });
  assert.match(usbStats(d2), /lock errors 2 · SOF readings repaired 3 · longest ride 300 ms$/);
  const d1 = seen({ diag: { ends: 0, holdovers: 0, rides: 0, suspends: 0, busResets: 0, unplugs: 0, longestRideUs: 0, version: 1 } });
  assert.doesNotMatch(usbStats(d1), /lock errors/);
});
