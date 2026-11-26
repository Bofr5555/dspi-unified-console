// WP ROLEW: the speaker layout on the web (MPLAN section 2.2): which output drives what, as the
// person confirmed it. The core keeps it (`speakerLayout*`, WP ROLE0) for its lifetime only, since
// the web has no files, so this page keeps a copy per serial in its own storage and sets it again
// when it opens the device. What is shown, without the page:
//
// - the stored layout, else the core's proposal (from the filters; never saved by itself);
// - whether the stored layout is confirmed, and whether it came with an imported configuration
//   (review RA F1: an import is always kept unconfirmed; this page tells it from its own writes by
//   the layout's stamp);
// - a copy this page could not set again or keep, in words (review RA: a failed migration is shown);
// - the Measure setup's rows pre-filled from it, and an edit there written back.
//
// No maths of the roles here: the core infers them and judges the tweeter rule (an output with an
// active high-pass at 1 kHz or above is a tweeter for safety, whatever its role).

import type { KeyValueStore } from "../../../fleet/local";
import type { MeasurePlacement, MeasureRole } from "../../../core/measure-protocol";
import { LAYOUT_UNCONFIRMED_PREFIX } from "../../../ui/texts";

/** One output of the layout (`dspi_core::SpeakerLayoutOutput`). */
export interface SpeakerLayoutOutputRecord {
  /** 0-based. */
  output: number;
  role: MeasureRole;
  speaker: string;
  group: number;
  // -- WP ADETW -- (AUTODES 3.1) absent in a copy kept before the speaker detection: the core reads
  // it as the person's in a confirmed layout, else as a suggestion
  /** Where its role comes from. */
  source?: RoleSource;
}

// -- WP ADETW -- the speaker detection's part of the layout (dspi_core speaker_layout.rs, WP ADET0)

/** `dspi_core::RoleSource`. */
export type RoleSource = "user" | "detected" | "suggested" | "stated"; // -- WP ROLES -- stated
/** `dspi_core::LayoutState`. */
export type LayoutState = "unconfirmed" | "autoDetected" | "confirmed" | "stated"; // -- WP ROLES -- stated
/** `dspi_core::DetectClass`. */
export type DetectClass = "tweeter" | "midrange" | "woofer" | "sub" | "fullRange" | "nothing" | "unknown" | "off";
/** `dspi_core::DetectConfidence`. */
export type DetectConfidence = "high" | "medium" | "low";
/** `dspi_core::DetectHeard`. */
export type DetectHeard = "top" | "aboveFloor" | "belowFloor" | "never";

/** `dspi_core::DetectedOutput`: one output's detection and its evidence. */
export interface DetectedOutputRecord {
  /** 0-based. */
  output: number;
  class: DetectClass;
  confidence: DetectConfidence;
  /** null for `nothing` and `off`. */
  role: MeasureRole | null;
  floorHz: number;
  heard: DetectHeard;
  levelDbfs: number | null;
  topFallDb: number | null;
  lowTurnDb: number | null;
  fLowHz: number | null;
  lowestHz: number | null;
  stoppedAtHz: number | null;
  /** A notice key (`detect.nothing`, ...). */
  note: string | null;
  probedMs: number;
}

/** `dspi_core::Detection`: the last speaker detection on the device. */
export interface DetectionRecord {
  atMs: number;
  /** Who ran it: an agent's label, "" for the app. */
  by: string;
  placement: MeasurePlacement;
  floorHz: number;
  microphone: string;
  outputs: DetectedOutputRecord[];
  stopped: boolean;
  summary: string;
  deviceOutputs: number;
}

// -- WP ROLES -- roles an AI agent set from the person's words (owner 2026-11-23)

/** `dspi_core::RoleCheckVerdict`: the top-fall check of a stated non-tweeter. */
export type RoleCheckVerdict = "topFalls" | "silent" | "topHolds" | "unfinished";

