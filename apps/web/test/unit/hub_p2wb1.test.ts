// WP P2WB1 unit tests: the hub screen's model (the device table with offline
// devices, the alarm board's order, the health panel's conditions, the hub's
// history merged and read afresh after a renumbering), the hub feed through a
// fake core (caps, reads, the hub's events, Identify, Acknowledge), the hub
// backend (the serving hub's inventory as an installation, a PIN first, an
// upstream hub, the role's access) and where the hub screen goes.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import {
  CoreCallError,
  type HubAlarmRecord,
  type HubCapsRecord,
  type HubDeviceRecord,
  type HubHealthRecord,
  type HubHistoryRecord,
  type HubServerEventRecord,
} from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import type { Session } from "../../src/ui/session";
import {
  alarmConditions,
  alarmIdOf,
  alarmKey,
  alarmSummary,
  deviceRows,
  durationText,
  historyRow,
  mergeHistory,
  resetWords,
  restartText,
  sortAlarms,
  withAlarm,
  withDevice,
} from "../../src/screens/hub/model";
import { HubFeed, hubScreenShown } from "../../src/screens/hub/feed";
import { HUB_INSTALLATION_ID, HubBackend, accessOf, inventorySources } from "../../src/fleet/hub";
import { pageBackend } from "../../src/fleet/env";
import { LocalBackend } from "../../src/fleet/local";
import { screensFor, withHubScreen } from "../../src/ui/screens";

const OPEN = "DEMO0000DSPI0001";
const AWAY = "DEMO-OFFLINE-0042";

function device(serial: string, online: boolean, extra: Partial<HubDeviceRecord> = {}): HubDeviceRecord {
  return {
    serial,
    handle: online ? 1 : null,
    online,
    name: serial === OPEN ? "Stage left" : "Bar",
    platform: "RP2350",
    fw: "1.1.6",
    fwBeta: false,
    outputs: 9,
    board: "",
    hwRev: "",
    chip: "",
    chipRev: null,
    location: serial === OPEN ? "Stage" : "Bar",
    note: "",
    tags: [],
    firstSeen: "2026-11-01T10:00:00Z",
    lastSeen: online ? "2026-11-15T12:00:00Z" : "2026-11-10T09:30:00Z",
    lastReset: null,
    ...extra,
  };
}

function health(serial: string, extra: Partial<HubHealthRecord> = {}): HubHealthRecord {
  return {
    serial,
    handle: 1,
    at: "2026-11-15T12:00:00Z",
    cpuPct: 37,
    clipMask: 0,
    tempC: 35,
    uptimeS: 3600,
    hangCount: 0,
    unresponsive: false,
    unresponsiveSince: "",
    reset: { reason: "watchdog", prevUptimeS: 100, at: "2026-11-15T11:00:00Z" },
    // -- WP P3HB --
    counters: null,
    clipPerMin: null,
    lastChange: null,
    ...extra,
  };
}

function alarm(id: string, extra: Partial<HubAlarmRecord> = {}): HubAlarmRecord {
  return {
    id,
    source: "device",
    code: "temperature",
    subject: OPEN,
    serial: OPEN,
    severity: "critical",
    message: "35.0 °C, at or above 0 °C",
    state: "raised",
    raisedAt: "2026-11-15T12:00:00Z",
    clearedAt: "",
    acknowledgedBy: "",
    count: 1,
    reason: "",
    noticeKey: "alarm.temperature",
    ...extra,
  };
}

function caps(extra: Partial<HubCapsRecord> = {}): HubCapsRecord {
  return {
    onHub: true,
    inventory: true,
    identify: true,
    health: true,
    events: true,
    hubSnapshots: false,
    paramApi: false,
    hubStats: true,
    alarms: true,
    outbound: false,
    schedule: false,
    mqtt: false,
    apiKeys: false,
    rest: false,
    caps: [],
    ...extra,
  };
}

