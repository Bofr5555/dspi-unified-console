// WP P5WMS: the measurement screens' rules, without the page (PLAN-P5 sections 1.1 to 1.6, T9).
// What a draft setup sends, what a state says in words, a stored curve decoded for the graph
// (docs/integrator/measurement.md section 7.2), a measurement document read for the results,
// and the graph's scales. No maths of the measurement here: the core computes every curve; this
// file only reads what it gave (a document's curves are decoded, never smoothed or fitted).

import type {
  MeasureCoupler,
  MeasureCurveRecord,
  MeasureIssueRecord,
  MeasureLiveStateRecord,
  MeasurePlacement,
  MeasurePreflightRecord,
  MeasureRole,
  MeasureSetupRecord,
  MeasureStateRecord,
  MeasureTimingRecord,
} from "../../core/measure-protocol";

// ---- words ----

export const ROLES: { id: MeasureRole; label: string }[] = [
  { id: "full", label: "Full range" },
  { id: "sub", label: "Subwoofer" },
  { id: "low", label: "Low (woofer)" },
  { id: "mid", label: "Mid" },
  { id: "high", label: "High (tweeter)" },
];

export const PLACEMENTS: { id: MeasurePlacement; label: string; detail: string }[] = [
  { id: "listeningPosition", label: "Listening position", detail: "Speakers: one position, with timing (alignment, crossover, EQ)." },
  { id: "multiPosition", label: "Several positions", detail: "Speakers: 3 to 9 positions averaged; position 1 is the timing reference." },
  { id: "nearField", label: "Near-field", detail: "Speakers: the microphone close to each driver; kept as a labelled near-field measurement." },
  { id: "headphones", label: "Headphones on a coupler", detail: "On the coupler, not on a head: 1 to 9 reseats averaged." },
];

export const COUPLERS: { id: MeasureCoupler; label: string }[] = [
  { id: "iec60318_4", label: "IEC 60318-4 ear simulator (711)" },
  { id: "flatPlate", label: "Flat-plate coupler" },
  { id: "hats", label: "HATS or another coupler with pinna" },
];

/** The live graph's smoothing choices, 1/N octave (0: none), and the RTA's averaging. */
export const SMOOTHING = [0, 1, 2, 3, 6, 12, 24, 48] as const;
export const AVERAGING = [
  { id: "fast", label: "Fast (125 ms)" },
  { id: "slow", label: "Slow (1 s)" },
  { id: "infinite", label: "Infinite" },
] as const;

export function smoothingLabel(n: number): string {
  return n ? `1/${n} octave` : "None";
}

/** A frequency in words: "950 Hz", "1.02 kHz". */
export function hz(f: number): string {
  if (!Number.isFinite(f)) return "—";
  if (f >= 1000) return `${(f / 1000).toFixed(f >= 10_000 ? 1 : 2)} kHz`;
  return `${Math.round(f)} Hz`;
}

export function db(v: number | null | undefined, digits = 1): string {
  return typeof v === "number" && Number.isFinite(v) ? `${v.toFixed(digits)} dB` : "—";
}

// ---- the draft setup ----

export interface OutputDraft {
  output: number;
  on: boolean;
  role: MeasureRole;
  speaker: string;
  group: number;
}

/** What the setup form holds. */
export interface MeasureDraft {
  /** WP P5FOLLOW: null until the person chooses (no default: headphones are measured much more quietly). */
  placement: MeasurePlacement | null;
  /** Positions (several positions: 3 to 9) or reseats (headphones: 1 to 9). */
  count: number;
  coupler: MeasureCoupler;
  outputs: OutputDraft[];
  /** One sweep, ms (1000 to 15000). */
  sweepMs: number;
  /** Passes of the walked run (2 to 8). */
  passes: number;
  name: string;
  /** The `mic` document's text ("": uncalibrated). */
  micDocument: string;
}

