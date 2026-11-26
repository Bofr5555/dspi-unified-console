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
import { KEYS, draftProblem, endedWords, guardWords, initialDraft, issueWords, placementProblem, proposalRights, refusalWords, runsWords, setupOf, shownIssue, withOutput, withUnit } from "../../src/screens/measure/system/model";
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

// -- WP FIN015 -- (REVF R2, probe P5) the core's sentence refusal is shown as the core said it
test("FIN015: Several units shows the core's words for a refusal, not the invalid notice's title over them", () => {
  const words = "Lower the volume to −12 dB or below to measure; it is 0.0 dB.";
  const e = new CoreCallError("invalid", `invalid argument: ${words}`, "invalid", words);
  const invalid = (k: string) => (k === "invalid" ? ({ key: "invalid", kind: "error", title: "Not a valid value", reason: "", action: "", detail: "" } as NoticeRecord) : null);
  assert.deepEqual(refusalWords(e, invalid), { key: "invalid", text: words });
  // A hub's refusal keeps the step's notice, whatever words the core has for it
  const hub = new CoreCallError("link", "DSPi Link: the units' domain ids differ", "dlink.not_one_domain", "Something went wrong: the units' domain ids differ");
  assert.deepEqual(refusalWords(hub, row), { key: KEYS.notOneDomain, text: "These units do not share a clock: The units' domain ids differ" });
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
function deskRig(role: "admin" | "control", opts: { caps?: string[]; sim?: boolean; guardS?: number; heldBy?: SystemSession["heldBy"] } = {}) { // -- WP WEBM3 -- heldBy
  let guardEnded = ""; // -- review AT-D2 -- the hub ended its guard: its words
  let guardNotice = ""; // -- WP REVERTW -- and the core's revert notice when it did not put everything back
  const calls: Call[] = [];
  let simOn = false; // -- WP ATDEMO --
  const simInfo = () => ({ available: !!opts.sim, on: simOn, label: "Simulated microphone", room: "the demo room", issue: null });
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
          if (guardEnded) {
            // -- WP REVERTW -- not wholly put back: the core's own stage and notice
            applied = guardNotice
              ? { ...applied, stage: "revert_incomplete", message: guardEnded, revertNotice: guardNotice }
              : { ...applied, stage: "reverted", message: guardEnded };
            throw new CoreCallError("invalid", `invalid argument: ${guardEnded}`, "invalid");
          }
          return { ...STATE, purpose: "verify" };
        case "measureSystemStop":
          return { ...STATE, active: false, phase: "aborted" };
        case "measureSystemApply":
          if (role !== "admin") throw new CoreCallError("denied", "this console's role on the hub does not allow that", "denied");
          applied = { ...APPLIED, stage: "applied", revisionBefore: 4, revisionAfter: 5, guardS: opts.guardS ?? 0 };
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
        // -- WP ATDEMO --
        case "measureSystemSimInfo":
          return simInfo();
        case "measureSystemSimSet":
          if ((params as { on: boolean }).on && !opts.sim) throw new CoreCallError("dlink", "DSPi Link: the simulated microphone is the demo hub's only", "unsupported");
          simOn = (params as { on: boolean }).on;
          return simInfo();
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
    heldBy: opts.heldBy, // -- WP WEBM3 --
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
    hubEndsGuard(words: string, notice = "") {
      guardEnded = words;
      guardNotice = notice;
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
  assert.deepEqual(rig.desk.units[1]!.outputs.map((o) => o.name), ["Out 1", "Out 2"]);
  // No placement chosen: nothing is sent, nothing plays.
  assert.equal(await rig.desk.start(), false);
  assert.match(rig.desk.error, /Choose where the microphone is/);
  assert.equal(rig.desk.mic, 0);
  rig.desk.setDraft({ ...rig.desk.draft, placement: "headphones" });
  assert.equal(await rig.desk.start(), false);
  assert.ok(!rig.calls.some((c) => c.method.startsWith("measureSystem") && !["measureSystemState", "measureSystemApplied", "measureSystemResult", "measureSystemProposal", "measureSystemCaptureOpen", "measureSystemSimInfo" /* -- WP ATDEMO -- no I/O */].includes(c.method)));
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

// -- WP ATDEMO --
test("ATDEMO: with the demo hub's simulated microphone the page opens none; without it, the browser's (and the simulated one goes off)", async () => {
  const rig = deskRig("admin", { sim: true });
  await rig.desk.enter();
  assert.equal(rig.desk.sim?.available, true);
  assert.equal(rig.desk.useSim, true, "the default where the demo hub offers it");
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.equal(await rig.desk.start(), true, rig.desk.error);
  assert.equal(rig.desk.mic, 0, "no browser microphone");
  const order = rig.calls.map((c) => c.method).filter((m) => m === "measureSystemSimSet" || m === "measureSystemStart");
  assert.deepEqual(order, ["measureSystemSimSet", "measureSystemStart"], "on before the start");
  const start = rig.calls.find((c) => c.method === "measureSystemStart")!.params as { setup: { captureRateHz: number } };
  assert.equal(start.setup.captureRateHz, 48_000);
  await rig.push({ ...STATE, active: false, phase: "done", resultReady: true });
  assert.equal(await rig.desk.summed("sum"), true, rig.desk.error);
  assert.equal(rig.desk.mic, 0, "the summed check hears the room too");
  await rig.push({ ...STATE, purpose: "sum_check", active: false, phase: "done" });
  // The browser's microphone chosen: opened first, the simulated one off before the start.
  rig.desk.setUseSim(false);
  assert.equal(await rig.desk.start(), true, rig.desk.error);
  assert.equal(rig.desk.mic, 1);
  const sets = rig.calls.filter((c) => c.method === "measureSystemSimSet").map((c) => (c.params as { on: boolean }).on);
  assert.deepEqual(sets.slice(-1), [false]);
  // No room on this hub: never offered, never asked.
  const plain = deskRig("admin");
  await plain.desk.enter();
  assert.equal(plain.desk.useSim, false);
  plain.desk.setUseSim(true);
  assert.equal(plain.desk.simOn, false);
});

test("ATDEMO: a unit's lost lock is the core's own key (no rewording on the page)", () => {
  const row = (k: string) => ({ key: k, kind: "warning" as const, title: k === "measure.system.abort.disturbance" ? "Unit lost lock" : "The hub stopped the measurement", reason: "", action: "", detail: "" });
  assert.deepEqual(shownIssue({ key: "measure.system.abort.disturbance", message: "DEMO-TB-C: its time base was disturbed (2 -> 3, last reanchor)" }, row), {
    key: "measure.system.abort.disturbance",
    text: "Unit lost lock: DEMO-TB-C: its time base was disturbed (2 -> 3, last reanchor)",
  });
  assert.equal(shownIssue({ key: "measure.system.abort.hub", message: "the hub stopped the measurement (lease)" }, row).key, "measure.system.abort.hub");
});

// -- review AT-D2 --
test("review AT-D2: the hub's guard of an apply counts down from the apply, and a guard the hub ended is shown in its words", async () => {
  const rig = deskRig("admin", { guardS: 300 });
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.equal(await rig.desk.start(), true);
  await rig.push({ ...STATE, active: false, phase: "done", resultReady: true });
  rig.passSumCheck();
  rig.desk.proposal = await (rig.desk.session.core.call("measureSystemProposal", undefined) as Promise<MeasureSystemProposalRecord>);
  assert.equal(rig.desk.guarded, false);
  assert.equal(rig.desk.guardLeftS(), null);
  assert.equal(await rig.desk.apply(), true, rig.desk.error);
  assert.equal(rig.desk.guarded, true);
  assert.equal(rig.desk.guardLeftS(), 300);
  rig.setNow(rig.now + 100_400);
  assert.equal(rig.desk.guardLeftS(), 200);
  assert.match(guardWords(300, rig.desk.guardLeftS()), /Keep or Revert within 200 s: without Keep the hub puts everything back by itself/);
  // The deadline passed: the page says so, and the verify the core refuses shows the hub's words.
  rig.setNow(rig.now + 200_000);
  assert.equal(rig.desk.guardLeftS(), 0);
  assert.match(guardWords(300, 0), /has run out/);
  rig.hubEndsGuard("the hub ended the guard (timeout), but the alignment was NOT wholly put back (not_written): nothing was set back", "revert.not_written");
  assert.equal(await rig.desk.summed("verify"), false);
  // -- WP REVERTW -- its own stage, never "reverted"
  assert.equal(rig.desk.applied?.stage, "revert_incomplete", "the hub's outcome is read, not left as applied");
  assert.equal(rig.desk.guarded, false);
  assert.match(rig.desk.error, /NOT wholly put back/);
  assert.match(
    endedWords(rig.desk.applied!),
    /^The revert was not fully successful\. The hub ended the guard \(timeout\), but the alignment was NOT wholly put back.* Check the settings, or restore a backup \(a snapshot\)\.$/,
  );
});

test("review AT-D2: the guard in words: none on an older hub, uncounted when this page did not see the apply; the end in the core's words", () => {
  assert.match(guardWords(0, null), /no guard .* nothing reverts by itself/);
  assert.match(guardWords(240, null), /unless it is kept within 240 s of the apply/);
  assert.equal(endedWords({ stage: "kept", message: "kept: the alignment stays" }), "Kept.");
  assert.equal(endedWords({ stage: "reverted", message: "" }), "Reverted: the installation and every unit are as they were filed.");
  assert.equal(endedWords({ stage: "reverted", message: "reverted: the model and every unit's output delays are as filed" }), "Reverted: the model and every unit's output delays are as filed");
  assert.equal(endedWords({ stage: "applied", message: "x" }), "");
  // -- WP REVERTW -- a revert not fully successful: the notice's title, what, and what to do
  assert.equal(
    endedWords({ stage: "revert_incomplete", message: "reverted, but not wholly: UNIT-2 out.delay: refused (not written)", revertNotice: "revert.incomplete" }),
    "The revert was not fully successful. Reverted, but not wholly: UNIT-2 out.delay: refused (not written). Check the settings, or restore a backup (a snapshot).",
  );
  assert.match(endedWords({ stage: "revert_incomplete", message: "" }), /^The revert was not fully successful\. .*restore a backup \(a snapshot\)\.$/);
});

// -- WP WEBM3 -- a unit open in another tab is judged on that tab's core: no second session, and
// the desk never closes it; the desk's own core goes before the unit is opened in a tab.

/** Another tab's session of unit `serial`: its core records its calls as `tab:<serial>`. */
function tabOf(serial: string, calls: Call[], status = "open") {
  const snapshot = { device: {}, state: { numOutputChannels: 3, numInputChannels: 2, outputEnabled: [true, true, true], channelNames: ["L", "R", "Out 1", "Out 2", "Out 3"] }, outputLabels: [] } as never;
  let disposed = false;
  const core = {
    async call(method: string) {
      calls.push({ core: `tab:${serial}`, method, params: null });
      if (method === "measureSystemUnitCheck") return { serial, preflight: { clear: true, blockers: [], warnings: [] }, context: "{}" };
      return null;
    },
    subscribe: () => () => undefined,
    dispose: async () => void (disposed = true),
  };
  return { held: { core: core as unknown as SystemSession["core"], state: { status, snapshot }, transport: "Hub" }, disposed: () => disposed };
}

test("WEBM3: a unit open in another tab is measured on that tab's core; none of its own is opened or closed", async () => {
  const shared: Call[] = [];
  const b = tabOf("B", shared);
  const rig = deskRig("control", { heldBy: (s) => (s === "B" ? b.held : null) });
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 3);
  assert.equal(rig.desk.units[1]!.tab, "Hub");
  assert.ok(!rig.calls.some((c) => c.method === "linkConnect" || c.method === "open"), "no second session for B");
  assert.equal(await rig.desk.start(), true, rig.desk.error);
  assert.ok(shared.some((c) => c.core === "tab:B" && c.method === "measureSystemUnitCheck"), "B judged on its tab's core");
  rig.desk.setDraft(withUnit(rig.desk.draft, "B", false));
  await rig.desk.leave();
  assert.equal(b.disposed(), false, "the tab's core stays the tab's");
  assert.deepEqual(rig.disposed, []);
});

test("WEBM3: a unit still opening in a tab is not opened a second time; it is named", async () => {
  const b = tabOf("B", [], "opening");
  const rig = deskRig("control", { heldBy: (s) => (s === "B" ? b.held : null) });
  await rig.desk.enter();
  rig.desk.setDraft(withUnit(withUnit(rig.desk.draft, "A", true), "B", true));
  await until(() => rig.desk.units[1]!.error !== "");
  assert.match(rig.desk.units[1]!.error, /^B: it is being opened in another tab/);
  assert.ok(!rig.calls.some((c) => c.method === "open"));
});

test("WEBM3: Open in the console closes the desk's own core first; the tab's core is used after", async () => {
  const shared: Call[] = [];
  const b = tabOf("B", shared);
  let inTab = false;
  const rig = deskRig("control", { heldBy: (s) => (s === "B" && inTab ? b.held : null) });
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  assert.ok(rig.calls.some((c) => c.core === "B" && c.method === "open"), "not open anywhere: a core of the desk's");
  assert.equal(await rig.desk.handOver("B"), true);
  assert.deepEqual(rig.disposed, ["B"], "closed before the tab opens it");
  inTab = true;
  assert.equal(await rig.desk.start(), true, rig.desk.error);
  assert.ok(shared.some((c) => c.core === "tab:B" && c.method === "measureSystemUnitCheck"));
  assert.equal(rig.desk.units[1]!.tab, "Hub");
  assert.equal(await rig.desk.handOver("B"), false, "not while it measures");
});

// -- WP WEBM5 -- (review RC8) the tab that lent its core closed: the borrowed core is dropped and
// the desk opens one of its own, rather than failing on the closed one until the unit is unchosen
test("WEBM5: a unit's tab closed: its core is no longer borrowed; the desk opens its own and measures on it", async () => {
  const shared: Call[] = [];
  const b = tabOf("B", shared);
  let inTab = true;
  const rig = deskRig("control", { heldBy: (s) => (s === "B" && inTab ? b.held : null) });
  await rig.desk.enter();
  rig.desk.setDraft({ ...withUnit(withUnit(rig.desk.draft, "A", true), "B", true), placement: "listeningPosition" });
  await until(() => rig.desk.units[1]!.outputs.length === 3);
  assert.equal(rig.desk.units[1]!.tab, "Hub");
  inTab = false; // the tab closed (its core with it)
  assert.equal(await rig.desk.start(), false);
  assert.match(rig.desk.error, /B: .*its tab was closed/);
  assert.equal(rig.desk.units[1]!.tab, "");
  await until(() => rig.calls.some((c) => c.core === "B" && c.method === "open"));
  await until(() => rig.desk.units[1]!.outputs.length === 2);
  const judged = shared.filter((c) => c.method === "measureSystemUnitCheck").length;
  assert.equal(await rig.desk.start(), true, rig.desk.error);
  assert.ok(rig.calls.some((c) => c.core === "B" && c.method === "measureSystemUnitCheck"), "judged on the desk's own core");
  assert.equal(shared.filter((c) => c.method === "measureSystemUnitCheck").length, judged, "never on the closed tab's");
  await rig.desk.leave();
  assert.deepEqual(rig.disposed, ["B"]);
  assert.equal(b.disposed(), false);
});
