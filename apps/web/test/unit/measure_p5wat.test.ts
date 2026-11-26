// WP P5WAT unit tests: the auto-tune's and the documents' rules (words, the refusals through the
// notice table, the layout's measurement, the merge, who may delete), the Worker's turns of the
// tune (its runs and its guard's timer), the tune desk against a stand-in core (the verify run
// by itself after an apply, Keep's second question with its 1 s rule and never a press of its
// own, the question withdrawn by Revert, a new apply and a new verify, starts withdrawn when the
// tab is left), the documents desk (Control cannot delete a hub document), and on the real core
// (the demo device as a stereo three-way, P5DEMO's simulated room): a tune measured, proposed,
// summed-checked, applied, verified by itself and kept, the A/B toggled; another reverted; a
// timing-unreliable measurement's timing writes withdrawn with their reason. No audio anywhere.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { MeasureWorkerRoute, TICK_MS, TUNE_TURN_RETRIES, isMeasureEvent, tuneActive, type MeasureStateRecord, type TuneEvent } from "../../src/core/measure-protocol";
import { CoreCallError, type CoreEvent, type GuardedApplyRecord, type NoticeRecord } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { MeasureDesk } from "../../src/screens/measure/MeasureScreen";
import { MEASURE_TABS } from "../../src/screens/measure/MeasureScreen";
import {
  CONFIRM_MS,
  DocsDesk,
  KEEP_UNVERIFIED,
  TuneDesk,
  checkWords,
  chosenLayout,
  docOutputs,
  edited,
  feedWords,
  guardSeconds,
  hubRights,
  issueWords,
  layoutSetup,
  mergeRows,
  proposalSeries,
  refusalWords,
  stageWords,
  tunePlaceOf,
  type DocRow,
  type DocsHost,
  type TuneLayoutRecord,
  type TuneProposalRecord,
  type TuneStateRecord,
} from "../../src/screens/measure/tune-model";

// ------------------------------------------------------------------ the rules

const ROWS: Record<string, NoticeRecord> = {
  [KEEP_UNVERIFIED]: { key: KEEP_UNVERIFIED, kind: "warning", title: "The verify did not confirm the tune", reason: "r", action: "a" } as NoticeRecord,
  "measure.tune.timing_unreliable": { key: "measure.tune.timing_unreliable", kind: "warning", title: "No timing changes from this measurement", reason: "", action: "" } as NoticeRecord,
  read_only: { key: "read_only", kind: "error", title: "Read-only", reason: "", action: "" } as NoticeRecord,
};
const row = (k: string) => ROWS[k] ?? null;
const name = (o: number) => `OUT${o + 1}`;

test("P5WAT: a refusal in words through the notice table; a key it does not know keeps the core's words", () => {
  const r = refusalWords(new CoreCallError("invalid", `${KEEP_UNVERIFIED}: the verify did not confirm the tune; Keep it only if you are sure`, "invalid"), row);
  assert.equal(r.key, KEEP_UNVERIFIED);
  assert.equal(r.text, "The verify did not confirm the tune: The verify did not confirm the tune; Keep it only if you are sure");
  // P5HOLD's new key needs no code: without a row its words alone.
  assert.equal(refusalWords(new CoreCallError("invalid", `invalid argument: ${KEEP_UNVERIFIED}: why`, "invalid"), row).key, KEEP_UNVERIFIED, "after the error's prefix");
  const hold = refusalWords(new CoreCallError("invalid", "measure.tune.keep_during_verify: the verify is still running", "invalid"), row);
  assert.deepEqual(hold, { key: "measure.tune.keep_during_verify", text: "The verify is still running" });
  assert.deepEqual(refusalWords(new CoreCallError("read_only", "refused", "read_only"), row), { key: "read_only", text: "Read-only: Refused" });
  assert.deepEqual(refusalWords(new Error("plain words"), row), { key: "", text: "Plain words" });
  assert.equal(issueWords({ key: "measure.tune.timing_unreliable", message: "L: no delay" }, row), "No timing changes from this measurement: L: no delay");
  assert.equal(issueWords({ key: "x.y", message: "as is" }, row), "As is");
});

// -- WP FIN015 -- (REVF R2, probe P5) the core's sentence refusal is shown as the core said it
test("FIN015: Auto-tune shows the core's words for a refusal, not the invalid notice's title over them", () => {
  const words = "Lower the volume to −12 dB or below to measure; it is 0.0 dB.";
  const e = new CoreCallError("invalid", `invalid argument: ${words}`, "invalid", words);
  assert.deepEqual(refusalWords(e, () => ({ key: "invalid", kind: "error", title: "Not a valid value", reason: "", action: "" }) as NoticeRecord), { key: "invalid", text: words });
  // A keyed refusal keeps its key (Keep's second question reads it) with the core's words
  const keyed = new CoreCallError("invalid", `invalid argument: ${KEEP_UNVERIFIED}: why`, "invalid", "The verify did not confirm the tune: Why");
  assert.deepEqual(refusalWords(keyed, row), { key: KEEP_UNVERIFIED, text: "The verify did not confirm the tune: Why" });
});

