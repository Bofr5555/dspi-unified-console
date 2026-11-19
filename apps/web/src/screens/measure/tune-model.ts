// WP P5WAT: the auto-tune and the documents of the Measure screen, without the page (PLAN-P5
// section 1.4, T10, T11, section 1.6). The core's tune calls typed (crates/dspi-wasm/src/
// measure_tune.rs, hub_server_measure.rs), what a tune's state, proposal and verify say in words,
// the tune's desk (`TuneDesk`: the layout, the proposal, the summed check, the guarded apply with
// the verify run by itself, Keep and its second question, Revert, the A/B compare) and the
// documents' desk (`DocsDesk`: this page's documents and the hub's, merged).
//
// Every rule and every number is the core's (R1): the proposal, its curves and its words, the
// summed check, the verify's verdict, the guard and Keep's gate. What is here is plumbing:
// which call when, and the rules of the screen itself:
// - After an apply the screen runs the verify by itself (review P5-D D3).
// - While a verify runs Keep is not offered (the core refuses it too, `keep_during_verify`).
// - Keep after a verify that did not pass: the core refuses the first press
//   (`measure.tune.keep_unverified`); the screen then asks a second question, and only the
//   person's own second press, at least 1 s later, keeps. Nothing here ever presses for them.
//   Keep of an applied tune with no verdict at all (its verify stopped or refused) asks the same
//   second question before anything is sent. The question is withdrawn by Revert, a new apply,
//   a new verify, or the tune leaving its guard.
// - Leaving the tab stops the tune's runs; a start still on its way when it was left is withdrawn
//   as soon as it answers (an apply is reverted: nothing stays applied out of sight).

import type { AbCompareRecord, CoreEvent, CoreMethods, GuardedApplyRecord } from "../../core/protocol";
import { CoreCallError } from "../../core/protocol";
import {
  tuneActive,
  type MeasureCaller,
  type MeasureCoupler,
  type MeasureMethod,
  type MeasureCurveRecord,
  type MeasureDocInfoRecord,
  type MeasureIssueRecord,
  type MeasureRole,
  type MeasureSetupRecord,
  type MeasureStateRecord,
} from "../../core/measure-protocol";
import { noticeRow, revertNotice } from "../../ui/texts"; // -- WP REVERTW -- revertNotice
import type { NoticeRecord } from "../../core/protocol";
import type { Series } from "./graph";
import { ROLES, db, hz } from "./model";

// ---- the core's records (camelCase; `dspi_core::MeasureLayout` ...) ----

export interface TuneLayoutOutputRecord {
  output: number;
  role: MeasureRole;
  speaker: string;
  group: number;
  name: string;
  feedInput?: number | null;
  feedGainDb: number;
  feedInverted: boolean;
}

export interface TuneLayoutRecord {
  outputs: TuneLayoutOutputRecord[];
  template: string;
  notes: string[];
}

export interface TuneOptionsRecord {
  targetDocument?: string;
  micDocument?: string;
  keepCrossover?: boolean;
  keepDelays?: boolean;
  keepLevels?: boolean;
  keepEq?: boolean;
  windowOctaves?: number;
  fitMinHz?: number;
  fitMaxHz?: number;
}

export type TuneKind = "crossover" | "delay" | "polarity" | "level" | "headroom" | "eq";

export interface TuneWriteRecord {
  output: number;
  kind: TuneKind;
  timing: boolean;
  words: string;
  params: unknown[];
}

export interface TuneCurvesRecord {
  speaker: string;
  coherent: boolean;
  before: MeasureCurveRecord;
  predicted: MeasureCurveRecord;
  target: MeasureCurveRecord;
}

export interface TuneTransitionRecord {
  speaker: string;
  lowOutput: number;
  highOutput: number;
  cornerBeforeHz: number;
  cornerHz: number;
  relativeDelayMs: number;
  flipped: boolean;
  sumCheckRmsDb?: number | null;
  verifyRmsDb?: number | null;
}

export type TuneCheck = "notNeeded" | "pending" | "passed" | "failed";

export interface TuneProposalRecord {
  id: number;
  serial: string;
  measurementId: string;
  measurementName: string;
  writes: TuneWriteRecord[];
  curves: TuneCurvesRecord[];
  transitions: TuneTransitionRecord[];
  held: string[];
  withdrawn: MeasureIssueRecord[];
  warnings: MeasureIssueRecord[];
  notes: string[];
  timingReliable: boolean;
  sumCheck: TuneCheck;
}

export type TuneStage = "idle" | "proposed" | "sumCheck" | "applied" | "verifying" | "verified" | "kept" | "reverted" | "revertIncomplete"; // -- WP REVERTW --

export interface TuneVerifyRecord {
  outputs: { output: number; rmsDb?: number | null }[];
  sums: { speaker: string; rmsDb?: number | null }[];
  transitions: TuneTransitionRecord[];
  timingReliable: boolean;
  passed: boolean;
  notes: string[];
}

export interface TuneStateRecord {
  stage: TuneStage;
  running: boolean;
  proposalId?: number | null;
  run: number;
  runs: number;
  session: MeasureStateRecord;
  guard: GuardedApplyRecord;
  verify?: TuneVerifyRecord | null;
  issue?: MeasureIssueRecord | null;
  snapshotBeforeId: string;
  snapshotAfterId: string;
}

