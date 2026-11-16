// WP P3WGR unit tests: the groups' model (the editor's bounds, members joined to the inventory
// with the offline ones marked, `group.changed` applied in id order, each member's value from a
// volume with offsets clamped as the hub clamps, the offsets sent, a request's rows in words with
// a partial failure kept and counted, the bulk apply's countdown and timeout bounds, the fleet
// history merged by time and bounded) and the store reading the hub's groups through the core's
// typed call and following `group.changed`.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { CoreCallError, type HubDeviceRecord, type HubGroupRecord, type HubGroupRowRecord, type HubHistoryRecord, type HubServerEventRecord, type NoticeRecord } from "../../src/core/protocol";
import type { Session } from "../../src/ui/session";
import {
  APPLY_TIMEOUT_MAX_S,
  APPLY_TIMEOUT_MIN_S,
  FLEET_HISTORY_PAGE,
  GROUP_MEMBERS_MAX,
  GroupStore,
  applyOk,
  clampTo,
  confirmOk,
  countdownText,
  dbFrom,
  draftOf,
  draftProblem,
  failures,
  groupsOf,
  memberRows,
  mergeFleet,
  offsetList,
  offsetPreview,
  paramOk,
  presetOk,
  resultLines,
  runPending,
  secondsLeft,
  sortedGroups,
  timeoutFrom,
  toggleMember,
  valueText,
  withGroup,
} from "../../src/screens/hub/groups-model";

const A = "P3WGR-A";
const B = "P3WGR-B";
const OFF = "P3WGR-OFF";

function device(serial: string, online: boolean, name = ""): HubDeviceRecord {
  return {
    serial,
    handle: online ? 1 : null,
    online,
    name,
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
    firstSeen: "",
    lastSeen: "",
    lastReset: null,
  };
}

function group(id: string, members: string[], name = "Bar"): HubGroupRecord {
  return { id, name, note: "", members };
}

function row(serial: string, extra: Partial<HubGroupRowRecord> = {}): HubGroupRowRecord {
  return {
    serial,
    ok: true,
    code: "",
    msg: "",
    noticeKey: "",
    limit: "",
    value: null,
    choice: "",
    valueB64: "",
    clamped: false,
    slot: null,
    name: "",
    image: false,
    lines: [],
    notes: [],
    guard: null,
    reverted: false,
    ...extra,
  };
}

function rec(serial: string, id: number, timeMs: number, by = "", text = "Volume"): HubHistoryRecord {
  return { id, timeMs, kind: "param", serial, text, source: "", session: null, by, channel: null, value: null, fw: null, uptimeS: null, lines: [], count: 1 };
}

const notice = (key: string): NoticeRecord | null =>
  key === "dlink.offline" ? { key, kind: "warning", title: "The DSPi is not attached", reason: "", action: "", detail: "" } : null;

test("P3WGR: the editor's bounds are the hub's", () => {
  assert.equal(draftProblem(draftOf(null)), "Give the group a name.");
  assert.equal(draftProblem({ id: "", name: "  Bar ", note: "", members: [A] }), "");
  assert.match(draftProblem({ id: "", name: "x".repeat(49), note: "", members: [] }), /48/);
  assert.equal(draftProblem({ id: "", name: "x".repeat(48), note: "", members: [] }), "");
  assert.match(draftProblem({ id: "", name: "Bar", note: "n".repeat(257), members: [] }), /256/);
  assert.match(draftProblem({ id: "", name: "a\u0007b", note: "", members: [] }), /control/);
  const many = Array.from({ length: GROUP_MEMBERS_MAX + 1 }, (_, i) => `S${i}`);
  assert.match(draftProblem({ id: "", name: "Bar", note: "", members: many }), /64/);
  assert.match(draftProblem({ id: "", name: "Bar", note: "", members: [A, A] }), /twice/);
  // Members picked keep the order they were picked in; a second click takes one out.
  let d = toggleMember(toggleMember(draftOf(null), B), A);
  assert.deepEqual(d.members, [B, A]);
  d = toggleMember(d, B);
  assert.deepEqual(d.members, [A]);
  assert.deepEqual(draftOf(group("g3", [A])), { id: "g3", name: "Bar", note: "", members: [A] });
});

test("P3WGR: members joined to the inventory, the offline one marked; the groups of a DSPi", () => {
  const devices = [device(A, true, "Stage"), device(OFF, false, "Bar")];
  const rows = memberRows(group("g1", [OFF, A, "P3WGR-GONE"]), devices);
  assert.deepEqual(
    rows.map((r) => [r.serial, r.name, r.online, r.known]),
    [
      [OFF, "Bar", false, true],
      [A, "Stage", true, true],
      ["P3WGR-GONE", "P3WGR-GONE", false, false],
    ],
  );
  const gs = [group("g1", [A]), group("g2", [B]), group("g3", [A, B])];
  assert.deepEqual(groupsOf(gs, A).map((g) => g.id), ["g1", "g3"]);
});

