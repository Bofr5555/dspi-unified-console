// WP SW3 unit tests: the System view's logic (rooms and zones, the topology, each
// unit's state, the slot matrix's edits, roles, the remembered mode, the words of
// a scene's per-unit results), the feed against a hub's answers (status polled,
// the model read again on a new revision, a refused scene named), and on the real
// core a device on its own as a system of one unit, with and without the audio link.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CoreCallError } from "../../src/core/protocol";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { screensFor } from "../../src/ui/screens";
import { SystemFeed } from "../../src/screens/system/feed";
import {
  DEFAULT_MODE,
  framesMs,
  loadMode,
  mayWrite,
  opWord,
  parseWhole,
  railAlarms,
  resultTone,
  resultWord,
  roomsView,
  saveMode,
  sceneTaken,
  writeWhat,
  type SceneReport,
  senderChannels,
  slotWord,
  systemMuted,
  topology,
  unitSlots,
  unitView,
  NO_ALINK,
  withMeasuredLatency,
  withSlot,
  type Alarm,
  type SystemModel,
  type SystemStatus,
} from "../../src/screens/system/model";

/** The installation of `uconsole --demo hub --alink`, trimmed. */
function model(): SystemModel {
  return {
    rooms: [
      { id: "main", name: "Main room" },
      { id: "terrace", name: "Terrace" },
    ],
    zones: [{ id: "inside", name: "Inside", rooms: ["main"] }],
    groups: [],
    sources: [{ id: "music", name: "Music", sender: "tx", channels: [0, 1, 2, 3, 4, 5, 6, 7] }],
    senders: [{ id: "tx", name: "Sender", serial: "TX" }],
    fanOuts: [{ id: "fan", name: "Fan-out", serial: "FAN", upstream: "tx", output: 1 }],
    units: [
      { id: "A", name: "Main left", serial: "A", room: "main", upstream: "tx", output: 0, slots: [0, 1, 255, 255, 255, 255, 255, 255] },
      { id: "B", name: "Main right", serial: "B", room: "main", upstream: "fan" },
      { id: "C", name: "Terrace", serial: "C", room: "terrace", upstream: "fan", repeater: true },
      { id: "L", name: "Loose", serial: "L", upstream: "C" },
    ],
    scenes: [{ id: "evening", name: "Evening", actions: [{ target: "all", op: "master_volume", value: -1200 }] }],
  };
}

function alarm(id: number, subject: string, state: Alarm["state"], severity = "critical", raisedAt = id): Alarm {
  return { id, code: "alink_lost", subject, severity, message: `${subject} lost`, state, raisedAt, clearedAt: state === "cleared" ? raisedAt + 1 : null, count: 1 };
}

function status(over: Partial<SystemStatus> = {}): SystemStatus {
  return {
    revision: 1,
    observed: {
      TX: { online: true, role: "sender", systemMute: false },
      FAN: { online: true, role: "fan_out", state: "locked", hopDelayFrames: 3 },
      A: { online: true, role: "receiver", state: "locked" },
      B: { online: true, role: "receiver", state: "holdover" },
      C: { online: false },
    },
    alignment: { dFrames: 176, dAuto: true, rateHz: 48000, latencyMaxFrames: 0, units: [], issues: [] },
    alarms: [alarm(1, "B", "raised"), alarm(2, "C", "cleared")],
    ...over,
  };
}

test("SW3: rooms with their zones and units, each unit's state; a unit without a room is not lost", () => {
  const rooms = roomsView(model(), status());
  assert.deepEqual(rooms.map((r) => r.name), ["Main room", "Terrace", "No room"]);
  assert.deepEqual(rooms[0]!.zones, ["Inside"]);
  const [a, b] = rooms[0]!.units;
  assert.deepEqual([a!.connected, a!.lock, a!.alarms.length], [true, "Locked", 0]);
  assert.deepEqual([b!.connected, b!.lock, b!.alarms.length], [true, "Holdover", 1]);
  const c = rooms[1]!.units[0]!;
  assert.deepEqual([c.connected, c.lock, c.alarms.length], [false, "Unknown", 0], "a cleared alarm is not on");
  assert.equal(rooms[2]!.units[0]!.name, "Loose");
});