function record(id: number, extra: Partial<HubHistoryRecord> = {}): HubHistoryRecord {
  return {
    id,
    timeMs: 1_700_000_000_000 + id * 1000,
    kind: "notification",
    serial: OPEN,
    text: `Change ${id}`,
    source: "",
    session: 3,
    by: "Front of house",
    channel: null,
    value: null,
    fw: null,
    uptimeS: null,
    lines: [],
    count: 1,
    ...extra,
  };
}

// -- the model --

test("P2WB1: the device table lists every DSPi of the inventory, an offline one too, with its health", () => {
  const rows = deviceRows([device(OPEN, true), device(AWAY, false)], [health(OPEN)], OPEN);
  assert.equal(rows.length, 2);
  const [open, away] = rows;
  assert.equal(open!.name, "Stage left");
  assert.equal(open!.online, true);
  assert.equal(open!.isOpen, true);
  assert.equal(open!.cpu, 37);
  assert.equal(open!.tempC, 35);
  assert.match(open!.restart, /^Watchdog, /);
  assert.equal(away!.online, false);
  assert.equal(away!.isOpen, false);
  assert.equal(away!.cpu, null, "an offline device has no health");
  assert.equal(away!.location, "Bar");
  // A device the health poll knows before the inventory does is listed from its health.
  const more = deviceRows([], [health("NEW1")], "");
  assert.equal(more[0]!.serial, "NEW1");
  assert.equal(more[0]!.online, true);
  // The firmware with its beta.
  assert.equal(deviceRows([device(OPEN, true, { fwBeta: true })], [], "")[0]!.firmware, "1.1.6 beta");
});

test("P2WB1: restart and uptime words", () => {
  assert.equal(resetWords("brownout"), "Brown-out");
  assert.equal(resetWords("something new"), "Unknown cause");
  assert.equal(restartText(null), "");
  assert.equal(restartText({ reason: "power_on", prevUptimeS: null, at: "" }), "Power-on");
  assert.equal(durationText(40), "40 s");
  assert.equal(durationText(3 * 3600 + 5 * 60), "3 h 5 min");
  assert.equal(durationText(2 * 86400 + 4 * 3600), "2 d 4 h");
});

test("P2WB1: inventory.changed replaces, adds and forgets a record", () => {
  const list = [device(OPEN, true), device(AWAY, false)];
  const renamed = withDevice(list, AWAY, device(AWAY, false, { location: "Cellar" }));
  assert.equal(renamed[1]!.location, "Cellar");
  assert.equal(withDevice(list, AWAY, null).length, 1);
  assert.equal(withDevice(list, "NEW1", device("NEW1", true))[0]!.serial, "NEW1", "a new online device first");
});

test("P2WB1: the alarm board: raised first, newest first; an acknowledgement takes the hub's answer", () => {
  const list = sortAlarms([
    alarm("device:1", { state: "cleared", clearedAt: "2026-11-15T12:10:00Z" }),
    alarm("device:2", { raisedAt: "2026-11-15T12:01:00Z" }),
    alarm("device:3", { state: "acknowledged" }),
    alarm("device:4", { raisedAt: "2026-11-15T12:05:00Z", severity: "warning", code: "cpu" }),
  ]);
  assert.deepEqual(list.map((a) => a.id), ["device:4", "device:2", "device:3", "device:1"]);
  assert.deepEqual(alarmSummary(list), { raised: 2, critical: true });
  const acked = withAlarm(list, alarm("device:2", { state: "acknowledged", acknowledgedBy: "Front of house" }));
  assert.deepEqual(alarmSummary(acked), { raised: 1, critical: false });
  assert.equal(acked.find((a) => a.id === "device:2")!.acknowledgedBy, "Front of house");
});

