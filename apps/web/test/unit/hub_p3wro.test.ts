// WP P3WRO unit tests: the firmware rollout's model and feed (PLAN-P3 F6/F8): who sees it
// (Admin with the `rollout` cap), the image list's words (version, platform, the 1.20 V stamp,
// verified or not), a file refused before upload (8 MiB, the name), every step's state from a
// record, the result's lines (version, extension channel, the core voltage read back,
// settings), "Recall the Export" only when the settings changed, cancel before the bootloader
// entry only, `fw.rollout.changed` with its serial delta, the `rollout_failed` alarm with its
// step, the plaintext refusal as the TLS notice; the feed's typed calls (nothing sent for a file
// it refuses, cancel's refusal said, the busy moment after a rollout ended asked again); the
// alarm rule kept on; a health cell coloured by its raised alarm's severity.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import {
  CoreCallError,
  type HubAlarmRecord,
  type HubCapsRecord,
  type HubFirmwareImageRecord,
  type HubRolloutDeviceRecord,
  type HubRolloutRecord,
  type HubServerEventRecord,
} from "../../src/core/protocol";
import type { Session } from "../../src/ui/session";
import {
  ALWAYS_ON_ALARMS,
  base64Of,
  currentDevice,
  failedAlarms,
  fileProblem,
  imageRows,
  isTlsRefusal,
  MAX_IMAGE_BYTES,
  mayCancel,
  recallOffer,
  resultLines,
  RolloutFeed,
  rolloutChange,
  rolloutShown,
  STEPS,
  stepRows,
  targetOf,
  volts,
  withRollout,
} from "../../src/screens/hub/rollout-model";
import { alarmTone, cellTone, type BoardRow, type Thresholds } from "../../src/screens/hub/health-model";

const DEV = "P3WRO0000DEV0001";

function caps(extra: Partial<HubCapsRecord> = {}): HubCapsRecord {
  return {
    onHub: true,
    inventory: true,
    identify: true,
    health: true,
    events: true,
    hubSnapshots: true,
    paramApi: true,
    hubStats: true,
    alarms: true,
    outbound: true,
    schedule: true,
    mqtt: true,
    apiKeys: true,
    rest: false,
    groups: true,
    groupParams: true,
    groupApply: true,
    rollout: true,
    caps: [],
    ...extra,
  };
}

function img(extra: Partial<HubFirmwareImageRecord> = {}): HubFirmwareImageRecord {
  return {
    name: "dspi-1.1.7.uf2",
    size: 8192,
    sha256: "ab".repeat(32),
    firmwareVersion: "1.1.7",
    platform: "RP2350",
    stampMv: 1200,
    verified: false,
    addedAt: "2026-11-16T20:00:00Z",
    addedBy: "Bench console",
    ...extra,
  };
}

const step = (s: string, ok = true, text = `${s} done`) => ({ step: s, at: "2026-11-16T20:10:00Z", ok, text, session: null, noticeKey: `rollout.${s}` });

function device(extra: Partial<HubRolloutDeviceRecord> = {}): HubRolloutDeviceRecord {
  return {
    serial: DEV,
    state: "running",
    step: "export",
    steps: [step("check"), step("guard")],
    before: null,
    after: null,
    failed: null,
    ...extra,
  };
}

function record(extra: Partial<HubRolloutRecord> = {}, d: HubRolloutDeviceRecord = device()): HubRolloutRecord {
  return {
    id: "ro1",
    target: DEV,
    image: "dspi-1.1.7.uf2",
    imageSha256: "ab".repeat(32),
    imageVersion: "1.1.7",
    by: "Bench console",
    whileLive: false,
    state: "running",
    startedAt: "2026-11-16T20:10:00Z",
    endedAt: "",
    devices: [d],
    ...extra,
  };
}

const BEFORE = { fw: "1.1.6", extList: [1, 2, 3], requiredMv: 1200, boardSource: "ext_core_vreg", vreg: { liveMv: 1200, storedMv: 1200, defaultMv: 1200 }, export: "s-1700000000" };