/** `dspi_core::RoleCheck`. */
export interface RoleCheckRecord {
  output: number;
  /** The stated role it checked. */
  role: MeasureRole;
  verdict: RoleCheckVerdict;
  topFallDb: number | null;
  floorHz: number;
  atMs: number;
  /** -- WP INTD -- unfinished because the output's own filters hide its top end (kept when true). */
  filtersHideTop?: boolean;
}

/** `dspi_core::Statement`: the person's words an agent set the roles from. */
export interface StatementRecord {
  atMs: number;
  /** The agent's label ("AI · Claude Code"). */
  by: string;
  words: string;
  whatChanged: string;
  deviceOutputs: number;
  checks: RoleCheckRecord[];
  /**
   * -- WP INTD -- (REVDFIX B1, the ratchet) the person's own rows the statement replaced: a tweeter
   * among them stays a tweeter for agents until the person changes its row. Kept in the page's copy
   * so a fresh core gets it back with the statement (absent when there are none).
   */
  replaced?: SpeakerLayoutOutputRecord[];
}

/** `dspi_core::SpeakerLayout`. */
export interface SpeakerLayoutRecord {
  /** Set by the core; "" in an exported copy. */
  serial: string;
  outputs: SpeakerLayoutOutputRecord[];
  confirmedByUser: boolean;
  /** When it was set, ms since the epoch; 0 for a proposal. */
  updatedMs: number;
  notes: string[];
  // -- WP ADETW --
  /** The last speaker detection, if one ran. */
  detected?: DetectionRecord | null;
  // -- WP ROLES --
  /** The person's words an agent set the roles from, if it did. */
  stated?: StatementRecord | null;
}

declare module "../../../core/protocol" {
  interface CoreMethods {
    speakerLayout: { params: undefined; result: SpeakerLayoutRecord | null };
    speakerLayoutPropose: { params: undefined; result: SpeakerLayoutRecord };
    speakerLayoutSet: { params: { layout: SpeakerLayoutRecord }; result: SpeakerLayoutRecord };
  }
}

/** What the page calls (a session's core, or a test's). */
export interface LayoutCaller {
  call(method: "speakerLayout", params: undefined): Promise<SpeakerLayoutRecord | null>;
  call(method: "speakerLayoutPropose", params: undefined): Promise<SpeakerLayoutRecord>;
  call(method: "speakerLayoutSet", params: { layout: SpeakerLayoutRecord }): Promise<SpeakerLayoutRecord>;
}

// ---- words ----

export const ROLE_WORDS: Record<MeasureRole, string> = {
  full: "Full range",
  sub: "Subwoofer",
  low: "Low (woofer)",
  mid: "Mid",
  high: "High (tweeter)",
};

export const SAVED_TO_LAYOUT = "Saved to your speaker layout.";
export const TWEETER_RULE = "An output with an active high-pass at 1 kHz or above is measured as a tweeter, whatever its role here.";

/** The notice key of a measurement refused for want of a confirmed layout (WP ROLE0). */
export const LAYOUT_UNCONFIRMED = "measure.layout_unconfirmed";

/**
 * The refusal for want of a confirmed layout. It comes as `invalid` with the key as its message's
 * prefix ("invalid argument: measure.layout_unconfirmed: …"), so the prefix is matched, never the
 * whole message (review RA).
 */
export function isLayoutUnconfirmed(e: unknown): boolean {
  const msg = e instanceof Error ? e.message : typeof e === "string" ? e : "";
  return LAYOUT_UNCONFIRMED_PREFIX.test(msg);
}

// ---- this page's copy (the web has no files) ----

const STORE_KEY = "dspi.speakerLayouts";

/** The copy of one device's layout: as the core last answered, and whether it came with an import. */
export interface KeptLayout {
  layout: SpeakerLayoutRecord;
  imported: boolean;
}

function readAll(storage: KeyValueStore | null): Record<string, KeptLayout> {
  try {
    const v = JSON.parse(storage?.getItem(STORE_KEY) ?? "{}") as unknown;
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, KeptLayout>) : {};
  } catch {
    return {};
  }
}