const LAYOUT: TuneLayoutRecord = {
  template: "3way",
  notes: [],
  outputs: [
    { output: 0, role: "low", speaker: "L", group: 0, name: "", feedInput: 0, feedGainDb: 0, feedInverted: false },
    { output: 1, role: "mid", speaker: "L", group: 0, name: "", feedInput: 0, feedGainDb: -3, feedInverted: true },
    { output: 2, role: "high", speaker: "L", group: 0, name: "", feedInput: null, feedGainDb: 0, feedInverted: false },
  ],
};

test("P5WAT: the layout in words; only the outputs left on are measured and tuned", () => {
  assert.equal(feedWords(LAYOUT.outputs[0]!), "fed by input 1 at 0.0 dB");
  assert.equal(feedWords(LAYOUT.outputs[1]!), "fed by input 1 at -3.0 dB, polarity inverted");
  assert.equal(feedWords(LAYOUT.outputs[2]!), "no programme input feeds it");
  const on = chosenLayout(LAYOUT, new Set([2]));
  assert.deepEqual(
    on.outputs.map((o) => o.output),
    [0, 1],
  );
  const s = layoutSetup(on, 1000, "", "Auto-tune layout", { placement: "listeningPosition" });
  assert.equal(s.kind, "walked");
  assert.deepEqual(
    s.outputs.map((o) => [o.output, o.role, o.speaker, o.group]),
    [
      [0, "low", "L", 0],
      [1, "mid", "L", 0],
    ],
  );
  assert.equal(s.placement, "listeningPosition");
  assert.match(checkWords("pending"), /cannot be applied before it confirms the timing/);
  assert.match(checkWords("failed"), /withdrawn/);
});

const GUARD: GuardedApplyRecord = { id: 7, phase: "waiting", label: "Auto-tune", hub: false, timeoutMs: 300_000, remainingMs: 41_200, reason: "", message: "", changes: [], notes: [] };
const SESSION: MeasureStateRecord = { phase: "idle", kind: "walked", active: false, wantsCapture: false, captureRateHz: 0, position: 0, positions: 0, run: 0, runs: 0, rampLevels: [], frames: 0, generatorStarts: 0, progress: 0, resultReady: false, frameFaults: 0 };
function tstate(stage: TuneStateRecord["stage"], patch: Partial<TuneStateRecord> = {}): TuneStateRecord {
  return { stage, running: false, run: 0, runs: 0, session: { ...SESSION }, guard: { ...GUARD, phase: stage === "kept" ? "kept" : stage === "applied" || stage === "verifying" || stage === "verified" ? "waiting" : "idle" }, snapshotBeforeId: "", snapshotAfterId: "", ...patch };
}

test("P5WAT: a tune's stage and guard in words", () => {
  assert.match(stageWords(tstate("verifying", { running: true, run: 2, runs: 3 })), /Verifying the tune.*run 2 of 3.*Keep is offered once the verify has its verdict/);
  assert.match(stageWords(tstate("proposed")), /nothing is written yet/);
  assert.match(stageWords(tstate("verified", { verify: { outputs: [], sums: [], transitions: [], timingReliable: true, passed: false, notes: [] } })), /does not agree/);
  assert.equal(guardSeconds(GUARD), 42);
  assert.equal(guardSeconds({ ...GUARD, phase: "kept" }), 0);
  assert.equal(tuneActive(tstate("applied")), true, "a waiting guard is turned for its timer");
  assert.equal(tuneActive(tstate("proposed", { running: true })), true);
  assert.equal(tuneActive(tstate("kept")), false);
  const curve = { freqsHz: [100, 200], magDb: [0, 1], valid: [true, true], spl: false };
  const p = { curves: [{ speaker: "L", coherent: true, before: curve, predicted: curve, target: curve }] } as unknown as TuneProposalRecord;
  assert.deepEqual(
    proposalSeries(p).map((s) => `${s.kind}:${s.label}`),
    ["measured:L now", "predicted:L predicted", "target:L target"],
  );
});

test("P5WAT: documents: who may delete a hub document, the merge, a page document renamed and noted", () => {
  assert.deepEqual(hubRights("control", false), { write: true, remove: false }, "Control cannot delete");
  assert.deepEqual(hubRights("admin", false), { write: true, remove: true });
  assert.deepEqual(hubRights("viewer", false), { write: false, remove: false });
  assert.deepEqual(hubRights("admin", true), { write: false, remove: false }, "read-only: nothing");
  const info = (id: string, kind: string, savedMs: number, bytes = 10) => ({ id, kind, name: id, note: "", serial: "", savedMs, bytes });
  const hub: DocRow[] = [{ key: "h1", where: "hub", info: info("m-1", "measurement", 5) }, { key: "h2", where: "hub", info: info("t-1", "target", 1) }];
  const local: DocRow[] = [
    { key: "local:1", where: "local", info: info("m-1", "measurement", 5), text: "{}" },
    { key: "local:2", where: "local", info: info("m-1", "measurement", 5, 99), text: "{}" },
    { key: "local:3", where: "local", info: info("", "mic", 2), text: "{}" },
  ];
  assert.deepEqual(
    mergeRows(local, hub).map((r) => r.key),
    ["h1", "local:2", "local:3", "h2"],
    "the hub's copy wins; a different local copy of its id stays beside it; by kind",
  );
  const doc = JSON.stringify({ schemaVersion: 1, meta: { kind: "mic", name: "A" }, mic: { name: "A" } });
  const renamed = JSON.parse(edited(doc, "mic", { name: "B" })!) as { meta: { name: string } };
  assert.equal(renamed.meta.name, "B");
  assert.equal((JSON.parse(edited(doc, "mic", { note: "on the stand" })!) as { mic: { note: string } }).mic.note, "on the stand");
  assert.equal(edited("{", "mic", { name: "x" }), null);
  assert.equal(edited(doc, "target", { note: "x" }), null, "no body of that kind");
  assert.deepEqual(docOutputs(JSON.stringify({ measurement: { outputs: [{ output: 0 }, { output: 2 }] } })), [0, 2]);
  assert.deepEqual(docOutputs("x"), []);
});

