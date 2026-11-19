// WP WB4a unit tests: WebUSB through the real core in Node under JSPI. The
// sessions run on the page's module (dspi_wasm.wasm) with the Worker's own USB
// host (src/core/usb.ts) over a WebUSB mock (test/pages/webusb-mock.ts) whose
// devices are the core's simulated DSPi in another instance of the test module.
// The core never opens its demo here: every byte goes through the USB imports.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CoreCallError, type CoreEvent } from "../../src/core/protocol";
import { UsbHost, bcdDeviceOf, setup } from "../../src/core/usb";
import { USB_IDS } from "../../src/core/usb-ids";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { pageEnv, usbAccess } from "../../src/fleet/env";
import { MockBackend, MockUsb } from "../pages/webusb-mock";

const dir = process.env.DSPI_WASM_DIR ?? "";
const testModule = dir ? join(dir, "dspi_wasm_test.wasm") : "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(testModule) && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";

const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until<T>(what: string, f: () => T | undefined | false, ms = 5000): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    const v = f();
    if (v) return v;
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await sleep(10);
  }
}

/** One session as a Worker holds it: its own module and USB host, over `usb`. */
async function worker(usb: MockUsb) {
  const m = await loadWasmModule(readFileSync(pageModule), new UsbHost(() => usb));
  const core = new WasmCore(m);
  made.push(core);
  const events: CoreEvent[] = [];
  core.onEvent((e) => events.push(e));
  return { core, events };
}

test("WB4a: two USB DSPis at once, each its own session: connect, write with read-back, a notification, unplug", { skip }, async () => {
  const backend = await MockBackend.load(readFileSync(testModule));
  const usb = new MockUsb(["USBA0001", "USBB0002"], { backend, permitted: "all" });
  const a = await worker(usb);
  const b = await worker(usb);

  const snapA = await a.core.call("open", { kind: "usb", serial: "USBA0001" });
  const snapB = await b.core.call("open", { kind: "usb", serial: "USBB0002" });
  assert.equal(snapA.device.demo, false, "a USB session, not the core's demo");
  assert.equal(snapA.device.serial, "USBA0001", "the serial WebUSB reported");
  assert.equal(snapB.device.serial, "USBB0002");
  assert.equal(snapA.device.readOnly, false);
  assert.ok(snapA.state.numChannels > 0);
  const devA = usb.device("USBA0001")!;
  assert.ok(devA.opened);
  assert.ok(devA.log.length > 5, "the connect sequence went over WebUSB");
  assert.ok(devA.log.every((s) => s.requestType === "vendor" && s.recipient === "interface"));
  // -- WP PGWIRE -- the generation probe first (SYS.PROTOCOL_INFO: page 1, 0x10, 16
  // bytes), then page 0 only: the simulated DSPi is 1.1.6 and refuses the probe
  assert.deepEqual(devA.log[0], { requestType: "vendor", recipient: "interface", request: 0x10, value: 0, index: 0x0102 });
  assert.ok(devA.log.slice(1).every((s) => s.index === 2), "nothing paged after the probe");

  // A write, read back from the device, and the two devices apart.
  assert.deepEqual(await a.core.call("write", { path: "vol.user", value: { float: -13.5 } }), { float: -13.5 });
  assert.deepEqual(await b.core.call("write", { path: "vol.user", value: { float: -7 } }), { float: -7 });
  assert.equal((await a.core.call("state", undefined)).state.userVolumeDb, -13.5);
  assert.equal((await b.core.call("state", undefined)).state.userVolumeDb, -7);
  await assert.rejects(a.core.call("demo", { action: "renameElsewhere", name: "x" }), (e: CoreCallError) => e.code === "refused");

  // A change at the device: its packet comes up the notification endpoint, the
  // Worker pushes it in, and the session reports the new state.
  devA.renameElsewhere("Stage left");
  const st = await until("the rename's state event", () =>
    a.events.find((e) => e.event === "state" && e.snapshot.device.name === "Stage left"),
  );
  assert.ok(st.event === "state" && st.flags !== 0);
  assert.equal((await a.core.call("state", undefined)).device.name, "Stage left");
  assert.ok(!b.events.some((e) => e.event === "state" && e.snapshot.device.name === "Stage left"), "only A changed");

  // Unplug A: its session ends in the core's words; B goes on.
  usb.unplug("USBA0001");
  const lost = await until("A's lost event", () => a.events.find((e) => e.event === "lost"));
  assert.deepEqual(lost, { event: "lost", reason: "Device not connected" });
  await assert.rejects(a.core.call("state", undefined), (e: CoreCallError) => e.code === "not_open");
  assert.ok(!b.events.some((e) => e.event === "lost"));
  assert.deepEqual(await b.core.call("write", { path: "vol.user", value: { float: -8.5 } }), { float: -8.5 });

  // Closing gives the device back.
  await b.core.call("close", undefined);
  await until("B released", () => !usb.device("USBB0002")!.opened);
});

