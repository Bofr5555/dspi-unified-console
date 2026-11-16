// WP P3WHB unit tests: the health board's model (rows from the inventory, the health
// records and the alarms, online above offline; the counters' columns only when the hub
// gives them; cells coloured from the hub's alarm rules and not without them; clip events
// a minute or the latch; sort by any column, missing values last; the tag filter) and the
// feed reading the alarm rules through the core's typed call (once a period, kept on a
// failed read, none off a hub).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import {
  CoreCallError,
  type HubAlarmRecord,
  type HubCapsRecord,
  type HubDeviceRecord,
  type HubHealthRecord,
  type HubServerEventRecord,
} from "../../src/core/protocol";
import type { HubAlarmRuleRecord } from "../../src/core/hub-settings-protocol";
import type { Session } from "../../src/ui/session";
import {
  allTags,
  boardColumns,
  boardRows,
  cellTone,
  clipText,
  filterRows,
  nextSort,
  sortRows,
  thresholdsFrom,
  type BoardRow,
} from "../../src/screens/hub/health-model";
import { HubFeed } from "../../src/screens/hub/feed";

const A = "P3WHB-A";
const B = "P3WHB-B";
const C = "P3WHB-C";
const GONE = "P3WHB-OFF";

function device(serial: string, online: boolean, extra: Partial<HubDeviceRecord> = {}): HubDeviceRecord {
  return {
    serial,
    handle: online ? 1 : null,
    online,
    name: serial.slice(6),
    platform: "RP2350",
    fw: "1.1.6",
    fwBeta: false,
    outputs: 9,
    board: "",
    hwRev: "",
    chip: "",
    chipRev: null,
    location: "",
    note: "",
    tags: [],
    firstSeen: "2026-11-01T10:00:00Z",
    lastSeen: online ? "2026-11-16T12:00:00Z" : "2026-11-10T09:30:00Z",
    lastReset: null,
    ...extra,
  };
}

function health(serial: string, extra: Partial<HubHealthRecord> = {}): HubHealthRecord {
  return {
    serial,
    handle: 1,
    at: "2026-11-16T12:00:00Z",
    cpuPct: 40,
    clipMask: 0,
    tempC: 35,
    uptimeS: 3600,
    hangCount: 0,
    unresponsive: false,
    unresponsiveSince: "",
    reset: { reason: "power_on", prevUptimeS: null, at: "2026-11-16T11:00:00Z" },
    counters: null,
    clipPerMin: null,
    lastChange: null,
    ...extra,
  };
}

function alarm(id: string, serial: string, extra: Partial<HubAlarmRecord> = {}): HubAlarmRecord {
  return {
    id,
    source: "device",
    code: "temperature",
    subject: serial,
    serial,
    severity: "critical",
    message: "hot",
    state: "raised",
    raisedAt: "2026-11-16T12:00:00Z",
    clearedAt: "",
    acknowledgedBy: "",
    count: 1,
    reason: "",
    noticeKey: "alarm.temperature",
    ...extra,
  };
}

function rule(code: string, extra: Partial<HubAlarmRuleRecord> = {}): HubAlarmRuleRecord {
  return { code, on: true, afterS: null, perMinute: null, muteOnStorm: null, aboveC: null, abovePct: null, forS: null, ...extra };
}

const RULES = [rule("temperature", { aboveC: 70 }), rule("cpu", { abovePct: 90, forS: 10 }), rule("clip", { perMinute: 10 })];

function rows(): BoardRow[] {
  return boardRows(
    [device(GONE, false, { tags: ["stage"] }), device(A, true, { tags: ["stage", "left"] }), device(B, true, { tags: ["bar"] })],
    [
      health(A, { cpuPct: 95, tempC: 71.5, clipPerMin: 12, counters: { underruns: 4, starvation: 1 }, lastChange: { at: "2026-11-16T11:59:00Z", by: "Front of house", text: "Master volume −12.0 dB" } }),
      health(B, { cpuPct: 20, tempC: 40, clipMask: 3, uptimeS: 60, hangCount: 2 }),
      health(C, { cpuPct: null, tempC: null, clipMask: null, uptimeS: null, hangCount: null, reset: null }),
    ],
    [alarm("device:1", A), alarm("device:2", A, { state: "acknowledged", code: "cpu", severity: "warning" }), alarm("device:3", B, { state: "cleared" }), alarm("alink:4", A, { source: "alink" }), alarm("device:5", GONE, { code: "device_offline" })],
    B,
  );
}