function tidyOutput(o: unknown): SpeakerLayoutOutputRecord | null {
  const x = (o ?? {}) as Partial<SpeakerLayoutOutputRecord>;
  if (!Number.isInteger(x.output) || x.output! < 0 || x.output! > 15) return null;
  if (typeof x.role !== "string" || !(x.role in ROLE_WORDS)) return null;
  const group = Number.isInteger(x.group) && x.group! >= 0 && x.group! <= 255 ? x.group! : 0;
  const row: SpeakerLayoutOutputRecord = { output: x.output!, role: x.role, speaker: typeof x.speaker === "string" ? x.speaker : "", group };
  // -- WP ADETW -- the row's source kept (a copy from before has none)
  if (x.source === "user" || x.source === "detected" || x.source === "suggested" || x.source === "stated") row.source = x.source; // -- WP ROLES -- stated
  return row;
}

// -- WP ADETW -- a kept detection, checked field by field (the core refuses a malformed one, and
// the whole layout with it): what does not read is dropped, a detection without outputs too
const CLASSES: readonly DetectClass[] = ["tweeter", "midrange", "woofer", "sub", "fullRange", "nothing", "unknown", "off"];
const CONFIDENCES: readonly DetectConfidence[] = ["high", "medium", "low"];
const HEARD: readonly DetectHeard[] = ["top", "aboveFloor", "belowFloor", "never"];
const PLACEMENTS: readonly MeasurePlacement[] = ["listeningPosition", "multiPosition", "nearField", "headphones"];

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function oneOf<T extends string>(v: unknown, all: readonly T[], or: T): T {
  return all.includes(v as T) ? (v as T) : or;
}

function tidyDetected(o: unknown): DetectedOutputRecord | null {
  const x = (o ?? {}) as Partial<Record<keyof DetectedOutputRecord, unknown>>;
  if (!Number.isInteger(x.output) || (x.output as number) < 0 || (x.output as number) > 15) return null;
  return {
    output: x.output as number,
    class: oneOf(x.class, CLASSES, "unknown"),
    confidence: oneOf(x.confidence, CONFIDENCES, "low"),
    role: typeof x.role === "string" && x.role in ROLE_WORDS ? (x.role as MeasureRole) : null,
    floorHz: num(x.floorHz) ?? 0,
    heard: oneOf(x.heard, HEARD, "never"),
    levelDbfs: num(x.levelDbfs),
    topFallDb: num(x.topFallDb),
    lowTurnDb: num(x.lowTurnDb),
    fLowHz: num(x.fLowHz),
    lowestHz: num(x.lowestHz),
    stoppedAtHz: num(x.stoppedAtHz),
    note: typeof x.note === "string" ? x.note : null,
    probedMs: num(x.probedMs) ?? 0,
  };
}

/** A kept detection as the core takes it; null when there is none or it does not read. */
export function tidyDetection(d: unknown): DetectionRecord | null {
  const x = (d ?? {}) as Partial<Record<keyof DetectionRecord, unknown>>;
  if (!d || typeof d !== "object" || !Array.isArray(x.outputs)) return null;
  const outputs = x.outputs.map(tidyDetected).filter((o): o is DetectedOutputRecord => !!o);
  if (!outputs.length) return null;
  const n = num(x.deviceOutputs);
  return {
    atMs: num(x.atMs) ?? 0,
    by: typeof x.by === "string" ? x.by : "",
    placement: oneOf(x.placement, PLACEMENTS, "listeningPosition"),
    floorHz: num(x.floorHz) ?? 0,
    microphone: typeof x.microphone === "string" ? x.microphone : "",
    outputs,
    stopped: x.stopped === true,
    summary: typeof x.summary === "string" ? x.summary : "",
    deviceOutputs: n !== null && Number.isInteger(n) && n >= 0 && n <= 255 ? n : 0,
  };
}

// -- WP ROLES -- a kept statement, checked as the detection is
const VERDICTS: readonly RoleCheckVerdict[] = ["topFalls", "silent", "topHolds", "unfinished"];