/** A draft for the device's enabled outputs: the first two chosen, full range, speaker L. */
export function initialDraft(outputs: number[]): MeasureDraft {
  return {
    placement: null,
    count: 3,
    coupler: "iec60318_4",
    outputs: outputs.map((o, i) => ({ output: o, on: i < 2, role: "full", speaker: "L", group: 0 })),
    sweepMs: 5500,
    passes: 3,
    name: "",
    micDocument: "",
  };
}

/** The draft with the device's outputs now (kept choices for outputs still there). */
export function withOutputs(d: MeasureDraft, outputs: number[]): MeasureDraft {
  const fresh = initialDraft(outputs).outputs;
  const kept = fresh.map((f) => d.outputs.find((o) => o.output === f.output) ?? { ...f, on: false });
  if (kept.length === d.outputs.length && kept.every((o, i) => o === d.outputs[i])) return d;
  return { ...d, outputs: kept };
}

export function countRange(p: MeasurePlacement | null): [number, number] | null {
  if (p === "multiPosition") return [3, 9];
  if (p === "headphones") return [1, 9];
  return null;
}

/** WP P5FOLLOW (coordinator, headphone safety): why nothing is measured before the person chose the placement. */
export const PLACEMENT_FIRST = "Choose the placement first: speakers or headphones on a coupler. Headphones are measured much more quietly.";

/** WP P5FOLLOW: the live view's placements, and why it waits for the person's choice (it plays too). */
export const LIVE_PLACEMENT_IDS: readonly MeasurePlacement[] = ["listeningPosition", "nearField", "headphones"];
export const LIVE_PLACEMENT_FIRST = "Choose the placement first: speakers or headphones on a coupler. Headphones are measured much more quietly.";

/** The live view's placement taken from the Setup tab's choice when it is one the live view has; null otherwise (no default). */
export function livePlaceOf(d: { placement?: MeasurePlacement | null } | null | undefined): MeasurePlacement | null {
  const p = d?.placement ?? null;
  return p && LIVE_PLACEMENT_IDS.includes(p) ? p : null;
}

/** The hint under Passes when it is 2 (review P5-E: a hint, no minimum). */
export const TWO_PASSES_HINT = "Two passes cannot judge timing; use three or more for auto-tune.";

/** What is wrong with the draft before the core is asked, or "". */
export function draftProblem(d: MeasureDraft): string {
  const on = d.outputs.filter((o) => o.on);
  if (on.length === 0) return "Choose at least one output to measure.";
  if (on.length > 16) return "At most 16 outputs a run.";
  if (!d.placement) return PLACEMENT_FIRST;
  const r = countRange(d.placement);
  if (r && !(Number.isInteger(d.count) && d.count >= r[0] && d.count <= r[1]))
    return d.placement === "headphones" ? "Reseats: 1 to 9." : "Positions: 3 to 9.";
  if (!(Number.isInteger(d.sweepMs) && d.sweepMs >= 1000 && d.sweepMs <= 15000)) return "A sweep is 1 to 15 s.";
  if (!(Number.isInteger(d.passes) && d.passes >= 2 && d.passes <= 8)) return "Passes: 2 to 8.";
  return "";
}

/** The session's setup (`MeasureSetup`) for the draft: a walked run; every other number the plan's default. Throws while no placement is chosen (never one assumed). */
export function setupOf(d: MeasureDraft, captureRateHz: number): MeasureSetupRecord {
  if (!d.placement) throw new Error(PLACEMENT_FIRST);
  const r = countRange(d.placement);
  return {
    kind: "walked",
    outputs: d.outputs
      .filter((o) => o.on)
      .map((o) => ({ output: o.output, role: o.role, speaker: o.speaker.trim(), group: o.group })),
    placement: d.placement,
    count: r ? d.count : 0,
    coupler: d.placement === "headphones" ? d.coupler : null,
    micDocument: d.micDocument,
    sweepMs: d.sweepMs,
    passes: d.passes,
    captureRateHz: Number.isInteger(captureRateHz) && captureRateHz > 0 ? captureRateHz : 0,
    name: d.name.trim(),
  };
}

