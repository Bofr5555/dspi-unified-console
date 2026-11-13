// WP WB3 unit tests: the real core (crates/dspi-wasm) under JSPI in Node, through
// the Worker's own glue (src/core/wasm.ts). The modules come from
// $DSPI_WASM_DIR (crates/dspi-wasm/tools/build.sh): dspi_wasm_test.wasm has two
// extra calls, `test.sleep` (it suspends) and `test.panic`; dspi_wasm.wasm is
// the page's. Without $DSPI_WASM_DIR these tests are skipped, unless
// $DSPI_WASM_REQUIRED is set (CI), where a missing module fails.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CoreCallError, type CoreEvent, type DeviceSnapshot } from "../../src/core/protocol";
import { WasmCore, type WasmModule, loadWasmModule } from "../../src/core/wasm";

const dir = process.env.DSPI_WASM_DIR ?? "";
const testModule = dir ? join(dir, "dspi_wasm_test.wasm") : "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const have = !!dir && existsSync(testModule) && existsSync(pageModule);
const skip = have ? false : "no core module (set DSPI_WASM_DIR)";

if (process.env.DSPI_WASM_REQUIRED && !have) {
  test("WB3: the core modules are there", () => assert.fail(`no dspi_wasm.wasm / dspi_wasm_test.wasm in "${dir}"`));
}

const load = (file: string) => loadWasmModule(readFileSync(file));
// Every session made here, stopped at the end even when a test failed: a live
// session's poll timer would keep Node running.
const made: WasmCore[] = [];
const session = (m: WasmModule) => {
  const c = new WasmCore(m);
  made.push(c);
  return c;
};
after(() => {
  for (const c of made) c.dispose();
});
const code = (c: string) => (e: unknown) => e instanceof CoreCallError && e.code === c;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until<T>(what: string, f: () => Promise<T | undefined | false>, ms = 5000): Promise<T> {
  const end = Date.now() + ms;
  for (;;) {
    const v = await f();
    if (v) return v;
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await sleep(10);
  }
}

test("WB3: the module imports only the page's seams", { skip }, () => {
  // dspi_random_fill joins these once Link is built in (WB4b); the glue
  // supplies all four either way.
  const seams = ["dspi.dspi_now_ms", "dspi.dspi_random_fill", "dspi.dspi_sleep_ms", "dspi.dspi_unix_ms"];
  // -- WP WB4a -- WebUSB's control transfers (src/core/usb.ts)
  seams.push("dspi.dspi_usb_in", "dspi.dspi_usb_out", "dspi.dspi_usb_error");
  // -- WP WB4b -- the browser's WebSocket (src/core/ws.ts)
  for (const f of ["open", "error", "send", "close", "poll", "wait", "len", "take", "closed"]) seams.push(`dspi.dspi_ws_${f}`);
  for (const file of [testModule, pageModule]) {
    const names = WebAssembly.Module.imports(new WebAssembly.Module(readFileSync(file))).map((i) => `${i.module}.${i.name}`);
    assert.deepEqual(names.filter((n) => !seams.includes(n)), []);
    for (const clock of ["dspi.dspi_now_ms", "dspi.dspi_sleep_ms", "dspi.dspi_unix_ms"]) assert.ok(names.includes(clock), clock);
  }
});

test("WB3: a second call while one is suspended is refused by the module, not run", { skip }, async () => {
  const core = session(await load(testModule));
  await core.call("open", { kind: "demo" });
  const before = (await core.call("state", undefined)).state.userVolumeDb;
  assert.notEqual(before, -33);

  // Straight into the module, past the Worker's queue: the first suspends in
  // the core's clock::sleep (a JSPI Suspending timer), the second arrives
  // meanwhile.
  const t0 = performance.now();
  const suspended = core.unqueued("test.sleep", { ms: 80 });
  await sleep(10);
  await assert.rejects(core.unqueued("write", { path: "vol.user", value: { float: -33 } }), code("busy"));
  await assert.rejects(core.unqueued("state", null), code("busy"));
  assert.equal(await suspended, 80);
  assert.ok(performance.now() - t0 >= 79, "the first call really was suspended");

  // The refused write did not run: the device still has the old volume.
  const after = (await core.call("state", undefined)).state.userVolumeDb;
  assert.equal(after, before);
  // And the module is free again.
  assert.deepEqual(await core.call("write", { path: "vol.user", value: { float: -33 } }), { float: -33 });
  core.dispose();
});

