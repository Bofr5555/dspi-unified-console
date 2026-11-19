// WP P5WMS unit tests: the measurement screens' rules (the draft's setup, the words of a state,
// the stored curve decoding of docs/integrator/measurement.md 7.2, the results' reading, the
// graph's scales), the Worker's capture port and ticks (MeasureWorkerRoute), and on the real core
// (the demo device with P5DEMO's simulated microphone, as the screen drives it through a stand-in
// Worker): a measurement end to end with its document, the live view's "cut this peak" applied
// and undone, Stop stopping the generator, and a viewer refused. No audio: the demo only.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { CAPTURE_OPEN, MeasureWorkerRoute, TICK_MS, isMeasureEvent, measureCall, type MeasureEvent, type MeasureStateRecord } from "../../src/core/measure-protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import type { CoreEvent } from "../../src/core/protocol";
import { Session } from "../../src/ui/session";
import { MeasureDesk } from "../../src/screens/measure/MeasureScreen";
import {
  decodeCurve,
  draftProblem,
  fileNameOf,
  freqOf,
  gridFreqs,
  highestPoint,
  initialDraft,
  issueText,
  micSummary,
  phaseText,
  playBlocker,
  preflightLines,
  progressPct,
  resultView,
  segments,
  setupOf,
  timingWords,
  withOutputs,
  xOf,
  yRange,
} from "../../src/screens/measure/model";

// ------------------------------------------------------------------ the rules

const STATE: MeasureStateRecord = {
  phase: "ramp",
  kind: "walked",
  active: true,
  wantsCapture: true,
  captureRateHz: 48000,
  position: 1,
  positions: 1,
  run: 1,
  runs: 1,
  output: 1,
  levelDb: -44,
  rampLevels: [],
  frames: 0,
  generatorStarts: 1,
  progress: 0.25,
  resultReady: false,
  frameFaults: 0,
};
const name = (o: number) => `OUT${o + 1}`;

test("P5WMS: a draft's setup is a walked run of the chosen outputs; its problems in words", () => {
  const d = initialDraft([0, 1, 2]);
  assert.deepEqual(
    d.outputs.map((o) => o.on),
    [true, true, false],
  );
  const s = setupOf({ ...d, name: "  Room  " }, 44100);
  assert.equal(s.kind, "walked");
  assert.deepEqual(
    s.outputs.map((o) => [o.output, o.role, o.speaker]),
    [
      [0, "full", "L"],
      [1, "full", "L"],
    ],
  );
  assert.equal(s.captureRateHz, 44100);
  assert.equal(s.name, "Room");
  assert.equal(s.count, 0, "one position: no count");
  assert.equal(s.coupler, null);
  const hp = setupOf({ ...d, placement: "headphones", count: 2, coupler: "flatPlate" }, 48000);
  assert.deepEqual([hp.count, hp.coupler], [2, "flatPlate"]);
  assert.equal(draftProblem(d), "");
  assert.match(draftProblem({ ...d, outputs: d.outputs.map((o) => ({ ...o, on: false })) }), /at least one output/);
  assert.match(draftProblem({ ...d, placement: "multiPosition", count: 2 }), /Positions: 3 to 9/);
  assert.match(draftProblem({ ...d, sweepMs: 500 }), /1 to 15 s/);
  assert.match(draftProblem({ ...d, passes: 1 }), /Passes/);
  // Outputs that come and go keep their choices.
  const kept = withOutputs({ ...d, outputs: d.outputs.map((o) => (o.output === 1 ? { ...o, role: "high" } : o)) }, [1, 3]);
  assert.deepEqual(
    kept.outputs.map((o) => [o.output, o.role, o.on]),
    [
      [1, "high", true],
      [3, "full", false],
    ],
  );
  assert.equal(withOutputs(d, [0, 1, 2]), d, "unchanged: the same draft");
});