test("P5WAT: the tab slot holds the auto-tune and the documents; the documents are a Viewer's too", () => {
  const ids = MEASURE_TABS.map((t) => `${t.id}${t.passive ? "*" : ""}`);
  // -- WP WEB4 -- in Qt's order: Several units (ATWEB's "system") after them, last
  assert.deepEqual(ids, ["measure", "results*", "live", "tune", "docs*", "system"]);
});

// ------------------------------------------------------------------ the Worker's turns of the tune

function tuneRig(states: TuneStateRecord[]) {
  const calls: string[] = [];
  const posted: unknown[] = [];
  const timers = new Map<number, () => void>();
  let n = 0;
  const route = new MeasureWorkerRoute(
    async (method) => {
      calls.push(method);
      if (method === "measureTuneTick") return states.shift() ?? tstate("kept");
      if (method === "measureTick") return { ...SESSION, active: false, phase: "done" };
      return null;
    },
    (e) => posted.push(e),
    {
      MessageChannel,
      workletUrl: "x",
      setTimeout: ((fn: () => void, ms: number) => {
        assert.equal(ms, TICK_MS);
        timers.set(++n, fn);
        return n as unknown as ReturnType<typeof setTimeout>;
      }) as unknown as typeof setTimeout,
      clearTimeout: ((t: number) => void timers.delete(t)) as unknown as typeof clearTimeout,
    },
  );
  const fire = async () => {
    const fns = [...timers.values()];
    timers.clear();
    for (const f of fns) f();
    await new Promise((r) => setTimeout(r, 5));
  };
  return { route, calls, posted, fire };
}

test("P5WAT: after an apply the Worker turns the tune (its guard's timer) until the guard ends, beside the session's turns", async () => {
  const rig = tuneRig([tstate("applied"), tstate("verifying", { running: true }), tstate("verified")]);
  rig.route.after("measureTuneApply", true, { ...GUARD, phase: "not_written" });
  assert.equal(rig.route.tuneTicking, false, "nothing applied: no turns");
  rig.route.after("measureTuneApply", true, GUARD);
  assert.equal(rig.route.tuneTicking, true);
  // A measurement started meanwhile has its own turns: the guard keeps its timer.
  rig.route.after("measureStart", true, { ...SESSION, active: true });
  assert.equal(rig.route.ticking, true);
  for (let i = 0; i < 5; i++) await rig.fire();
  const tune = rig.posted.filter((e) => (e as TuneEvent).event === "measureTune") as TuneEvent[];
  assert.deepEqual(
    tune.map((e) => e.state.stage),
    ["applied", "verifying", "verified", "kept"],
  );
  assert.equal(rig.route.tuneTicking, false, "kept: no more turns");
  assert.ok(tune.every((e) => !isMeasureEvent(e)), "the screen's measure events stay apart");
  // The summed check's runs are turned too; a refused one is not; a closed session ends them.
  const r2 = tuneRig([]);
  r2.route.after("measureTuneSumCheck", false, null);
  assert.equal(r2.route.tuneTicking, false);
  r2.route.after("measureTuneSumCheck", true, tstate("sumCheck", { running: true }));
  assert.equal(r2.route.tuneTicking, true);
  r2.route.after("close", true, null);
  assert.equal(r2.route.tuneTicking, false);
  await r2.fire();
  assert.deepEqual(r2.calls, []);
});

test("review P5-E: a failed turn does not end the guard's timer; only turns that keep failing do", async () => {
  const calls: string[] = [];
  const timers = new Map<number, () => void>();
  let n = 0;
  let fail = 3;
  const route = new MeasureWorkerRoute(
    async (method) => {
      calls.push(method);
      if (fail > 0) {
        fail--;
        throw new Error("a turn that failed");
      }
      return tstate("applied");
    },
    () => undefined,
    {
      MessageChannel,
      workletUrl: "x",
      setTimeout: ((fn: () => void) => (timers.set(++n, fn), n as unknown as ReturnType<typeof setTimeout>)) as unknown as typeof setTimeout,
      clearTimeout: ((t: number) => void timers.delete(t)) as unknown as typeof clearTimeout,
    },
  );
  const fire = async () => {
    const fns = [...timers.values()];
    timers.clear();
    for (const f of fns) f();
    await new Promise((r) => setTimeout(r, 2));
  };
  route.after("measureTuneApply", true, GUARD);
  for (let i = 0; i < 6; i++) await fire();
  assert.equal(route.tuneTicking, true, "three failed turns, then the guard is turned on");
  assert.equal(calls.length, 6);
  assert.equal(timers.size, 1, "one chain of turns");
  // A start while a turn is in flight does not double the turns.
  const turning = fire();
  route.after("measureTuneVerify", true, tstate("verifying", { running: true }));
  await turning;
  assert.equal(timers.size, 1, "still one chain");
  // Turns that never succeed end the timer (a core that is gone).
  fail = Infinity;
  for (let i = 0; i < TUNE_TURN_RETRIES + 2; i++) await fire();
  assert.equal(route.tuneTicking, false);
  assert.equal(timers.size, 0);
});