/** A kept statement as the core takes it; null when there is none or it does not read. */
export function tidyStatement(s: unknown): StatementRecord | null {
  const x = (s ?? {}) as Partial<Record<keyof StatementRecord, unknown>>;
  if (!s || typeof s !== "object" || typeof x.words !== "string") return null;
  const n = num(x.deviceOutputs);
  const checks = (Array.isArray(x.checks) ? x.checks : [])
    .map((c): RoleCheckRecord | null => {
      const y = (c ?? {}) as Partial<Record<keyof RoleCheckRecord, unknown>>;
      if (!Number.isInteger(y.output) || (y.output as number) < 0 || (y.output as number) > 15) return null;
      if (typeof y.role !== "string" || !(y.role in ROLE_WORDS)) return null;
      return {
        output: y.output as number,
        role: y.role as MeasureRole,
        verdict: oneOf(y.verdict, VERDICTS, "unfinished"),
        topFallDb: num(y.topFallDb),
        floorHz: num(y.floorHz) ?? 0,
        atMs: num(y.atMs) ?? 0,
        ...(y.filtersHideTop === true ? { filtersHideTop: true } : {}), // -- WP INTD --
      };
    })
    .filter((c): c is RoleCheckRecord => !!c);
  // -- WP INTD -- (REVDFIX B1) the rows it replaced, each checked as a layout's row; one that does not read is dropped
  const replaced = (Array.isArray(x.replaced) ? x.replaced : []).map(tidyOutput).filter((o): o is SpeakerLayoutOutputRecord => !!o);
  return {
    atMs: num(x.atMs) ?? 0,
    by: typeof x.by === "string" ? x.by : "",
    words: x.words,
    whatChanged: typeof x.whatChanged === "string" ? x.whatChanged : "",
    deviceOutputs: n !== null && Number.isInteger(n) && n >= 0 && n <= 255 ? n : 0,
    ...(replaced.length ? { replaced } : {}),
    checks,
  };
}

/** `serial`'s copy, checked; null when there is none or it does not read. */
export function loadKept(storage: KeyValueStore | null, serial: string): KeptLayout | null {
  if (!serial) return null;
  const k = readAll(storage)[serial] as Partial<KeptLayout> | undefined;
  const l = k?.layout as Partial<SpeakerLayoutRecord> | undefined;
  if (!l || !Array.isArray(l.outputs)) return null;
  const outputs = l.outputs.map(tidyOutput);
  if (outputs.some((o) => !o)) return null;
  return {
    layout: {
      serial,
      outputs: outputs as SpeakerLayoutOutputRecord[],
      confirmedByUser: l.confirmedByUser === true,
      updatedMs: typeof l.updatedMs === "number" ? l.updatedMs : 0,
      notes: [],
      detected: tidyDetection(l.detected), // -- WP ADETW --
      stated: tidyStatement(l.stated), // -- WP ROLES --
    },
    imported: k!.imported === true,
  };
}

/** Keep `kept` as `serial`'s copy; the reason it could not be kept, or "". */
export function saveKept(storage: KeyValueStore | null, serial: string, kept: KeptLayout): string {
  if (!serial) return "";
  if (!storage) return KEEP_REFUSED;
  const all = readAll(storage);
  all[serial] = kept;
  try {
    storage.setItem(STORE_KEY, JSON.stringify(all));
    return "";
  } catch {
    return KEEP_REFUSED;
  }
}

export const KEEP_REFUSED = "This browser does not keep the speaker layout (its storage is off or full): it holds until the page is closed.";

// ---- the layout as the page shows it ----

export interface LayoutView {
  /** The layout the core keeps for the device, null for none. */
  stored: SpeakerLayoutRecord | null;
  /** What the filters suggest; null when the core could not say. */
  proposal: SpeakerLayoutRecord | null;
  /** The stored layout came with an imported configuration and is not confirmed yet. */
  imported: boolean;
  /** A copy this page could not set again or keep, in words; "" when all is well. */
  problem: string;
}

export const NO_VIEW: LayoutView = { stored: null, proposal: null, imported: false, problem: "" };