test("WB3: through the Worker's queue, a call made during a suspended one waits and then runs", { skip }, async () => {
  const core = session(await load(testModule));
  await core.call("open", { kind: "demo" });
  const slow = (core as unknown as { queued(m: string, p: unknown): Promise<unknown> }).queued("test.sleep", { ms: 40 });
  const write = core.call("write", { path: "vol.user", value: { float: -21.5 } });
  assert.equal(await slow, 40);
  assert.deepEqual(await write, { float: -21.5 });
  core.dispose();
});

test("WB3: two sessions in one module: demo connect, write with read-back, apart", { skip }, async () => {
  const m = await load(pageModule);
  const a = session(m);
  const b = session(m);
  assert.notEqual(a.handle, b.handle);
  const snap = await a.call("open", { kind: "demo" });
  await b.call("open", { kind: "demo" });
  assert.equal(snap.device.demo, true);
  assert.equal(snap.device.readOnly, false);
  assert.match(snap.device.firmware, /^\d+\.\d+\.\d+/);
  assert.ok(snap.state.numChannels > 0);
  assert.ok(!("bulkRaw" in snap.state));

  assert.deepEqual(await a.call("write", { path: "vol.user", value: { float: -17.5 } }), { float: -17.5 });
  assert.equal((await a.call("state", undefined)).state.userVolumeDb, -17.5);
  assert.notEqual((await b.call("state", undefined)).state.userVolumeDb, -17.5);
  assert.deepEqual(await a.call("write", { path: "out.gain", indices: [0], value: { float: -4 } }), { float: -4 });
  await assert.rejects(a.call("write", { path: "vol.user", value: { float: 6 } }), (e: CoreCallError) =>
    e.code === "out_of_range" && e.message === "Volume must be between -60 and 0 dB");
  assert.equal((await a.call("describe", { path: "vol.user" })).max, 0);
  await assert.rejects(a.call("simulate", { what: "link-lost" }), code("unknown_method"));
  // -- WP WB4a -- USB is offered; without a chosen DSPi the core says so.
  await assert.rejects(a.call("open", { kind: "usb" }), code("invalid"));

  // The page's module has no test calls.
  await assert.rejects(a.unqueued("test.sleep", { ms: 1 }), code("unknown_method"));
  a.dispose();
  b.dispose();
});

test("WB3: an older firmware demo is read only, in the core's words", { skip }, async () => {
  const core = session(await load(pageModule));
  const snap = await core.call("open", { kind: "demo", readOnly: true });
  assert.equal(snap.device.readOnly, true);
  assert.equal(snap.device.readOnlyReason, "the connected firmware is read-only in this version");
  await assert.rejects(core.call("write", { path: "vol.user", value: { float: -10 } }), code("read_only"));
  core.dispose();
});

test("WB3: notifications: the Worker's poll turns a foreign rename into a state event", { skip }, async () => {
  const core = session(await load(pageModule));
  const events: CoreEvent[] = [];
  core.onEvent((e) => events.push(e));
  await core.call("open", { kind: "demo" });
  await core.call("demo", { action: "renameElsewhere", name: "Elsewhere" });
  const ev = await until("a state event with the new name", async () =>
    events.find((e): e is { event: "state"; snapshot: DeviceSnapshot; flags: number } =>
      e.event === "state" && e.snapshot.device.name === "Elsewhere"),
  );
  assert.ok(ev.flags & (1 << 8), "NOTIFY_NAMES");
  core.dispose();
});

test("WB3: meters and RTA frames on the Worker's timers", { skip }, async () => {
  const core = session(await load(pageModule));
  await core.call("open", { kind: "demo" });
  const m = await core.call("meters", undefined);
  assert.ok(m.peaks.length >= m.numChannels && m.numChannels > 0);
  await core.call("rtaStart", { tap: 1, mask: 0b11 });
  const view = await until("RTA frames", async () => {
    const v = await core.call("rta", undefined);
    return v.snapshot.count >= 1 && v.snapshot.frames[0]!.nBands > 0 ? v : undefined;
  }, 10000);
  assert.equal(view.caps.supported, true);
  assert.equal(view.snapshot.frames[0]!.avgDb.length > 0, true);
  await core.call("rtaStop", undefined);
  core.dispose();
});