test("P2WB1: the health panel's conditions: this DSPi's device alarms and the hub's own, in the notice's words", () => {
  const list = [
    alarm("device:1"),
    alarm("device:2", { serial: AWAY, subject: AWAY }),
    alarm("device:3", { serial: "", subject: "hub", code: "clock_unset", severity: "warning", noticeKey: "alarm.clock_unset" }),
    alarm("alink:4", { source: "alink", serial: "", code: "alink_lost" }),
    alarm("device:5", { state: "cleared" }),
  ];
  const conds = alarmConditions(list, OPEN, (a) => (a.code === "temperature" ? { key: "alarm.temperature", kind: "error", title: "Running hot", reason: "", action: "", detail: a.message } : null), () => "Stage left");
  assert.deepEqual(conds.map((c) => c.key), ["hub:device:1", "hub:device:3"]);
  assert.equal(conds[0]!.text, "Running hot: Stage left");
  assert.equal(conds[0]!.severity, "error");
  assert.equal(conds[0]!.kind, "hub");
  assert.equal(conds[1]!.severity, "warning");
  assert.equal(alarmIdOf(alarmKey(list[0]!)), "device:1");
  assert.equal(alarmIdOf("clip:3"), "");
  for (const c of conds) assert.doesNotMatch(c.text, /\b(phone|tablet|computer|PC|Mac)\b/i, "no host-device nouns");
});

test("P2WB1: the hub's history: who did it by name; merged without repeats; afresh after a renumbering", () => {
  const row = historyRow(record(7, { count: 3 }));
  assert.equal(row.by, "Front of house");
  assert.equal(row.kind, "Change");
  assert.equal(row.text, "Change 7 (3×)");
  assert.equal(historyRow(record(1, { kind: "identify" })).kind, "Identify");
  const kept = mergeHistory([record(1), record(2)], [record(2), record(3)], false);
  assert.deepEqual(kept.map((r) => r.id), [1, 2, 3]);
  assert.deepEqual(mergeHistory(kept, [record(1, { text: "again" })], true).map((r) => r.text), ["again"]);
});

// -- the feed --

function fakeSession(answers: Partial<Record<string, unknown>> = {}) {
  const calls: { method: string; params: unknown }[] = [];
  const watchers = new Set<() => void>();
  const hubWatchers = new Set<(e: HubServerEventRecord[]) => void>();
  const s = {
    answers: { hubCaps: caps(), hubRole: { role: "admin" }, hubInventory: [device(OPEN, true), device(AWAY, false)], hubHealth: [health(OPEN)], hubAlarms: [alarm("device:1")], hubStats: { sessions: 2, uptimeS: 90, devices: [] }, ...answers } as Record<string, unknown>,
    state: { status: "open", snapshot: { device: { serial: OPEN } }, link: { phase: "bound" } },
    deviceId: OPEN,
    readOnlyReason: "",
    openDevice: null,
    watch(fn: () => void) {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
    watchHub(fn: (e: HubServerEventRecord[]) => void) {
      hubWatchers.add(fn);
      return () => hubWatchers.delete(fn);
    },
    emit(events: HubServerEventRecord[]) {
      for (const w of hubWatchers) w(events);
    },
    core: {
      call: async (method: string, params: unknown) => {
        calls.push({ method, params });
        if (method === "hubAlarmNotice") {
          const a = (params as { alarm: HubAlarmRecord }).alarm;
          return { key: a.noticeKey, kind: "error", title: a.code === "temperature" ? "Running hot" : "Hub alarm", reason: "", action: "Let it cool.", detail: a.message };
        }
        if (method === "hubAlarmAck") {
          const id = (params as { id: string }).id;
          return alarm(id, { state: "acknowledged", acknowledgedBy: "uConsole for DSPi (web)" });
        }
        if (method === "hubIdentify" || method === "hubIdentifySerial") /* -- WP P2WB3 -- */ {
          const e = s.answers.hubIdentify;
          if (e instanceof CoreCallError) throw e;
          return e ?? { remainingS: 10, lastToneOutput: null, chip: "RP2350", chipRev: 2, chipId: 0, board: "pico2", hwRev: "" };
        }
        const a = s.answers[method];
        if (a instanceof CoreCallError) throw a;
        if (a === undefined) throw new CoreCallError("unknown_method", method);
        return a;
      },
    },
    calls,
  };
  return s;
}

const feeds: HubFeed[] = [];
after(() => feeds.forEach((f) => f.stop()));
function feedOn(s: ReturnType<typeof fakeSession>) {
  const f = new HubFeed(s as unknown as Session, { periodMs: 60_000, visible: () => true });
  feeds.push(f);
  return f;
}

test("P2WB1: the feed reads the inventory, health, alarms and (Admin) the statistics through the core's calls", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.tick();
  assert.ok(hubScreenShown(f.view.caps));
  assert.equal(f.view.role, "admin");
  assert.equal(f.view.devices.length, 2);
  assert.equal(f.view.health.length, 1);
  assert.equal(f.view.alarms[0]!.id, "device:1");
  assert.equal(f.view.stats?.sessions, 2);
  assert.equal(f.view.notices["device:1"]?.title, "Running hot");
  // Only the typed calls: never a raw hub request.
  for (const c of s.calls) assert.match(c.method, /^hub[A-Z]/);
});

