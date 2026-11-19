// WP P5FOLLOW unit tests: no placement is assumed for a measurement (the Setup tab's draft and the
// tune tab's own measurement wait for the person's choice, said in words), the 2-pass hint, and
// the global guard bar adopting a guard the auto-tune started (its countdown driven by the bar's
// own poll, Keep the core's gated applyConfirm). No core, no audio: scripted stand-ins.

import { test } from "node:test";
import assert from "node:assert/strict";
import type { GuardedApplyRecord } from "../../src/core/protocol";
import type { Session } from "../../src/ui/session";
import { SnapshotDesk } from "../../src/screens/snapshots/model";
import { LIVE_PLACEMENT_FIRST, PLACEMENT_FIRST, TWO_PASSES_HINT, draftProblem, initialDraft, livePlaceOf, setupOf } from "../../src/screens/measure/model";
import { PLACE_FIRST, TuneDesk, type TuneHost } from "../../src/screens/measure/tune-model";

test("P5FOLLOW: the Setup tab assumes no placement: its problem says so in words, and no setup is made", () => {
  const d = initialDraft([0, 1]);
  assert.equal(d.placement, null);
  assert.equal(draftProblem(d), PLACEMENT_FIRST);
  assert.match(PLACEMENT_FIRST, /placement/);
  assert.match(PLACEMENT_FIRST, /headphones/i);
  assert.throws(() => setupOf(d, 48_000), /placement/);
  // Chosen: nothing in the way, and the setup is the person's.
  for (const placement of ["listeningPosition", "headphones"] as const) {
    const chosen = { ...d, placement };
    assert.equal(draftProblem(chosen), "", placement);
    assert.equal(setupOf(chosen, 48_000).placement, placement);
  }
  // The outputs' problem still comes first.
  assert.match(draftProblem({ ...d, outputs: d.outputs.map((o) => ({ ...o, on: false })) }), /at least one output/);
});

test("P5FOLLOW: the live view takes only a valid Setup choice, else none", () => {
  assert.equal(livePlaceOf(null), null);
  assert.equal(livePlaceOf(initialDraft([0])), null, "the Setup tab's draft starts with none");
  assert.equal(livePlaceOf({ placement: "multiPosition" }), null, "not a live placement");
  for (const p of ["listeningPosition", "nearField", "headphones"] as const) assert.equal(livePlaceOf({ placement: p }), p);
  assert.match(LIVE_PLACEMENT_FIRST, /^Choose the placement first/);
});

test("P5FOLLOW: the 2-pass hint's words (a hint, no minimum: two passes stay allowed)", () => {
  assert.equal(TWO_PASSES_HINT, "Two passes cannot judge timing; use three or more for auto-tune.");
  assert.equal(draftProblem({ ...initialDraft([0]), placement: "listeningPosition", passes: 2 }), "");
});

test("P5FOLLOW: the tune's own measurement waits for the person's placement, said in words", () => {
  const host = { draft: null } as unknown as TuneHost;
  const t = new TuneDesk(host);
  assert.equal(t.place, null, "no default");
  assert.equal(t.placeProblem, PLACE_FIRST);
  assert.match(PLACE_FIRST, /speakers at the listening position, or headphones on a coupler/);
  // A Setup choice that is not the tune's (several positions) is no choice here.
  (host as { draft: unknown }).draft = { micDocument: "", placement: "multiPosition", count: 5 };
  assert.equal(t.place, null);
  // The Setup tab's speakers, or the person's choice on the tab.
  (host as { draft: unknown }).draft = { micDocument: "", placement: "listeningPosition" };
  assert.deepEqual(t.place, { placement: "listeningPosition" });
  assert.equal(t.placeProblem, "");
  (host as { draft: unknown }).draft = null;
  t.setHeadphones(true);
  assert.deepEqual(t.place, { placement: "headphones", coupler: "iec60318_4", count: 1 });
  t.setHeadphones(false);
  assert.deepEqual(t.place, { placement: "listeningPosition" });
});

// ---- the guard bar ----

const guard = (id: number, phase: GuardedApplyRecord["phase"], remainingMs = 50_000): GuardedApplyRecord => ({
  id,
  phase,
  label: "Auto-tune",
  hub: false,
  timeoutMs: 60_000,
  remainingMs: phase === "waiting" ? remainingMs : 0,
  reason: phase === "kept" ? "confirmed" : "",
  message: phase === "kept" ? "Kept." : "",
  changes: [],
  notes: [],
});

