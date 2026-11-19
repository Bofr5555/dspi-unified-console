// WP ATWEB unit tests: the "Several units" step's rules (the placement chosen before anything
// plays, the form, a refusal in words with its notice), the Worker's part (the capture port into
// `measureSystemCapturePush`, the turns after a start, a sum check or a verify), and the desk
// against stand-in cores: each unit judged by its own core right before every call that plays,
// the result and the proposal, Apply refused for Control and done for Admin, the summed check and
// the verify on fresh checks, Keep after a failed verify asked twice (the person's own second
// press, 1 s later, never one of its own), Revert Admin only, the tab left stopping what it
// started. The real hub's flow is the e2e's (test/e2e/measure_atweb.spec.ts). No audio anywhere.

import { test } from "node:test";
import assert from "node:assert/strict";
import { SYSTEM_TICK_MS, SystemWorkerRoute, isMeasureSystemEvent, type MeasureSystemEvent } from "../../src/core/measure-system";
import { CoreCallError, type MeasureSystemAppliedRecord, type MeasureSystemProposalRecord, type MeasureSystemResultRecord, type MeasureSystemStateRecord, type NoticeRecord } from "../../src/core/protocol";
import { MEASURE_TABS } from "../../src/screens/measure/MeasureScreen";
import { SystemDesk, type SystemSession, type UnitCore } from "../../src/screens/measure/system/desk";
import { KEYS, draftProblem, initialDraft, issueWords, placementProblem, proposalRights, refusalWords, runsWords, setupOf, withOutput, withUnit } from "../../src/screens/measure/system/model";
import { NO_TIME_BASE, noTimeBase, withSpdifFollower, withSpdifSource, withoutSpdif } from "../../src/screens/system/SpdifUnits";
import type { SystemModel } from "../../src/screens/system/model";

const ROWS: Record<string, NoticeRecord> = {
  [KEYS.notOneDomain]: { key: KEYS.notOneDomain, kind: "warning", title: "These units do not share a clock", reason: "r", action: "a", detail: "" },
  [KEYS.runs]: { key: KEYS.runs, kind: "info", title: "The group needs several runs", reason: "r", action: "Keep the microphone still.", detail: "" },
  [KEYS.lostLock]: { key: KEYS.lostLock, kind: "warning", title: "Unit lost lock", reason: "r", action: "a", detail: "" },
};
const row = (k: string) => ROWS[k] ?? null;

// ------------------------------------------------------------------ the rules

test("ATWEB: nothing plays before the person chose the listening position; headphones never across units", () => {
  assert.match(placementProblem(""), /Choose where the microphone is/);
  assert.match(placementProblem("headphones"), /Headphones are never measured across units/);
  assert.match(placementProblem("nearField"), /listening position only/);
  assert.equal(placementProblem("listeningPosition"), "");
  let d = initialDraft();
  assert.equal(d.placement, "", "no placement is assumed");
  assert.match(draftProblem(d), /Choose where the microphone is/);
  d = { ...d, placement: "listeningPosition" };
  assert.match(draftProblem(d), /at least two units/);
  d = withUnit(withUnit(d, "A", true), "B", true);
  assert.equal(draftProblem(d), "");
  d = withOutput(d, "B", 0, false);
  assert.match(draftProblem(d), /at least one output of B/);
  d = withOutput(withOutput(d, "B", 1, true), "A", 2, true);
  const s = setupOf({ ...d, anchor: "A:2", sweepS: 1.5 }, 44_100, "mic text");
  assert.deepEqual(s.units, [
    { serial: "A", outputs: [{ output: 0, role: "full" }, { output: 2, role: "full" }] },
    { serial: "B", outputs: [{ output: 1, role: "full" }] },
  ]);
  assert.equal(s.anchorSerial, "A");
  assert.equal(s.anchorOutput, 2);
  assert.equal(s.sweepMs, 1500);
  assert.equal(s.captureRateHz, 44_100);
  assert.equal(s.micDocument, "mic text");
});