test("P5WMS: a state, a preflight and the timing in words; 'timing unreliable' says what it withholds", () => {
  assert.match(phaseText(STATE, name), /Finding the level of OUT2 at -44 dBFS/);
  assert.match(phaseText({ ...STATE, phase: "waitPosition", position: 2, positions: 3 }, name), /position 2 of 3, then choose Next position/);
  assert.match(phaseText({ ...STATE, phase: "aborted" }, name), /nothing was kept/);
  assert.equal(progressPct(STATE), 25);
  assert.equal(progressPct({ ...STATE, progress: Number.NaN }), 0);
  const title = (k: string) => (k === "measure.volume_high" ? "Volume too high to measure" : null);
  assert.equal(issueText({ key: "measure.volume_high", message: "the volume is 0.0 dB" }, title, name), "Volume too high to measure. The volume is 0.0 dB");
  assert.equal(issueText({ key: "x.y", output: 2, message: "no high-pass" }, title, name), "OUT3: No high-pass");
  const lines = preflightLines({ clear: false, blockers: [{ key: "measure.volume_high", message: "lower it" }], warnings: [{ key: "measure.uncalibrated", message: "relative" }] }, title, name);
  assert.deepEqual(
    lines.map((l) => [l.kind, l.key]),
    [
      ["blocker", "measure.volume_high"],
      ["warning", "measure.uncalibrated"],
    ],
  );
  const t = { reliable: true, ratioPpm: 37.5, ratioFitted: true, maxResidualUs: 1.25, residualBoundUs: 10, maxPhaseSpreadDeg: 0.2, passes: 3, reasons: [], cycleOutputs: [], cycleResidualsUs: [] };
  assert.match(timingWords(t)!.text, /^Timing reliable: largest cycle residual 1.3 µs \(bound 10 µs\)/);
  const bad = timingWords({ ...t, reliable: false, reasons: ["cycle 3 is 14 µs off the fit"] })!;
  assert.equal(bad.reliable, false);
  assert.match(bad.text, /^Timing unreliable: .*no delay, polarity or crossover change.*cycle 3 is 14 µs off the fit\.$/);
  assert.equal(timingWords(null), null);
});

test("P5WMS: a stored curve decodes as section 7.2's example; a bad one is null", () => {
  const c = decodeCurve({
    grid: { minHz: 100, maxHz: 1000, pointsPerOctave: 3 },
    mag: "QB88IFEgah9dHiAe7B4FIGkgvB8=",
    phase: "AAAAAJj+//8w/f//yPv//2D6///4+P//kPf//yj2///A9P//WPP//w==",
    valid: "/gM=",
    spl: true,
  })!;
  assert.equal(c.freqsHz.length, 10);
  assert.ok(Math.abs(c.freqsHz[3]! - 200) < 1e-9);
  assert.deepEqual(c.magDb.slice(0, 3).map((v) => Math.round(v * 100) / 100), [80, 82.52, 82.73]);
  assert.deepEqual(c.phaseDeg!.slice(0, 3).map((v) => Math.round(v * 10) / 10), [0, -36, -72]);
  assert.deepEqual(c.valid, [false, true, true, true, true, true, true, true, true, true]);
  assert.equal(c.spl, true);
  assert.equal(gridFreqs({ minHz: 10, maxHz: 22000, pointsPerOctave: 96 }).length, 1066, "the standard grid");
  assert.deepEqual(gridFreqs({ minHz: 10, maxHz: 5, pointsPerOctave: 96 }), []);
  assert.deepEqual(gridFreqs({ minHz: 10, maxHz: 22000, pointsPerOctave: 0 }), []);
  assert.equal(decodeCurve({ grid: { minHz: 100, maxHz: 1000, pointsPerOctave: 3 }, mag: "QB88" }), null, "a wrong length");
  assert.equal(decodeCurve({ grid: { minHz: 100, maxHz: 1000, pointsPerOctave: 3 }, mag: "not base64!" }), null);
  assert.equal(decodeCurve({ grid: { minHz: 100, maxHz: 1000, pointsPerOctave: 3 }, mag: "QB88IFEgah9dHiAe7B4FIGkgvB8=", valid: "/w==" }), null, "a short mask");
  assert.equal(decodeCurve(null), null);
  // No mask: every point trusted.
  assert.ok(decodeCurve({ grid: { minHz: 100, maxHz: 1000, pointsPerOctave: 3 }, mag: "QB88IFEgah9dHiAe7B4FIGkgvB8=" })!.valid.every((v) => v));
});