test("SW3: the topology hangs fan-outs and units under their upstream, a repeater's units under it", () => {
  const t = topology(model(), status());
  assert.equal(t.length, 1);
  const tx = t[0]!;
  assert.deepEqual([tx.kind, tx.lock, tx.connected], ["sender", "Clock", true]);
  assert.deepEqual(tx.children.map((n) => n.id), ["fan", "A"]);
  const fan = tx.children[0]!;
  assert.deepEqual([fan.hopDelayFrames, fan.output], [3, 1]);
  assert.deepEqual(fan.children.map((n) => n.id), ["B", "C"]);
  const c = fan.children[1]!;
  assert.deepEqual(c.children.map((n) => n.id), ["L"], "behind the repeater");
  assert.equal(tx.children[1]!.slots![0], 0);
  // An upstream the model does not have: the node is listed at the top, not dropped.
  const m = model();
  m.units[3]!.upstream = "nowhere";
  assert.deepEqual(topology(m, null).map((n) => n.id), ["tx", "L"]);
});

test("SW3: the slot matrix: rows are the audio link channels in use, an edit sets one input, a second press silences it", () => {
  const m = model();
  assert.deepEqual(senderChannels(m), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.deepEqual(unitSlots(m.units[1]!), [255, 255, 255, 255, 255, 255, 255, 255]);
  const edited = withSlot(m, "B", 2, 6);
  assert.deepEqual(unitSlots(edited.units.find((u) => u.id === "B")!).slice(0, 4), [255, 255, 6, 255]);
  assert.deepEqual(unitSlots(m.units[1]!)[2], 255, "the model itself is not changed");
  const wide = withSlot(m, "A", 7, 40);
  assert.equal(senderChannels(wide).length, 41);
  assert.equal(slotWord(255), "Silent");
  assert.equal(slotWord(6), "Ch 6");
  assert.equal(withMeasuredLatency(m, "C", 250).units[2]!.measuredLatencyUs, 250);
});

test("SW3: roles: the model and the alignment are admin-only, scenes, the mute and acknowledging need control", () => {
  for (const w of ["model", "align"] as const) {
    assert.equal(mayWrite("admin", w), true);
    assert.equal(mayWrite("control", w), false);
  }
  for (const w of ["scene", "mute", "ack"] as const) {
    assert.equal(mayWrite("control", w), true);
    assert.equal(mayWrite("viewer", w), false);
  }
  assert.equal(mayWrite("", "scene"), false);
});

test("SW3: the mode is remembered per device, or for every device, Advanced by default; Simple has no System view", () => {
  const mem = new Map<string, string>();
  const store = { getItem: (k: string) => mem.get(k) ?? null, setItem: (k: string, v: string) => void mem.set(k, v), removeItem: (k: string) => void mem.delete(k) };
  assert.equal(loadMode(store, "S1"), DEFAULT_MODE);
  assert.equal(DEFAULT_MODE, "advanced");
  saveMode(store, "S1", "installer", false);
  assert.equal(loadMode(store, "S1"), "installer");
  assert.equal(loadMode(store, "S2"), "advanced");
  saveMode(store, "S2", "simple", true);
  assert.equal(loadMode(store, "S1"), "simple", "every device");
  mem.set("dspi-web.mode", "{nonsense");
  assert.equal(loadMode(store, "S1"), "advanced");
  assert.equal(loadMode(null, "S1"), "advanced");
  assert.deepEqual(screensFor("simple", true).map((s) => s.id), ["main", "eq", "inputs", "matrix", "snapshots"]); // -- WP P1DCFIX -- recall and A/B only (-- WP WB6G -- Inputs after EQ)
  // -- WP P1WB1 -- Snapshots and Files in Advanced and Installer
  // -- WP WB6J -- the tool screens (generator, analyser, Stats for nerbs, Interrupt Monitor) before the event log
  assert.deepEqual(screensFor("advanced", true).map((s) => s.id), ["system", "main", "eq", "inputs", "matrix", "snapshots", "files", "generator", "analyser", "stats", "monitor", "events"]);
  // -- WP P1WB2 -- the Installer page last, in the Installer mode
  assert.deepEqual(screensFor("installer", false).map((s) => s.id), ["main", "eq", "inputs", "matrix", "snapshots", "files", "system", "generator", "analyser", "stats", "monitor", "events", "installer"]);
});

test("SW3: words: results, ops, frames; the rail puts alarms on first, critical first", () => {
  assert.equal(resultWord("queued"), "Queued");
  assert.equal(resultWord("refused_at_apply"), "Refused");
  assert.equal(resultWord("late"), "Late");
  assert.equal(resultWord("clamped (ok)"), "Clamped");
  assert.equal(resultTone("late"), "warn");
  assert.equal(resultTone("refused"), "bad");
  assert.equal(resultTone("applied"), "ok");
  assert.equal(opWord("master_volume", 0, -1200), "Master volume -12.0 dB");
  assert.equal(opWord("preset_load", 2), "Preset 3");
  assert.equal(opWord("click", 0), "Test click OUT1");
  assert.equal(framesMs(176, 48000), "176 (3.67 ms)");
  assert.equal(parseWhole("250", 0, 1000), 250);
  assert.equal(parseWhole(" ", 0, 1000), null);
  assert.equal(parseWhole("2.5", 0, 1000), undefined);
  assert.equal(parseWhole("-1", 0, 1000), undefined);
  const r = railAlarms([alarm(1, "A", "cleared"), alarm(2, "B", "raised", "warning", 5), alarm(3, "C", "acknowledged", "critical", 4)]);
  assert.deepEqual(r.map((a) => a.id), [3, 2, 1]);
  assert.equal(systemMuted(model(), status()), false);
  assert.equal(systemMuted(model(), null), null);
});

// -- The feed against a hub's answers --

function hubSession(answer: (t: string, p: Record<string, unknown>) => unknown, outcomes: Record<string, unknown>[] = []) {
  const calls: string[] = [];
  const core = {
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === "hubRole") return { role: "control" };
      // -- WP WB8 -- the core's words for a refused recall: what the feed asked, answered in kind
      if (method === "sceneOutcome") {
        outcomes.push(params);
        return { message: `core words for ${String(params.scene)} held ${String(params.heldFrame)}`, taken: false };
      }
      if (method === "hubSystem") {
        calls.push(String(params.t));
        return answer(String(params.t), params);
      }
      throw new CoreCallError("unknown_method", method);
    },
    subscribe: () => () => undefined,
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "hub", url: "ws://h" }, "control", core);
  session.state = { ...session.state, status: "open" };
  return { session, calls };
}