test("ATWEB: the hub's refusals and the session's issues in words, with their notices", () => {
  const e = new CoreCallError("link", "DSPi Link: the units' domain ids differ", "dlink.not_one_domain");
  assert.deepEqual(refusalWords(e, row), { key: KEYS.notOneDomain, text: "These units do not share a clock: The units' domain ids differ" });
  // `open` refusing with `device`, the summed check with the polarity: plain errors with words.
  const dev = new CoreCallError("link", "DSPi Link: the unit's reads took too long to tell its rate (a busy hub); try again", "dlink.device");
  assert.equal(refusalWords(dev, () => null).text, "The unit's reads took too long to tell its rate (a busy hub); try again");
  const pol = new CoreCallError("invalid", "invalid argument: B output 1 is inverted against A output 1: check the polarity first", "invalid");
  assert.equal(refusalWords(pol, () => null).text, "B output 1 is inverted against A output 1: check the polarity first");
  // A microphone the browser would not start is the microphone's, never "needs the hub".
  assert.equal(refusalWords(new CoreCallError("unsupported", "The browser did not start its audio processing (suspended)", "mic_unsupported"), () => null).key, "mic_unsupported");
  assert.equal(issueWords({ key: KEYS.lostLock, message: "DEMO-TB-C: its time base re-anchored" }, row), "Unit lost lock: DEMO-TB-C: its time base re-anchored");
  const st = { runs: 3, run: 1 } as MeasureSystemStateRecord;
  assert.equal(runsWords(st, row), "The group needs several runs: this group needs 3 runs, chained through the anchor output. Keep the microphone still.");
  assert.equal(runsWords({ ...st, runs: 1 }, row), "");
});

test("ATWEB: the tab sits in P5WMS's slot and is never offered to a connection that may not play", () => {
  const t = MEASURE_TABS.find((x) => x.id === "system");
  assert.ok(t, "the Several units tab");
  assert.equal(t!.label, "Several units");
  assert.notEqual(t!.passive, true);
});

test("ATWEB: an S/PDIF follower that hears no DSPi time base says so; the edits keep what S/PDIF cannot carry off the unit", () => {
  const seen = { mode: "follower", d: 192, onLockError: "mute", txMask: 0, state: "lock_error", superframe: false, holdover: false, lockError: true, foreign: false, sourceId: 0, dMin: 129, dMax: 399, lockCount: 0, lossCount: 0 };
  assert.equal(noTimeBase(seen), true);
  assert.equal(noTimeBase({ ...seen, lockError: false, state: "locked", superframe: true }), false);
  assert.equal(noTimeBase({ ...seen, lockError: false, superframe: true, foreign: true }), true);
  assert.equal(NO_TIME_BASE, "spdif_no_time_base");
  const model = {
    rooms: [], zones: [], groups: [], sources: [], senders: [{ id: "tx", name: "Sender" }], fanOuts: [], scenes: [],
    units: [
      { id: "A", name: "A", serial: "SA", upstream: "tx", slots: [0, 1, 255, 255, 255, 255, 255, 255], hopDelayFrames: 2 },
      { id: "B", name: "B", serial: "SB", upstream: "tx" },
    ],
  } as unknown as SystemModel;
  const src = withSpdifSource(model, "B");
  assert.equal(src.units[1]!.spdifSource, true);
  assert.equal(src.units[1]!.spdifTxMask, 8);
  const f = withSpdifFollower(src, "A", "B");
  assert.equal(f.units[0]!.transport, "spdif");
  assert.equal(f.units[0]!.upstream, "B");
  assert.equal(f.units[0]!.slots, undefined);
  assert.equal(f.units[0]!.hopDelayFrames, undefined);
  const back = withoutSpdif(f, "A", "tx");
  assert.equal(back.units[0]!.transport, undefined);
  assert.equal(back.units[0]!.upstream, "tx");
  assert.equal(withoutSpdif(back, "B", "tx").units[1]!.spdifSource, undefined);
});

// ------------------------------------------------------------------ the Worker's part

function routeRig(states: MeasureSystemStateRecord[]) {
  const calls: { method: string; params: unknown }[] = [];
  const posted: MeasureSystemEvent[] = [];
  let pending: (() => void) | null = null;
  const route = new SystemWorkerRoute(
    async (method, params) => {
      calls.push({ method, params });
      if (method === "measureSystemTick") return states.shift() ?? { ...STATE, active: false, phase: "done" };
      return null;
    },
    (e) => posted.push(e),
    {
      MessageChannel,
      workletUrl: "http://x/capture-worklet.js",
      setTimeout: ((fn: () => void, ms: number) => {
        assert.equal(ms, SYSTEM_TICK_MS);
        pending = fn;
        return 1 as unknown as ReturnType<typeof setTimeout>;
      }) as unknown as typeof setTimeout,
      clearTimeout: (() => {
        pending = null;
      }) as typeof clearTimeout,
    },
  );
  const fire = async () => {
    const f = pending;
    pending = null;
    f?.();
    await new Promise((r) => setTimeout(r, 5));
  };
  return { route, calls, posted, fire, get due() {
    return pending !== null;
  } };
}