test("P5WMS: the graph's scales; segments split where the trust changes; the highest trusted peak", () => {
  assert.ok(Math.abs(xOf(20, 100)) < 1e-9);
  assert.ok(Math.abs(xOf(20000, 100) - 100) < 1e-9);
  assert.ok(Math.abs(freqOf(50, 100) - Math.sqrt(20 * 20000)) < 1e-6);
  assert.equal(freqOf(-5, 100), 20);
  const c = { freqsHz: [10, 100, 200, 400, 800, 30000], magDb: [0, -3, 9, 2, -1, 50], valid: [true, true, false, true, true, true], spl: false };
  const segs = segments(c);
  assert.deepEqual(
    segs.map((s) => [s.trusted, s.points.length]),
    [
      [true, 1],
      [false, 2],
      [true, 3],
    ],
  );
  const [lo, hi] = yRange([c]);
  assert.ok(lo <= -3 && hi >= 2 && hi - lo >= 36 && lo % 6 === 0);
  assert.deepEqual(yRange([null]), [-30, 6]);
  assert.equal(highestPoint(c), 400, "the untrusted 9 dB at 200 Hz is not a peak");
  assert.equal(highestPoint(null), null);
  assert.equal(fileNameOf("Room: left/right", "measurement"), "Room- left-right.dspipreset");
  assert.equal(fileNameOf("", "measurement"), "measurement.dspipreset");
});