// ------------------------------------------------------------------ the tune desk on a stand-in core

type Handler = (p: unknown) => unknown;
function standIn(handlers: Record<string, Handler>) {
  const calls: string[] = [];
  const listeners = new Set<(e: unknown) => void>();
  const core = {
    call: async (m: string, p: unknown) => {
      calls.push(m);
      const h = handlers[m];
      if (!h) throw new CoreCallError("unknown_method", m);
      return h(p);
    },
    subscribe: (fn: (e: unknown) => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
  };
  let mic = 0;
  const host = {
    core,
    result: "{\"measurement\":{}}" as string | null,
    resultFrom: "run",
    micDocument: "",
    targetDocument: "",
    targetName: "",
    draft: null,
    hubMeasure: true,
    hubRole: "control",
    setResult(text: string | null) {
      host.result = text;
    },
    setDocs() {},
    openMic: async () => {
      mic++;
      return 48000;
    },
    closeMic: async () => {
      mic--;
    },
  };
  const push = (s: TuneStateRecord) => listeners.forEach((l) => l({ event: "measureTune", state: s }));
  return { core, host: host as unknown as DocsHost, calls, push, mics: () => mic };
}

const PROPOSAL = { id: 3, writes: [{ output: 2, kind: "delay", timing: true, words: "delay", params: [] }], curves: [], transitions: [], held: [], withdrawn: [], warnings: [], notes: [], timingReliable: true, sumCheck: "passed", serial: "", measurementId: "", measurementName: "" } as unknown as TuneProposalRecord;
const failedVerify = { outputs: [{ output: 0, rmsDb: 3.1 }], sums: [], transitions: [], timingReliable: true, passed: false, notes: [] };
const unverifiedRefusal = () => {
  // As the wasm core says it: the error's prefix, then the key.
  throw new CoreCallError("invalid", `invalid argument: ${KEEP_UNVERIFIED}: the verify did not confirm the tune`, "invalid");
};

test("P5WAT: after an apply the verify runs by itself; while it runs Keep is not offered", async () => {
  let st = tstate("proposed");
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => PROPOSAL,
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneApply: () => ((st = tstate("applied")), GUARD),
    measureTuneVerify: () => (st = tstate("verifying", { running: true, runs: 3, run: 1 })),
    measureTuneKeep: () => assert.fail("no Keep while the verify runs"),
  });
  const t = new TuneDesk(s.host);
  await t.enter();
  assert.equal(await t.apply(), true);
  assert.deepEqual(
    s.calls.filter((c) => c.startsWith("measureTune") && c !== "measureTuneState" && c !== "measureTuneProposal"),
    ["measureTuneApply", "measureTuneVerify"],
    "the verify follows the apply with no press",
  );
  assert.equal(s.mics(), 1, "the microphone is open for the verify");
  assert.equal(t.stage, "verifying");
  assert.equal(t.keepOffered, false);
  assert.equal(await t.keep(), false);
  // The verdict: the microphone closes, Keep is offered.
  await t.onState((st = tstate("verified", { verify: { ...failedVerify, passed: true } })));
  assert.equal(s.mics(), 0);
  assert.equal(t.keepOffered, true);
  await t.leave();
});

test("P5WAT: Keep after a failed verify: the core refuses the first, the second question, only the person's own press at least 1 s later keeps", async () => {
  let clock = 1_000_000;
  let st = tstate("verified", { verify: failedVerify });
  let refusals = 0;
  let kept = 0;
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => PROPOSAL,
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneKeep: () => {
      if (refusals === 0) {
        refusals++;
        unverifiedRefusal();
      }
      kept++;
      st = tstate("kept");
      return { guard: { ...GUARD, phase: "kept" }, snapshotBeforeId: "", snapshotBefore: "b", snapshotAfterId: "", snapshotAfter: "a", measurementId: "", measurement: "", notes: [] };
    },
    measureTuneRevert: () => ((st = tstate("reverted")), { ...GUARD, phase: "reverted", message: "Reverted." }),
    measureTuneApply: () => ((st = tstate("applied")), GUARD),
    measureTuneVerify: () => (st = tstate("verifying", { running: true })),
  });
  const t = new TuneDesk(s.host, () => clock);
  await t.enter();
  assert.equal(await t.keep(), false);
  assert.equal(refusals, 1, "the core's first refusal");
  assert.ok(t.question, "the second question");
  assert.equal(t.question!.why, "unverified");
  assert.match(t.question!.text, /^The verify did not confirm the tune/);
  // Nothing presses for the person: no time alone keeps, and a press too early sends nothing.
  clock += 5000;
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(kept, 0, "never confirmed by itself");
  clock -= 4500;
  assert.equal(t.confirmReady(), false);
  assert.equal(await t.confirmKeep(), false);
  assert.equal(s.calls.filter((c) => c === "measureTuneKeep").length, 1, "an early press sends nothing");
  clock += CONFIRM_MS;
  assert.equal(t.confirmReady(), true);
  assert.equal(await t.confirmKeep(), true);
  assert.equal(kept, 1);
  assert.equal(t.question, null);
  assert.equal(t.stage, "kept");
  await t.leave();
});