test("WB4a: a DSPi the Worker cannot find, or no WebUSB at all, says why", { skip }, async () => {
  const backend = await MockBackend.load(readFileSync(testModule));
  const usb = new MockUsb(["USBA0001"], { backend, permitted: [] });
  const w = await worker(usb);
  await assert.rejects(
    w.core.call("open", { kind: "usb", serial: "USBA0001" }),
    (e: CoreCallError) => e.code === "not_open" && e.message === "No DSPi with serial USBA0001",
  );
  await assert.rejects(w.core.call("open", { kind: "usb" }), (e: CoreCallError) => e.code === "invalid");
  const m = await loadWasmModule(readFileSync(pageModule), new UsbHost(() => undefined));
  const none = new WasmCore(m);
  made.push(none);
  await assert.rejects(none.call("open", { kind: "usb", serial: "X" }), (e: CoreCallError) => e.code === "unsupported" && /Chromium/.test(e.message));
});

test("WB4a: the page's USB constants are the protocol's", { skip }, async () => {
  const w = await worker(new MockUsb([]));
  const ids = (await (w.core as unknown as { queued(m: string, p: unknown): Promise<unknown> }).queued("usbIds", null)) as Record<string, number>;
  assert.deepEqual(ids, { ...USB_IDS });
});

test("WB4a: page env offers USB on a secure Chromium page: the picker, the allowed DSPis, plug events", async () => {
  const usb = new MockUsb(["ZZ000002", "AA000001", "CC000003"], { permitted: ["ZZ000002"] });
  const env = pageEnv({ isSecureContext: true, navigator: { usb } });
  assert.deepEqual(env.usb, { ok: true, reason: "" });
  const access = env.usbDevices!;
  assert.deepEqual(await access.permitted(), [{ serial: "ZZ000002", vendorId: USB_IDS.vendorId }]);
  let changes = 0;
  const stop = access.watch(() => changes++);
  usb.choose = (offered) => (offered.includes("AA000001") ? "AA000001" : null);
  assert.deepEqual(await access.choose(), { serial: "AA000001", vendorId: USB_IDS.vendorId });
  assert.deepEqual((await access.permitted()).map((d) => d.serial), ["AA000001", "ZZ000002"], "sorted by serial");
  usb.choose = () => null;
  assert.equal(await access.choose(), null, "the picker closed without a choice");
  usb.unplug("AA000001");
  assert.equal(changes, 1);
  stop();
  usb.unplug("ZZ000002");
  assert.equal(changes, 1);
  assert.deepEqual(await usbAccess(usb).permitted(), []);
  // Insecure pages and other browsers still say why (WB5's reasons).
  assert.equal(pageEnv({ isSecureContext: false, navigator: { usb } }).usb.ok, false);
  assert.equal(pageEnv({ isSecureContext: false, navigator: { usb } }).usbDevices, undefined);
});

// -- WP PGWIRE --
test("PGWIRE: wIndex comes from the core's call: the interface, with a page in its high byte", () => {
  assert.deepEqual(setup(0x7f, 0, 2), { requestType: "vendor", recipient: "interface", request: 0x7f, value: 0, index: 2 });
  // The guide's probe SETUP C1 10 00 00 02 01 10 00: wIndex 0x0102.
  assert.deepEqual(setup(0x10, 0, 0x0102), { requestType: "vendor", recipient: "interface", request: 0x10, value: 0, index: 0x0102 });
  assert.equal(setup(0x10, 0xe0, 0xe002)?.index, 0xe002, "our extension tunnel on page 0xE0");
  // Anything whose low byte is not interface 2 is never sent.
  for (const bad of [0, 1, 3, 0x0100, 0x0103, 0x10002, -1, 2.5]) assert.equal(setup(0x10, 0, bad), null, `wIndex ${bad}`);
});

// -- WP PGZERO --
test("wp_pgzero bcdDevice from WebUSB's version fields", () => {
  const d = (major?: number, minor?: number, sub?: number) =>
    ({ deviceVersionMajor: major, deviceVersionMinor: minor, deviceVersionSubminor: sub }) as unknown as Parameters<typeof bcdDeviceOf>[0];
  assert.equal(bcdDeviceOf(d(3, 0, 0)), 0x0300);
  assert.equal(bcdDeviceOf(d(2, 0, 4)), 0x0204);
  assert.equal(bcdDeviceOf(d(2, 8, 0)), 0x0280);
  assert.equal(bcdDeviceOf(d()), undefined);
});
