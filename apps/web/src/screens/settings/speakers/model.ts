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
import type { MeasureRole } from "../../../core/measure-protocol";
import { LAYOUT_UNCONFIRMED_PREFIX } from "../../../ui/texts";

/** One output of the layout (`dspi_core::SpeakerLayoutOutput`). */
export interface SpeakerLayoutOutputRecord {
  /** 0-based. */
  output: number;
  role: MeasureRole;
  speaker: string;
  group: number;
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
  return { output: x.output!, role: x.role, speaker: typeof x.speaker === "string" ? x.speaker : "", group };
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

/** `layout` with output `o`'s row changed (added when it has none), in output order. */
export function withRow(layout: SpeakerLayoutRecord | null, o: number, patch: Partial<Omit<SpeakerLayoutOutputRecord, "output">>): SpeakerLayoutRecord {
  const base: SpeakerLayoutRecord = layout ?? { serial: "", outputs: [], confirmedByUser: false, updatedMs: 0, notes: [] };
  const had = base.outputs.find((x) => x.output === o);
  const row = { ...(had ?? { output: o, role: "full" as MeasureRole, speaker: "", group: 0 }), ...patch };
  const outputs = [...base.outputs.filter((x) => x.output !== o), row].sort((a, b) => a.output - b.output);
  return { ...base, outputs };
}

/** `layout` without output `o`. */
export function withoutRow(layout: SpeakerLayoutRecord, o: number): SpeakerLayoutRecord {
  return { ...layout, outputs: layout.outputs.filter((x) => x.output !== o) };
}

/** What the banner says, elsewhere (`text`) and on Settings › Speakers (`here`); null when the layout is confirmed. */
export function bannerOf(v: LayoutView): { kind: "imported" | "unconfirmed" | "none"; text: string; here: string } | null {
  if (v.stored?.confirmedByUser) return null;
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

/** When the layout was confirmed or set, in words; "" for a proposal. */
export function stampWords(l: SpeakerLayoutRecord | null, now = Date.now()): string {
  if (!l || !(l.updatedMs > 0)) return "";
  const d = new Date(l.updatedMs);
  const today = new Date(now).toDateString() === d.toDateString();
  const at = today ? d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" }) : d.toLocaleDateString();
  return `${l.confirmedByUser ? "Confirmed" : "Kept, not confirmed,"} ${today ? "today at " : "on "}${at}.`;
}