function ended(settings: string, failedAt: string | null = null): HubRolloutDeviceRecord {
  const done = STEPS.filter((s) => s !== failedAt).map((s) => step(s));
  return device({
    state: failedAt ? "failed" : "ok",
    step: failedAt ?? "verify",
    steps: failedAt ? [...done.slice(0, STEPS.indexOf(failedAt as (typeof STEPS)[number])), step(failedAt, false, "the device came back with firmware 1.1.7, not 2.0.0")] : done,
    before: BEFORE,
    after: { fw: "1.1.7", extList: [1, 2, 3], vreg: { liveMv: 1200, storedMv: 1200, defaultMv: 1200 }, settings },
    failed: failedAt ? { step: failedAt, code: "", text: "the device came back with firmware 1.1.7, not 2.0.0", noticeKey: `rollout.${failedAt}` } : null,
  });
}

test("P3WRO: shown to an Admin of a hub with the rollout cap only", () => {
  assert.equal(rolloutShown(caps(), "admin"), true);
  assert.equal(rolloutShown(caps(), "control"), false);
  assert.equal(rolloutShown(caps(), "viewer"), false);
  assert.equal(rolloutShown(caps({ rollout: false }), "admin"), false);
  assert.equal(rolloutShown(caps({ rollout: undefined }), "admin"), false);
  assert.equal(rolloutShown(caps({ onHub: false }), "admin"), false);
  assert.equal(rolloutShown(null, "admin"), false);
});

test("P3WRO: the image list: version, platform, the 1.20 V stamp, verified or not", () => {
  const [a, b, c] = imageRows([img(), img({ name: "plain.uf2", stampMv: null, verified: true, firmwareVersion: "" }), img({ name: "x.uf2", stampMv: 1150, size: 640 * 1024 })]);
  assert.deepEqual([a!.version, a!.platform, a!.stamp, a!.k340, a!.label], ["1.1.7", "RP2350", "1.20 V", true, "not verified"]);
  assert.equal(b!.label, "verified");
  assert.equal(b!.version, "–");
  assert.match(b!.stamp, /Unstamped.*1\.15 V/);
  assert.equal(b!.k340, false);
  assert.equal(c!.stamp, "1.15 V");
  assert.equal(c!.size, "640 KiB");
  assert.equal(a!.size, "8 KiB");
  assert.equal(a!.added, "2026-11-16 by Bench console");
  assert.equal(volts(1200), "1.20 V");
});

test("P3WRO: a file the hub would refuse is refused before upload", () => {
  assert.equal(fileProblem("dspi-1.1.7.uf2", 630_000), "");
  assert.match(fileProblem("dspi.uf2", MAX_IMAGE_BYTES + 1), /8\.0 MiB/);
  assert.equal(fileProblem("dspi.uf2", MAX_IMAGE_BYTES), "");
  assert.match(fileProblem("dspi.uf2", 0), /empty/);
  for (const bad of ["dspi.bin", "a b.uf2", "../x.uf2", `${"x".repeat(61)}.uf2`, "X.UF2"]) assert.notEqual(fileProblem(bad, 10), "", bad);
  assert.equal(base64Of(new Uint8Array([1, 2, 3, 255])), "AQID/w==");
  const big = new Uint8Array(200_000).fill(7);
  assert.equal(base64Of(big), Buffer.from(big).toString("base64"));
});

test("P3WRO: targets: one DSPi or one group", () => {
  assert.deepEqual(targetOf(`serial:${DEV}`), { serials: [DEV] });
  assert.deepEqual(targetOf("group:g2"), { group: "g2" });
  assert.equal(targetOf("serial:"), null);
  assert.equal(targetOf(""), null);
});

test("P3WRO: every step's state from the record: done, running, pending; failed and the rest skipped", () => {
  const running = stepRows(device());
  assert.deepEqual(
    running.map((s) => s.state),
    ["done", "done", "running", "pending", "pending", "pending", "pending", "pending"],
  );
  assert.equal(running[0]!.text, "check done");
  const failed = stepRows(ended("changed", "verify"));
  assert.equal(failed.at(-1)!.state, "failed");
  assert.match(failed.at(-1)!.text, /not 2\.0\.0/);
  assert.ok(failed.slice(0, 7).every((s) => s.state === "done"));
  const atFind = stepRows(device({ state: "failed", step: "find", steps: [step("check"), step("guard"), step("export"), step("bootloader")], failed: { step: "find", code: "", text: "no board appeared", noticeKey: "rollout.find" } }));
  assert.deepEqual(
    atFind.map((s) => s.state),
    ["done", "done", "done", "done", "failed", "skipped", "skipped", "skipped"],
  );
  assert.equal(atFind[4]!.text, "no board appeared");
});