test("P3WGR: group.changed applied in the hub's id order, a delete drops it", () => {
  let gs = sortedGroups([group("g10", []), group("g2", [])]);
  assert.deepEqual(gs.map((g) => g.id), ["g2", "g10"]);
  gs = withGroup(gs, group("g3", [A], "New"), "set");
  assert.deepEqual(gs.map((g) => g.id), ["g2", "g3", "g10"]);
  gs = withGroup(gs, group("g3", [A, B], "Renamed"), "forgot");
  assert.equal(gs.find((g) => g.id === "g3")?.name, "Renamed");
  assert.equal(gs.length, 3);
  gs = withGroup(gs, group("g2", []), "delete");
  assert.deepEqual(gs.map((g) => g.id), ["g3", "g10"]);
});

test("P3WGR: each member's volume with its offset, clamped into the range as the hub clamps it", () => {
  const range = { min: -60, max: 0 };
  const p = offsetPreview(-20, [A, B, OFF], { [A]: -3, [B]: 25 }, range);
  assert.deepEqual(p, [
    { serial: A, offset: -3, value: -23, clamped: false },
    { serial: B, offset: 25, value: 0, clamped: true },
    { serial: OFF, offset: 0, value: -20, clamped: false },
  ]);
  assert.deepEqual(offsetPreview(-58, [A], { [A]: -5 }, range)[0], { serial: A, offset: -5, value: -60, clamped: true });
  // Unknown bounds: nothing clamped.
  assert.equal(clampTo(5, undefined, undefined), 5);
  // Only the members' non-zero offsets are sent.
  assert.deepEqual(offsetList([A, B], { [A]: -3, [B]: 0, [OFF]: 4 }), [{ serial: A, db: -3 }]);
  assert.equal(dbFrom(" −3,5 "), -3.5);
  assert.equal(dbFrom("abc"), null);
  assert.equal(dbFrom(""), null);
});

test("P3WGR: a request's rows in words, a partial failure kept and counted", () => {
  const name = (s: string) => (s === A ? "Stage" : s === OFF ? "Bar" : s);
  const rows = [
    row(A, { value: { float: -23 } }),
    row(B, { value: { float: 0 }, clamped: true }),
    row(OFF, { ok: false, code: "offline", msg: "not attached", noticeKey: "dlink.offline" }),
    row("P3WGR-X", { ok: false, code: "limit_exceeded", msg: "past the limit" }),
  ];
  const lines = resultLines(rows, name, paramOk("vol.user"), notice);
  assert.deepEqual(
    lines.map((l) => [l.name, l.ok, l.text, l.tone, l.key]),
    [
      ["Stage", true, "-23.0 dB", "", ""],
      [B, true, "0.0 dB (clamped to its range)", "", ""],
      ["Bar", false, "The DSPi is not attached: not attached", "warn", "dlink.offline"],
      ["P3WGR-X", false, "past the limit", "bad", ""],
    ],
  );
  assert.equal(failures(lines), 2);
  assert.equal(valueText({ bool: true }, "vol.mute"), "Muted");
  assert.equal(valueText({ bool: false }, "vol.mute"), "Not muted");
  assert.equal(presetOk(row(A, { slot: 2, name: "Evening" })), "Preset 3 loaded (Evening)");
  assert.equal(presetOk(row(A, { slot: 4 })), "Preset 5 loaded");
  assert.equal(applyOk(row(A, { guard: 17 })), "Applied, waiting for Keep");
  assert.equal(applyOk(row(A)), "Applied");
  assert.equal(applyOk(row(A, { guard: 17, reverted: true })), "Reverted");
  assert.equal(confirmOk(row(A, { guard: 17 })), "Kept");
  assert.equal(confirmOk(row(A, { guard: 17, reverted: true })), "Reverted");
});