function words(e: unknown): string {
  const m = e instanceof Error ? e.message : String(e);
  return m ? m[0]!.toUpperCase() + m.slice(1) : m;
}

/**
 * The open device's layout: the core's, else this page's copy set again (it holds for the core's
 * lifetime only), with the proposal beside it. A layout the core has that this page did not write
 * and that is not confirmed came with an import.
 */
export async function loadLayout(core: LayoutCaller, serial: string, storage: KeyValueStore | null): Promise<LayoutView> {
  const kept = loadKept(storage, serial);
  let problem = "";
  let stored = await core.call("speakerLayout", undefined);
  let imported: boolean;
  if (!stored && kept) {
    try {
      stored = await core.call("speakerLayoutSet", { layout: { ...kept.layout, serial } });
    } catch (e) {
      problem = `Your saved speaker layout could not be set again: ${words(e)}`;
    }
    imported = !!stored && kept.imported && !stored.confirmedByUser;
  } else if (stored && kept && kept.layout.updatedMs === stored.updatedMs) {
    imported = kept.imported && !stored.confirmedByUser;
  } else {
    imported = !!stored && !stored.confirmedByUser;
  }
  if (stored) {
    const why = saveKept(storage, serial, { layout: stored, imported });
    problem ||= why;
  }
  const proposal = await core.call("speakerLayoutPropose", undefined).catch(() => null);
  return { stored, proposal, imported, problem };
}

// -- WP ADETW --
/**
 * The open device's layout as it stands, read only (the main screen's line): the core's, else this
 * page's copy (the session's open sets it again; this never writes, so it cannot race that). The
 * copy is read after the core answers, so a copy just set again is the one compared.
 */
export async function peekLayout(core: Pick<LayoutCaller, "call">, serial: string, storage: KeyValueStore | null): Promise<LayoutView> {
  const stored = await core.call("speakerLayout", undefined);
  const kept = loadKept(storage, serial);
  if (!stored) return { ...NO_VIEW, stored: kept?.layout ?? null, imported: !!kept && kept.imported && !kept.layout.confirmedByUser };
  const imported = !stored.confirmedByUser && (kept && kept.layout.updatedMs === stored.updatedMs ? kept.imported : true);
  return { ...NO_VIEW, stored, imported };
}

/** Keep `layout` as the device's, confirmed or not, and this page's copy of it. */
export async function keepLayout(core: LayoutCaller, serial: string, storage: KeyValueStore | null, layout: SpeakerLayoutRecord, confirmed: boolean, was: LayoutView): Promise<LayoutView> {
  const stored = await core.call("speakerLayoutSet", { layout: { ...layout, serial, confirmedByUser: confirmed, notes: [] } });
  const imported = was.imported && !stored.confirmedByUser;
  return { ...was, stored, imported, problem: saveKept(storage, serial, { layout: stored, imported }) };
}

/** The layout the rows start from: the stored one, else the proposal. */
export function baseOf(v: LayoutView): SpeakerLayoutRecord | null {
  return v.stored ?? v.proposal;
}

/**
 * `layout` with output `o`'s row changed (added when it has none), in output order. A row the
 * person adds or gives another role is theirs (WP ADETW, AUTODES 3.1: the detection still stands
 * for the other rows).
 */
export function withRow(layout: SpeakerLayoutRecord | null, o: number, patch: Partial<Omit<SpeakerLayoutOutputRecord, "output">>): SpeakerLayoutRecord {
  const base: SpeakerLayoutRecord = layout ?? { serial: "", outputs: [], confirmedByUser: false, updatedMs: 0, notes: [] };
  const had = base.outputs.find((x) => x.output === o);
  const row = { ...(had ?? { output: o, role: "full" as MeasureRole, speaker: "", group: 0 }), ...patch };
  if (!patch.source && (!had || (patch.role !== undefined && patch.role !== had.role))) row.source = "user"; // -- WP ADETW --
  const outputs = [...base.outputs.filter((x) => x.output !== o), row].sort((a, b) => a.output - b.output);
  return { ...base, outputs };
}

