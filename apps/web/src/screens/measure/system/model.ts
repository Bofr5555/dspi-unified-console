// WP ATWEB: the rules and words of the "Several units" step (PLAN-AT 1.4 to 1.6): the placement
// the person must choose, the setup the core gets, a phase in words, an issue or a refusal in
// words through the core's notice table, what the proposal lets this role do. Every judgement is
// the core's; this file only words it and gathers the form.

import { CoreCallError, type MeasureSystemArrivalRecord, type MeasureSystemIssueRecord, type MeasureSystemProposalRecord, type MeasureSystemSetupRecord, type MeasureSystemStateRecord, type NoticeRecord } from "../../../core/protocol";
import type { MeasurePlacement, MeasureRole } from "../../../core/measure-protocol";
import { noticeRow } from "../../../ui/texts";

/** The second Keep after a failed verify counts from this long after the core's refusal, ms. */
export const CONFIRM_MS = 1000;

/** The notice keys this step shows (assets/notices/notices.json; ATWEB's rows and ATCORE's). */
export const KEYS = {
  needsHub: "measure.system.needs_hub",
  notOneDomain: "measure.system.not_one_domain",
  notLocked: "measure.system.not_locked",
  noTimeline: "measure.system.no_timeline",
  runs: "measure_system_runs",
  placement: "measure_system_placement",
  admin: "measure.system.admin",
  verifyFailed: "measure.system.verify_failed",
  lostLock: "measure.system.abort.disturbance",
} as const;

/** The hub's refusals of `system.measure.open` (the Link error's notice key) and the step's own rows. */
const LINK_KEYS: Record<string, string> = {
  "dlink.not_one_domain": KEYS.notOneDomain,
  "dlink.not_locked": KEYS.notLocked,
  "dlink.no_timeline": KEYS.noTimeline,
};

/** Where the microphone is; "" until the person chose (no placement is assumed). */
export type SystemPlacement = MeasurePlacement | "";

export const SYSTEM_PLACEMENTS: { id: MeasurePlacement; label: string }[] = [
  { id: "listeningPosition", label: "Listening position (speakers)" },
  { id: "multiPosition", label: "Several positions" },
  { id: "nearField", label: "Near-field" },
  { id: "headphones", label: "Headphones on a coupler" },
];

/**
 * Why the placement stops every run, or "". The coordinator's headphone-safety decision: nothing
 * plays before the person said where the microphone is; units are measured together with their
 * speakers at the listening position only (the core plans every unit there).
 */
export function placementProblem(p: SystemPlacement): string {
  if (!p) return "Choose where the microphone is first: nothing plays before you have.";
  if (p === "headphones") return "Headphones are never measured across units: measure them one unit at a time on the Measure tab, at headphone levels.";
  if (p !== "listeningPosition") return "Units are measured together at the listening position only: put the microphone there, or measure one unit at a time on the Measure tab.";
  return "";
}

/** One unit of the form: its serial and the outputs chosen on it. */
export interface UnitDraft {
  serial: string;
  outputs: { output: number; role: MeasureRole }[];
}

export interface SystemDraft {
  placement: SystemPlacement;
  units: UnitDraft[];
  /** One sweep, seconds (1 to 15). */
  sweepS: number;
  /** Passes, 2 to 8; 0: the plan's (5 where the run fits one capture, else 3). */
  passes: number;
  /** The anchor output for chained runs, as "serial:output"; "": the first unit's first output. */
  anchor: string;
  name: string;
}

export function initialDraft(): SystemDraft {
  return { placement: "", units: [], sweepS: 2, passes: 0, anchor: "", name: "" };
}

/** What keeps the form from being checked, or "": the placement, two units, an output on each. */
export function draftProblem(d: SystemDraft): string {
  const p = placementProblem(d.placement);
  if (p) return p;
  if (d.units.length < 2) return "Choose at least two units: one unit is measured on the Measure tab.";
  const empty = d.units.find((u) => !u.outputs.length);
  if (empty) return `Choose at least one output of ${empty.serial}.`;
  if (!(d.sweepS >= 1 && d.sweepS <= 15)) return "The sweep is 1 to 15 seconds.";
  if (d.passes !== 0 && !(d.passes >= 2 && d.passes <= 8)) return "Passes are 2 to 8 (or the plan's).";
  return "";
}

/** The core's setup from the form. */
export function setupOf(d: SystemDraft, captureRateHz: number, micDocument = ""): MeasureSystemSetupRecord {
  const [aSerial, aOut] = d.anchor ? d.anchor.split(":") : ["", ""];
  return {
    units: d.units.map((u) => ({ serial: u.serial, outputs: u.outputs.map((o) => ({ output: o.output, role: o.role })) })),
    passes: d.passes,
    anchorSerial: aSerial ?? "",
    anchorOutput: aOut ? Number(aOut) : null,
    micDocument,
    sweepMs: Math.round(d.sweepS * 1000),
    captureRateHz,
    name: d.name,
  };
}

/** Toggle unit `serial` in the form (its first output chosen when added). */
export function withUnit(d: SystemDraft, serial: string, on: boolean): SystemDraft {
  if (!on) return { ...d, units: d.units.filter((u) => u.serial !== serial), anchor: d.anchor.startsWith(`${serial}:`) ? "" : d.anchor };
  if (d.units.some((u) => u.serial === serial)) return d;
  return { ...d, units: [...d.units, { serial, outputs: [{ output: 0, role: "full" }] }] };
}