const STATE: MeasureSystemStateRecord = {
  phase: "pre_roll", purpose: "measure", active: true, wantsCapture: true, captureRateHz: 48000, serial: "", output: null, levelDb: null,
  rampLevels: [], run: 0, runs: 1, frames: 0, progress: 0, issue: null, resultReady: false, timingReliable: null, words: "",
};

test("ATWEB: the Worker's capture port pushes each block, in order, into measureSystemCapturePush; a close ends it", async () => {
  const rig = routeRig([]);
  assert.equal(rig.route.handles("measureSystemCaptureOpen"), true);
  assert.equal(rig.route.handles("measureCaptureOpen"), false, "P5WMS's own stays P5WMS's");
  const a = rig.route.answer("measureSystemCaptureOpen");
  assert.ok(a.value && a.value.port instanceof MessagePort);
  assert.deepEqual(a.transfer, [a.value!.port]);
  const port = a.value!.port;
  port.postMessage({ kind: "block", firstFrame: 0, samples: new Float32Array([0.5, -0.25]), rateHz: 48000, clipped: false });
  port.postMessage({ kind: "block", firstFrame: 2, samples: new Float32Array([0.125]), rateHz: 48000, clipped: true });
  for (let i = 0; i < 50 && rig.calls.length < 2; i++) await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(rig.calls, [
    { method: "measureSystemCapturePush", params: { firstFrame: 0, samples: [0.5, -0.25], rateHz: 48000, clipped: false } },
    { method: "measureSystemCapturePush", params: { firstFrame: 2, samples: [0.125], rateHz: 48000, clipped: true } },
  ]);
  rig.route.answer("measureSystemCaptureClose");
  port.postMessage({ kind: "block", firstFrame: 3, samples: new Float32Array([0]), rateHz: 48000, clipped: false });
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(rig.calls.length, 2);
  port.close();
});

test("ATWEB: after a start, a summed check or a verify the Worker turns the session until it ends; Stop pushes its last state", async () => {
  const rig = routeRig([{ ...STATE }, { ...STATE, phase: "ramp", words: "waiting for the units' clocks to settle" }]);
  rig.route.after("measureSystemStart", false, null);
  assert.equal(rig.route.ticking, false, "a refused start: no turns");
  rig.route.after("measureSystemStart", true, { ...STATE });
  assert.equal(rig.route.ticking, true);
  await rig.fire();
  await rig.fire();
  assert.equal(rig.posted.length, 2);
  assert.ok(isMeasureSystemEvent(rig.posted[1]));
  assert.equal(rig.posted[1]!.state.words, "waiting for the units' clocks to settle");
  await rig.fire();
  assert.equal(rig.posted[2]!.state.phase, "done");
  assert.equal(rig.route.ticking, false, "done: no more turns");
  for (const m of ["measureSystemSumCheck", "measureSystemVerify"]) {
    rig.route.after(m, true, { ...STATE, purpose: m === "measureSystemVerify" ? "verify" : "sum_check" });
    assert.equal(rig.route.ticking, true, m);
    rig.route.after("measureSystemStop", true, { ...STATE, active: false, phase: "aborted" });
    assert.equal(rig.route.ticking, false);
    assert.equal(rig.posted.at(-1)!.state.phase, "aborted");
  }
});

// ------------------------------------------------------------------ the desk

type Call = { core: string; method: string; params: unknown };

const RESULT: MeasureSystemResultRecord = {
  documents: [],
  arrivals: [
    { serial: "A", output: 0, arrivalUs: -120.5, measuredUs: 980, uncertaintyUs: 1.2, offered: true, reason: "" },
    { serial: "B", output: 0, arrivalUs: 0, measuredUs: 1100, uncertaintyUs: 1.1, offered: true, reason: "" },
  ],
  delays: [{ fromSerial: "A", fromOutput: 0, toSerial: "B", toOutput: 0, delayUs: 120.5, uncertaintyUs: 1.6, offered: true, reason: "" }],
  transitions: [],
  timingReliable: true,
  reasons: [],
  runIds: [1, 2],
  runs: 1,
};

function proposal(p: Partial<MeasureSystemProposalRecord>): MeasureSystemProposalRecord {
  return { id: 7, arrivals: RESULT.arrivals, withheld: [], transitions: [], sumCheckNeeded: false, withdrawn: false, offered: true, mayApply: true, notice: null, ...p };
}