// ---- what a state says ----

/** The preflight in words: each blocker and warning with its notice title where the table has one. */
export function preflightLines(
  p: MeasurePreflightRecord,
  title: (key: string) => string | null,
  outputName: (o: number) => string,
): { kind: "blocker" | "warning"; key: string; text: string }[] {
  const line = (kind: "blocker" | "warning") => (i: MeasureIssueRecord) => ({ kind, key: i.key, text: issueText(i, title, outputName) });
  return [...p.blockers.map(line("blocker")), ...p.warnings.map(line("warning"))];
}

/** One issue in words: "Volume too high to measure: the volume is 0.0 dB; ...". */
export function issueText(i: MeasureIssueRecord, title: (key: string) => string | null, outputName: (o: number) => string): string {
  const t = title(i.key);
  const where = typeof i.output === "number" ? `${outputName(i.output)}: ` : "";
  const msg = i.message ? i.message[0]!.toUpperCase() + i.message.slice(1) : "";
  return t ? `${t}. ${where}${msg}` : `${where}${msg || i.key}`;
}

/** Where a run stands, in words. */
export function phaseText(st: MeasureStateRecord, outputName: (o: number) => string): string {
  const out = typeof st.output === "number" ? outputName(st.output) : "";
  const level = typeof st.levelDb === "number" ? ` at ${st.levelDb.toFixed(0)} dBFS` : "";
  const pos = st.positions > 1 ? ` (position ${st.position} of ${st.positions})` : "";
  switch (st.phase) {
    case "idle":
      return "Not measuring.";
    case "preRoll":
      return `Listening to the room's noise (the generator is idle)${pos}.`;
    case "ramp":
      return `Finding the level${out ? ` of ${out}` : ""}${level}: quiet first, louder in steps${pos}.`;
    case "run":
      return `Measuring: run ${Math.max(1, st.run)} of ${Math.max(1, st.runs)}${level}${pos}.`;
    case "analysing":
      return `Analysing${pos}.`;
    case "waitPosition":
      return `Move the microphone (or reseat the headphones) to position ${st.position} of ${st.positions}, then choose Next position.`;
    case "done":
      return "The measurement is complete.";
    case "aborted":
      return "Stopped: nothing was kept.";
    case "failed":
      return "Stopped, with a question: nothing was kept.";
  }
}

/** The run's timing verdict in words; null before a walked run was analysed. */
export function timingWords(t: MeasureTimingRecord | null | undefined): { reliable: boolean; text: string } | null {
  if (!t) return null;
  if (t.reliable)
    return { reliable: true, text: `Timing reliable: largest cycle residual ${t.maxResidualUs.toFixed(1)} µs (bound ${t.residualBoundUs.toFixed(0)} µs), microphone clock ${t.ratioPpm.toFixed(1)} ppm.` };
  const why = t.reasons.length ? ` ${t.reasons.join("; ")}.` : "";
  return { reliable: false, text: `Timing unreliable: the magnitudes are kept, but no delay, polarity or crossover change is offered from this measurement.${why}` };
}

/** 0 to 100: the phase's progress. */
export function progressPct(st: MeasureStateRecord): number {
  const p = Number.isFinite(st.progress) ? st.progress : 0;
  return Math.round(Math.min(1, Math.max(0, p)) * 100);
}

/** The live view's state in words. */
export function liveText(st: MeasureLiveStateRecord | null): string {
  if (!st) return "Not running.";
  if (!st.active) return "Not running.";
  const level = typeof st.levelDb === "number" ? ` at ${st.levelDb.toFixed(0)} dBFS` : "";
  return st.mode === "pinkRta" ? `Pink noise${level}: the microphone RTA.` : `Sweeping${level}: ${st.seq} curve${st.seq === 1 ? "" : "s"} measured.`;
}

