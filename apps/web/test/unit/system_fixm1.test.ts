// WP FIXM1 unit tests: "Keep USB-direct units aligned" in the installation the
// Installer edits, and the hub's "Align again" issue read from its status.

import { test } from "node:test";
import assert from "node:assert/strict";
import { USB_OFFSETS_CHANGED, usbAlignDue, withKeepUsbAligned, type SystemModel, type SystemStatus } from "../../src/screens/system/model";

function model(): SystemModel {
  return {
    rooms: [{ id: "main", name: "Main room" }],
    zones: [], groups: [], sources: [], senders: [], fanOuts: [],
    units: [{ id: "R", name: "Rear L/R", serial: "R", room: "main", upstream: "", transport: "usb_direct", usbMode: "phase_lock" }],
    scenes: [],
  };
}

function status(issues: SystemStatus["alignment"]["issues"]): SystemStatus {
  return {
    revision: 1,
    observed: {},
    alignment: { dFrames: 192, dAuto: false, rateHz: 48000, latencyMaxFrames: 0, units: [], issues },
    alarms: [],
  };
}

test("FIXM1: Keep USB-direct units aligned is set, and off is not written", () => {
  const m = model();
  assert.equal(m.keepUsbAligned, undefined, "off by default");
  const on = withKeepUsbAligned(m, true);
  assert.equal(on.keepUsbAligned, true);
  assert.equal(m.keepUsbAligned, undefined, "the model given is not changed");
  const off = withKeepUsbAligned(on, false);
  assert.ok(!("keepUsbAligned" in off), "off is the hub's default: not sent");
  assert.deepEqual(off, m);
});

test("FIXM1: the hub's moved start offsets make an alignment due", () => {
  assert.equal(usbAlignDue(null), false);
  assert.equal(usbAlignDue(status([])), false);
  assert.equal(usbAlignDue(status([{ code: "usb_start_offsets", message: "USB-direct streams started apart" }])), false);
  assert.equal(
    usbAlignDue(status([{ code: USB_OFFSETS_CHANGED, message: "USB-direct start offsets changed — Align again (Rear L/R moved)" }])),
    true,
  );
});