const APPLIED: MeasureSystemAppliedRecord = { stage: "none", revisionBefore: 0, revisionAfter: 0, snapshots: [], writesOk: 0, writeFailures: [], verified: null, message: "" };

/** A hub session's core and the units' own cores, all stand-ins; every call is recorded in order. */
function deskRig(role: "admin" | "control", opts: { caps?: string[] } = {}) {
  const calls: Call[] = [];
  const listeners = new Set<(e: unknown) => void>();
  let applied: MeasureSystemAppliedRecord = { ...APPLIED };
  let keeps = 0;
  let prop = proposal({ mayApply: role === "admin", sumCheckNeeded: true });
  const unitCheck = (serial: string) => ({ serial, preflight: { clear: true, blockers: [], warnings: [] }, context: `{"serial":"${serial}"}` });
  const hub = {
    async call(method: string, params: unknown) {
      calls.push({ core: "A", method, params });
      switch (method) {
        case "hubCaps":
          return { onHub: true, caps: opts.caps ?? ["system_measure"] };
        case "hubRole":
          return { role };
        case "linkDevices":
          return [
            { handle: 1, serial: "A", name: "Unit A", firmware: "", platform: "", state: "online" },
            { handle: 2, serial: "B", name: "Unit B", firmware: "", platform: "", state: "online" },
            { handle: 3, serial: "Z", name: "Gone", firmware: "", platform: "", state: "offline" },
          ];
        case "measureSystemState":
          return { ...STATE, active: false, phase: "idle" };
        case "measureSystemApplied":
          return applied;
        case "measureSystemResult":
          return RESULT;
        case "measureSystemProposal":
          return prop;
        case "measureSystemUnitCheck":
          return unitCheck("A");
        case "measureSystemPreflight":
          return { clear: true, blockers: [], warnings: [] };
        case "measureSystemStart":
          return { ...STATE };
        case "measureSystemSumCheck":
          return { ...STATE, purpose: "sum_check" };
        case "measureSystemVerify":
          return { ...STATE, purpose: "verify" };
        case "measureSystemStop":
          return { ...STATE, active: false, phase: "aborted" };
        case "measureSystemApply":
          if (role !== "admin") throw new CoreCallError("denied", "this console's role on the hub does not allow that", "denied");
          applied = { ...APPLIED, stage: "applied", revisionBefore: 4, revisionAfter: 5 };
          return applied;
        case "measureSystemKeep":
          keeps++;
          if (applied.verified === false && keeps === 1) throw new CoreCallError("invalid", "invalid argument: the verify did not confirm the alignment", "invalid");
          applied = { ...applied, stage: "kept", message: "Kept." };
          return applied;
        case "measureSystemRevert":
          applied = { ...applied, stage: "reverted", message: "" };
          return applied;
        case "measureDocCurves":
          return { smoothing: 6, timingReliable: true, outputs: [] };
        default:
          return null;
      }
    },
    subscribe(fn: (e: unknown) => void) {
      listeners.add(fn as (e: unknown) => void);
      return () => listeners.delete(fn as (e: unknown) => void);
    },
  };
  const disposed: string[] = [];
  const spawn = (): UnitCore => {
    let serial = "";
    return {
      async call(method: string, params: unknown) {
        if (method === "open") serial = (params as { deviceId: string }).deviceId;
        calls.push({ core: serial || "new", method, params });
        if (method === "linkConnect") return { hubId: "h", hubName: "Hub", needsPairing: false, tokenRefused: false, caps: [] };
        if (method === "open") return { device: {}, state: { numOutputChannels: 2, numInputChannels: 2, outputEnabled: [true, true], channelNames: ["L", "R", "Out 1", "Out 2"] }, outputLabels: [] };
        if (method === "measureSystemUnitCheck") return unitCheck(serial);
        return null;
      },
      subscribe: () => () => undefined,
      async dispose() {
        disposed.push(serial);
      },
    } as unknown as UnitCore;
  };
  const session: SystemSession = {
    core: hub as unknown as SystemSession["core"],
    source: { kind: "hub", url: "ws://127.0.0.1:1/dspi/v1", deviceId: "A" },
    deviceId: "A",
    linkContext: { loc: null, proxy: false },
    state: { snapshot: { device: {}, state: { numOutputChannels: 2, numInputChannels: 2, outputEnabled: [true, true], channelNames: ["L", "R", "Out 1", "Out 2"] }, outputLabels: [] } as never },
  };
  let now = 10_000;
  class TestDesk extends SystemDesk {
    mic = 0;
    override async openMic(): Promise<number> {
      this.mic++;
      return 48_000;
    }
    override async closeMic(): Promise<void> {
      this.mic = Math.max(0, this.mic - 1);
    }
  }
  const desk = new TestDesk(session, { spawn, tokens: { get: async () => null, save() {}, forget() {} }, now: () => now });
  const push = async (state: MeasureSystemStateRecord) => {
    for (const l of listeners) l({ event: "measureSystem", state });
    await new Promise((r) => setTimeout(r, 5));
  };
  return {
    desk, calls, disposed, push,
    setNow: (t: number) => (now = t),
    get now() {
      return now;
    },
    failVerify() {
      applied = { ...applied, stage: "verified", verified: false, message: "The summed runs after applying did not agree with the prediction (2.3 dB RMS)." };
    },
    passSumCheck() {
      prop = proposal({ mayApply: role === "admin", sumCheckNeeded: false });
    },
  };
}

