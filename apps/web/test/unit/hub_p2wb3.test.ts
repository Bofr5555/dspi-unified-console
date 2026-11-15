// WP P2WB3 unit tests: the hub screen's row actions (Identify on every online row, an Admin's
// Edit and Forget) and the tags as typed; the hub feed's writes through the core's typed calls
// (hubIdentifySerial, hubInventorySet, hubInventoryForget, hubRename); the hub's history kept
// current by the hub's `event.added` after `hubEventsWatch`, read afresh after a renumbering and
// watched again after a reconnect, no poll; a recall on the hub whose guard the core adopted, the
// desk's recall as the way back, and an unguarded one.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { CoreCallError, type GuardedApplyRecord, type HubDeviceRecord, type HubHistoryRecord, type HubServerEventRecord } from "../../src/core/protocol";
import type { Session } from "../../src/ui/session";
import { deviceRows, rowActions, tagsFrom } from "../../src/screens/hub/model";
import { HubFeed } from "../../src/screens/hub/feed";
import { HubHistoryFeed } from "../../src/screens/events/hub-history";
import { SnapshotDesk } from "../../src/screens/snapshots/model";

const OPEN = "DEMO0000DSPI0001";
const AWAY = "DEMO-OFFLINE-0042";
const OTHER = "DEMO0000DSPI0002";