test("P5WAT: a pending second Keep is withdrawn by Revert, a new apply, a new verify and the guard's end", async () => {
  let clock = 0;
  let st = tstate("verified", { verify: failedVerify });
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => PROPOSAL,
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneKeep: unverifiedRefusal,
    measureTuneRevert: () => ((st = tstate("reverted")), { ...GUARD, phase: "reverted" }),
    measureTuneApply: () => ((st = tstate("applied")), GUARD),
    measureTuneVerify: () => (st = tstate("verifying", { running: true })),
  });
  const t = new TuneDesk(s.host, () => clock);
  await t.enter();
  const ask = async () => {
    st = tstate("verified", { verify: failedVerify });
    t.state = st;
    await t.keep();
    assert.ok(t.question, "asked");
    clock += 2000;
  };
  await ask();
  await t.revert();
  assert.equal(t.question, null, "Revert");
  assert.equal(await t.confirmKeep(), false);
  await ask();
  await t.verify();
  assert.equal(t.question, null, "a new verify");
  await ask();
  st = tstate("proposed");
  t.state = tstate("verified", { verify: failedVerify });
  await t.apply();
  assert.equal(t.question, null, "a new apply");
  await ask();
  await t.onState(tstate("reverted"));
  assert.equal(t.question, null, "the guard ended (its timeout)");
  await ask();
  await t.onState(tstate("verifying", { running: true }));
  assert.equal(t.question, null, "a verify started elsewhere");
  assert.equal(s.calls.filter((c) => c === "measureTuneKeep").length, 5, "one press each, no second one sent");
  await t.leave();
});

test("P5WAT: Keep with no verdict at all asks the same question before anything is sent", async () => {
  let clock = 0;
  let st = tstate("applied");
  let keeps = 0;
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => PROPOSAL,
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneKeep: () => {
      keeps++;
      st = tstate("kept");
      return { guard: { ...GUARD, phase: "kept" }, snapshotBeforeId: "", snapshotBefore: "", snapshotAfterId: "", snapshotAfter: "", measurementId: "", measurement: "", notes: [] };
    },
  });
  const t = new TuneDesk(s.host, () => clock);
  await t.enter();
  assert.equal(await t.keep(), false);
  assert.equal(keeps, 0, "nothing sent on the first press");
  assert.equal(t.question?.why, "noVerdict");
  assert.equal(await t.confirmKeep(), false);
  clock += CONFIRM_MS;
  assert.equal(await t.confirmKeep(), true);
  assert.equal(keeps, 1);
  await t.leave();
});

test("P5WAT: leaving the tab withdraws a start still on its way: an apply is reverted, a verify and a summed check stopped", async () => {
  let st = tstate("proposed");
  let release: (() => void) | null = null;
  const gate = () => new Promise<void>((r) => (release = r));
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => ({ ...PROPOSAL, sumCheck: "pending" }),
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneApply: async () => {
      await gate();
      st = tstate("applied");
      return GUARD;
    },
    measureTuneSumCheck: async () => {
      await gate();
      return (st = tstate("sumCheck", { running: true }));
    },
    measureTuneVerify: async () => {
      await gate();
      return (st = tstate("verifying", { running: true }));
    },
    measureTuneRevert: () => ((st = tstate("reverted")), { ...GUARD, phase: "reverted" }),
    measureTuneStop: () => (st = tstate(st.stage === "verifying" ? "applied" : "proposed")),
  });
  const t = new TuneDesk(s.host);
  await t.enter();
  const applied = t.apply();
  await new Promise((r) => setTimeout(r, 5));
  await t.leave();
  release!();
  assert.equal(await applied, false);
  assert.ok(s.calls.lastIndexOf("measureTuneRevert") > s.calls.indexOf("measureTuneApply"), s.calls.join(" "));
  assert.equal(s.calls.includes("measureTuneVerify"), false, "no verify behind a tab that is gone");
  await t.enter();
  st = tstate("proposed");
  s.calls.length = 0;
  const checked = t.sumCheck();
  await new Promise((r) => setTimeout(r, 5));
  await t.leave();
  release!();
  assert.equal(await checked, false);
  assert.ok(s.calls.lastIndexOf("measureTuneStop") > s.calls.indexOf("measureTuneSumCheck"), s.calls.join(" "));
  assert.equal(s.mics(), 0, "the microphone closed");
  await t.enter();
  st = tstate("applied");
  t.state = st;
  s.calls.length = 0;
  const verified = t.verify();
  await new Promise((r) => setTimeout(r, 5));
  await t.leave();
  release!();
  assert.equal(await verified, false);
  assert.ok(s.calls.lastIndexOf("measureTuneStop") > s.calls.indexOf("measureTuneVerify"), s.calls.join(" "));
  assert.equal(s.mics(), 0);
});

// ---- review P5-E ----