/** Toggle output `o` of unit `serial`. */
export function withOutput(d: SystemDraft, serial: string, o: number, on: boolean): SystemDraft {
  return {
    ...d,
    units: d.units.map((u) => {
      if (u.serial !== serial) return u;
      const rest = u.outputs.filter((x) => x.output !== o);
      return { ...u, outputs: on ? [...rest, { output: o, role: "full" as MeasureRole }].sort((a, b) => a.output - b.output) : rest };
    }),
  };
}

export function withRole(d: SystemDraft, serial: string, o: number, role: MeasureRole): SystemDraft {
  return { ...d, units: d.units.map((u) => (u.serial !== serial ? u : { ...u, outputs: u.outputs.map((x) => (x.output === o ? { ...x, role } : x)) })) };
}

type Row = (key: string) => NoticeRecord | null;

function sentence(s: string): string {
  return s ? s[0]!.toUpperCase() + s.slice(1) : s;
}

/** An issue in words: its notice's title, then the core's words (which name the unit). */
export function issueWords(i: Pick<MeasureSystemIssueRecord, "key" | "message">, row: Row = noticeRow): string {
  const title = row(i.key)?.title;
  const msg = sentence(i.message);
  return title && title !== msg ? `${title}: ${msg}` : msg;
}

/** The notice key a refusal stands for ("" none): the hub's open refusals map onto the step's rows. */
export function refusalKey(e: unknown): string {
  if (!(e instanceof CoreCallError)) return "";
  const k = e.noticeKey ?? "";
  return LINK_KEYS[k] ?? k;
}

/** A refusal in words (a hub's, the core's): its notice's title, then the words the core gave. */
export function refusalWords(e: unknown, row: Row = noticeRow): { key: string; text: string } {
  const key = refusalKey(e);
  const raw = e instanceof Error ? e.message : String(e);
  // The core's prefixes say nothing to a person.
  const msg = sentence(raw.replace(/^(invalid argument|DSPi Link): /, ""));
  const title = key ? row(key)?.title : undefined;
  return { key, text: title && title !== msg ? `${title}: ${msg}` : msg };
}

/**
 * A unit's time base moving ends a run; the hub may see it first, in its own `status` or on its
 * own tick, and the core then reports the hub's stop (`abort.hub`) or, for a `status` the hub
 * answered `aborted`, the hub as gone (`abort.disconnected`) with the hub's words. Those words
 * name the unit and its time base: shown as the unit having lost its lock, as they are.
 */
/**
 * The issue a run ended with, in words, under its notice. WP ATDEMO: the core names a unit whose
 * time base moved itself (`abort.disturbance` from the hub's `aborted` answer and event), so the
 * key is the core's.
 */
export function shownIssue(i: Pick<MeasureSystemIssueRecord, "key" | "message">, row: Row = noticeRow): { key: string; text: string } {
  return { key: i.key, text: issueWords(i, row) };
}

const PURPOSE: Record<string, string> = { measure: "Measuring", sum_check: "The summed check", verify: "Verifying" };

/** Where the session is, in words; the settle wait in the core's words (`words`). */
export function phaseWords(st: MeasureSystemStateRecord | null): string {
  if (!st || st.phase === "idle") return "";
  const what = PURPOSE[st.purpose] ?? "Measuring";
  const run = st.runs > 1 ? ` (run ${Math.max(1, st.run)} of ${st.runs})` : "";
  const wait = st.words ? `: ${st.words}` : "";
  switch (st.phase) {
    case "pre_roll":
      return `${what}: listening to the room first, nothing plays${wait}`;
    case "ramp":
      return `${what}: finding the level of ${st.serial || "a unit"} output ${(st.output ?? 0) + 1}${st.levelDb != null ? ` (${st.levelDb.toFixed(1)} dBFS)` : ""}${wait}`;
    case "run":
      return `${what}: every unit's generator plays on the shared time base${run}${wait}`;
    case "analysing":
      return `${what}: analysing${run}`;
    case "done":
      return `${what}: finished.`;
    case "aborted":
      return `${what}: stopped.`;
    case "failed":
      return `${what}: could not finish.`;
    default:
      return what;
  }
}

/** "Needs N runs": a group that does not fit one capture is chained through the anchor. */
export function runsWords(st: MeasureSystemStateRecord | null, row: Row = noticeRow): string {
  if (!st || st.runs < 2) return "";
  const r = row(KEYS.runs);
  return `${r?.title ?? "The group needs several runs"}: this group needs ${st.runs} runs, chained through the anchor output. ${r?.action ?? "Keep the microphone and the room still until every run has finished."}`;
}

/** Microseconds as the result shows them. */
export function us(v: number | null | undefined, digits = 1): string {
  return v == null || !Number.isFinite(v) ? "–" : `${v.toFixed(digits)} µs`;
}

export function outputLabel(serial: string, o: number): string {
  return `${serial} output ${o + 1}`;
}

export function arrivalWords(a: MeasureSystemArrivalRecord): string {
  const u = a.uncertaintyUs != null ? ` ± ${us(a.uncertaintyUs)}` : "";
  return `${us(a.arrivalUs)}${u}`;
}

/** What this role may do with the proposal (Admin applies; Control measures, sees and checks). */
export function proposalRights(p: MeasureSystemProposalRecord | null, role: string): { apply: boolean; why: string } {
  if (!p) return { apply: false, why: "" };
  if (!p.offered) return { apply: false, why: p.notice ? issueWords(p.notice) : "Nothing can be applied from this measurement." };
  if (p.sumCheckNeeded) return { apply: false, why: issueWords({ key: "measure.system.sum_check_needed", message: "run the summed check first" }) };
  if (!p.mayApply || role !== "admin") {
    const r = noticeRow(KEYS.admin);
    return { apply: false, why: r ? `${r.title}: ${r.reason}` : "Applying an alignment across units needs the Admin role." };
  }
  return { apply: true, why: "" };
}