test("P2WB1: a Control session reads no statistics; a hub without the families shows no screen and is not read", async () => {
  const s = fakeSession({ hubRole: { role: "control" } });
  const f = feedOn(s);
  f.start();
  await f.tick();
  assert.equal(f.view.stats, null);
  assert.ok(!s.calls.some((c) => c.method === "hubStats"));

  const up = fakeSession({ hubCaps: caps({ inventory: false, health: false, alarms: false, events: false, hubStats: false, identify: false }) });
  const g = feedOn(up);
  g.start();
  await g.tick();
  assert.equal(hubScreenShown(g.view.caps), false, "an upstream hub: no hub screen");
  assert.deepEqual(up.calls.map((c) => c.method).filter((m) => m !== "hubCaps" && m !== "hubRole"), []);

  const off = fakeSession({ hubCaps: caps({ onHub: false }) });
  const h = feedOn(off);
  h.start();
  await h.tick();
  assert.equal(hubScreenShown(h.view.caps), false);
  assert.deepEqual(off.calls.map((c) => c.method), ["hubCaps"]);
});

test("WEB5: while the link reconnects the hub's last view stays; once it is closed the view goes", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.tick();
  assert.ok(hubScreenShown(f.view.caps));
  s.state.link.phase = "reconnecting";
  s.answers.hubCaps = new CoreCallError("disconnected", "no hub");
  await f.tick();
  assert.ok(hubScreenShown(f.view.caps), "the hub page stays while reconnecting");
  assert.equal(f.view.devices.length, 2);
  s.state.link.phase = "closed";
  await f.tick();
  assert.equal(hubScreenShown(f.view.caps), false);
  assert.equal(f.view.devices.length, 0);
});

// -- WP REVEFIX -- (REVE C4)
test("REVEFIX C4: the page is stale only when the hub does not answer while the link reconnects; then no alarm is offered", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  const src = f.alarmSource();
  const seen: string[][] = [];
  const stop = src.subscribe((on) => seen.push(on.map((c) => c.key)));
  f.start();
  await f.tick();
  assert.equal(f.view.stale, false);
  // A device-only reconnect, or choosing a device: the hub answers, nothing is stale
  for (const phase of ["reconnecting", "choose_device", "no_device"]) {
    s.state.link.phase = phase;
    await f.tick();
    assert.equal(f.view.stale, false, phase);
  }
  assert.deepEqual(seen.at(-1), ["hub:device:1"]);
  // The hub itself does not answer: its last word stays, stale; its alarms are not offered nor acknowledged
  s.state.link.phase = "reconnecting";
  const good = s.answers.hubCaps;
  s.answers.hubCaps = new CoreCallError("disconnected", "no hub");
  await f.tick();
  assert.equal(f.view.stale, true);
  assert.equal(f.view.devices.length, 2);
  assert.deepEqual(seen.at(-1), []);
  await src.acknowledge("hub:device:1");
  assert.equal(s.calls.filter((c) => c.method === "hubAlarmAck").length, 0);
  // It answers again: fresh
  s.answers.hubCaps = good;
  s.state.link.phase = "bound";
  await f.tick();
  assert.equal(f.view.stale, false);
  assert.deepEqual(seen.at(-1), ["hub:device:1"]);
  stop();
});