/** Wait until `f` holds (the desk opens unit cores in the background). */
async function until(f: () => boolean) {
  for (let i = 0; i < 100 && !f(); i++) await new Promise((r) => setTimeout(r, 5));
  assert.ok(f());
}

/** The calls of `method` and the ones right before each. */
function before(calls: Call[], method: string, n: number): Call[][] {
  return calls.flatMap((c, i) => (c.method === method ? [calls.slice(Math.max(0, i - n), i)] : []));
}

test("ATWEB: without the hub's system_measure the step says it needs the DSPi hub and offers nothing", async () => {
  const rig = deskRig("admin", { caps: [] });
  await rig.desk.enter();
  assert.match(rig.desk.unavailable, /needs the DSPi hub/);
  assert.equal(rig.desk.errorKey, KEYS.needsHub);
  assert.equal(await rig.desk.start(), false);
  assert.ok(!rig.calls.some((c) => c.method === "measureSystemStart" || c.method === "linkDevices"));
});

test("ATWEB: the placement first; then each unit's own core judges it right before the start", async () => {
  const rig = deskRig("control");
  await rig.desk.enter();
  assert.deepEqual(rig.desk.units.map((u) => [u.serial, u.online, u.own]), [["A", true, true], ["B", true, false], ["Z", false, false]]);
  rig.desk.setDraft(withUnit(withUnit(rig.desk.draft, "A", true), "B", true));
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.deepEqual(rig.desk.units[1]!.outputs.map((o) => o.name), ["OUT1 Out 1", "OUT2 Out 2"]);
  // No placement chosen: nothing is sent, nothing plays.
  assert.equal(await rig.desk.start(), false);
  assert.match(rig.desk.error, /Choose where the microphone is/);
  assert.equal(rig.desk.mic, 0);
  rig.desk.setDraft({ ...rig.desk.draft, placement: "headphones" });
  assert.equal(await rig.desk.start(), false);
  assert.ok(!rig.calls.some((c) => c.method.startsWith("measureSystem") && !["measureSystemState", "measureSystemApplied", "measureSystemResult", "measureSystemProposal", "measureSystemCaptureOpen"].includes(c.method)));
  rig.desk.setDraft({ ...rig.desk.draft, placement: "listeningPosition" });
  assert.equal(await rig.desk.start(), true, rig.desk.error);
  const [last] = before(rig.calls, "measureSystemStart", 2);
  assert.deepEqual(last!.map((c) => [c.core, c.method]), [["A", "measureSystemUnitCheck"], ["B", "measureSystemUnitCheck"]], "fresh checks, each on its own core, right before the start");
  const start = rig.calls.find((c) => c.method === "measureSystemStart")!.params as { checks: { serial: string }[]; setup: { units: { serial: string }[] } };
  assert.deepEqual(start.checks.map((c) => c.serial), ["A", "B"]);
  assert.deepEqual(start.setup.units.map((u) => u.serial), ["A", "B"]);
  assert.equal(rig.desk.mic, 1, "the microphone is open while it runs");
  // The settle wait in words, then the run done: the result and the proposal, the microphone closed.
  await rig.push({ ...STATE, phase: "ramp", serial: "A", output: 0, words: "waiting for the units' clocks to settle" });
  assert.equal(rig.desk.state?.words, "waiting for the units' clocks to settle");
  await rig.push({ ...STATE, active: false, phase: "done", resultReady: true, timingReliable: true });
  assert.equal(rig.desk.mic, 0);
  assert.equal(rig.desk.result?.arrivals.length, 2);
  assert.equal(rig.desk.proposal?.id, 7);
});