test("P3WGR: the bulk apply's countdown, its timeout bounds and when a run waits", () => {
  const now = Date.parse("2026-11-16T20:00:00Z");
  assert.equal(secondsLeft("2026-11-16T20:01:05Z", now), 65);
  assert.equal(secondsLeft("2026-11-16T19:59:00Z", now), 0);
  assert.equal(secondsLeft("", now), null);
  assert.equal(secondsLeft("soon", now), null);
  assert.equal(countdownText(65), "1:05");
  assert.equal(countdownText(9), "0:09");
  assert.equal(timeoutFrom("45"), 45);
  assert.equal(timeoutFrom("0"), APPLY_TIMEOUT_MIN_S);
  assert.equal(timeoutFrom("9000"), APPLY_TIMEOUT_MAX_S);
  assert.equal(timeoutFrom("x"), null);
  assert.equal(timeoutFrom(""), null);
  assert.equal(runPending([row(A, { guard: 1 }), row(B, { ok: false, code: "offline" })]), true);
  assert.equal(runPending([row(A, { guard: 1, reverted: true }), row(B)]), false);
});

test("P3WGR: the fleet history merged by time, newest first, with who did it, bounded", () => {
  const name = (s: string) => `name of ${s}`;
  const pages = [
    { serial: A, records: [rec(A, 1, 1000, "uConsole"), rec(A, 2, 3000, "uConsole")] },
    { serial: B, records: [rec(B, 1, 2000, "Kiosk"), rec(B, 1, 2000, "Kiosk")] },
    { serial: OFF, records: [] },
  ];
  const rows = mergeFleet(pages, name, (k) => k.toUpperCase());
  assert.deepEqual(
    rows.map((r) => [r.serial, r.t, r.by, r.name, r.kind]),
    [
      [A, 3000, "uConsole", `name of ${A}`, "PARAM"],
      [B, 2000, "Kiosk", `name of ${B}`, "PARAM"],
      [A, 1000, "uConsole", `name of ${A}`, "PARAM"],
    ],
  );
  // At most FLEET_HISTORY_PAGE of one member, and at most `keep` in all.
  const long = { serial: A, records: Array.from({ length: FLEET_HISTORY_PAGE + 20 }, (_, i) => rec(A, i + 1, i)) };
  assert.equal(mergeFleet([long], name, (k) => k).length, FLEET_HISTORY_PAGE);
  assert.equal(mergeFleet([long], name, (k) => k)[0]!.t, FLEET_HISTORY_PAGE + 19, "the newest kept");
  assert.equal(mergeFleet([long, pages[1]!], name, (k) => k, 5).length, 5);
});

// -- the store --

function fakeSession(answers: Record<string, unknown> = {}) {
  const calls: string[] = [];
  const hubFns = new Set<(e: HubServerEventRecord[]) => void>();
  const s = {
    answers: { hubGroups: [group("g2", [B]), group("g1", [A])], ...answers } as Record<string, unknown>,
    state: { link: { phase: "bound" } },
    watch: () => () => undefined,
    watchHub: (fn: (e: HubServerEventRecord[]) => void) => {
      hubFns.add(fn);
      return () => hubFns.delete(fn);
    },
    emit: (e: HubServerEventRecord[]) => hubFns.forEach((f) => f(e)),
    hubFns,
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

function changed(g: HubGroupRecord, change: string): HubServerEventRecord {
  return { kind: "group.changed", serial: "", record: null, device: null, snapshot: "", change, alarm: null, run: null, group: g };
}

const stores: GroupStore[] = [];
after(() => stores.forEach((x) => x.stop()));

test("P3WGR: the store reads the hub's groups through the core's call and follows group.changed", async () => {
  const s = fakeSession();
  const store = new GroupStore(s as unknown as Session);
  stores.push(store);
  store.start();
  await store.refresh();
  assert.deepEqual(store.view.groups.map((g) => g.id), ["g1", "g2"]);
  assert.equal(store.view.read, true);
  assert.deepEqual([...new Set(s.calls)], ["hubGroups"], "only the typed call");
  s.emit([changed(group("g3", [A, B], "Both"), "set")]);
  assert.deepEqual(store.view.groups.map((g) => g.id), ["g1", "g2", "g3"]);
  s.emit([changed(group("g1", [A]), "delete")]);
  assert.deepEqual(store.view.groups.map((g) => g.id), ["g2", "g3"]);
  // An answer taken at once, and a delete.
  store.took(group("g4", [A], "Four"));
  assert.equal(store.view.groups.at(-1)?.name, "Four");
  store.dropped("g4");
  assert.equal(store.view.groups.some((g) => g.id === "g4"), false);
  store.stop();
  assert.equal(s.hubFns.size, 0, "the watch ends with the store");
});

test("P3WGR: a failed read keeps the groups read before and says why", async () => {
  const s = fakeSession();
  const store = new GroupStore(s as unknown as Session);
  stores.push(store);
  store.start();
  await store.refresh();
  s.answers.hubGroups = new CoreCallError("link", "gone", "dlink.offline");
  await store.refresh();
  assert.equal(store.view.groups.length, 2);
  assert.match(store.view.error, /gone/);
});