export interface TuneKeepRecord {
  guard: GuardedApplyRecord;
  snapshotBeforeId: string;
  snapshotBefore: string;
  snapshotAfterId: string;
  snapshotAfter: string;
  measurementId: string;
  measurement: string;
  notes: string[];
}

export interface MergedDocRecord {
  doc: MeasureDocInfoRecord;
  where: string;
  localCopy: boolean;
}

export interface MergedDocsRecord {
  entries: MergedDocRecord[];
  hubListed: boolean;
  hubNoticeKey: string;
}

/** The tune's calls and the hub's document calls (merged into `MeasureMethods`, so `CoreMethods`). */
declare module "../../core/measure-protocol" {
  interface MeasureMethods {
    measureLayoutProposal: { params: undefined; result: TuneLayoutRecord };
    measureTunePropose: { params: { measurement: string; layout: TuneLayoutRecord; options?: TuneOptionsRecord }; result: TuneProposalRecord };
    measureTuneProposal: { params: undefined; result: TuneProposalRecord | null };
    measureTuneSumCheck: { params: { proposalId: number }; result: TuneStateRecord };
    measureTuneTick: { params: undefined; result: TuneStateRecord };
    measureTuneState: { params: undefined; result: TuneStateRecord };
    measureTuneApply: { params: { proposalId: number; guardS?: number }; result: GuardedApplyRecord };
    measureTuneVerify: { params: undefined; result: TuneStateRecord };
    measureTuneKeep: { params: undefined; result: TuneKeepRecord };
    measureTuneRevert: { params: undefined; result: GuardedApplyRecord };
    measureTuneAb: { params: undefined; result: AbCompareRecord };
    measureTuneStop: { params: undefined; result: TuneStateRecord };

    hubMeasureDocs: { params: { kind?: string; serial?: string }; result: MeasureDocInfoRecord[] };
    hubMeasureDocGet: { params: { id: string }; result: { doc: MeasureDocInfoRecord; document: string } };
    hubMeasureDocRename: { params: { id: string; name: string }; result: MeasureDocInfoRecord };
    hubMeasureDocNote: { params: { id: string; note: string }; result: MeasureDocInfoRecord };
    hubMeasureDocDelete: { params: { id: string }; result: null };
    measureDocsMerged: { params: { kind?: string; serial?: string }; result: MergedDocsRecord };
  }
}

// ---- refusals in words ----

/**
 * A refusal's notice key and its words: "key: why" messages (the measurement's and the tune's),
 * after the error's own prefix when it has one ("invalid argument: measure.tune.x: why").
 */
const KEYED = /^(?:[a-z][a-z ]*: )?([a-z][a-z0-9_]*(?:\.[a-z0-9_]+)+): (.+)$/s;

export const KEEP_UNVERIFIED = "measure.tune.keep_unverified";

/**
 * A failure in words, as P5QUI2 says them: a refusal that names its notice ("measure.tune.x: why")
 * is that notice's title and the core's words; else the call's own notice; else its message. Any
 * key the table has, so a new refusal needs no code here.
 */
export function refusalWords(e: unknown, row: (key: string) => NoticeRecord | null = noticeRow): { key: string; text: string } {
  const msg = e instanceof Error ? e.message : String(e);
  const m = KEYED.exec(msg);
  if (m) {
    const why = m[2]!.trim();
    const Why = why ? why[0]!.toUpperCase() + why.slice(1) : why;
    const title = row(m[1]!)?.title;
    return { key: m[1]!, text: title ? `${title}: ${Why}` : Why };
  }
  const key = e instanceof CoreCallError ? (e.noticeKey ?? "") : "";
  const n = key ? row(key) : null;
  const text = msg ? msg[0]!.toUpperCase() + msg.slice(1) : key;
  return { key, text: n && !key.startsWith("core.") && n.title !== text ? `${n.title}: ${text}` : text };
}

/** An issue of a proposal (withdrawn, warned) in words: its notice title, then the core's words. */
export function issueWords(i: MeasureIssueRecord, row: (key: string) => NoticeRecord | null = noticeRow): string {
  const t = row(i.key)?.title;
  const msg = i.message ? i.message[0]!.toUpperCase() + i.message.slice(1) : i.key;
  return t ? `${t}: ${msg}` : msg;
}

// ---- words ----

export function roleLabel(r: MeasureRole): string {
  return ROLES.find((x) => x.id === r)?.label ?? r;
}

/** An output's programme feed in words (RC1: what feeds it, and its net polarity). */
export function feedWords(o: TuneLayoutOutputRecord): string {
  if (typeof o.feedInput !== "number") return "no programme input feeds it";
  const pol = o.feedInverted ? ", polarity inverted" : "";
  return `fed by input ${o.feedInput + 1} at ${o.feedGainDb.toFixed(1)} dB${pol}`;
}

export function templateWords(t: string): string {
  if (!t) return "No layout recognised: check each output's way and speaker.";
  return `Recognised: ${t === "3way" ? "a three-way speaker" : t === "2way" ? "a two-way speaker" : t === "headphones" ? "headphones" : t}.`;
}

export function checkWords(c: TuneCheck): string {
  switch (c) {
    case "notNeeded":
      return "No timing change: no summed check is needed.";
    case "pending":
      return "The summed check has not run: the delay, polarity, crossover and level changes cannot be applied before it confirms the timing.";
    case "passed":
      return "The summed check confirmed the timing.";
    case "failed":
      return "The summed check did not confirm the timing: those changes are withdrawn (see below); the EQ may remain.";
  }
}