test("review P5-E: the tune's own measurement takes the person's placement (headphone levels on a coupler) and the core's gap", () => {
  const speakers = layoutSetup(LAYOUT, 1500, "", "x", { placement: "listeningPosition" });
  assert.equal(speakers.placement, "listeningPosition");
  assert.equal(speakers.gapMs, undefined, "the gap is the core's for the placement, not a short one of the screen's");
  assert.equal(speakers.passes, 3);
  assert.equal(speakers.coupler, null);
  // The Setup tab's placement decides until the person chooses on the tune tab; WP P5FOLLOW:
  // none is assumed (no Setup choice, or one that is not the tune's: nothing).
  assert.equal(tunePlaceOf(null), null);
  assert.equal(tunePlaceOf({ placement: null }), null);
  assert.equal(tunePlaceOf({ placement: "multiPosition", count: 5 }), null);
  assert.deepEqual(tunePlaceOf({ placement: "listeningPosition" }), { placement: "listeningPosition" });
  const place = tunePlaceOf({ placement: "headphones", coupler: "flatPlate", count: 2 });
  assert.deepEqual(place, { placement: "headphones", coupler: "flatPlate", count: 2 });
  const phones = layoutSetup(LAYOUT, 1500, "", "x", place!);
  assert.equal(phones.placement, "headphones", "the core then starts at -60 dBFS and stops at -30 dBFS (S3)");
  assert.equal(phones.coupler, "flatPlate");
  assert.equal(phones.count, 2);
  assert.equal(phones.gapMs, undefined);
  const s = standIn({});
  (s.host as unknown as { draft: unknown }).draft = { micDocument: "", placement: "headphones", coupler: "hats", count: 3 };
  const t = new TuneDesk(s.host);
  assert.deepEqual(t.place, { placement: "headphones", coupler: "hats", count: 3 });
  t.setHeadphones(false);
  assert.deepEqual(t.place, { placement: "listeningPosition" });
  t.setHeadphones(true);
  assert.deepEqual(t.place, { placement: "headphones", coupler: "hats", count: 3 });
});

test("review P5-E: a guard that was lost or whose revert failed is not said to be reverted", () => {
  assert.match(stageWords(tstate("reverted", { guard: { ...GUARD, phase: "reverted" } })), /^Reverted: the settings from before are back/);
  const lost = stageWords(tstate("reverted", { guard: { ...GUARD, phase: "lost", message: "The connection went." } }));
  assert.match(lost, /may still hold the tune/);
  assert.doesNotMatch(lost, /are back/);
  const failed = stageWords(tstate("reverted", { guard: { ...GUARD, phase: "revert_failed" } }));
  assert.match(failed, /revert failed/);
  assert.doesNotMatch(failed, /are back/);
});

test("review P5-E: a no-verdict question the core answers with its refusal becomes the core's question, with its own second", async () => {
  let clock = 5_000_000;
  // A verify failed, then "Verify again" was stopped: applied, no verdict shown, the core still holds the failure.
  let st = tstate("applied");
  let asked = 0;
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => PROPOSAL,
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneKeep: () => {
      asked++;
      if (asked === 1) unverifiedRefusal();
      st = tstate("kept");
      return { guard: { ...GUARD, phase: "kept" }, snapshotBeforeId: "", snapshotBefore: "", snapshotAfterId: "", snapshotAfter: "", measurementId: "", measurement: "", notes: [] };
    },
  });
  const t = new TuneDesk(s.host, () => clock);
  await t.enter();
  assert.equal(await t.keep(), false);
  assert.equal(t.question?.why, "noVerdict");
  assert.equal(asked, 0);
  clock += CONFIRM_MS;
  assert.equal(await t.confirmKeep(), false, "the core refuses: the verify had failed");
  assert.equal(asked, 1);
  assert.equal(t.question?.why, "unverified", "the question now says what the core said");
  assert.match(t.question!.text, /^The verify did not confirm the tune/);
  assert.equal(t.confirmReady(), false, "and its second counts from the refusal");
  assert.equal(await t.confirmKeep(), false);
  assert.equal(asked, 1, "a press at once sends nothing");
  clock += CONFIRM_MS;
  assert.equal(await t.confirmKeep(), true);
  assert.equal(asked, 2);
  await t.leave();
});

test("review P5-E: a summed check or a verify whose tab was left while the microphone was asked for is not started at all", async () => {
  let st = tstate("proposed");
  let grant: (() => void) | null = null;
  const s = standIn({
    measureTuneState: () => st,
    measureTuneProposal: () => ({ ...PROPOSAL, sumCheck: "pending" }),
    abState: () => ({ active: false }),
    measureLayoutProposal: () => LAYOUT,
    measureTuneSumCheck: () => (st = tstate("sumCheck", { running: true })),
    measureTuneVerify: () => (st = tstate("verifying", { running: true })),
    measureTuneStop: () => st,
  });
  let open = 0;
  const host = s.host as unknown as { openMic: () => Promise<number>; closeMic: () => Promise<void> };
  host.openMic = () =>
    new Promise<number>((r) => {
      grant = () => {
        open++;
        r(48000);
      };
    });
  host.closeMic = async () => {
    open--;
  };
  const t = new TuneDesk(s.host);
  await t.enter();
  const checked = t.sumCheck();
  await new Promise((r) => setTimeout(r, 5));
  await t.leave();
  grant!();
  assert.equal(await checked, false);
  assert.equal(s.calls.includes("measureTuneSumCheck"), false, s.calls.join(" "));
  assert.equal(open, 0, "the microphone closed again");
  await t.enter();
  st = tstate("applied");
  t.state = st;
  const verified = t.verify();
  await new Promise((r) => setTimeout(r, 5));
  await t.leave();
  grant!();
  assert.equal(await verified, false);
  assert.equal(s.calls.includes("measureTuneVerify"), false, s.calls.join(" "));
  assert.equal(open, 0);
});