function device(serial: string, online: boolean, extra: Partial<HubDeviceRecord> = {}): HubDeviceRecord {
  return {
    serial,
    handle: online ? 1 : null,
    online,
    name: serial === OPEN ? "Stage left" : serial === OTHER ? "Stage right" : "Bar",
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
    ...extra,
  };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

type Answer = unknown | ((params: unknown) => unknown);

/** A session on a hub with a fake core: `answers` per method (a function gets the parameters). */
function fakeSession(answers: Record<string, Answer> = {}) {
  const calls: { method: string; params: unknown }[] = [];
  const watchers = new Set<() => void>();
  const hubWatchers = new Set<(e: HubServerEventRecord[]) => void>();
  const s = {
    answers: { ...answers } as Record<string, Answer>,
    state: { status: "open", snapshot: { device: { serial: OPEN } }, link: { phase: "bound" }, hubName: "Venue hub" } as Record<string, unknown>,
    deviceId: OPEN,
    readOnlyReason: "",
    renamed: [] as string[],
    hubRenamed(name: string) {
      s.renamed.push(name);
    },
    refresh: async () => undefined,
    watch(fn: () => void) {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
    watchHub(fn: (e: HubServerEventRecord[]) => void) {
      hubWatchers.add(fn);
      return () => hubWatchers.delete(fn);
    },
    emit(events: HubServerEventRecord[]) {
      for (const w of [...hubWatchers]) w(events);
    },
    setPhase(phase: string) {
      s.state = { ...s.state, link: { phase } };
      for (const w of [...watchers]) w();
    },
    hubWatchers,
    core: {
      call: async (method: string, params: unknown) => {
        calls.push({ method, params });
        let a = s.answers[method];
        if (typeof a === "function") a = (a as (p: unknown) => unknown)(params);
        if (a instanceof CoreCallError) throw a;
        if (a === undefined) throw new CoreCallError("unknown_method", method);
        return a;
      },
    },
    calls,
  };
  return s;
}

const caps = { onHub: true, inventory: true, identify: true, health: true, events: true, hubSnapshots: true, paramApi: false, hubStats: true, alarms: false, outbound: false, schedule: false, mqtt: false, apiKeys: false, rest: false, caps: [] };

const feeds: { stop(): void }[] = [];
after(() => feeds.forEach((f) => f.stop()));

test("P2WB3: Identify on every online row; Edit for an Admin on every row and Forget on an offline one only", () => {
  const rows = deviceRows([device(OPEN, true), device(OTHER, true), device(AWAY, false)], [], OPEN);
  const admin = { role: "admin", identify: true, inventory: true, readOnly: "", canOpen: true };
  const [own, other, away] = rows.map((r) => rowActions(r, admin));
  assert.deepEqual(own, { identify: true, identifyOff: "", open: false, edit: true, forget: false });
  assert.deepEqual(other, { identify: true, identifyOff: "", open: true, edit: true, forget: false });
  assert.deepEqual(away, { identify: false, identifyOff: "", open: false, edit: true, forget: true });
  // Control: Identify, no Admin edits; a Viewer sees Identify off with the reason.
  const ctl = rowActions(rows[1]!, { ...admin, role: "control" });
  assert.equal(ctl.identify && !ctl.identifyOff && !ctl.edit && !ctl.forget, true);
  const viewer = rowActions(rows[1]!, { ...admin, role: "viewer" });
  assert.equal(viewer.identifyOff, "A Viewer cannot identify a DSPi");
  assert.equal(rowActions(rows[2]!, { ...admin, role: "viewer" }).forget, false);
  // Read-only: Identify off with the session's reason, no edits; a hub without the families: none.
  const ro = rowActions(rows[2]!, { ...admin, readOnly: "Read-only firmware" });
  assert.equal(ro.forget || ro.edit, false);
  assert.equal(rowActions(rows[1]!, { ...admin, readOnly: "Read-only firmware" }).identifyOff, "Read-only firmware");
  assert.deepEqual(rowActions(rows[1]!, { ...admin, identify: false, inventory: false, canOpen: false }), { identify: false, identifyOff: "", open: false, edit: false, forget: false });
});

test("P2WB3: tags as typed: trimmed, empty ones and repeats dropped", () => {
  assert.deepEqual(tagsFrom(" main, stage ,, main,  "), ["main", "stage"]);
  assert.deepEqual(tagsFrom(""), []);
});

test("P2WB3: the feed's writes go through the core's typed calls and the table takes the hub's answers", async () => {
  const s = fakeSession({
    hubCaps: caps,
    hubRole: { role: "admin" },
    hubInventory: [device(OPEN, true), device(OTHER, true), device(AWAY, false)],
    hubHealth: [],
    hubStats: { sessions: 1, uptimeS: 5, devices: [] },
    hubIdentifySerial: { remainingS: 10, lastToneOutput: null, chip: "RP2350", chipRev: 2, chipId: 0, board: "", hwRev: "" },
    hubInventorySet: (p: unknown) => device(AWAY, false, { location: (p as { location: string }).location, tags: (p as { tags: string[] }).tags }),
    hubInventoryForget: { serial: AWAY, forgot: ["record", "events", "snapshots"] },
    hubRename: "Studio",
  });
  const f = new HubFeed(s as unknown as Session, { periodMs: 60_000, visible: () => true });
  feeds.push(f);
  f.start();
  await f.tick();

  // Identify a DSPi that is not the open one.
  const id = await f.identify(OTHER);
  assert.equal(id.bad, false);
  assert.equal(id.text, "Stage right: blinking for 10 s.");
  assert.deepEqual(s.calls.find((c) => c.method === "hubIdentifySerial")!.params, { serial: OTHER, seconds: 10 });
  assert.ok(!s.calls.some((c) => c.method === "hubIdentify"), "never only the open one's call");

  const set = await f.setDetails(AWAY, { location: "Bar", note: "", tags: ["main"] });
  assert.equal(set.bad, false);
  assert.deepEqual(s.calls.find((c) => c.method === "hubInventorySet")!.params, { serial: AWAY, location: "Bar", note: "", tags: ["main"] });
  assert.deepEqual(f.view.devices.find((d) => d.serial === AWAY)!.tags, ["main"]);

  const gone = await f.forget(AWAY, true);
  assert.equal(gone.text, "Forgot Bar and its snapshots on the hub.");
  assert.deepEqual(s.calls.find((c) => c.method === "hubInventoryForget")!.params, { serial: AWAY, withData: true });
  assert.ok(!f.view.devices.some((d) => d.serial === AWAY));

  const named = await f.rename("Studio");
  assert.equal(named.text, "The hub is now called Studio.");
  assert.deepEqual(s.renamed, ["Studio"]);

  // The hub's refusals in the core's words: forget while online is `conflict`.
  s.answers.hubInventoryForget = new CoreCallError("conflict", "hub refused: conflict", "dlink.conflict");
  const no = await f.forget(OTHER, false);
  assert.equal(no.bad, true);
  assert.ok(f.view.devices.some((d) => d.serial === OTHER), "kept");
  s.answers.hubRename = new CoreCallError("denied", "hub refused: denied");
  assert.equal((await f.rename("X")).bad, true);
  assert.deepEqual(s.renamed, ["Studio"]);
  for (const c of s.calls) assert.match(c.method, /^hub[A-Z]/);
});

function rec(id: number, serial = OPEN, kind = "notification"): HubHistoryRecord {
  return { id, timeMs: 1_000 * id, kind, serial, text: `${kind} ${id}`, source: "hub", session: null, by: "", channel: null, value: null, fw: null, uptimeS: null, lines: [], count: 1 };
}
const added = (r: HubHistoryRecord): HubServerEventRecord => ({ kind: "event.added", serial: r.serial, record: r, device: null, snapshot: "", change: "", alarm: null, run: null });

test("P2WB3: the hub's history: watched, then the hub's own event.added; no poll", async () => {
  let page = [rec(1), rec(2)];
  let newest = 2;
  const s = fakeSession({
    hubEventsWatch: null,
    hubEventsRead: (p: unknown) => {
      const after = (p as { afterId: number }).afterId;
      return { records: page.filter((r) => r.id > after), more: false, newestId: newest };
    },
  });
  const h = new HubHistoryFeed(s as unknown as Session);
  feeds.push(h);
  h.start();
  await h.sync();
  assert.deepEqual(h.view.records.map((r) => r.id), [1, 2]);
  // The watch first, for the open DSPi, then the read.
  assert.deepEqual(s.calls.map((c) => c.method).slice(0, 2), ["hubEventsWatch", "hubEventsRead"]);
  assert.deepEqual(s.calls[0]!.params, { on: true, serial: OPEN });

  // The hub's records as they come; another DSPi's and repeats are not this one's.
  const reads = () => s.calls.filter((c) => c.method === "hubEventsRead").length;
  const before = reads();
  s.emit([added(rec(3)), added(rec(3)), added(rec(9, OTHER)), { kind: "inventory.changed", serial: OPEN, record: null, device: null, snapshot: "", change: "", alarm: null, run: null }]);
  assert.deepEqual(h.view.records.map((r) => r.id), [1, 2, 3]);
  await sleep(50);
  assert.equal(reads(), before, "no read for what the hub sent, and no poll");

  // A forget renumbered the log: an id below the last one kept, unknown: read afresh.
  page = [rec(1, OPEN, "connected")];
  newest = 1;
  s.emit([added(page[0]!)]);
  await sleep(20);
  assert.deepEqual(h.view.records.map((r) => r.kind), ["connected"]);

  // The DSPi Link lost and bound again: the watch asked again, the gap read.
  page = [rec(1, OPEN, "connected"), rec(2)];
  newest = 2;
  const watches = () => s.calls.filter((c) => c.method === "hubEventsWatch").length;
  const w = watches();
  s.setPhase("reconnecting");
  s.setPhase("bound");
  await sleep(20);
  assert.equal(watches(), w + 1, "once");
  assert.deepEqual(h.view.records.map((r) => r.id), [1, 2]);

  // Unmounted: the watch ends, nothing more is followed.
  h.stop();
  await sleep(0);
  assert.deepEqual(s.calls.at(-1), { method: "hubEventsWatch", params: { on: false, serial: OPEN } });
  assert.equal(s.hubWatchers.size, 0);
});

test("P2WB3: the hub's history says the core's refusal; a page of more read on at once", async () => {
  const all = Array.from({ length: 5 }, (_, i) => rec(i + 1));
  const s = fakeSession({
    hubEventsWatch: null,
    hubEventsRead: (p: unknown) => {
      const after = (p as { afterId: number }).afterId;
      const rest = all.filter((r) => r.id > after);
      return { records: rest.slice(0, 2), more: rest.length > 2, newestId: 5 };
    },
  });
  const h = new HubHistoryFeed(s as unknown as Session);
  feeds.push(h);
  h.start();
  await h.sync();
  assert.deepEqual(h.view.records.map((r) => r.id), [1, 2, 3, 4, 5]);
  h.stop();
  const off = fakeSession({ hubEventsWatch: new CoreCallError("unsupported", "the hub has no history") });
  const g = new HubHistoryFeed(off as unknown as Session);
  feeds.push(g);
  g.start();
  await g.sync();
  assert.equal(g.view.error, "the hub has no history");
});

const guardRec = (phase: GuardedApplyRecord["phase"]): GuardedApplyRecord => ({
  id: 3,
  phase,
  label: "Evening",
  hub: true,
  timeoutMs: 30_000,
  remainingMs: phase === "waiting" ? 30_000 : 0,
  reason: phase === "reverted" ? "reverted" : "",
  message: phase === "reverted" ? "The settings from before are back." : "",
  changes: ["Volume"],
  notes: [],
});
const RECALLED = { serial: OPEN, snapshot: "7", name: "Evening", image: false, lines: ["Volume"], notes: [], guard: "11" };

function deskOn(answers: Record<string, Answer>) {
  const s = fakeSession({ abState: { active: false, side: null, aName: "", bName: "", recall: null }, ...answers });
  const desk = new SnapshotDesk(s as unknown as Session, 20);
  feeds.push(desk);
  return { s, desk };
}

test("P2WB3: a guarded recall on the hub: the core adopted the hub's guard; Revert is the way back", async () => {
  let phase: GuardedApplyRecord["phase"] = "idle";
  const { s, desk } = deskOn({
    applyGuardDefault: true,
    applyStatus: () => guardRec(phase),
    applyPoll: () => guardRec(phase),
    applyRevert: () => {
      phase = "reverted";
      return guardRec("reverted");
    },
  });
  const hubRecall = async (guard: boolean, timeoutMs: number) => {
    await s.core.call("hubSnapshotsRecall", { id: "7", guard, timeoutMs });
    phase = "waiting";
    return RECALLED;
  };
  s.answers.hubSnapshotsRecall = RECALLED;
  assert.equal(await desk.recallOnHub(hubRecall, async () => "{}"), true);
  assert.deepEqual(s.calls.find((c) => c.method === "hubSnapshotsRecall")!.params, { id: "7", guard: true, timeoutMs: 30_000 });
  assert.equal(desk.state.guard?.phase, "waiting");
  assert.equal(desk.state.guard?.hub, true);
  assert.ok(!s.calls.some((c) => c.method === "applyGuardedSnapshot"), "nothing recalled here");
  assert.equal(await desk.revert(), true);
  assert.equal(desk.state.guard?.phase, "reverted");
  assert.equal(desk.state.outcome?.tone, "warn");
});

test("P2WB3: the way back where the hub cannot guard it: the hub's document recalled here, guarded here", async () => {
  const { s, desk } = deskOn({
    applyGuardDefault: true,
    hubSnapshotsRecall: new CoreCallError("unsupported", "not on this hub"),
    applyGuardedSnapshot: guardRec("waiting"),
  });
  const hubRecall = async (guard: boolean, timeoutMs: number) => s.core.call("hubSnapshotsRecall", { id: "7", guard, timeoutMs }) as Promise<typeof RECALLED>;
  assert.equal(await desk.recallOnHub(hubRecall, async () => "{\"doc\":1}"), true);
  assert.deepEqual(s.calls.find((c) => c.method === "applyGuardedSnapshot")!.params, { text: "{\"doc\":1}", timeoutS: 30 });
  assert.equal(desk.state.guard?.phase, "waiting");
  // Another refusal is said, not worked around.
  const other = deskOn({ applyGuardDefault: true, hubSnapshotsRecall: new CoreCallError("conflict", "another console's guarded apply waits") });
  const r = await other.desk.recallOnHub(async () => other.s.core.call("hubSnapshotsRecall", {}) as Promise<typeof RECALLED>, async () => "{}");
  assert.equal(r, false);
  assert.equal(other.desk.state.outcome?.tone, "error");
  assert.ok(!other.s.calls.some((c) => c.method === "applyGuardedSnapshot"));
});

test("P2WB3: an unguarded recall on the hub: the hub's result, no guard", async () => {
  const { s, desk } = deskOn({ applyGuardDefault: false, hubSnapshotsRecall: RECALLED, applyStatus: guardRec("idle") });
  let asked: { guard: boolean; timeoutMs: number } | null = null;
  assert.equal(
    await desk.recallOnHub(async (guard, timeoutMs) => {
      asked = { guard, timeoutMs };
      return RECALLED;
    }, async () => "{}"),
    true,
  );
  assert.equal(asked!.guard, false);
  assert.equal(desk.state.guard, null);
  assert.equal(desk.state.outcome?.title, "Recalled “Evening” on the hub");
  assert.ok(!s.calls.some((c) => c.method === "applyStatus"), "nothing to adopt");
});