export function stageWords(st: TuneStateRecord | null): string {
  if (!st) return "No tune.";
  const run = st.running && st.runs ? ` (run ${Math.max(1, st.run)} of ${st.runs})` : "";
  switch (st.stage) {
    case "idle":
      return "No tune yet: propose one from a measurement.";
    case "proposed":
      return "Proposed: nothing is written yet.";
    case "sumCheck":
      return `The summed check plays both ways of each transition together${run}.`;
    case "applied":
      return "Applied inside the guard; not verified.";
    case "verifying":
      return `Verifying the tune: measuring what it does${run}. Keep is offered once the verify has its verdict.`;
    case "verified":
      return st.verify?.passed ? "Verified: the measurement agrees with the prediction." : "Verified: the measurement does not agree with the prediction.";
    case "kept":
      return "Kept.";
    case "reverted":
      // Review P5-E: the core's stage is "reverted" for every end but kept; only the guard knows
      // whether the settings from before are really back (a lost connection, a failed revert).
      if (st.guard.phase === "lost") return `The connection was lost while the tune waited in its guard: the device may still hold the tune.${st.guard.message ? ` ${st.guard.message}` : ""}`;
      if (st.guard.phase === "revert_failed") return `The revert failed: the device may still hold part of the tune.${st.guard.message ? ` ${st.guard.message}` : ""}`;
      return "Reverted: the settings from before are back.";
    // -- WP REVERTW -- the guard ended without putting everything back: the notice's words
    case "revertIncomplete": {
      const n = revertNotice(st.issue?.key || "revert.unknown");
      return `${n!.title}. ${n!.action}`;
    }
  }
}

/** The guard's countdown, in whole seconds. */
export function guardSeconds(g: GuardedApplyRecord | null | undefined): number {
  return g && g.phase === "waiting" ? Math.max(0, Math.ceil(g.remainingMs / 1000)) : 0;
}

export function verifyLines(v: TuneVerifyRecord, outputName: (o: number) => string): string[] {
  const rms = (x: number | null | undefined) => (typeof x === "number" ? `${x.toFixed(2)} dB RMS from the prediction` : "too little to compare");
  return [
    ...v.transitions.map((t) => `${t.speaker}: ${outputName(t.lowOutput)} with ${outputName(t.highOutput)} summed, ${rms(t.verifyRmsDb)}`),
    ...v.sums.map((s) => `${s.speaker}, the whole speaker: ${rms(s.rmsDb)}`),
    ...v.outputs.map((o) => `${outputName(o.output)}: ${rms(o.rmsDb)}`),
    ...v.notes,
  ];
}

export function transitionWords(t: TuneTransitionRecord, outputName: (o: number) => string): string {
  const corner = Math.abs(t.cornerHz - t.cornerBeforeHz) < 0.5 ? `corner ${hz(t.cornerHz)}, kept` : `corner ${hz(t.cornerBeforeHz)} → ${hz(t.cornerHz)}`;
  const check = typeof t.sumCheckRmsDb === "number" ? `; summed check ${db(t.sumCheckRmsDb, 2)} RMS` : "";
  return `${t.speaker}: ${outputName(t.lowOutput)} to ${outputName(t.highOutput)}, ${corner}, upper way ${t.relativeDelayMs.toFixed(3)} ms${t.flipped ? ", polarity flipped" : ""}${check}`;
}

/** The proposal's curves for the graph: each speaker's before (solid), predicted (dotted), target (dashed). */
export function proposalSeries(p: TuneProposalRecord | null): Series[] {
  if (!p) return [];
  const out: Series[] = [];
  p.curves.forEach((c, i) => {
    out.push({ label: `${c.speaker} now`, kind: "measured", curve: c.before, colour: i });
    out.push({ label: `${c.speaker} predicted`, kind: "predicted", curve: c.predicted });
    out.push({ label: `${c.speaker} target`, kind: "target", curve: c.target });
  });
  return out;
}

// ---- the layout's measurement ----

/** The layout the user confirmed: only the outputs left on. */
export function chosenLayout(l: TuneLayoutRecord, off: ReadonlySet<number>): TuneLayoutRecord {
  return { ...l, outputs: l.outputs.filter((o) => !off.has(o.output)) };
}

/** Where the tune's own measurement is taken: speakers at the listening position, or headphones on a coupler. */
export type TunePlace = { placement: "listeningPosition" } | { placement: "headphones"; coupler: MeasureCoupler; count: number };

/**
 * The tune's place for the Setup tab's draft: headphones or speakers at the listening position when
 * the person chose that there; null otherwise (WP P5FOLLOW, coordinator: no default, the person
 * chooses on the tune tab; several positions or near-field are not the tune's places).
 */
export function tunePlaceOf(d: { placement?: string | null; coupler?: MeasureCoupler; count?: number } | null | undefined): TunePlace | null {
  if (d?.placement === "listeningPosition") return { placement: "listeningPosition" };
  if (d?.placement !== "headphones") return null;
  const count = Number.isInteger(d.count) && d.count! >= 1 && d.count! <= 9 ? d.count! : 1;
  return { placement: "headphones", coupler: d.coupler ?? "iec60318_4", count };
}

/**
 * A walked run of the layout (one per alignment group, the session plans them), three passes (the
 * least for "timing reliable"). Review P5-E: the placement is the person's (S3: the start level,
 * the ceiling and the sound level cap are the placement's, lower for headphones), never assumed to
 * be speakers; the gap is the core's for the placement (1 s for speakers, 0.3 s on a coupler).
 */