// ---- stored curves (docs/integrator/measurement.md section 7.2) ----

export interface GridRecord {
  minHz: number;
  maxHz: number;
  pointsPerOctave: number;
}

/** The grid's frequencies, as `LogGrid::freqs`; [] for a bad grid. */
export function gridFreqs(g: GridRecord | null | undefined): number[] {
  if (!g || !(g.minHz > 0) || !(g.maxHz > g.minHz) || !Number.isFinite(g.maxHz)) return [];
  const ppo = g.pointsPerOctave;
  if (!Number.isInteger(ppo) || ppo < 1 || ppo > 384) return [];
  const n = Math.floor(Math.log2(g.maxHz / g.minHz) * ppo) + 1;
  if (!(n >= 1 && n <= 2048)) return [];
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) out[i] = g.minHz * 2 ** (i / ppo);
  return out;
}

function bytesOf(b64: unknown): Uint8Array | null {
  if (typeof b64 !== "string") return null;
  try {
    const s = atob(b64);
    const out = new Uint8Array(s.length);
    for (let i = 0; i < s.length; i++) out[i] = s.charCodeAt(i);
    return out;
  } catch {
    return null;
  }
}

function ints(b64: unknown, width: 2 | 4, n: number, step: number): number[] | null {
  const b = bytesOf(b64);
  if (!b || b.length !== n * width) return null;
  const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) out[i] = (width === 2 ? v.getInt16(i * 2, true) : v.getInt32(i * 4, true)) * step;
  return out;
}

/** A stored curve, decoded: what the graph draws. */
export interface DecodedCurve extends MeasureCurveRecord {
  phaseDeg?: number[];
  /** Spread across positions or reseats (max minus min), dB. */
  spreadDb?: number[];
}

/** A document's curve (`{grid, mag, phase?, spread?, valid?, spl?}`), or null when it does not read. */
export function decodeCurve(c: unknown): DecodedCurve | null {
  if (typeof c !== "object" || c === null) return null;
  const x = c as Record<string, unknown>;
  const freqsHz = gridFreqs(x.grid as GridRecord);
  const n = freqsHz.length;
  if (!n) return null;
  const magDb = ints(x.mag, 2, n, 0.01);
  if (!magDb) return null;
  let valid: boolean[];
  if (x.valid === undefined) valid = new Array<boolean>(n).fill(true);
  else {
    const b = bytesOf(x.valid);
    if (!b || b.length !== Math.ceil(n / 8)) return null;
    valid = Array.from({ length: n }, (_, i) => ((b[i >> 3]! >> (i & 7)) & 1) === 1);
  }
  const out: DecodedCurve = { freqsHz, magDb, valid, spl: x.spl === true };
  if (x.phase !== undefined) {
    const p = ints(x.phase, 4, n, 0.1);
    if (p) out.phaseDeg = p;
  }
  if (x.spread !== undefined) {
    const s = ints(x.spread, 2, n, 0.01);
    if (s) out.spreadDb = s;
  }
  return out;
}

// ---- a measurement document, for the results ----

export interface ResultOutput {
  output: number;
  role: string;
  speaker: string;
  arrivalMs: number | null;
  curve: DecodedCurve | null;
}

export interface ResultView {
  name: string;
  serial: string;
  savedMs: number;
  placement: string;
  positions: number;
  mic: string;
  levelDb: number | null;
  /** null: no timing (a single sweep), else the gate's verdict. */
  timingReliable: boolean | null;
  reasons: string[];
  maxResidualUs: number | null;
  outputs: ResultOutput[];
}