test("P3WHB: the board's rows: online first, offline below, health and alarms joined, a device only in the health list too", () => {
  const r = rows();
  assert.deepEqual(r.map((x) => x.serial), [A, B, C, GONE]);
  const a = r[0]!;
  assert.equal(a.name, "A");
  assert.equal(a.cpu, 95);
  assert.equal(a.underruns, 4);
  assert.equal(a.starvation, 1);
  assert.equal(a.lastChange?.by, "Front of house");
  // The alarms that are on about it, not the cleared, not the audio link's.
  assert.deepEqual(a.alarms.map((x) => x.id), ["device:1", "device:2"]);
  assert.equal(r[1]!.isOpen, true);
  assert.equal(r[1]!.clipped, true);
  assert.equal(r[1]!.restart.startsWith("Power-on"), true);
  assert.equal(r[2]!.name, C);
  assert.equal(r[2]!.clipped, null);
  const off = r[3]!;
  assert.equal(off.online, false);
  assert.equal(off.cpu, null);
  assert.deepEqual(off.alarms.map((x) => x.id), ["device:5"]);
});

test("P3WHB: the counters' columns show only when the hub gives counters", () => {
  assert.equal(boardColumns(rows()).counters, true);
  const none = boardRows([device(A, true)], [health(A)], [], "");
  assert.equal(boardColumns(none).counters, false);
  assert.equal(none[0]!.underruns, null);
  // Half a reading (the starvation read before the underruns') still shows the columns.
  const half = boardRows([device(A, true)], [health(A, { counters: { underruns: null, starvation: 0 } })], [], "");
  assert.equal(boardColumns(half).counters, true);
});

test("P3WHB: a cell is coloured from the hub's rules: at or above the threshold, not below, not with the rule off or absent", () => {
  const [a, b, c] = rows();
  const t = thresholdsFrom(RULES);
  assert.deepEqual(t, { cpuPct: 90, tempC: 70, clipPerMin: 10, clipLatch: true });
  assert.equal(cellTone("temp", a!, t), "bad");
  assert.equal(cellTone("cpu", a!, t), "warn");
  assert.equal(cellTone("clip", a!, t), "warn");
  assert.equal(cellTone("temp", b!, t), "");
  assert.equal(cellTone("cpu", b!, t), "");
  // Latch-only firmware: a set latch is the clip alarm while the rule is on.
  assert.equal(cellTone("clip", b!, t), "warn");
  assert.equal(cellTone("temp", c!, t), "", "no reading, no colour");
  // The thresholds are the hub's: a different rule moves the colour.
  assert.equal(cellTone("temp", b!, thresholdsFrom([rule("temperature", { aboveC: 40 })])), "bad");
  assert.equal(cellTone("temp", b!, thresholdsFrom([rule("temperature", { aboveC: 40.5 })])), "");
  // Off, or not read: nothing coloured (nothing hard-coded).
  const off = thresholdsFrom(RULES.map((x) => ({ ...x, on: false })));
  for (const col of ["temp", "cpu", "clip"] as const) assert.equal(cellTone(col, a!, off), "");
  for (const col of ["temp", "cpu", "clip"] as const) assert.equal(cellTone(col, a!, thresholdsFrom([])), "");
});

test("P3WHB: clip as events a minute, or the latch's word on firmware with only the latch", () => {
  const [a, b, c] = rows();
  assert.equal(clipText(a!), "12/min");
  assert.equal(clipText(b!), "Clipped");
  assert.equal(clipText(c!), "–");
  assert.equal(clipText(boardRows([device(A, true)], [health(A)], [], "")[0]!), "No");
});