test("P2WB1: the hub's alarm.changed and inventory.changed update the view between reads", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.tick();
  s.emit([
    { kind: "alarm.changed", serial: OPEN, record: null, device: null, snapshot: "", change: "", alarm: alarm("device:9", { code: "cpu", severity: "warning" }), run: null },
    { kind: "inventory.changed", serial: AWAY, record: null, device: null, snapshot: "", change: "", alarm: null, run: null },
  ]);
  assert.ok(f.view.alarms.some((a) => a.id === "device:9"));
  assert.ok(!f.view.devices.some((d) => d.serial === AWAY), "forgotten");
});

test("P2WB1: the health panel's source: conditions as they change; an acknowledgement goes to the hub", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  const src = f.alarmSource();
  const seen: string[][] = [];
  const stop = src.subscribe((on) => seen.push(on.map((c) => c.key)));
  f.start();
  await f.tick();
  assert.deepEqual(seen.at(-1), ["hub:device:1"]);
  await src.acknowledge("hub:device:1");
  assert.deepEqual(s.calls.filter((c) => c.method === "hubAlarmAck").map((c) => c.params), [{ id: "device:1" }]);
  assert.equal(f.view.alarms[0]!.state, "acknowledged");
  // Acknowledged, still on: listed until cleared, not acknowledged again.
  await src.acknowledge("hub:device:1");
  assert.equal(s.calls.filter((c) => c.method === "hubAlarmAck").length, 1);
  await src.acknowledge("clip:1");
  assert.equal(s.calls.filter((c) => c.method === "hubAlarmAck").length, 1);
  stop();
});

test("P2WB1: Identify answers: the blink, or the core's refusal with its notice", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  // -- WP P2WB3 -- through hubIdentifySerial, the device named
  f.start();
  await f.tick();
  const ok = await f.identify(OPEN);
  assert.equal(ok.bad, false);
  assert.equal(ok.text, "Stage left: blinking for 10 s.");
  assert.deepEqual(s.calls.find((c) => c.method === "hubIdentifySerial")!.params, { serial: OPEN, seconds: 10 });
  s.answers.hubIdentify = new CoreCallError("rate_limited", "hub refused: rate_limited", "dlink.rate_limited");
  const no = await f.identify(OPEN);
  assert.equal(no.bad, true);
  assert.match(no.text, /rate_limited|Too many/);
});

// -- the backend --

function fakeCore(answers: Record<string, unknown>) {
  const calls: string[] = [];
  let disposed = false;
  const core = {
    call: async (method: string) => {
      calls.push(method);
      const a = answers[method];
      if (a instanceof Error) throw a;
      if (a === undefined) throw new CoreCallError("unknown_method", method);
      return a;
    },
    dispose: async () => {
      disposed = true;
    },
    subscribe: () => () => undefined,
  };
  return { core: core as unknown as CoreClient, calls, disposed: () => disposed };
}

const HELLO = { hubId: "hub-1", hubName: "Venue hub", hubKind: "console", auth: "none", pinSource: "", needsPairing: false, tokenRefused: false, caps: ["inventory"], maxInflight: 4 };
const URL_ = "ws://127.0.0.1:9000";