test("WB3: a panic marks the core dead: an error reply, a lost event, every later call refused", { skip }, async () => {
  const m = await load(testModule);
  const core = session(m);
  const events: CoreEvent[] = [];
  core.onEvent((e) => events.push(e));
  await core.call("open", { kind: "demo" });
  await assert.rejects(core.call("state", undefined).then(() => core.unqueued("test.panic", null)), (e: CoreCallError) =>
    e.code === "dead" && /test\.panic was called/.test(e.message));
  await assert.rejects(core.call("state", undefined), code("dead"));
  assert.ok(m.dead.includes("test.panic was called"));
  assert.ok(events.some((e) => e.event === "lost" && /internal error/.test(e.reason)));
  assert.throws(() => m.newSession(), code("dead"));
});

// -- BUG1 --
test("BUG1: the core's snapshot carries each output's label and the dashboard's cards", { skip }, async () => {
  const core = session(await load(pageModule));
  const snap: DeviceSnapshot = await core.call("open", { kind: "demo" });
  const labels = snap.outputLabels ?? [];
  assert.equal(labels.length, snap.state.numOutputChannels);
  // The demo routes USB 1 to output 1 and USB 2 to output 2, unfiltered, under the slots' default names
  assert.equal(labels[0]?.label, "Out 1 · L · Full");
  assert.equal(labels[1]?.label, "Out 2 · R · Full");
  assert.equal(labels[0]?.slotName, "SPDIF 1 L");
  assert.ok(Array.isArray(snap.outputCards));
  // A rename wins, in the next snapshot
  await core.call("write", { path: "ch.name", indices: [snap.state.numInputChannels], value: { text: "Left" } });
  const after: DeviceSnapshot = await core.call("state", undefined);
  assert.equal(after.outputLabels?.[0]?.label, "Left");
  assert.equal(after.outputLabels?.[0]?.userNamed, true);
});


// -- WP WB8 -- undo and redo, the drag mark, a demo clip in the event log, the
// diagnostics zip and a refused recall's words, on the page's module

test("WB8: undo and redo through the core's journal; a drag is one step", { skip }, async () => {
  const core = session(await load(pageModule));
  await core.call("open", { kind: "demo" });
  assert.deepEqual(await core.call("canUndo", undefined), { canUndo: false, canRedo: false });
  const start = (await core.call("state", undefined)).state.userVolumeDb;
  await core.call("write", { path: "vol.user", value: { float: -20 } });
  // A drag: three writes inside one mark are one step.
  await core.call("setDragging", { dragging: true });
  for (const v of [-21, -22, -23]) await core.call("write", { path: "vol.user", value: { float: v } });
  await core.call("setDragging", { dragging: false });
  assert.deepEqual(await core.call("canUndo", undefined), { canUndo: true, canRedo: false });
  const back = await core.call("undo", undefined);
  assert.equal(back.entry?.path, "vol.user");
  assert.deepEqual(back.entry?.before, { float: -20 });
  assert.equal(back.note, null);
  assert.equal((await core.call("state", undefined)).state.userVolumeDb, -20, "the whole drag in one step");
  await core.call("undo", undefined);
  assert.equal((await core.call("state", undefined)).state.userVolumeDb, start);
  assert.deepEqual(await core.call("canUndo", undefined), { canUndo: false, canRedo: true });
  await core.call("redo", undefined);
  assert.equal((await core.call("state", undefined)).state.userVolumeDb, -20);
  const h = await core.call("history", { n: 3 });
  assert.equal(h[0]!.source, "redo");
  core.dispose();
});

test("WB8: undo on a read-only demo moves nothing", { skip }, async () => {
  const core = session(await load(pageModule));
  await core.call("open", { kind: "demo", readOnly: true });
  assert.deepEqual(await core.call("canUndo", undefined), { canUndo: false, canRedo: false });
  const m = await core.call("undo", undefined);
  assert.equal(m.entry, null);
  core.dispose();
});