test("SW3: the feed reads the model and the status, and the model again on a new revision", async () => {
  let revision = 1;
  const { session, calls } = hubSession((t) => {
    if (t === "system.get") return { model: model(), revision };
    if (t === "system.status") return status({ revision });
    throw new Error(t);
  });
  const feed = new SystemFeed(session, 60_000);
  await feed.refresh();
  assert.equal(feed.state.kind, "hub");
  assert.equal(feed.state.role, "control");
  assert.equal(feed.state.status?.alignment.dFrames, 176);
  assert.deepEqual(calls, ["system.get", "system.status"]);
  revision = 2;
  await feed.refresh();
  assert.deepEqual(calls.slice(2), ["system.status", "system.get"]);
  feed.stop();
});

test("SW3: a refused scene keeps the hub's words, which name the op; a scene's report is kept", async () => {
  const { session } = hubSession((t, p) => {
    if (t === "system.get") return { model: model(), revision: 1 };
    if (t === "system.status") return status();
    if (t === "system.scene.apply" && p.scene === "click") throw new CoreCallError("denied", "op click on unit:A needs a role that may send it directly; this one may not");
    if (t === "system.scene.apply")
      return { plan: { scene: "evening", frame: 9, path: "per_unit", ops: [], leadFrames: 4800 }, writes: [{ serial: "A", element: "A", what: "master_volume", result: "queued" }], onTime: true };
    throw new Error(t);
  });
  const feed = new SystemFeed(session, 60_000);
  await feed.refresh();
  await feed.recall("click");
  assert.equal(feed.state.scene?.error, "Op click on unit:A needs a role that may send it directly; this one may not");
  await feed.recall("evening");
  assert.equal(feed.state.scene?.name, "Evening");
  assert.equal(feed.state.scene?.report?.writes[0]?.result, "queued");
  assert.equal(feed.state.scene?.error, "");
  feed.stop();
});

test("SW3: a hub without an installation shows the open device as one unit; rate_limited is no error", async () => {
  const { session } = hubSession((t) => {
    if (t === "system.get") return { model: { ...model(), units: [], senders: [], fanOuts: [] }, revision: 0 };
    throw new CoreCallError("rate_limited", "system.status at most once a second per session");
  });
  session.state = { ...session.state, snapshot: { device: { name: "DSPi Demo", serial: "X1" } } as never };
  const feed = new SystemFeed(session, 60_000);
  await feed.refresh();
  assert.equal(feed.state.kind, "single");
  assert.equal(feed.state.model?.units[0]?.name, "DSPi Demo");
  assert.equal(feed.state.error, "");
  feed.stop();
});