test("P2WB1: a page served by a hub lists the hub's inventory as an installation, offline DSPis listed but not offered", async () => {
  const c = fakeCore({ linkConnect: HELLO, hubCaps: caps(), hubInventory: [device(OPEN, true), device(AWAY, false)], hubRole: { role: "viewer" } });
  const b = new HubBackend({ servingHub: URL_, spawnCore: () => c.core, storage: null });
  const list = await b.installations();
  assert.equal(list[0]!.id, HUB_INSTALLATION_ID);
  assert.equal(list[0]!.name, "Venue hub");
  assert.equal(list[0]!.editable, false);
  assert.deepEqual(list[0]!.sources, inventorySources(URL_, [device(OPEN, true), device(AWAY, false)]));
  assert.deepEqual(list[0]!.sources[1], { kind: "hub", url: URL_, deviceId: AWAY, name: "Bar", online: false });
  // This console's own installation stays, the serving hub's own entry in it.
  assert.equal(list[1]!.id, "local");
  assert.ok(list[1]!.sources.some((s) => s.kind === "hub" && s.url === URL_ && !s.deviceId));
  // Access from the role the hub gave: a Viewer views; elsewhere, control.
  assert.equal(await b.access(HUB_INSTALLATION_ID, list[0]!.sources[0]!, OPEN), "view");
  assert.equal(await b.access("local", { kind: "demo" }, "demo"), "control");
  assert.equal(await b.pinnedMode(), null, "the hub's own pin applies through each session's core");
  // One DSPi Link session of the backend's own, reused.
  await b.installations();
  assert.equal(c.calls.filter((m) => m === "linkConnect").length, 1);
});

test("P2WB1: a hub that wants a PIN first, or has no inventory, gives the standalone page", async () => {
  const pin = fakeCore({ linkConnect: { ...HELLO, needsPairing: true } });
  const b = new HubBackend({ servingHub: URL_, spawnCore: () => pin.core, storage: null });
  const list = await b.installations();
  assert.deepEqual(list.map((i) => i.id), ["local"]);
  assert.ok(pin.disposed(), "no session left waiting for a PIN");

  const up = fakeCore({ linkConnect: HELLO, hubCaps: caps({ inventory: false }) });
  let spawned = 0;
  const u = new HubBackend({ servingHub: URL_, spawnCore: () => (spawned++, up.core), storage: null });
  assert.deepEqual((await u.installations()).map((i) => i.id), ["local"]);
  assert.ok(up.disposed());
  await u.installations();
  assert.equal(spawned, 1, "an upstream hub is not asked again");

  const down = fakeCore({ linkConnect: new CoreCallError("link", "connection refused") });
  const d = new HubBackend({ servingHub: URL_, spawnCore: () => down.core, storage: null });
  assert.deepEqual((await d.installations()).map((i) => i.id), ["local"]);
  assert.equal(accessOf("control"), "control");
  assert.equal(accessOf("admin"), "control");
});

test("P2WB1: the backend choice: a hub's page gets the hub backend, any other page the standalone one", () => {
  const spawnCore = () => fakeCore({}).core;
  assert.ok(pageBackend({ servingHub: URL_, spawnCore, storage: null }) instanceof HubBackend);
  assert.ok(pageBackend({ servingHub: null, spawnCore, storage: null }) instanceof LocalBackend);
});

test("P2WB1: the hub screen, in Advanced and Installer before the event log, only on a hub with the families", () => {
  const ids = (m: "simple" | "advanced" | "installer", on: boolean) => withHubScreen(screensFor(m, false), m, on).map((x) => x.id);
  assert.ok(!ids("simple", true).includes("hub"));
  assert.ok(!ids("advanced", false).includes("hub"));
  const adv = ids("advanced", true);
  assert.equal(adv.indexOf("hub"), adv.indexOf("events") - 1);
  assert.ok(ids("installer", true).includes("hub"));
});