export function layoutSetup(l: TuneLayoutRecord, sweepMs: number, micDocument: string, name: string, place: TunePlace): MeasureSetupRecord {
  return {
    kind: "walked",
    outputs: l.outputs.map((o) => ({ output: o.output, role: o.role, speaker: o.speaker, group: o.group })),
    placement: place.placement,
    count: place.placement === "headphones" ? place.count : 0,
    coupler: place.placement === "headphones" ? place.coupler : null,
    micDocument,
    sweepMs,
    passes: 3,
    captureRateHz: 48_000,
    name,
  };
}

// ---- the tune's desk ----

/** What the tune desk needs of the Measure screen's desk (MeasureDesk is one). */
export interface TuneHost {
  readonly core: MeasureCaller & { subscribe(fn: (e: CoreEvent) => void): () => void };
  result: string | null;
  resultFrom: string;
  micDocument: string;
  targetDocument: string;
  targetName: string;
  draft: { micDocument: string; placement?: string | null; coupler?: MeasureCoupler; count?: number } | null;
  setResult(text: string | null, from: string): void;
  openMic(): Promise<number>;
  closeMic(): Promise<void>;
}

/** WP P5FOLLOW: why the tune's own measurement waits for the person's placement. */
export const PLACE_FIRST = "Choose where it is measured first: speakers at the listening position, or headphones on a coupler. Headphones are measured much more quietly.";

/** The second Keep counts from this long after the core's refusal (`TUNE_KEEP_CONFIRM_MIN_MS`), ms. */
export const CONFIRM_MS = 1000;

/** The open second question: why, and since when (the person's second press counts from `since + CONFIRM_MS`). */
export interface KeepQuestion {
  /** `unverified`: the verify did not pass (the core refused); `noVerdict`: no verify had its verdict. */
  why: "unverified" | "noVerdict";
  since: number;
  text: string;
}

export interface TuneOptionsDraft {
  keepCrossover: boolean;
  keepDelays: boolean;
  keepLevels: boolean;
  keepEq: boolean;
}

export class TuneDesk {
  layout: TuneLayoutRecord | null = null;
  /** Outputs left out of the layout. */
  off = new Set<number>();
  options: TuneOptionsDraft = { keepCrossover: false, keepDelays: false, keepLevels: false, keepEq: false };
  /** The measurement to tune from: its text and where it came from; null: the screen's result. */
  measurement: { text: string; from: string } | null = null;
  proposal: TuneProposalRecord | null = null;
  state: TuneStateRecord | null = null;
  question: KeepQuestion | null = null;
  kept: TuneKeepRecord | null = null;
  ab: AbCompareRecord | null = null;
  error = "";
  said = "";
  busy = "";
  sweepMs = 1500;
  /** Where "Measure this layout" measures; null: as the Setup tab's placement says (`place`). */
  private placeChoice: TunePlace | null = null;
  private alive = false;
  /** Bumped when the tab is left: a start still on its way then knows. */
  private epoch = 0;
  private micOpen = false;
  private unsubscribe: (() => void) | null = null;
  private readonly listeners = new Set<() => void>();

  constructor(
    readonly host: TuneHost,
    private readonly now: () => number = () => Date.now(),
  ) {}