test("P5WAT: the documents desk: Control cannot delete a hub document (nothing sent); Admin can; a hub rename goes to the hub", async () => {
  const s = standIn({
    measureDocsMerged: () => ({ entries: [{ doc: { id: "m-1", kind: "measurement", name: "Room", note: "", serial: "S", savedMs: 1, bytes: 10 }, where: "hub", localCopy: false }], hubListed: true, hubNoticeKey: "" }),
    measureDocInfo: (p) => ({ id: "", kind: "measurement", name: (JSON.parse((p as { text: string }).text) as { meta: { name: string } }).meta.name, note: "", serial: "", savedMs: 2, bytes: 5 }),
    hubMeasureDocDelete: () => null,
    hubMeasureDocRename: (p) => ({ id: "m-1", kind: "measurement", name: (p as { name: string }).name, note: "", serial: "S", savedMs: 1, bytes: 10 }),
  });
  (s.host as { result: string | null }).result = JSON.stringify({ meta: { kind: "measurement", name: "Mine" }, measurement: {} });
  const d = new DocsDesk(s.host);
  await d.refresh();
  assert.deepEqual(
    d.rows.map((r) => [r.where, r.info.name]),
    [
      ["hub", "Room"],
      ["local", "Mine"],
    ],
  );
  const hubRow = d.rows[0]!;
  assert.equal(await d.remove(hubRow), false);
  assert.match(d.error, /needs the hub's Admin role/);
  assert.equal(s.calls.includes("hubMeasureDocDelete"), false, "nothing sent");
  assert.equal(await d.rename(hubRow, "Living room"), true);
  assert.equal(d.rows[0]!.info.name, "Living room");
  // This page's document: renamed in its own text, deleted from the list.
  const mine = d.rows[1]!;
  assert.equal(await d.rename(mine, "Mine, renamed"), true);
  assert.match(d.rows[1]!.text!, /Mine, renamed/);
  assert.equal(await d.remove(d.rows[1]!), true);
  assert.equal(d.rows.length, 1);
  (s.host as { hubRole: string }).hubRole = "admin";
  assert.equal(await d.remove(d.rows[0]!), true);
  assert.ok(s.calls.includes("hubMeasureDocDelete"));
});

// ------------------------------------------------------------------ on the real core (demo device)

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: { dispose(): void }[] = [];
after(() => {
  for (const c of made) c.dispose();
});

/** A session on the real core whose client does what the Worker does (worker.ts): the route's turns. */
async function sessionOn() {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  const listeners = new Set<(e: CoreEvent) => void>();
  core.onEvent((e) => listeners.forEach((l) => l(e)));
  const route = new MeasureWorkerRoute(
    (m, p) => core.call(m as never, p as never),
    (e) => listeners.forEach((l) => l(e as unknown as CoreEvent)),
    { MessageChannel, workletUrl: "capture-worklet.js", setTimeout, clearTimeout },
  );
  made.push({ dispose: () => (route.after("close", true, null), core.dispose()) });
  const client = {
    call: async (m: string, p: unknown) => {
      if (route.handles(m)) return route.answer(m).value;
      try {
        const v = await core.call(m as never, p as never);
        route.after(m, true, v);
        return v;
      } catch (e) {
        route.after(m, false, null);
        throw e;
      }
    },
    subscribe: (fn: (e: CoreEvent) => void) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  return new Session("s", "i", "Here", { kind: "demo" }, "control", client);
}

const until = async (what: () => boolean | Promise<boolean>, ms = 120_000) => {
  const end = Date.now() + ms;
  while (!(await what())) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 25));
  }
};

/** The demo device as the stereo three-way of P5SIM's room (the 3-way template), the volume below S2's limit. */
export async function threeWay(session: Session): Promise<void> {
  const text = readFileSync(join(process.cwd(), "../../assets/templates/3way-rp2350.dspipreset"), "utf8");
  const token = await session.token("config_import");
  await session.core.call("configImport", { text, ...(token ? { token } : {}) });
  await session.core.call("write", { path: "vol.user", value: { float: -20 } } as never);
}

/** What a tune may change: the outputs' EQ, crossovers, delays and gains, the matrix. */
const CHAIN = ["filters", "xover", "outputDelayMs", "outputGainDb", "outputMuted", "matrixGain", "matrixInvert"];
const chain = async (session: Session) => {
  const st = (await session.core.call("state", undefined)).state as unknown as Record<string, unknown>;
  return JSON.stringify(CHAIN.map((k) => st[k]));
};