/** `layout` without output `o`. */
export function withoutRow(layout: SpeakerLayoutRecord, o: number): SpeakerLayoutRecord {
  return { ...layout, outputs: layout.outputs.filter((x) => x.output !== o) };
}

// -- WP ADETW -- the three states and the detection in words (AUTODES 2, 5.1, 5.2)

/**
 * How far `l` can be trusted (`SpeakerLayout::state`): confirmed by the person; else
 * auto-detected when a detection is recorded and no row is a mere suggestion; else unconfirmed.
 */
export function layoutState(l: SpeakerLayoutRecord | null): LayoutState {
  if (!l) return "unconfirmed";
  if (l.confirmedByUser) return "confirmed";
  // A row kept before the detection has no source: the core reads it as a suggestion
  if (!l.outputs.length || l.outputs.some((o) => !o.source || o.source === "suggested")) return "unconfirmed";
  // -- WP ROLES -- set by an agent from the person's words: the least checked
  if (l.stated && l.outputs.some((o) => o.source === "stated")) return "stated";
  if (l.detected) return "autoDetected";
  return "unconfirmed";
}

// -- WP ROLES --
/** The top-fall check of output `o`'s stated role, if one was made for the role it has now. */
export function checkOf(l: SpeakerLayoutRecord | null, o: number): RoleCheckRecord | null {
  const row = l?.outputs.find((x) => x.output === o);
  if (!row) return null;
  return l?.stated?.checks.find((c) => c.output === o && c.role === row.role) ?? null;
}

/** The detection's record of output `o`, if one. */
export function detectedOf(l: SpeakerLayoutRecord | null, o: number): DetectedOutputRecord | null {
  return l?.detected?.outputs.find((x) => x.output === o) ?? null;
}

const PLACED: Record<MeasurePlacement, string> = {
  listeningPosition: "microphone at the listening position",
  multiPosition: "microphone at several positions",
  nearField: "microphone close to each driver",
  headphones: "headphones on a coupler",
};

/** Who ran the detection, as the page shows it: an agent's label, "" for the app's own. */
function whoDetected(d: DetectionRecord): string {
  return d.by.replace(/[\p{Cc}\p{Cf}]/gu, "").trim();
}

/** "23 Nov at 14:02", or "today at 14:02". */
function when(ms: number, now: number): string {
  const d = new Date(ms);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  if (new Date(now).toDateString() === d.toDateString()) return `today at ${time}`;
  return `${d.toLocaleDateString([], { day: "numeric", month: "short" })} at ${time}`;
}

/** A row's marker after its role (AUTODES 5.1): whose word it is, and how sure a detection was. */
export type PillKind =
  | "detected"
  | "detectedUnsure"
  | "uncertain"
  | "unknown"
  | "nothing"
  | "off"
  | "you"
  | "suggested"
  // -- WP ROLES -- set by AI from the person's words; its check found a tweeter's top; not named
  | "stated"
  | "statedTweeter"
  | "notNamed";

export const PILL_WORDS: Record<PillKind, string> = {
  detected: "detected",
  detectedUnsure: "detected?",
  uncertain: "uncertain",
  unknown: "unknown, treated as a tweeter",
  nothing: "nothing heard",
  off: "off, not probed",
  you: "you",
  suggested: "suggested",
  stated: "from your words", // -- WP ROLES --
  statedTweeter: "tweeter-like",
  notNamed: "unknown, treated as a tweeter",
};

/** Hover text when the notices are not read yet (the notice's reason is shown when they are). */
const PILL_REASONS: Record<PillKind, string> = {
  detected: "Found by the speaker detection.",
  detectedUnsure: "Found by the speaker detection, not sure: correct it if it is wrong.",
  uncertain: "The detection was not sure; it is treated as a tweeter to be safe.",
  unknown: "The probe did not finish this output; it is treated as a tweeter.",
  nothing: "Nothing reached the microphone from this output: nothing is wired here, or it does not reach the microphone. Left out of the layout.",
  off: "The output was off or muted, so the detection did not probe it.",
  you: "Set or confirmed by you.",
  suggested: "Suggested from the filters and the routing: nobody's word yet.",
  // -- WP ROLES --
  stated: "Set by AI from your words, not checked. Correct it if it is wrong, then confirm.",
  statedTweeter:
    "Set by AI from your words, but its top end holds like a tweeter's: nothing below 1 kHz plays on it for an agent, and it is guarded as a tweeter. Check the wiring and the role.",
  notNamed: "Not in what you told the AI: unknown, treated as a tweeter until you set its role.",
};