test("P3WHB: sort by any column: the order asked, missing values last, offline always below", () => {
  const r = rows();
  const by = (key: Parameters<typeof sortRows>[1]["key"], desc: boolean) => sortRows(r, { key, desc }).map((x) => x.serial);
  assert.deepEqual(by("cpu", true), [A, B, C, GONE]);
  assert.deepEqual(by("cpu", false), [B, A, C, GONE]);
  assert.deepEqual(by("temp", false), [B, A, C, GONE]);
  assert.deepEqual(by("hangs", true), [B, A, C, GONE]);
  assert.deepEqual(by("uptime", false), [B, A, C, GONE]);
  assert.deepEqual(by("alarms", true), [A, B, C, GONE]);
  assert.deepEqual(by("name", false), [A, B, C, GONE]);
  assert.deepEqual(by("name", true), [C, B, A, GONE]);
  assert.deepEqual(by("change", true), [A, B, C, GONE]);
  assert.deepEqual(by("clip", true), [A, B, C, GONE]);
  for (const key of ["state", "underruns", "starvation", "restart"] as const) assert.equal(by(key, true).at(-1), GONE);
  assert.deepEqual(nextSort({ key: "name", desc: false }, "name"), { key: "name", desc: true });
  assert.deepEqual(nextSort({ key: "name", desc: false }, "temp"), { key: "temp", desc: true });
  assert.deepEqual(nextSort({ key: "temp", desc: true }, "name"), { key: "name", desc: false });
});

test("P3WHB: the tag filter: any tag containing the text, ignoring case; offline rows filtered too", () => {
  const r = rows();
  assert.deepEqual(filterRows(r, "STAGE").map((x) => x.serial), [A, GONE]);
  assert.deepEqual(filterRows(r, " ba ").map((x) => x.serial), [B]);
  assert.equal(filterRows(r, "").length, 4);
  assert.equal(filterRows(r, "nowhere").length, 0);
  assert.deepEqual(allTags(r), ["bar", "left", "stage"]);
});

// -- the feed's alarm rules --

function caps(extra: Partial<HubCapsRecord> = {}): HubCapsRecord {
  return {
    onHub: true,
    inventory: true,
    identify: true,
    health: true,
    events: true,
    hubSnapshots: false,
    paramApi: false,
    hubStats: false,
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

function fakeSession(answers: Record<string, unknown> = {}) {
  const calls: string[] = [];
  const s = {
    answers: { hubCaps: caps(), hubRole: { role: "viewer" }, hubInventory: [device(A, true)], hubHealth: [health(A)], hubAlarms: [], hubAlarmRulesGet: RULES, ...answers } as Record<string, unknown>,
    state: { status: "open", snapshot: { device: { serial: A } }, link: { phase: "bound" } },
    deviceId: A,
    readOnlyReason: "",
    openDevice: null,
    watch: () => () => undefined,
    watchHub: (_fn: (e: HubServerEventRecord[]) => void) => () => undefined,
    core: {
      call: async (method: string) => {
        calls.push(method);
        const a = s.answers[method];
        if (a instanceof Error) throw a;
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

test("P3WHB: the feed reads the alarm rules through the core's call, once a period, and keeps them on a failed read", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.tick();
  assert.deepEqual(f.view.rules, RULES);
  assert.equal(s.calls.filter((c) => c === "hubAlarmRulesGet").length, 1);
  // The next read within the period does not ask again.
  await f.tick();
  assert.equal(s.calls.filter((c) => c === "hubAlarmRulesGet").length, 1);
  for (const c of s.calls) assert.match(c, /^hub[A-Z]/, "only the typed calls");
  // A failed read later keeps the last rules and is not the page's error.
  const g = feedOn(fakeSession({ hubAlarmRulesGet: new CoreCallError("offline", "gone") }));
  g.start();
  await g.tick();
  assert.deepEqual(g.view.rules, []);
  assert.equal(g.view.error, "");
});

test("P3WHB: no alarms family, or off a hub: no rules read, none kept", async () => {
  const s = fakeSession({ hubCaps: caps({ alarms: false }) });
  const f = feedOn(s);
  f.start();
  await f.tick();
  assert.deepEqual(f.view.rules, []);
  assert.equal(s.calls.includes("hubAlarmRulesGet"), false);
  const t = fakeSession({ hubCaps: caps({ onHub: false }) });
  const g = feedOn(t);
  g.start();
  await g.tick();
  assert.deepEqual(g.view.rules, []);
  assert.equal(t.calls.includes("hubAlarmRulesGet"), false);
});