function placementWords(p: unknown): string {
  const x = (p ?? {}) as { mode?: string; count?: number; coupler?: string; reseats?: number };
  const label = PLACEMENTS.find((q) => q.id === x.mode)?.label ?? String(x.mode ?? "—");
  if (x.mode === "multiPosition" && x.count) return `${label}: ${x.count}`;
  if (x.mode === "headphones") {
    const c = COUPLERS.find((q) => q.id === x.coupler)?.label;
    return `${label}${c ? `, ${c}` : ""}${x.reseats ? `, ${x.reseats} reseat${x.reseats === 1 ? "" : "s"}` : ""}`;
  }
  return label;
}

/** A measurement document's text, read for the results screen; null when it is not one. */
export function resultView(text: string): ResultView | null {
  let doc: { meta?: { kind?: string; name?: string }; measurement?: Record<string, unknown> };
  try {
    doc = JSON.parse(text) as typeof doc;
  } catch {
    return null;
  }
  if (doc?.meta?.kind?.trim().toLowerCase() !== "measurement" || typeof doc.measurement !== "object" || !doc.measurement) return null;
  const m = doc.measurement as {
    serial?: string;
    savedMs?: number;
    placement?: unknown;
    positions?: number;
    mic?: { name?: string; approximate?: boolean; sensitivityDb?: number };
    stimulus?: { levelDb?: number; kind?: string };
    capture?: { timingReliable?: boolean; reasons?: string[]; maxResidualUs?: number };
    outputs?: { output?: number; role?: string; speaker?: string; arrivalMs?: number; response?: unknown }[];
  };
  const walked = m.stimulus?.kind === "walked";
  return {
    name: doc.meta?.name ?? "",
    serial: m.serial ?? "",
    savedMs: typeof m.savedMs === "number" ? m.savedMs : 0,
    placement: placementWords(m.placement),
    positions: typeof m.positions === "number" ? m.positions : 1,
    mic: m.mic?.name ? `${m.mic.name}${m.mic.approximate ? " (dB SPL approximate)" : ""}` : "Uncalibrated (relative dB)",
    levelDb: typeof m.stimulus?.levelDb === "number" ? m.stimulus.levelDb : null,
    timingReliable: walked && typeof m.capture?.timingReliable === "boolean" ? m.capture.timingReliable : null,
    reasons: Array.isArray(m.capture?.reasons) ? m.capture!.reasons!.filter((r) => typeof r === "string") : [],
    maxResidualUs: typeof m.capture?.maxResidualUs === "number" ? m.capture.maxResidualUs : null,
    outputs: (m.outputs ?? [])
      .filter((o) => typeof o.output === "number")
      .map((o) => ({
        output: o.output!,
        role: ROLES.find((r) => r.id === o.role)?.label ?? String(o.role ?? ""),
        speaker: o.speaker ?? "",
        arrivalMs: typeof o.arrivalMs === "number" ? o.arrivalMs : null,
        curve: decodeCurve(o.response),
      })),
  };
}

/** The `mic` document's summary for the setup: name, sensitivity, the angle, approximate or not. */
export function micSummary(text: string): string {
  try {
    const d = JSON.parse(text) as { meta?: { kind?: string; name?: string }; mic?: { name?: string; sensitivityDb?: number; approximate?: boolean; angle?: number; splOffsetDb?: number } };
    if (d?.meta?.kind !== "mic" || !d.mic) return "";
    const m = d.mic;
    const parts = [m.name || d.meta.name || "Microphone"];
    if (typeof m.sensitivityDb === "number") parts.push(`sensitivity ${m.sensitivityDb.toFixed(1)} dB`);
    if (m.angle === 90) parts.push("90° calibration");
    parts.push(m.approximate === false ? "dB SPL calibrated" : "dB SPL approximate (no calibrator step)");
    return parts.join(", ");
  } catch {
    return "";
  }
}

/** A file name for a document: its name, safe for a file system, and `.dspipreset`. */
export function fileNameOf(name: string, fallback: string): string {
  const base = (name || fallback).replace(/[^\p{L}\p{N} ._-]+/gu, "-").replace(/\s+/g, " ").trim().slice(0, 80) || fallback;
  return `${base}.dspipreset`;
}