// -- On the real core: one device on its own --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demoSession(alink: boolean) {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const client = {
    call: (m: never, p: never) => core.call(m, p),
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo", alink });
  return session;
}

test("SW3: on the real core, the demo is one unit; with the audio link, a locked receiver with its slots", { skip }, async () => {
  const plain = new SystemFeed(await demoSession(false), 60_000);
  await plain.refresh();
  assert.equal(plain.state.kind, "single");
  const u = plain.state.model!.units[0]!;
  const o = plain.state.status!.observed[u.serial]!;
  assert.deepEqual([o.online, o.role], [true, null]);
  plain.stop();

  const linked = new SystemFeed(await demoSession(true), 60_000);
  await linked.refresh();
  const lu = linked.state.model!.units[0]!;
  const lo = linked.state.status!.observed[lu.serial]!;
  assert.deepEqual([lo.role, lo.state], ["receiver", "locked"]);
  assert.equal(lu.slots?.length, 8);
  // The hub's calls are not this session's: there is no hub.
  await assert.rejects(() => linked["session"].core.call("hubSystem", { t: "system.get" }), (e: CoreCallError) => e.code === "not_open");
  linked.stop();
});

// -- fixup sw3x --

function report(path: string, results: string[], frame = 2_000_000): SceneReport {
  return {
    plan: { scene: "closed", frame, path, ops: [], leadFrames: 4800, systemMute: true },
    writes: results.map((result, i) => ({ serial: i ? `U${i}` : "TX", element: i ? `U${i}` : "tx", what: "mute", result })),
    frameAfterWrites: frame - 4790,
    onTime: false,
  };
}

test("SW3 fixup sw3x: a recall none of whose writes was taken is refused", () => {
  assert.equal(sceneTaken(report("superframe", ["queued"])), true);
  assert.equal(sceneTaken(report("per_unit", ["refused_at_apply", "late"])), true, "one late unit: recalled, late");
  assert.equal(sceneTaken(report("superframe", ["refused"])), false);
  // -- WP WB8 -- why is the core's sentence (`sceneOutcome`): wasm.test.ts and the e2e check its words
  assert.equal(writeWhat("phase"), "Audio link delay");
  assert.equal(writeWhat("slots"), "Audio link slots");
  assert.equal(writeWhat("out_delay:1"), "Delay OUT2");
});

test("SW3 fixup sw3x: a refused superframe scene is not the recalled one; the sender's held frame is named", async () => {
  let held: number | null = null;
  const outcomes: Record<string, unknown>[] = [];
  const { session } = hubSession((t, p) => {
    if (t === "system.get") return { model: model(), revision: 1 };
    if (t === "system.status") {
      const s = status();
      s.observed.TX = { ...s.observed.TX!, pendingApplyFrame: held };
      return s;
    }
    if (t === "system.scene.apply" && p.scene === "evening") {
      held = 1_000_000;
      return { ...report("superframe", ["queued"], 1_000_000), plan: { ...report("superframe", ["queued"], 1_000_000).plan, scene: "evening" } };
    }
    if (t === "system.scene.apply") return report("superframe", ["refused"], 1_004_000);
    throw new Error(t);
  }, outcomes);
  const feed = new SystemFeed(session, 60_000);
  await feed.refresh();
  await feed.recall("evening");
  assert.deepEqual(feed.state.recalled, { scene: "evening", name: "Evening" });
  await feed.recall("closed");
  assert.deepEqual(feed.state.recalled, { scene: "evening", name: "Evening" }, "the refused one is not recalled");
  assert.equal(feed.state.scene?.scene, "closed");
  // -- WP WB8 -- the core's sentence, asked with the frame the sender holds, the report and the model
  // -- WP HWLK1F -- the taken recall asks too, for its outcome (no held frame); then the refused one
  assert.equal(outcomes.length, 2, "both recalls ask");
  assert.equal(outcomes[0]!.scene, "evening");
  assert.equal(outcomes[0]!.heldFrame, undefined);
  assert.equal(outcomes[1]!.heldFrame, 1_000_000);
  assert.equal(outcomes[1]!.scene, "closed");
  assert.equal((outcomes[1]!.report as { plan: { frame: number } }).plan.frame, 1_004_000);
  assert.ok((outcomes[1]!.model as { scenes: unknown[] }).scenes.length > 0);
  assert.equal(feed.state.scene!.refusal, "core words for closed held 1000000");
  assert.equal(feed.state.scene!.outcome, "", "a refused recall has no outcome sentence");
  feed.stop();
});