test("P5WAT: on the demo's three-way room: measured, proposed, summed-checked, applied, verified by itself and reverted; again and kept, A/B; unreliable timing withdrawn", { skip, timeout: 900_000 }, async () => {
  const session = await sessionOn();
  await session.open({ kind: "demo", blank: true }); // -- WP UB12 -- a new board
  await threeWay(session);
  const desk = new MeasureDesk(session);
  await desk.start();
  assert.deepEqual(desk.mic, { kind: "sim" });
  const t = new TuneDesk(desk);
  await t.enter();
  const l = t.layout!;
  console.log("P5WAT layout:", l.template, l.outputs.map((o) => `${o.output}:${o.role}/${o.speaker}/${o.group}`).join(" "));
  assert.equal(l.template, "3way");
  // The left speaker only: three ways, two transitions.
  for (const o of l.outputs) if (o.speaker !== l.outputs[0]!.speaker) t.setOn(o.output, false);
  const on = chosenLayout(l, t.off);
  assert.equal(on.outputs.length, 3);
  assert.equal(await desk.startRun(layoutSetup(on, 1000, "", "P5WAT", { placement: "listeningPosition" })), true, `${desk.error} ${JSON.stringify(desk.preflight)}`);
  await until(() => !!desk.result || desk.state?.phase === "aborted" || desk.state?.phase === "failed", 300_000);
  assert.ok(desk.result, `measured: ${JSON.stringify(desk.state?.issue)}`);
  const measurement = desk.result!;
  const before = await chain(session);

  assert.equal(await t.propose(), true, t.error);
  const p = t.proposal!;
  console.log("P5WAT proposal:", p.sumCheck, p.writes.map((w) => w.words).join(" | "), "withdrawn:", p.withdrawn.map((w) => w.key).join(","));
  assert.ok(p.writes.length > 0, "a tune is proposed");
  assert.equal(await chain(session), before, "nothing written by a proposal");
  if (p.sumCheck === "pending") {
    assert.equal(await t.apply(), false, "no apply before the summed check");
    assert.match(t.error, /summed check/i);
    assert.equal(await t.sumCheck(), true, t.error);
    await until(() => t.stage === "proposed" && !t.running && t.proposal!.id !== p.id, 300_000);
    console.log("P5WAT after the summed check:", t.proposal!.sumCheck, t.proposal!.writes.length);
  }
  assert.notEqual(t.proposal!.sumCheck, "pending");
  const verified = async () => {
    assert.equal(await t.apply(), true, t.error);
    // The verify runs by itself; Keep only after its verdict.
    assert.equal(t.stage, "verifying");
    assert.equal(t.keepOffered, false);
    await until(() => t.stage === "verified" || t.stage === "applied", 400_000);
    console.log("P5WAT verify:", JSON.stringify(t.state!.verify));
    assert.equal(t.stage, "verified");
    assert.notEqual(await chain(session), before, "applied");
  };
  // Applied, verified, reverted: the settings from before are back.
  await verified();
  assert.equal(await t.revert(), true, t.error);
  assert.equal(t.stage, "reverted");
  assert.equal(await chain(session), before, "Revert puts the settings back");

  // Proposed again from the same measurement, applied, verified, kept, compared.
  t.useMeasurement(measurement, "the measurement");
  assert.equal(await t.propose(), true, t.error);
  if (t.proposal!.sumCheck === "pending") {
    assert.equal(await t.sumCheck(), true, t.error);
    await until(() => t.stage === "proposed" && !t.running && t.proposal!.sumCheck !== "pending", 300_000);
  }
  assert.ok(t.proposal!.writes.length, "the same tune again");
  await verified();
  const passed = t.state!.verify!.passed;
  await t.keep();
  if (!passed) {
    assert.ok(t.question, `the second question after a failed verify: ${t.error}`);
    assert.equal(t.question!.why, "unverified");
    assert.equal(t.stage, "verified", "the core refused the first Keep");
    await new Promise((r) => setTimeout(r, CONFIRM_MS + 50));
    assert.equal(await t.confirmKeep(), true, t.error);
  }
  assert.equal(t.stage, "kept", t.error);
  assert.ok(t.kept!.snapshotBefore && t.kept!.snapshotAfter, "both snapshots");
  const tuned = await chain(session);
  await t.abStart();
  assert.equal(t.ab?.active, true, t.error);
  await t.abToggle();
  assert.equal(t.ab?.side, "b");
  assert.equal(await chain(session), before, "B: before the tune");
  await t.abToggle();
  assert.equal(t.ab?.side, "a");
  await t.abEnd("a");
  assert.equal(await chain(session), tuned, "the compare ended on the tune");

  // A measurement marked timing unreliable: its timing writes withdrawn, with the reason.
  const doc = JSON.parse(measurement) as { measurement: { capture: { timingReliable: boolean } } };
  doc.measurement.capture.timingReliable = false;
  t.useMeasurement(JSON.stringify(doc), "an unreliable copy");
  assert.equal(await t.propose(), true, t.error);
  assert.equal(t.proposal!.timingReliable, false);
  assert.ok(t.proposal!.withdrawn.some((w) => w.key === "measure.tune.timing_unreliable"), JSON.stringify(t.proposal!.withdrawn));
  assert.ok(t.proposal!.writes.every((w) => !w.timing), "no timing write");
  assert.equal((await session.core.call("siggenStatus", undefined)).state, 0);
  await t.leave();
  await desk.stop();
});