export interface Pill {
  kind: PillKind;
  word: string;
  /** The notice whose reason says why (`detect.*`), null for none. */
  note: string | null;
  /** The reason when the notice is not at hand. */
  reason: string;
}

function pill(kind: PillKind, note: string | null = null): Pill {
  return { kind, word: PILL_WORDS[kind], note, reason: PILL_REASONS[kind] };
}

/**
 * Output `o`'s marker in `draft` (the rows as shown) against the layout `stored`: a row the
 * detection left out says why (nothing heard, off); a row says whose word it is. None on a
 * confirmed layout's rows as kept (they are all the person's) or with no detection and no layout.
 */
export function pillOf(draft: SpeakerLayoutRecord | null, stored: SpeakerLayoutRecord | null, o: number): Pill | null {
  const row = draft?.outputs.find((x) => x.output === o);
  const det = detectedOf(draft, o) ?? detectedOf(stored, o);
  if (!row) {
    if (det?.class === "nothing") return pill("nothing", det.note ?? "detect.nothing");
    if (det?.class === "off") return pill("off", det.note ?? "detect.off");
    // -- WP ROLES -- an output the person's words did not name
    if (layoutState(stored) === "stated") return pill("notNamed");
    return null;
  }
  const kept = stored?.outputs.find((x) => x.output === o);
  const unchanged = !!kept && kept.role === row.role && kept.source === row.source;
  if (stored?.confirmedByUser && unchanged) return null;
  if (row.source === "user") return pill("you");
  // -- WP ROLES -- set by AI from the person's words
  if (row.source === "stated" && unchanged) return pill(checkOf(stored, o)?.verdict === "topHolds" ? "statedTweeter" : "stated");
  if (row.source === "detected" && det) {
    if (det.class === "unknown") return pill("unknown", det.note ?? "detect.unknown");
    if (det.confidence === "high") return pill("detected", det.note);
    return pill(det.confidence === "medium" ? "detectedUnsure" : "uncertain", det.note);
  }
  if (!stored && !draft?.detected) return null;
  return pill("suggested");
}

/** The detection's evidence of one output, in words, for the details table. */
export function evidenceWords(d: DetectedOutputRecord): { heard: string; topFall: string; fLow: string; lowest: string } {
  const heard: Record<DetectHeard, string> = { top: "4–16 kHz", aboveFloor: "above the floor", belowFloor: "below the floor", never: "never" };
  const hz = (v: number | null) => (v === null ? "—" : v >= 1000 ? `${+(v / 1000).toFixed(2)} kHz` : `${Math.round(v)} Hz`);
  return {
    heard: d.class === "off" ? "not probed" : heard[d.heard],
    topFall: d.topFallDb === null ? "—" : `${d.topFallDb.toFixed(1)} dB`,
    fLow: hz(d.fLowHz),
    lowest: hz(d.lowestHz),
  };
}

export const CLASS_WORDS: Record<DetectClass, string> = {
  tweeter: "Tweeter",
  midrange: "Midrange",
  woofer: "Woofer",
  sub: "Subwoofer",
  fullRange: "Full range",
  nothing: "Nothing heard",
  unknown: "Unknown, treated as a tweeter",
  off: "Off, not probed",
};