test("P5WMS: a viewer and a read-only connection may not play; documents read for the screens", () => {
  assert.equal(playBlocker("", "control"), "");
  assert.match(playBlocker("", "viewer"), /Viewer role: measuring needs Control.*passive microphone RTA/);
  assert.match(playBlocker("View only", ""), /^View only: measuring plays the DSPi's generator/);
  const doc = {
    schemaVersion: 1,
    meta: { kind: "measurement", name: "Living room" },
    measurement: {
      serial: "S1",
      placement: { mode: "multiPosition", count: 5 },
      positions: 5,
      stimulus: { kind: "walked", levelDb: -24 },
      capture: { timingReliable: false, reasons: ["clock wander"] },
      outputs: [{ output: 2, role: "high", speaker: "L", response: { grid: { minHz: 100, maxHz: 1000, pointsPerOctave: 3 }, mag: "QB88IFEgah9dHiAe7B4FIGkgvB8=" } }],
    },
  };
  const v = resultView(JSON.stringify(doc))!;
  assert.equal(v.placement, "Several positions: 5");
  assert.equal(v.timingReliable, false);
  assert.deepEqual(v.reasons, ["clock wander"]);
  assert.equal(v.outputs[0]!.role, "High (tweeter)");
  assert.equal(v.outputs[0]!.curve!.magDb.length, 10);
  assert.equal(v.mic, "Uncalibrated (relative dB)");
  assert.equal(resultView(JSON.stringify({ ...doc, meta: { kind: "mic" } })), null);
  assert.equal(resultView("{"), null);
  const live = resultView(JSON.stringify({ ...doc, measurement: { ...doc.measurement, stimulus: { kind: "liveSweep" } } }))!;
  assert.equal(live.timingReliable, null, "no timing from a single sweep");
  assert.equal(micSummary(JSON.stringify({ meta: { kind: "mic" }, mic: { name: "UMIK-1", sensitivityDb: -18.3, approximate: true, angle: 90 } })), "UMIK-1, sensitivity -18.3 dB, 90° calibration, dB SPL approximate (no calibrator step)");
  assert.equal(micSummary("{}"), "");
});

// ------------------------------------------------------------------ the Worker's part

/** Timers by hand, a fake core and the events posted. */
function routeRig(states: MeasureStateRecord[]) {
  const calls: { method: string; params: unknown }[] = [];
  const posted: MeasureEvent[] = [];
  let pending: (() => void) | null = null;
  const route = new MeasureWorkerRoute(
    async (method, params) => {
      calls.push({ method, params });
      if (method === "measureTick") return states.shift() ?? { ...STATE, active: false, phase: "done" };
      return null;
    },
    (e) => posted.push(e),
    {
      MessageChannel,
      workletUrl: "http://x/capture-worklet.js",
      setTimeout: ((fn: () => void, ms: number) => {
        assert.equal(ms, TICK_MS);
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

test("P5WMS: the Worker's capture port pushes each block, in order, into measureCapturePush; a close ends it", async () => {
  const rig = routeRig([]);
  assert.equal(rig.route.handles(CAPTURE_OPEN), true);
  assert.equal(rig.route.handles("measureStart"), false);
  const a = rig.route.answer(CAPTURE_OPEN);
  assert.ok(a.value && a.value.port instanceof MessagePort);
  assert.equal(a.value!.workletUrl, "http://x/capture-worklet.js");
  assert.deepEqual(a.transfer, [a.value!.port]);
  const port = a.value!.port;
  port.postMessage({ kind: "block", firstFrame: 0, samples: new Float32Array([0.5, -0.25]), rateHz: 48000, clipped: false });
  port.postMessage({ kind: "nonsense" });
  port.postMessage({ kind: "block", firstFrame: 2, samples: new Float32Array([0.125]), rateHz: 48000, clipped: true });
  for (let i = 0; i < 50 && rig.calls.length < 2; i++) await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(rig.calls, [
    { method: "measureCapturePush", params: { firstFrame: 0, samples: [0.5, -0.25], rateHz: 48000, clipped: false } },
    { method: "measureCapturePush", params: { firstFrame: 2, samples: [0.125], rateHz: 48000, clipped: true } },
  ]);
  assert.equal(rig.route.pushed, 2);
  // Closed: later blocks go nowhere.
  rig.route.answer("measureCaptureClose");
  port.postMessage({ kind: "block", firstFrame: 3, samples: new Float32Array([0]), rateHz: 48000, clipped: false });
  await new Promise((r) => setTimeout(r, 20));
  assert.equal(rig.calls.length, 2);
  port.close();
});

test("P5WMS: after measureStart the Worker turns the session and pushes each state until it is done; Stop ends the turns", async () => {
  const rig = routeRig([{ ...STATE }, { ...STATE, phase: "run" }]);
  rig.route.after("measureStart", false, null);
  assert.equal(rig.route.ticking, false, "a refused start: no turns");
  rig.route.after("measureStart", true, { ...STATE });
  assert.equal(rig.route.ticking, true);
  await rig.fire();
  await rig.fire();
  await rig.fire();
  assert.deepEqual(
    rig.posted.map((e) => (e.event === "measure" ? e.state.phase : "live")),
    ["ramp", "run", "done"],
  );
  assert.equal(rig.route.ticking, false, "done: no more turns");
  assert.equal(rig.due, false);
  assert.ok(rig.posted.every(isMeasureEvent));
  assert.equal(isMeasureEvent({ event: "state" }), false);
  // A Stop while it runs ends the turns at once.
  const r2 = routeRig([{ ...STATE }]);
  r2.route.after("measureStart", true, { ...STATE });
  r2.route.after("measureStop", true, { ...STATE, active: false });
  assert.equal(r2.route.ticking, false);
  await r2.fire();
  assert.equal(r2.posted.length, 0);
  // The live view's start turns `measureLiveTick`.
  const r3 = routeRig([]);
  r3.route.after("measureLiveStart", true, { active: true });
  await r3.fire();
  assert.deepEqual(
    r3.calls.map((c) => c.method),
    ["measureLiveTick"],
  );
});

test("P5WMS: a passive capture port pushes its blocks into measureRtaPassivePush, never into the session", async () => {
  const rig = routeRig([]);
  const a = rig.route.answer(CAPTURE_OPEN, { to: "passive" });
  a.value!.port.postMessage({ kind: "block", firstFrame: 0, samples: new Float32Array([0.5, -0.25]), rateHz: 48000, clipped: false });
  for (let i = 0; i < 50 && rig.calls.length < 1; i++) await new Promise((r) => setTimeout(r, 5));
  assert.deepEqual(rig.calls, [{ method: "measureRtaPassivePush", params: { samples: [0.5, -0.25], rateHz: 48000 } }]);
  // A port asked for ahead does not close the one in use.
  const b = rig.route.answer(CAPTURE_OPEN, { to: "measure" });
  a.value!.port.postMessage({ kind: "block", firstFrame: 2, samples: new Float32Array([0.125]), rateHz: 48000, clipped: false });
  for (let i = 0; i < 50 && rig.calls.length < 2; i++) await new Promise((r) => setTimeout(r, 5));
  assert.equal(rig.calls[1]?.method, "measureRtaPassivePush");
  rig.route.answer("measureCaptureClose");
  a.value!.port.close();
  b.value!.port.close();
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
async function sessionOn(access: "control" | "view") {
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
  return new Session("s", "i", "Here", { kind: "demo" }, access, client);
}

const until = async (what: () => boolean | Promise<boolean>, ms = 60_000) => {
  const end = Date.now() + ms;
  while (!(await what())) {
    if (Date.now() > end) throw new Error("timed out");
    await new Promise((r) => setTimeout(r, 25));
  }
};

test("P5WMS: on the demo with the simulated microphone: preflight in words, a measurement end to end, its document", { skip, timeout: 180_000 }, async () => {
  const session = await sessionOn("control");
  await session.open({ kind: "demo" });
  const desk = new MeasureDesk(session);
  await desk.start();
  assert.deepEqual(desk.mic, { kind: "sim" }, "the demo offers its simulated microphone first");
  assert.equal(desk.sim?.label, "Simulated microphone");
  const d = { ...initialDraft([0, 1]), sweepMs: 1000 };
  const setup = setupOf(d, 48000);
  // The demo's volume is above what a measurement allows (S2): the preflight says so.
  const p = await desk.runPreflight(setup);
  assert.equal(p!.clear, false);
  assert.deepEqual(
    p!.blockers.map((b) => b.key),
    ["measure.volume_high"],
  );
  await session.core.call("write", { path: "vol.user", value: { float: -20 } } as never);
  assert.equal((await desk.runPreflight(setup))!.clear, true);
  assert.equal(await desk.startRun({ ...setup, gapMs: 300 }), true);
  assert.equal(desk.running, true);
  await until(() => !!desk.result);
  assert.equal(desk.state!.phase, "done");
  assert.equal(desk.running, false);
  const v = resultView(desk.result!)!;
  assert.equal(v.timingReliable, true);
  assert.deepEqual(
    v.outputs.map((o) => o.output),
    [0, 1],
  );
  for (const o of v.outputs) {
    assert.equal(o.curve!.freqsHz.length, 1066);
    assert.ok(o.curve!.valid.filter((x) => x).length > 300, "trusted points");
    assert.ok(o.curve!.valid.some((x) => !x), "and untrusted ones (drawn faint)");
  }
  // The results' curves are the core's: smoothed, each with its levelled target, on the standard grid.
  const curves = await measureCall(session.core, "measureDocCurves", { text: desk.result!, options: { smoothing: 6, target: true } });
  assert.equal(curves.smoothing, 6);
  assert.equal(curves.timingReliable, true);
  assert.deepEqual(
    curves.outputs.map((o) => [o.output, o.measured.freqsHz.length, o.target?.freqsHz.length]),
    [
      [0, 1066, 1066],
      [1, 1066, 1066],
    ],
  );
  const plain = await measureCall(session.core, "measureDocCurves", { text: desk.result!, options: { smoothing: 0 } });
  assert.equal(plain.outputs[0]!.target ?? null, null, "no target unless asked");
  assert.notDeepEqual(plain.outputs[0]!.measured.magDb, curves.outputs[0]!.measured.magDb, "smoothing changes the curve");
  // REW text of an output, from the document.
  const rew = await measureCall(session.core, "measurementExportRew", { text: desk.result!, output: 1 });
  assert.match(rew.text, /\d/);
  assert.equal((await session.core.call("siggenStatus", undefined)).state, 0);
  await desk.stop();
});

test("P5WMS: the live view: a curve per sweep, 'cut this peak' previewed, applied as one undo step, Stop stops the generator", { skip, timeout: 180_000 }, async () => {
  const session = await sessionOn("control");
  await session.open({ kind: "demo" });
  await session.core.call("write", { path: "vol.user", value: { float: -20 } } as never);
  const desk = new MeasureDesk(session);
  await desk.start();
  assert.equal(await desk.startLive({ output: 0, sweepMs: 1000, gapMs: 300, smoothing: 6 }), true);
  await until(() => (desk.live?.seq ?? 0) >= 2);
  const frame = await measureCall(session.core, "measureLiveFrame", undefined);
  assert.ok(frame.measured && frame.target, "measured and target");
  const peak = highestPoint(frame.measured)!;
  const prop = await measureCall(session.core, "measureLiveCutPeak", { output: 0, freqHz: peak });
  assert.equal(prop.changes.length, 1);
  assert.ok(prop.predicted, "the predicted curve of the change");
  const band = prop.changes[0]!.band;
  const peqOf = async () => JSON.stringify((await session.core.call("state", undefined)).state.filters);
  const before = await peqOf();
  await measureCall(session.core, "measureLiveApply", { proposalId: prop.id });
  assert.notEqual(await peqOf(), before, "written");
  await session.core.call("undo", undefined);
  assert.equal(await peqOf(), before, `one undo puts band ${band + 1} back`);
  await desk.stopAll();
  assert.equal(desk.live!.active, false);
  assert.equal((await session.core.call("siggenStatus", undefined)).state, 0);
  assert.equal(desk.running, false);
  await desk.stop();
});

test("P5WMS: a view-only session is refused a run with nothing played", { skip, timeout: 60_000 }, async () => {
  const session = await sessionOn("view");
  await session.open({ kind: "demo" });
  const desk = new MeasureDesk(session);
  await desk.start();
  assert.match(playBlocker(session.readOnlyReason, desk.hubRole), /only the passive microphone RTA/);
  assert.equal(await desk.startRun(setupOf(initialDraft([0]), 48000)), false);
  // Refused before anything plays: by the preflight's blocker, or the call's refusal.
  assert.ok(desk.error !== "" || (desk.preflight && !desk.preflight.clear), `${desk.error} ${JSON.stringify(desk.preflight)}`);
  if (desk.preflight) console.log("view-only preflight:", desk.preflight.blockers.map((b) => b.key).join(", "));
  assert.equal((await session.core.call("siggenStatus", undefined)).state, 0);
  // The passive RTA: the core's, from pushed blocks, with nothing sent to the DSPi.
  assert.equal(desk.passiveBlocker, "");
  const st = await measureCall(session.core, "measureRtaPassiveStart", { setup: { captureRateHz: 48000, averaging: "fast" } });
  assert.equal(st.active, true);
  let x = 1;
  const noise = () => Array.from({ length: 4800 }, () => ((x = (x * 1103515245 + 12345) % 2147483648) / 2147483648 - 0.5) * 0.1);
  for (let i = 0; i < 10; i++) await measureCall(session.core, "measureRtaPassivePush", { samples: noise(), rateHz: 48000 });
  const f = await measureCall(session.core, "measureRtaPassiveFrame", undefined);
  assert.ok(f.state.frames > 0, "frames averaged");
  assert.ok(f.rta?.current && f.rta.current.freqsHz.length > 10, "a spectrum to draw");
  await measureCall(session.core, "measureRtaPassiveHold", { on: true });
  assert.ok((await measureCall(session.core, "measureRtaPassiveFrame", undefined)).rta?.held, "held");
  assert.equal((await measureCall(session.core, "measureRtaPassiveStop", undefined)).active, false);
  assert.equal((await session.core.call("siggenStatus", undefined)).state, 0);
  await desk.stop();
});

test("P5WMS: the passive RTA is not offered while a run plays the generator", () => {
  const desk = new MeasureDesk({ core: {}, readOnlyReason: "" } as unknown as Session);
  assert.equal(desk.passiveBlocker, "");
  desk.state = { ...STATE, active: true };
  assert.match(desk.passiveBlocker, /would show the test signal/);
  desk.state = null;
  desk.live = { active: true, mode: "sweep", output: 0, seq: 0, runs: 0, session: STATE };
  assert.notEqual(desk.passiveBlocker, "");
});