  private call<M extends MeasureMethod>(m: M, p: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    return this.host.core.call(m, p);
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private changed(): void {
    for (const l of [...this.listeners]) l();
  }

  /** The measurement the tune uses: the one chosen, else the screen's result. */
  get source(): { text: string; from: string } | null {
    if (this.measurement) return this.measurement;
    return this.host.result ? { text: this.host.result, from: this.host.resultFrom === "run" ? "this session's measurement" : this.host.resultFrom } : null;
  }

  get stage(): TuneStage {
    return this.state?.stage ?? "idle";
  }

  /** A guard waits for Keep or Revert. */
  get applied(): boolean {
    return this.stage === "applied" || this.stage === "verifying" || this.stage === "verified";
  }

  /** A summed or verify run plays. */
  get running(): boolean {
    return !!this.state?.running || this.stage === "verifying" || this.stage === "sumCheck";
  }

  /** Keep is offered: a guard waits and no verify runs. */
  get keepOffered(): boolean {
    return this.applied && !this.running && this.state?.guard.phase === "waiting";
  }

  /** The second question's Keep may be pressed now. */
  confirmReady(): boolean {
    return !!this.question && this.now() - this.question.since >= CONFIRM_MS;
  }

  /** The tab is shown: the tune's state as the core has it, its events, the layout. */
  async enter(): Promise<void> {
    this.alive = true;
    this.unsubscribe?.();
    const subscribe = this.host.core.subscribe as (fn: (e: unknown) => void) => () => void;
    this.unsubscribe = subscribe.call(this.host.core, (e) => {
      const x = e as { event?: unknown; state?: unknown } | null;
      if (x && x.event === "measureTune" && typeof x.state === "object" && x.state) void this.onState(x.state as TuneStateRecord);
    });
    const [st, p, ab] = await Promise.all([
      this.call("measureTuneState", undefined).catch(() => null),
      this.call("measureTuneProposal", undefined).catch(() => null),
      this.host.core.call("abState", undefined).catch(() => null),
    ]);
    if (!this.alive) return;
    if (st) this.state = st;
    this.proposal = p;
    this.ab = ab?.active ? ab : null;
    if (!this.layout) await this.loadLayout();
    this.changed();
  }

  /** The tab is gone: the tune's runs stop (a guard stays: the Worker keeps its timer). */
  async leave(): Promise<void> {
    this.alive = false;
    this.epoch++;
    this.unsubscribe?.();
    this.unsubscribe = null;
    if (this.running) await this.call("measureTuneStop", undefined).catch(() => null);
    this.question = null;
    await this.closeMic();
  }

  async loadLayout(): Promise<void> {
    try {
      this.layout = await this.call("measureLayoutProposal", undefined);
      this.off = new Set();
    } catch (e) {
      this.fail(e);
    }
    this.changed();
  }

  editOutput(output: number, patch: Partial<Pick<TuneLayoutOutputRecord, "role" | "speaker" | "group">>): void {
    if (!this.layout) return;
    this.layout = { ...this.layout, outputs: this.layout.outputs.map((o) => (o.output === output ? { ...o, ...patch } : o)) };
    this.changed();
  }

  setOn(output: number, on: boolean): void {
    const n = new Set(this.off);
    if (on) n.delete(output);
    else n.add(output);
    this.off = n;
    this.changed();
  }

  setOptions(patch: Partial<TuneOptionsDraft>): void {
    this.options = { ...this.options, ...patch };
    this.changed();
  }

  /** Where the tune's own measurement is taken: the person's choice here, else the Setup tab's; null: not chosen yet, nothing is measured. */
  get place(): TunePlace | null {
    return this.placeChoice ?? tunePlaceOf(this.host.draft);
  }

  /** Why "Measure this layout" waits for the person, or "" (WP P5FOLLOW: no placement is assumed). */
  get placeProblem(): string {
    return this.place ? "" : PLACE_FIRST;
  }

  setHeadphones(on: boolean): void {
    const d = this.host.draft;
    this.placeChoice = on ? tunePlaceOf({ placement: "headphones", coupler: d?.coupler, count: d?.placement === "headphones" ? d.count : 1 }) : { placement: "listeningPosition" };
    this.changed();
  }

  setSweepMs(ms: number): void {
    if (Number.isFinite(ms)) this.sweepMs = Math.min(15_000, Math.max(1000, Math.round(ms)));
    this.changed();
  }

  /** Tune from this document (the documents tab's "Tune from this"); null: the screen's result. */
  useMeasurement(text: string | null, from = ""): void {
    this.measurement = text === null ? null : { text, from };
    this.changed();
  }

  fail(e: unknown): void {
    this.error = refusalWords(e).text;
    this.changed();
  }

  clear(): void {
    this.error = "";
    this.said = "";
  }

  private get micDocument(): string {
    return this.host.micDocument || this.host.draft?.micDocument || "";
  }

  /** Propose from the measurement and the confirmed layout: nothing is written. */
  async propose(): Promise<boolean> {
    this.clear();
    const src = this.source;
    if (!src || !this.layout) {
      this.error = !src ? "Measure the layout first, or open a measurement on the Documents tab." : "No layout.";
      this.changed();
      return false;
    }
    this.busy = "Proposing";
    this.changed();
    try {
      this.proposal = await this.call("measureTunePropose", {
        measurement: src.text,
        layout: chosenLayout(this.layout, this.off),
        options: { ...this.options, targetDocument: this.host.targetDocument, micDocument: this.micDocument },
      });
      this.kept = null;
      this.state = await this.call("measureTuneState", undefined);
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  private async openMic(): Promise<boolean> {
    try {
      await this.host.openMic();
      this.micOpen = true;
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  private async closeMic(): Promise<void> {
    if (!this.micOpen) return;
    this.micOpen = false;
    await this.host.closeMic().catch(() => undefined);
  }

  /** The mandatory summed check (RC5f): the runs play; the proposal comes back with a new id. */
  async sumCheck(): Promise<boolean> {
    const p = this.proposal;
    if (!p) return false;
    this.clear();
    const epoch = this.epoch;
    if (!(await this.openMic())) return false;
    // Review P5-E: left while the microphone was asked for: nothing is started at all.
    if (epoch !== this.epoch || !this.alive) {
      await this.closeMic();
      return false;
    }
    try {
      const st = await this.call("measureTuneSumCheck", { proposalId: p.id });
      if (epoch !== this.epoch || !this.alive) {
        await this.call("measureTuneStop", undefined).catch(() => null);
        await this.closeMic();
        return false;
      }
      await this.onState(st);
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      return false;
    }
  }

  /** Apply the proposal inside the guard, then the verify, by itself (review P5-D D3). */
  async apply(): Promise<boolean> {
    const p = this.proposal;
    if (!p) return false;
    this.clear();
    this.question = null;
    const epoch = this.epoch;
    this.busy = "Applying";
    this.changed();
    try {
      const g = await this.call("measureTuneApply", { proposalId: p.id });
      if (epoch !== this.epoch || !this.alive) {
        // The tab was left while it applied: nothing stays applied out of sight.
        if (g.phase === "waiting") await this.call("measureTuneRevert", undefined).catch(() => null);
        return false;
      }
      this.state = await this.call("measureTuneState", undefined);
      if (g.phase !== "waiting") {
        this.error = g.message || "The tune was not applied.";
        return false;
      }
    } catch (e) {
      this.fail(e);
      this.state = await this.call("measureTuneState", undefined).catch(() => this.state);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
    await this.verify();
    return true;
  }

  /** The verify inside the guard: walked and summed runs against the prediction. */
  async verify(): Promise<boolean> {
    this.question = null;
    this.clear();
    const epoch = this.epoch;
    this.changed();
    if (!(await this.openMic())) return false;
    // Review P5-E: as the summed check (the guard runs on, turned by the Worker).
    if (epoch !== this.epoch || !this.alive) {
      await this.closeMic();
      return false;
    }
    try {
      const st = await this.call("measureTuneVerify", undefined);
      if (epoch !== this.epoch || !this.alive) {
        await this.call("measureTuneStop", undefined).catch(() => null);
        await this.closeMic();
        return false;
      }
      await this.onState(st);
      return true;
    } catch (e) {
      await this.closeMic();
      this.fail(e);
      this.state = await this.call("measureTuneState", undefined).catch(() => this.state);
      this.changed();
      return false;
    }
  }

  /** The verify's verdict did not pass, as far as the screen knows. */
  private get unverified(): boolean {
    return this.stage === "verified" && this.state?.verify?.passed === false;
  }

  /**
   * Keep, the first press. With a passed verify it keeps. Without a verdict the second question
   * opens before anything is sent; after a failed verify the core refuses and the question opens.
   */
  async keep(): Promise<boolean> {
    if (!this.keepOffered) return false;
    this.clear();
    if (this.stage === "applied") {
      this.question = { why: "noVerdict", since: this.now(), text: "The tune is not verified: no verify has its verdict yet. Keep it without one, or verify it first." };
      this.changed();
      return false;
    }
    return this.sendKeep(false);
  }

  /** The second question's Keep: only the person's own press, at least `CONFIRM_MS` after the question opened. */
  async confirmKeep(): Promise<boolean> {
    if (!this.question || !this.confirmReady() || !this.keepOffered) return false;
    return this.sendKeep(true);
  }

  /** Close the second question without keeping. */
  cancelKeep(): void {
    this.question = null;
    this.changed();
  }

  private async sendKeep(second: boolean): Promise<boolean> {
    this.busy = "Keeping";
    this.changed();
    try {
      const k = await this.call("measureTuneKeep", undefined);
      this.question = null;
      this.kept = k;
      // The measurement given as text comes back with the tune's row.
      if (k.measurement && this.measurement) this.measurement = { ...this.measurement, text: k.measurement };
      else if (k.measurement && this.host.result) this.host.setResult(k.measurement, this.host.resultFrom);
      this.state = await this.call("measureTuneState", undefined).catch(() => this.state);
      this.said = k.guard.phase === "kept" ? "Kept. The \"before\" and \"after\" snapshots are below; compare them with A/B." : k.guard.message || "The guard ended before Keep.";
      return k.guard.phase === "kept";
    } catch (e) {
      const r = refusalWords(e);
      if (r.key === KEEP_UNVERIFIED) {
        // The core's first refusal opens the question; a second press too early keeps it open.
        // Review P5-E: a "no verdict" question the core answers with this refusal (a verify had
        // failed before the one that was stopped) becomes the core's question, with its own second.
        if (!second || this.question?.why !== "unverified") this.question = { why: "unverified", since: this.now(), text: r.text };
      } else this.error = r.text;
      this.state = await this.call("measureTuneState", undefined).catch(() => this.state);
      return false;
    } finally {
      this.busy = "";
      this.changed();
    }
  }

  async revert(): Promise<boolean> {
    this.question = null;
    this.clear();
    this.changed();
    try {
      const g = await this.call("measureTuneRevert", undefined);
      this.state = await this.call("measureTuneState", undefined).catch(() => this.state);
      this.said = g.message || "Reverted.";
      await this.closeMic();
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    } finally {
      this.changed();
    }
  }

  /** Stop the summed check's or the verify's runs (an applied tune stays in its guard). */
  async stop(): Promise<void> {
    try {
      this.state = await this.call("measureTuneStop", undefined);
    } catch (e) {
      this.fail(e);
    }
    await this.closeMic();
    this.changed();
  }

  async abStart(): Promise<void> {
    try {
      this.ab = await this.call("measureTuneAb", undefined);
    } catch (e) {
      this.fail(e);
    }
    this.changed();
  }

  async abToggle(): Promise<void> {
    try {
      this.ab = await this.host.core.call("abToggle", undefined);
    } catch (e) {
      this.fail(e);
    }
    this.changed();
  }

  /** End the compare on one side ("a": the tune, "b": before). */
  async abEnd(side: "a" | "b"): Promise<void> {
    try {
      await this.host.core.call("abKeep", { side });
      this.ab = null;
      this.said = side === "a" ? "The compare ended on the tune." : "The compare ended on the settings from before the tune.";
    } catch (e) {
      this.fail(e);
    }
    this.changed();
  }

  /** A state from the Worker's turns or a call. */
  async onState(st: TuneStateRecord): Promise<void> {
    const was = this.state;
    this.state = st;
    // The question is the applied tune's, in its verdict: a new verify, the guard's end (the
    // timeout, a revert elsewhere) or a Keep elsewhere withdraws it.
    if (this.question && (!this.applied || st.stage === "verifying" || st.guard.phase !== "waiting" || (this.question.why === "noVerdict" && st.stage === "verified"))) this.question = null;
    if (was && tuneActive(was) && was.running && !st.running) {
      await this.closeMic();
      // After the summed check the proposal is made again (a new id; timing withdrawn on a failure).
      if (st.stage === "proposed") this.proposal = await this.call("measureTuneProposal", undefined).catch(() => this.proposal);
    }
    if (!st.running && this.micOpen && !this.applied && st.stage !== "sumCheck") await this.closeMic();
    this.changed();
  }
}

// ---- the documents' desk ----

export interface DocRow {
  /** "local:<n>" for this page's, else the hub's id. */
  key: string;
  where: "local" | "hub";
  info: MeasureDocInfoRecord;
  /** The text, for this page's documents (the hub's is read when needed). */
  text?: string;
  /** Where a page document came from ("a file", "this session's measurement"). */
  from?: string;
}

export const KINDS = [
  { id: "measurement", label: "Measurements" },
  { id: "mic", label: "Microphones" },
  { id: "target", label: "Targets" },
] as const;

/** Who may do what with a hub document: Control and Admin rename, note and file; only Admin deletes. */
export function hubRights(role: string, readOnly: boolean): { write: boolean; remove: boolean } {
  const r = role.toLowerCase();
  const write = !readOnly && (r === "control" || r === "admin");
  return { write, remove: write && r === "admin" };
}

/** The list shown: the hub's rows and this page's, the hub's winning on an id both hold with the same name and size. */
export function mergeRows(local: DocRow[], hub: DocRow[]): DocRow[] {
  const ids = new Set(hub.map((h) => `${h.info.id}|${h.info.bytes}`));
  const rows = [...hub, ...local.filter((l) => !l.info.id || !ids.has(`${l.info.id}|${l.info.bytes}`))];
  const order = (k: string) => KINDS.findIndex((x) => x.id === k);
  return rows.sort((a, b) => order(a.info.kind) - order(b.info.kind) || a.info.savedMs - b.info.savedMs || a.info.name.localeCompare(b.info.name));
}

/** A page document with a new name or note (its own fields), or null when it does not read. */
export function edited(text: string, kind: string, patch: { name?: string; note?: string }): string | null {
  try {
    const d = JSON.parse(text) as Record<string, unknown> & { meta?: Record<string, unknown> };
    if (!d || typeof d !== "object" || !d.meta || typeof d.meta !== "object") return null;
    if (patch.name !== undefined) d.meta.name = patch.name;
    if (patch.note !== undefined) {
      const body = d[kind];
      if (!body || typeof body !== "object") return null;
      (body as Record<string, unknown>).note = patch.note;
    }
    return JSON.stringify(d, null, 2);
  } catch {
    return null;
  }
}

/** The outputs a measurement document holds (for REW export), [] for another kind. */
export function docOutputs(text: string): number[] {
  try {
    const d = JSON.parse(text) as { measurement?: { outputs?: { output?: unknown }[] } };
    return (d.measurement?.outputs ?? []).map((o) => o.output).filter((o): o is number => typeof o === "number");
  } catch {
    return [];
  }
}

/** What the documents desk needs of the screen's desk. */
export interface DocsHost extends TuneHost {
  hubMeasure: boolean;
  hubRole: string;
  setResult(text: string | null, from: string): void;
  setDocs(patch: { micDocument?: string; targetDocument?: string; targetName?: string }): void;
}

export class DocsDesk {
  local: DocRow[] = [];
  hub: DocRow[] = [];
  hubNotice = "";
  error = "";
  said = "";
  private n = 0;
  private readonly listeners = new Set<() => void>();

  constructor(
    readonly host: DocsHost,
    private readonly readOnly: () => boolean = () => false,
  ) {}

  private call<M extends MeasureMethod>(m: M, p: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    return this.host.core.call(m, p);
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private changed(): void {
    for (const l of [...this.listeners]) l();
  }

  get rights(): { write: boolean; remove: boolean } {
    return this.host.hubMeasure ? hubRights(this.host.hubRole, this.readOnly()) : { write: false, remove: false };
  }

  get rows(): DocRow[] {
    return mergeRows(this.local, this.hub);
  }

  private fail(e: unknown): void {
    this.error = refusalWords(e).text;
    this.said = "";
    this.changed();
  }

  /** The hub's list (on the web the merged list is the hub's rows), and the screen's own documents. */
  async refresh(): Promise<void> {
    await this.adoptScreen();
    if (!this.host.hubMeasure) {
      this.hub = [];
      this.changed();
      return;
    }
    try {
      const m = await this.call("measureDocsMerged", {});
      this.hub = m.entries.filter((e) => e.where !== "local").map((e) => ({ key: e.doc.id, where: "hub" as const, info: e.doc }));
      this.hubNotice = m.hubListed ? "" : (noticeRow(m.hubNoticeKey)?.title ?? m.hubNoticeKey);
    } catch (e) {
      this.fail(e);
    }
    this.changed();
  }

  /** The screen's measurement, microphone and target documents are this page's too. */
  async adoptScreen(): Promise<void> {
    const h = this.host;
    if (h.result) await this.addText(h.result, h.resultFrom === "run" ? "this session's measurement" : h.resultFrom || "this session", true);
    if (h.micDocument) await this.addText(h.micDocument, "the setup's microphone", true);
    if (h.targetDocument) await this.addText(h.targetDocument, "the target in use", true);
  }

  /** Add a document's text to this page's list (validated by the core); the row, or null. */
  async addText(text: string, from: string, quiet = false): Promise<DocRow | null> {
    const have = this.local.find((r) => r.text === text);
    if (have) return have;
    try {
      const info = await this.call("measureDocInfo", { text });
      const row: DocRow = { key: `local:${++this.n}`, where: "local", info, text, from };
      this.local = [...this.local, row];
      this.changed();
      return row;
    } catch (e) {
      if (!quiet) this.fail(e);
      return null;
    }
  }

  /** A microphone's calibration file, made a `mic` document. */
  async importMic(fileName: string, text: string): Promise<DocRow | null> {
    try {
      const r = await this.call("micImport", { name: fileName.replace(/\.[^.]*$/, ""), fileName, text });
      return await this.addText(r.text, fileName);
    } catch (e) {
      this.fail(e);
      return null;
    }
  }

  /** A target file (REW or CSV points), made a `target` document. */
  async importTarget(fileName: string, text: string, use: "speakers" | "headphones"): Promise<DocRow | null> {
    try {
      const r = await this.call("targetImport", { name: fileName.replace(/\.[^.]*$/, ""), text, use, coupler: "" });
      return await this.addText(r.text, fileName);
    } catch (e) {
      this.fail(e);
      return null;
    }
  }

  /** A row's text (the hub's read when asked). */
  async textOf(row: DocRow): Promise<string | null> {
    if (row.text !== undefined) return row.text;
    try {
      return (await this.call("hubMeasureDocGet", { id: row.info.id })).document;
    } catch (e) {
      this.fail(e);
      return null;
    }
  }

  /** Use a row: a measurement as the screen's result, a microphone or a target in the setup and the tune. */
  async use(row: DocRow): Promise<string | null> {
    const text = await this.textOf(row);
    if (text === null) return null;
    const name = row.info.name || row.info.kind;
    if (row.info.kind === "measurement") this.host.setResult(text, name);
    else if (row.info.kind === "mic") this.host.setDocs({ micDocument: text });
    else if (row.info.kind === "target") this.host.setDocs({ targetDocument: text, targetName: name });
    if (row.info.kind === "mic" && this.host.draft) this.host.draft.micDocument = text;
    this.said = `Using "${name}".`;
    this.error = "";
    this.changed();
    return text;
  }

  async rename(row: DocRow, name: string): Promise<boolean> {
    return this.edit(row, { name });
  }

  async note(row: DocRow, note: string): Promise<boolean> {
    return this.edit(row, { note });
  }

  private async edit(row: DocRow, patch: { name?: string; note?: string }): Promise<boolean> {
    try {
      if (row.where === "hub") {
        if (!this.rights.write) throw new Error("Renaming or noting a hub document needs the hub's Control or Admin role.");
        const info = patch.name !== undefined ? await this.call("hubMeasureDocRename", { id: row.info.id, name: patch.name }) : await this.call("hubMeasureDocNote", { id: row.info.id, note: patch.note ?? "" });
        this.hub = this.hub.map((h) => (h.key === row.key ? { ...h, info } : h));
      } else {
        const text = row.text !== undefined ? edited(row.text, row.info.kind, patch) : null;
        if (!text) throw new Error("This document does not read as one of the measurement family.");
        const info = await this.call("measureDocInfo", { text });
        this.local = this.local.map((l) => (l.key === row.key ? { ...l, info, text } : l));
      }
      this.error = "";
      this.said = patch.name !== undefined ? `Renamed to "${patch.name}".` : "Note saved.";
      this.changed();
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  /** Delete: a hub document by its Admin only (the hub refuses Control too); this page's from the list. */
  async remove(row: DocRow): Promise<boolean> {
    try {
      if (row.where === "hub") {
        if (!this.rights.remove) throw new Error("Deleting a hub document needs the hub's Admin role.");
        await this.call("hubMeasureDocDelete", { id: row.info.id });
        this.hub = this.hub.filter((h) => h.key !== row.key);
      } else this.local = this.local.filter((l) => l.key !== row.key);
      this.said = `"${row.info.name}" deleted.`;
      this.error = "";
      this.changed();
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  /** File this page's document on the hub (Control and Admin). */
  async saveToHub(row: DocRow): Promise<boolean> {
    if (row.text === undefined) return false;
    try {
      const r = await this.call("hubMeasureDocPut", { text: row.text });
      this.said = r.created ? `Saved on the hub as "${r.doc.name}".` : `The hub already holds "${r.doc.name}".`;
      this.error = "";
      await this.refresh();
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  /** Output `output`'s response of a measurement as REW text. */
  async exportRew(row: DocRow, output: number): Promise<string | null> {
    const text = await this.textOf(row);
    if (text === null) return null;
    try {
      return (await this.call("measurementExportRew", { text, output })).text;
    } catch (e) {
      this.fail(e);
      return null;
    }
  }
}

/** One desk of each kind per Measure screen (they outlive a tab switch). */
const tunes = new WeakMap<object, TuneDesk>();
const docs = new WeakMap<object, DocsDesk>();

export function tuneDeskOf(host: TuneHost): TuneDesk {
  let d = tunes.get(host);
  if (!d) tunes.set(host, (d = new TuneDesk(host)));
  return d;
}

export function docsDeskOf(host: DocsHost, readOnly: () => boolean): DocsDesk {
  let d = docs.get(host);
  if (!d) docs.set(host, (d = new DocsDesk(host, readOnly)));
  return d;
}