/** What the banner says, elsewhere (`text`) and on Settings › Speakers (`here`); null when the layout is confirmed. */
export function bannerOf(
  v: LayoutView,
  now = Date.now(),
): { kind: "autoDetected" | "stated" | "imported" | "unconfirmed" | "none"; text: string; here: string } | null {
  if (v.stored?.confirmedByUser) return null;
  // -- WP ROLES -- (owner 2026-11-23) set by AI from the person's words, not checked
  const s = v.stored?.stated;
  if (s && layoutState(v.stored) === "stated") {
    const who = s.by.replace(/[\p{Cc}\p{Cf}]/gu, "").replace(/^AI · /, "").trim() || "An AI agent";
    const at = s.atMs > 0 ? ` ${when(s.atMs, now)}` : "";
    const words = s.words.replace(/\s+/g, " ").trim();
    const quoted = words.length > 160 ? `${words.slice(0, 159)}…` : words;
    return {
      kind: "stated",
      text: "Your speaker roles were set by AI from your words and are not checked. Check them in Settings › Speakers.",
      here: `Set by AI from your words, not checked: ${who} set these roles${at} from what you told it (“${quoted}”). Check each role, correct what is wrong, then confirm.`,
    };
  }
  // -- WP ADETW -- detected, not confirmed (AUTODES 5.1): its roles guard the outputs meanwhile
  const d = v.stored?.detected;
  if (d && layoutState(v.stored) === "autoDetected") {
    const who = whoDetected(d);
    const at = d.atMs > 0 ? ` ${when(d.atMs, now)}` : "";
    const lead = who ? `Detected by AI, not confirmed: ${who} probed` : "Detected, not confirmed: the speaker detection probed";
    return {
      kind: "autoDetected",
      text: who ? "Your speaker layout was detected by AI and is not confirmed. Check it in Settings › Speakers." : "Your speaker layout was detected and is not confirmed. Check it in Settings › Speakers.",
      here: `${lead} this DSPi's outputs${at} (${PLACED[d.placement] ?? PLACED.listeningPosition}). Check each role, correct what is wrong, then confirm.`,
    };
  }
  // -- WP MCP5 -- (review RB C4) "imported" is any unconfirmed layout this app did not
  // write: a configuration file or another app (never an agent: no tool writes it)
  if (v.stored && v.imported)
    return {
      kind: "imported",
      text: "Imported speaker layout: these roles were set outside this app (a configuration file or another app). Check that they match this DSPi's speakers, then confirm them in Settings › Speakers.",
      here: "Imported speaker layout: these roles were set outside this app (a configuration file or another app). Check that they match this DSPi's speakers, then confirm them.",
    };
  if (v.stored)
    return {
      kind: "unconfirmed",
      text: "Your speaker layout is not confirmed yet. Check which output drives what, then confirm it in Settings › Speakers.",
      here: "Not confirmed yet: check which output drives what, then confirm.",
    };
  return {
    kind: "none",
    text: "No speaker layout yet: the roles are suggested from the filters. Confirm which output drives what in Settings › Speakers.",
    here: "Suggested from the filters and the routing: check which output drives what, then confirm.",
  };
}

/** When the layout was confirmed, detected or set, in words; "" for a proposal. */
export function stampWords(l: SpeakerLayoutRecord | null, now = Date.now()): string {
  // -- WP ROLES --
  if (l?.stated && layoutState(l) === "stated")
    return l.stated.atMs > 0 ? `Set by AI from your words, not checked, ${stampAt(l.stated.atMs, now)}.` : "Set by AI from your words, not checked.";
  // -- WP ADETW -- an auto-detected layout says when it was detected, and by whom
  if (l?.detected && layoutState(l) === "autoDetected") {
    const what = whoDetected(l.detected) ? "Detected by AI, not confirmed" : "Detected, not confirmed";
    return l.detected.atMs > 0 ? `${what}, ${stampAt(l.detected.atMs, now)}.` : `${what}.`;
  }
  if (!l || !(l.updatedMs > 0)) return "";
  return `${l.confirmedByUser ? "Confirmed" : "Kept, not confirmed,"} ${stampAt(l.updatedMs, now)}.`;
}

function stampAt(ms: number, now: number): string {
  const d = new Date(ms);
  if (new Date(now).toDateString() === d.toDateString()) return `today at ${d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  return `on ${d.toLocaleDateString()}`;
}