test("P3WRO: cancel is offered before the bootloader entry only", () => {
  for (const s of ["check", "guard", "export"]) assert.equal(mayCancel(record({}, device({ step: s }))), true, s);
  for (const s of ["bootloader", "find", "flash", "wait", "verify"]) assert.equal(mayCancel(record({}, device({ step: s }))), false, s);
  assert.equal(mayCancel(record({ state: "ok" }, ended("unchanged"))), false);
  // A group: the second member pending after the first is done.
  const g = record({ target: "group:g1", devices: [ended("unchanged"), device({ serial: "B", state: "pending", step: "", steps: [] })] });
  assert.equal(mayCancel(g), true);
  assert.equal(currentDevice(g)!.serial, DEV);
});

test("P3WRO: the result's lines, and Recall the Export only when the settings changed", () => {
  const ok = ended("unchanged");
  assert.deepEqual(resultLines(ok), [
    "Firmware 1.1.7 (was 1.1.6)",
    "Extension channel: 3 commands, none lost (3 before)",
    "Core voltage 1.20 V read back",
    "Settings unchanged",
  ]);
  assert.equal(recallOffer(ok), null);
  const changed = ended("changed", "verify");
  assert.ok(resultLines(changed).includes("Settings changed since the export"));
  assert.deepEqual(recallOffer(changed), { serial: DEV, id: "s-1700000000" });
  // A stock board (1.15 V): no core-voltage line; a wrong read-back is said.
  const stock = ended("unknown");
  stock.before = { ...BEFORE, requiredMv: 1150 };
  assert.ok(!resultLines(stock).some((l) => l.startsWith("Core voltage")));
  assert.ok(resultLines(stock).includes("Settings could not be compared with the export"));
  const low = ended("unchanged");
  low.after = { ...low.after!, vreg: { liveMv: 1150, storedMv: 1200, defaultMv: 1200 }, extList: [1] };
  assert.ok(resultLines(low).includes("Core voltage not 1.20 V: live 1.15 V, stored 1.20 V"));
  assert.ok(resultLines(low).includes("Extension channel: 1 commands, 2 fewer than before"));
  // No export kept: nothing to recall.
  const none = ended("changed");
  none.before = { ...BEFORE, export: "" };
  assert.equal(recallOffer(none), null);
  assert.deepEqual(resultLines(device()), []);
});

test("P3WRO: fw.rollout.changed with its serial delta; records newest first", () => {
  const e = (r: HubRolloutRecord | null, serial: string): HubServerEventRecord => ({ kind: "fw.rollout.changed", serial, record: null, device: null, snapshot: "", change: "", alarm: null, run: null, rollout: r });
  const two = record({ target: "group:g1", devices: [ended("unchanged"), device({ serial: "B", step: "flash" })] });
  assert.deepEqual(rolloutChange(e(two, "B")), { rollout: two, serial: "B", step: "flash" });
  assert.equal(rolloutChange(e(two, ""))!.serial, "B", "without the delta: the running device");
  assert.equal(rolloutChange(e(null, "B")), null);
  assert.equal(rolloutChange({ ...e(two, "B"), kind: "alarm.changed" }), null);
  const list = withRollout(withRollout([record({ id: "ro2" })], record({ id: "ro10" })), record({ id: "ro2", state: "ok" }));
  assert.deepEqual(
    list.map((r) => [r.id, r.state]),
    [
      ["ro10", "running"],
      ["ro2", "ok"],
    ],
  );
});

function alarm(extra: Partial<HubAlarmRecord> = {}): HubAlarmRecord {
  return {
    id: "a1",
    source: "device",
    code: "rollout_failed",
    subject: DEV,
    serial: DEV,
    severity: "critical",
    message: "the firmware rollout ro1 failed at verify: the device came back with firmware 1.1.7",
    state: "raised",
    raisedAt: "2026-11-16T20:11:00Z",
    clearedAt: "",
    acknowledgedBy: "",
    count: 1,
    reason: "",
    noticeKey: "alarm.rollout_failed",
    ...extra,
  };
}

test("P3WRO: the rollout_failed alarm with its step", () => {
  const rs = [record({ state: "failed" }, ended("changed", "verify"))];
  assert.deepEqual(
    failedAlarms([alarm(), alarm({ id: "a2", code: "temperature" }), alarm({ id: "a3", state: "cleared" })], rs).map((x) => [x.alarm.id, x.step]),
    [["a1", "verify"]],
  );
  // Without a record, the step from the hub's words.
  assert.equal(failedAlarms([alarm({ message: "the firmware rollout ro4 failed at find: no board" })], [])[0]!.step, "find");
  assert.ok(ALWAYS_ON_ALARMS.has("rollout_failed"));
  assert.ok(!ALWAYS_ON_ALARMS.has("temperature"));
});