// ---- the graph's scales ----

export const GRAPH_MIN_HZ = 20;
export const GRAPH_MAX_HZ = 20_000;

export function xOf(f: number, width: number, lo = GRAPH_MIN_HZ, hi = GRAPH_MAX_HZ): number {
  return (Math.log(f / lo) / Math.log(hi / lo)) * width;
}

export function freqOf(x: number, width: number, lo = GRAPH_MIN_HZ, hi = GRAPH_MAX_HZ): number {
  const t = Math.min(1, Math.max(0, width > 0 ? x / width : 0));
  return lo * (hi / lo) ** t;
}

/** The dB range that shows every trusted point of `curves` in the graph's span: 6 dB steps, at least 36 dB. */
export function yRange(curves: (MeasureCurveRecord | null | undefined)[], lo = GRAPH_MIN_HZ, hi = GRAPH_MAX_HZ): [number, number] {
  let min = Infinity;
  let max = -Infinity;
  for (const c of curves) {
    if (!c) continue;
    for (let i = 0; i < c.freqsHz.length; i++) {
      const f = c.freqsHz[i]!;
      const m = c.magDb[i]!;
      if (f < lo || f > hi || !Number.isFinite(m) || c.valid[i] === false) continue;
      if (m < min) min = m;
      if (m > max) max = m;
    }
  }
  if (!Number.isFinite(min)) return [-30, 6];
  let a = Math.floor(min / 6) * 6 - 6;
  let b = Math.ceil(max / 6) * 6 + 6;
  if (b - a < 36) {
    const mid = (a + b) / 2;
    a = Math.floor((mid - 18) / 6) * 6;
    b = a + 36;
  }
  return [a, b];
}

/** A curve cut where its trust changes: trusted runs are drawn strong, the others faint. */
export function segments(c: MeasureCurveRecord, lo = GRAPH_MIN_HZ, hi = GRAPH_MAX_HZ): { trusted: boolean; points: [number, number][] }[] {
  const out: { trusted: boolean; points: [number, number][] }[] = [];
  for (let i = 0; i < c.freqsHz.length; i++) {
    const f = c.freqsHz[i]!;
    const m = c.magDb[i]!;
    if (f < lo || f > hi || !Number.isFinite(m)) continue;
    const trusted = c.valid[i] !== false;
    const last = out[out.length - 1];
    if (last && last.trusted === trusted) last.points.push([f, m]);
    else {
      // Joined to the run before, so the line has no hole where the trust changes.
      const prev = last?.points[last.points.length - 1];
      out.push({ trusted, points: prev ? [prev, [f, m]] : [[f, m]] });
    }
  }
  return out;
}

/** The highest trusted point of `c` between `lo` and `hi` (where "cut this peak" starts), or null. */
export function highestPoint(c: MeasureCurveRecord | null | undefined, lo = 100, hi = 5000): number | null {
  if (!c) return null;
  let best = -1;
  for (let i = 0; i < c.freqsHz.length; i++) {
    const f = c.freqsHz[i]!;
    if (f < lo || f > hi || c.valid[i] === false || !Number.isFinite(c.magDb[i]!)) continue;
    if (best < 0 || c.magDb[i]! > c.magDb[best]!) best = i;
  }
  return best < 0 ? null : c.freqsHz[best]!;
}

/** Who may play the generator: the reason not, or "" (S11; the preflight says it too). */
export function playBlocker(readOnlyReason: string, hubRole: string): string {
  if (readOnlyReason) return `${readOnlyReason}: measuring plays the DSPi's generator, so only the passive microphone RTA and the documents are offered.`;
  if (hubRole === "viewer") return "This connection has the hub's Viewer role: measuring needs Control. Only the passive microphone RTA and the documents are offered.";
  return "";
}