test("ATWEB: Control measures and runs the summed check but never applies: Apply is not offered and not sent", async () => {
  const rig = deskRig("control");
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.equal(await rig.desk.start(), true);
  await rig.push({ ...STATE, active: false, phase: "done", resultReady: true });
  // The summed check is still needed: it plays, so each unit is judged again right before it.
  assert.equal(proposalRights(rig.desk.proposal, "control").apply, false);
  assert.equal(await rig.desk.summed("sum"), true, rig.desk.error);
  const [b] = before(rig.calls, "measureSystemSumCheck", 2);
  assert.deepEqual(b!.map((c) => [c.core, c.method]), [["A", "measureSystemUnitCheck"], ["B", "measureSystemUnitCheck"]]);
  rig.passSumCheck();
  await rig.push({ ...STATE, purpose: "sum_check", active: false, phase: "done" });
  const r = proposalRights(rig.desk.proposal, "control");
  assert.equal(r.apply, false);
  assert.match(r.why, /Admin/);
  assert.equal(await rig.desk.apply(), false);
  assert.match(rig.desk.error, /needs the Admin role/);
  assert.equal(await rig.desk.revert(), false);
  assert.ok(!rig.calls.some((c) => c.method === "measureSystemApply" || c.method === "measureSystemRevert"), "nothing sent");
});

test("ATWEB: Admin applies, verifies on fresh checks; after a failed verify Keep is asked twice and only a press 1 s later keeps", async () => {
  const rig = deskRig("admin");
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.equal(await rig.desk.start(), true);
  await rig.push({ ...STATE, active: false, phase: "done", resultReady: true });
  rig.passSumCheck();
  rig.desk.proposal = await (rig.desk.session.core.call("measureSystemProposal", undefined) as Promise<MeasureSystemProposalRecord>);
  assert.equal(proposalRights(rig.desk.proposal, "admin").apply, true);
  assert.equal(await rig.desk.apply(), true, rig.desk.error);
  assert.deepEqual(rig.calls.find((c) => c.method === "measureSystemApply")!.params, { proposalId: 7 });
  assert.equal(rig.desk.applied?.stage, "applied");
  assert.equal(await rig.desk.summed("verify"), true);
  const [b] = before(rig.calls, "measureSystemVerify", 2);
  assert.deepEqual(b!.map((c) => [c.core, c.method]), [["A", "measureSystemUnitCheck"], ["B", "measureSystemUnitCheck"]], "fresh checks right before the verify");
  rig.failVerify();
  await rig.push({ ...STATE, purpose: "verify", active: false, phase: "done" });
  assert.equal(rig.desk.applied?.verified, false);
  // The first Keep: refused by the core; its words open the second question.
  assert.equal(await rig.desk.keep(), false);
  assert.match(rig.desk.question!.text, /did not agree/);
  assert.equal(rig.desk.confirmReady(), false);
  const keeps = () => rig.calls.filter((c) => c.method === "measureSystemKeep").length;
  assert.equal(keeps(), 1);
  // Too early: nothing sent.
  rig.setNow(rig.now + 500);
  assert.equal(await rig.desk.confirmKeep(), false);
  assert.equal(keeps(), 1);
  // Nothing presses it by itself: no call happens while time passes.
  rig.setNow(rig.now + 600);
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(keeps(), 1);
  assert.equal(rig.desk.confirmReady(), true);
  assert.equal(await rig.desk.confirmKeep(), true);
  assert.equal(keeps(), 2);
  assert.equal(rig.desk.applied?.stage, "kept");
  assert.equal(rig.desk.question, null);
});

test("ATWEB: Admin reverts; leaving the tab stops a running session and closes every unit's own core", async () => {
  const rig = deskRig("admin");
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.equal(await rig.desk.revert(), true);
  assert.ok(rig.calls.some((c) => c.method === "measureSystemRevert"));
  assert.equal(await rig.desk.start(), true);
  assert.equal(rig.desk.running, true);
  await rig.desk.leave();
  assert.ok(rig.calls.some((c) => c.method === "measureSystemStop"), "the session stopped");
  assert.deepEqual(rig.disposed, ["B"], "B's own core closed; A's is the screen's");
  assert.equal(rig.desk.mic, 0);
});