// -- the feed --

function fakeSession(answers: Record<string, unknown> = {}) {
  const calls: { method: string; params: unknown }[] = [];
  let hubListener: ((e: HubServerEventRecord[]) => void) | null = null;
  const s = {
    answers: {
      hubFirmwareImages: { images: [img()], maxImages: 4, maxBytes: MAX_IMAGE_BYTES },
      hubRollouts: [] as HubRolloutRecord[],
      hubGroups: [{ id: "g1", name: "Hall", note: "", members: [DEV] }],
      ...answers,
    } as Record<string, unknown>,
    state: { status: "open", link: { phase: "bound" } },
    watch: () => () => undefined,
    watchHub: (fn: (e: HubServerEventRecord[]) => void) => {
      hubListener = fn;
      return () => (hubListener = null);
    },
    emit: (e: HubServerEventRecord[]) => hubListener?.(e),
    core: {
      call: async (method: string, params: unknown) => {
        calls.push({ method, params });
        let a = s.answers[method];
        if (Array.isArray(a) && method === "hubRolloutStart") a = a.shift();
        if (typeof a === "function") a = (a as (p: unknown) => unknown)(params);
        if (a instanceof Error) throw a;
        if (a === undefined) throw new CoreCallError("unknown_method", method);
        return a;
      },
    },
    calls,
  };
  return s;
}

const feeds: RolloutFeed[] = [];
after(() => feeds.forEach((f) => f.stop()));
function feedOn(s: ReturnType<typeof fakeSession>) {
  const f = new RolloutFeed(s as unknown as Session, { periodMs: 60_000 });
  feeds.push(f);
  return f;
}

test("P3WRO: the feed reads the store, the rollouts and the groups through the typed calls", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.read();
  assert.equal(f.view.images!.length, 1);
  assert.equal(f.view.groups[0]!.name, "Hall");
  assert.equal(f.view.tls, null);
  for (const c of s.calls) assert.match(c.method, /^hub[A-Z]/, "only the typed calls");
  // An event updates the record in place.
  s.emit([{ kind: "fw.rollout.changed", serial: DEV, record: null, device: null, snapshot: "", change: "", alarm: null, run: null, rollout: record({}, device({ step: "flash" })) }]);
  assert.equal(f.view.rollouts[0]!.devices[0]!.step, "flash");
});

test("P3WRO: over plaintext the store's read is the TLS notice, and so is an upload", async () => {
  const tls = () => new CoreCallError("link", "DSPi Link: fw.* is taken over an encrypted connection only (tls_required)", "dlink.tls_required");
  const s = fakeSession({ hubFirmwareImages: tls(), hubFirmwarePut: tls() });
  const f = feedOn(s);
  f.start();
  await f.read();
  assert.equal(f.view.tls?.key, "dlink.tls_required");
  assert.equal(f.view.error, "");
  assert.ok(isTlsRefusal(tls()));
  assert.ok(!isTlsRefusal(new CoreCallError("link", "conflict")));
  const r = await f.upload("dspi.uf2", new Uint8Array([1, 2, 3]));
  assert.equal(r.bad, true);
  assert.equal(r.notice?.key, "dlink.tls_required");
});

test("P3WRO: an upload past 8 MiB sends nothing; a good one goes as base64 and lists the image", async () => {
  const s = fakeSession({ hubFirmwarePut: (p: { name: string }) => ({ image: img({ name: p.name }), created: true }) });
  const f = feedOn(s);
  const big = await f.upload("big.uf2", new Uint8Array(MAX_IMAGE_BYTES + 1));
  assert.equal(big.bad, true);
  assert.equal(s.calls.filter((c) => c.method === "hubFirmwarePut").length, 0);
  const ok = await f.upload("new.uf2", new Uint8Array([1, 2, 3]));
  assert.equal(ok.bad, false);
  assert.deepEqual(s.calls.find((c) => c.method === "hubFirmwarePut")!.params, { name: "new.uf2", data: "AQID" });
  assert.deepEqual(
    f.view.images!.map((i) => i.name),
    ["new.uf2"],
  );
  assert.equal(f.view.uploading, null);
});