/** A desk on a scripted core that pushes events, as the Worker's tune turns do. */
function scripted(answers: Record<string, () => unknown>) {
  const calls: string[] = [];
  const subs = new Set<(e: unknown) => void>();
  const session = {
    readOnlyReason: "",
    state: { status: "open" },
    core: {
      call: async (m: string) => {
        calls.push(m);
        const f = answers[m];
        if (!f) throw new Error(`not scripted: ${m}`);
        return f();
      },
      subscribe: (fn: (e: unknown) => void) => {
        subs.add(fn);
        return () => subs.delete(fn);
      },
    },
    refresh: async () => undefined,
    watch: () => () => undefined,
  };
  const desk = new SnapshotDesk(session as unknown as Session, 20);
  const push = (e: unknown) => {
    for (const s of subs) s(e);
  };
  return { desk, calls, push, subs };
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function until(what: string, f: () => boolean, ms = 2000): Promise<void> {
  const end = Date.now() + ms;
  while (!f()) {
    if (Date.now() > end) throw new Error(`timed out: ${what}`);
    await sleep(5);
  }
}

test("P5FOLLOW: the guard bar adopts the tune's guard from the Worker's events, shows its countdown, and its Keep is the core's applyConfirm", async () => {
  let now = guard(9, "waiting", 50_000);
  let confirmRefused = true;
  let started = false;
  const { desk, calls, push, subs } = scripted({
    applyGuardDefault: () => false,
    abState: () => ({ active: false, side: null, aName: "", bName: "", recall: null }),
    // Before the tune applies the core has no guard; then where the guard stands.
    applyStatus: () => (started ? now : null),
    noticeFor: () => null,
    applyPoll: () => now,
    applyConfirm: () => {
      // The core's gate: refused once after a failed verify (keep_unverified), as through the tab.
      if (confirmRefused) {
        confirmRefused = false;
        throw new Error("measure.tune.keep_unverified: the verify did not confirm the tune");
      }
      now = guard(9, "kept");
      return now;
    },
  });
  try {
    await desk.start();
    /** Read anew each time (an assertion of null does not narrow it for later reads). */
    const guardNow = (): GuardedApplyRecord | null => desk.state.guard;
    assert.equal(guardNow(), null, "no guard at the start");
    assert.equal(subs.size, 1, "the desk listens to the core's events");
    // Events of a tune without a waiting guard: nothing to show.
    push({ event: "measureTune", state: { stage: "proposed", running: false, guard: guard(0, "idle") } });
    push({ event: "measure", state: { active: true } });
    assert.equal(guardNow(), null);
    // The tune applied: its guard waits, and the bar shows it with its countdown.
    started = true;
    push({ event: "measureTune", state: { stage: "applied", running: false, guard: now } });
    assert.equal(guardNow()?.id, 9);
    assert.equal(guardNow()?.phase, "waiting");
    // Driven as the bar's own: its poll moves the countdown.
    now = guard(9, "waiting", 40_000);
    await until("the bar's poll", () => guardNow()?.remainingMs === 40_000);
    assert.ok(calls.includes("applyPoll"));
    // The same guard again (the next tune turn) is not adopted twice.
    const polls = calls.filter((c) => c === "applyPoll").length;
    push({ event: "measureTune", state: { stage: "applied", running: false, guard: guard(9, "waiting", 45_000) } });
    assert.equal(guardNow()?.remainingMs, 40_000, "the bar's own countdown, not the event's");
    // Keep: the core's gated applyConfirm; refused once (said, the guard still waits), then kept.
    assert.equal(await desk.keep(), false);
    assert.equal(desk.state.outcome?.tone, "error");
    assert.match(desk.state.outcome?.lines[0] ?? "", /keep_unverified|verify/);
    assert.equal(guardNow()?.phase, "waiting", "still waiting, as the core says");
    assert.ok(calls.filter((c) => c === "applyPoll").length >= polls);
    assert.equal(await desk.keep(), true);
    assert.equal(calls.filter((c) => c === "applyConfirm").length, 2, "only ever applyConfirm: never measureTuneKeep");
    assert.equal(calls.includes("measureTuneKeep"), false);
    assert.equal(guardNow()?.phase, "kept");
    // An ended guard in a later event is not adopted again.
    push({ event: "measureTune", state: { stage: "kept", running: false, guard: guard(9, "kept") } });
    assert.equal(guardNow()?.phase, "kept");
  } finally {
    desk.stop();
  }
  assert.equal(subs.size, 0, "stopped: no longer listening");
});