test("WB8: a demo clip reaches the event log with its time, unit and text", { skip }, async () => {
  const core = session(await load(pageModule));
  const snap = await core.call("open", { kind: "demo" });
  const t0 = Date.now() - 1000;
  await core.call("demo", { action: "clip", mask: 1 });
  await core.call("meters", undefined);
  const serials = await core.call("eventLogDevices", undefined);
  assert.ok(serials.includes(snap.device.serial), String(serials));
  const log = await core.call("eventLogRead", { serial: snap.device.serial });
  const clip = log.records.find((r) => r.kind === "clip");
  assert.ok(clip, JSON.stringify(log.records));
  assert.ok(clip.t >= t0);
  assert.ok(clip.text.length > 0);
  assert.ok(log.records.some((r) => r.kind === "connected"));
  // Only what is new after `lastId`.
  assert.deepEqual((await core.call("eventLogRead", { serial: snap.device.serial, afterId: log.lastId })).records, []);
  await assert.rejects(core.call("demo", { action: "clip", mask: 0 }), code("invalid"));
  core.dispose();
});

test("WB8: the diagnostics bundle is the core's zip, named and dated", { skip }, async () => {
  const core = session(await load(pageModule));
  await core.call("open", { kind: "demo" });
  const d = await core.call("diagnostics", { app: "web test", files: [{ name: "browser.txt", text: "User agent: test" }] });
  assert.match(d.name, /^dspi-diagnostics-\d{8}-\d{6}Z\.zip$/);
  const bytes = Buffer.from(d.base64, "base64");
  assert.equal(bytes.length, d.size);
  assert.equal(bytes.subarray(0, 4).toString("latin1"), "PK\x03\x04");
  const names = bytes.toString("latin1");
  for (const f of ["README.txt", "versions.json", "device/dump.json", "device/stats.json", "client/browser.txt", "notes.txt"]) assert.ok(names.includes(f), f);
  core.dispose();
});

test("WB8: a refused recall in the core's words, one sentence per sender's answer", { skip }, async () => {
  const core = session(await load(pageModule));
  const model = { rateHz: 48000, scenes: [{ id: "closed", name: "Closed" }], units: [], senders: [], fanOuts: [] };
  const report = (result: string) => ({
    plan: { scene: "closed", frame: 2_000_000, path: "superframe", ops: [], leadFrames: 4800, systemMute: true },
    writes: [{ serial: "TX", element: "tx", what: "mute", result }],
    frameAfterWrites: 2_000_000 - 4790,
    onTime: false,
  });
  const held = 2_000_000 - 4790 + 4800;
  const refused = await core.call("sceneOutcome", { scene: "closed", report: report("refused"), model, heldFrame: held });
  assert.equal(refused.taken, false);
  assert.equal(refused.heldFrame, held);
  assert.equal(
    refused.message,
    "Closed was refused: a scene is already scheduled on the sender for link frame 2000010, 100 ms after this recall. The sender takes one scene at a time; recall Closed again after that. System mute unchanged.",
  );
  assert.match((await core.call("sceneOutcome", { scene: "closed", report: report("refused"), model, heldFrame: null })).message, /still holds one/);
  const timeout = await core.call("sceneOutcome", { scene: "closed", report: report("timeout"), model, heldFrame: held });
  assert.equal(timeout.message, "Closed was not recalled: the sender did not answer in time. Check the sender and recall Closed again. System mute unchanged.");
  assert.equal(timeout.heldFrame, null);
  const locked = await core.call("sceneOutcome", { scene: "closed", report: report("locked"), model, heldFrame: held });
  assert.match(locked.message, /^Closed was not recalled: another console is applying changes to the sender\./);
  const other = await core.call("sceneOutcome", { scene: "closed", report: report("bad_frame"), model, heldFrame: held });
  assert.match(other.message, /^Closed was not recalled: the sender's answer was "/);
  for (const m of [timeout.message, locked.message, other.message]) assert.ok(!m.includes("already scheduled"), m);
  const perUnit = { ...report("refused"), plan: { ...report("refused").plan, path: "per_unit" } };
  assert.equal((await core.call("sceneOutcome", { scene: "closed", report: perUnit, model })).message, "Closed was refused by every unit; nothing changed. System mute unchanged.");
  core.dispose();
});