test("SW3 fixup sw3x: the role follows the hub's answers; a stale Save is a conflict and the model is read again", async () => {
  let role = "admin";
  let revision = 1;
  const { session, calls } = hubSession((t, p) => {
    if (t === "system.get") return { model: model(), revision, role };
    if (t === "system.status") return { ...status({ revision }), role };
    if (t === "system.set") {
      if (p.revision !== revision) throw new CoreCallError("conflict", "the installation changed on the hub (revision 2); read it again");
      revision++;
      return { revision };
    }
    throw new Error(t);
  });
  const feed = new SystemFeed(session, 60_000);
  await feed.refresh();
  assert.equal(feed.state.role, "admin", "system.get's role over hubRole's");
  role = "viewer";
  await feed.refresh();
  assert.equal(feed.state.role, "viewer", "a live change, at the next status");
  role = "admin";
  revision = 2; // someone else saved
  assert.equal(await feed.setModel(model(), 1), "conflict");
  assert.equal(feed.state.revision, 2, "read again");
  assert.equal(await feed.setModel(model(), 2), "ok");
  assert.equal(feed.state.revision, 3);
  assert.ok(calls.includes("system.set"));
  feed.stop();
});

// -- WP HWLK1F --
test("HWLK1F: a unit whose firmware has no audio link reads as such, never Unknown", () => {
  const m = model();
  const st = status();
  const serial = m.units[0]!.serial;
  st.observed[serial] = { online: true, alink: "none" };
  assert.equal(unitView(m.units[0]!, st).lock, NO_ALINK);
  st.observed[serial] = { online: true };
  assert.equal(unitView(m.units[0]!, st).lock, "Unknown", "not told: as before");
  st.observed[serial] = { online: false, alink: "none" };
  assert.equal(unitView(m.units[0]!, st).lock, "Unknown", "offline");
  assert.equal(resultWord("applied_now"), "Applied now");
  assert.equal(resultTone("applied_now"), "ok");
  assert.equal(resultWord("unsupported"), "Needs the audio link");
  assert.equal(resultWord("no_frame"), "No audio link frame");
  assert.ok(sceneTaken({ plan: { scene: "s", frame: 0, path: "direct", ops: [], leadFrames: 0 }, writes: [{ serial: "K", element: "K", what: "mute", result: "applied_now" }], onTime: false }));
});

test("HWLK1F: a written alignment is the core's sentence; a save keeps the hub's warnings", async () => {
  const asked: Record<string, unknown>[] = [];
  const core = {
    call: async (method: string, params: Record<string, unknown>) => {
      if (method === "hubRole") return { role: "admin" };
      if (method === "alignOutcome") {
        asked.push(params);
        return { applied: true, message: "Nothing was written: skipped on K340 (no audio link (firmware))." };
      }
      if (method === "hubSystem") {
        const t = String(params.t);
        if (t === "system.get") return { model: model(), revision: 1 };
        if (t === "system.status") return status();
        if (t === "system.align") return { applied: params.apply, alignment: status().alignment, writes: [] };
        if (t === "system.set") return { revision: 2, warnings: ["K340 (K): its firmware has no audio link"] };
      }
      throw new CoreCallError("unknown_method", method);
    },
    subscribe: () => () => undefined,
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "hub", url: "ws://h" }, "control", core);
  session.state = { ...session.state, status: "open" };
  const feed = new SystemFeed(session, 60_000);
  await feed.refresh();
  await feed.align(false);
  assert.equal(asked.length, 0, "a computation is not a write");
  await feed.align(true);
  assert.equal(asked.length, 1);
  assert.equal(feed.state.align?.message, "Nothing was written: skipped on K340 (no audio link (firmware)).");
  assert.equal(await feed.setModel(model(), 1), "ok");
  assert.deepEqual(feed.state.warnings, ["K340 (K): its firmware has no audio link"]);
  feed.stop();
});