test("P3WRO: start, Mute First through the group write, the mute read, cancel refused after the entry", async () => {
  const s = fakeSession({
    hubRolloutStart: [new CoreCallError("link", "DSPi Link: a rollout runs on this hub; one at a time"), record()],
    hubGroupParamSet: { path: "vol.mute", idx: [], results: [{ serial: DEV, ok: true }] },
    hubGroupParamGet: { path: "vol.mute", idx: [], results: [{ serial: DEV, ok: true, value: { bool: false } }] },
    hubRolloutCancel: new CoreCallError("link", "DSPi Link: the device is past its bootloader entry: a flash is never interrupted"),
  });
  const f = feedOn(s);
  assert.equal(await f.muted({ serials: [DEV] }), false);
  const m = await f.muteFirst({ serials: [DEV] });
  assert.equal(m.bad, false);
  assert.deepEqual(s.calls.find((c) => c.method === "hubGroupParamSet")!.params, { target: { serials: [DEV] }, path: "vol.mute", value: { bool: true } });
  // The busy moment after the last rollout ended: asked once more.
  const r = await f.startRollout({ serials: [DEV] }, "dspi-1.1.7.uf2", false);
  assert.equal(r.bad, false, r.text);
  assert.equal(s.calls.filter((c) => c.method === "hubRolloutStart").length, 2);
  assert.deepEqual(s.calls.find((c) => c.method === "hubRolloutStart")!.params, { target: { serials: [DEV] }, image: "dspi-1.1.7.uf2", whileLive: false });
  assert.equal(f.view.rollouts[0]!.id, "ro1");
  const c = await f.cancel("ro1");
  assert.equal(c.bad, true);
  assert.match(c.text, /Too late to cancel/);
});

test("P3WRO: a cancel before the entry, the not-muted refusal, and Recall the Export", async () => {
  const s = fakeSession({
    hubRolloutCancel: record({ state: "cancelled" }, device({ state: "cancelled" })),
    hubRolloutStart: [new CoreCallError("confirmation_required", "the device is not muted")],
    hubSnapshotsRecall: { serial: DEV, snapshot: "s-1", name: "Before firmware 2.0.0, 2026-11-16", image: true, lines: [], notes: [], guard: null },
  });
  const f = feedOn(s);
  f.view.rollouts = [record()];
  const c = await f.cancel("ro1");
  assert.equal(c.bad, false);
  assert.match(c.text, /cancelled before the bootloader entry/);
  const st = await f.startRollout({ serials: [DEV] }, "x.uf2", false);
  assert.equal(st.code, "confirmation_required");
  const back = await f.recall(DEV, "s-1");
  assert.equal(back.bad, false);
  assert.match(back.text, /Before firmware 2\.0\.0/);
  assert.deepEqual(s.calls.find((x) => x.method === "hubSnapshotsRecall")!.params, { serial: DEV, id: "s-1" });
});

test("P3WRO: a health cell takes its raised alarm's severity", () => {
  const row = (alarms: HubAlarmRecord[]): BoardRow => ({
    serial: DEV,
    name: "A",
    tags: [],
    online: true,
    isOpen: false,
    unresponsive: false,
    cpu: 50,
    tempC: 40,
    clipPerMin: null,
    clipped: false,
    underruns: null,
    starvation: null,
    hasCounters: false,
    uptimeS: null,
    hangCount: null,
    restart: "",
    restartAt: null,
    lastChange: null,
    lastSeen: "",
    alarms,
  });
  const none: Thresholds = { cpuPct: null, tempC: null, clipPerMin: null, clipLatch: false };
  assert.equal(alarmTone("temp", row([])), "");
  assert.equal(alarmTone("temp", row([alarm({ code: "temperature", severity: "critical" })])), "bad");
  assert.equal(alarmTone("cpu", row([alarm({ code: "cpu", severity: "warning" })])), "warn");
  assert.equal(alarmTone("cpu", row([alarm({ code: "cpu", severity: "critical" })])), "bad");
  assert.equal(alarmTone("cpu", row([alarm({ code: "cpu", state: "acknowledged" })])), "");
  assert.equal(alarmTone("clip", row([alarm({ code: "temperature" })])), "");
  // Without the alarm, the rule decides as before.
  assert.equal(cellTone("temp", row([]), { ...none, tempC: 30 }), "bad");
});
