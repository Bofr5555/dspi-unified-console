// WP P1WB2: the Installer page's small rules, apart from its view (PRO1 S1, S2, S4, S5).
//
// - The limits profile (P1LM) is a document of the `.dspipreset` family (`meta.kind =
//   "limits"`, the profile under `limits`). The web has no files: this page keeps each
//   device's document in its own storage, by serial, and gives it to the core
//   (`limitsApply`) whenever that device opens; it can also be downloaded and loaded.
// - A field the installer empties leaves that part as the device allows: it is removed
//   from the profile, never written as 0.
// - The PIN is never here: the web keeps none (the hub keeps a managed device's lock).

import type { LimitFieldRangeRecord } from "../../core/protocol";
import type { KeyValueStore } from "../../fleet/local";
import { noticeRow } from "../../ui/texts";

/** One output's limits (`dspi_session::limits::OutputLimits`). */
export interface OutputLimits {
  /** 0-based. */
  output: number;
  maxGainDb?: number;
  highpassMinHz?: number;
  highpassMinSlopeDb?: number;
  delayMinMs?: number;
  delayMaxMs?: number;
  limiterCeilingDb?: number;
}

/** A device's limits profile (`dspi_session::limits::LimitsProfile`), as the document has it. */
export interface LimitsProfile {
  serial?: string;
  outputs?: OutputLimits[];
  maxPeqBoostDb?: number;
  masterVolumeMaxDb?: number;
  lockedPages?: string[];
  note?: string;
  contact?: string;
}

/** The fields of an output's limits, in the order the page shows them, with their words. Their ranges and units are the core's ([fieldRange]). */
export const OUTPUT_FIELDS: { key: Exclude<keyof OutputLimits, "output">; label: string }[] = [
  { key: "maxGainDb", label: "Highest gain" },
  { key: "limiterCeilingDb", label: "Limiter on, ceiling at most" },
  { key: "highpassMinHz", label: "High-pass at least" },
  { key: "highpassMinSlopeDb", label: "High-pass slope at least" },
  { key: "delayMinMs", label: "Shortest delay" },
  { key: "delayMaxMs", label: "Longest delay" },
];

// -- WP P1EFIX -- one range table for every client's limits editor: the core's
// (`limitsFieldRanges`, review p1e-a F7), read once like the notice table; no copy here.
let ranges: Map<string, LimitFieldRangeRecord> | null = null;
let loadingRanges: Promise<void> | null = null;

/** Read the core's range table, once per page; a failure leaves it unread (asked again next time). */
export function loadFieldRanges(core: { call(method: "limitsFieldRanges", params: undefined): Promise<LimitFieldRangeRecord[]> }): Promise<void> {
  if (ranges) return Promise.resolve();
  loadingRanges ??= core.call("limitsFieldRanges", undefined).then(
    (list) => {
      if (Array.isArray(list)) ranges = new Map(list.map((r) => [r.key, r]));
      else loadingRanges = null;
    },
    () => {
      loadingRanges = null;
    },
  );
  return loadingRanges;
}

/** The range of the profile's field `key`, or null before the table is read (the core clamps all the same). */
export function fieldRange(key: string): LimitFieldRangeRecord | null {
  return ranges?.get(key) ?? null;
}

/** Tests only: forget the table. */
export function resetFieldRanges(): void {
  ranges = null;
  loadingRanges = null;
}

/** The profile in `text`: a limits document, or one carrying it in `meta.limits`. Null when there is none. */
export function parseProfile(text: string): LimitsProfile | null {
  if (!text.trim()) return null;
  try {
    const doc = JSON.parse(text) as { meta?: { kind?: unknown; limits?: unknown }; limits?: unknown };
    const kind = typeof doc.meta?.kind === "string" ? doc.meta.kind.trim().toLowerCase() : "";
    const block = kind === "limits" && doc.limits && typeof doc.limits === "object" ? doc.limits : doc.meta?.limits;
    return block && typeof block === "object" ? (block as LimitsProfile) : null;
  } catch {
    return null;
  }
}

/** `profile` as a limits document (`LimitsProfile::to_document`'s shape). */
export function profileDocument(profile: LimitsProfile): string {
  return JSON.stringify({ schemaVersion: 1, meta: { kind: "limits", name: "Limits profile" }, limits: tidyProfile(profile) }, null, 2);
}

/** Whether `profile` limits anything at all (a note alone does not). */
export function limitsSomething(profile: LimitsProfile | null): boolean {
  if (!profile) return false;
  return !!(profile.outputs?.length || profile.maxPeqBoostDb !== undefined || profile.masterVolumeMaxDb !== undefined || profile.lockedPages?.length);
}

/** `profile` without empty parts: outputs with no limit dropped, empty strings and lists left out. */
export function tidyProfile(profile: LimitsProfile): LimitsProfile {
  const out: LimitsProfile = {};
  if (profile.serial) out.serial = profile.serial;
  const outputs = (profile.outputs ?? []).filter((o) => OUTPUT_FIELDS.some((f) => o[f.key] !== undefined)).sort((a, b) => a.output - b.output);
  if (outputs.length) out.outputs = outputs;
  if (profile.maxPeqBoostDb !== undefined) out.maxPeqBoostDb = profile.maxPeqBoostDb;
  if (profile.masterVolumeMaxDb !== undefined) out.masterVolumeMaxDb = profile.masterVolumeMaxDb;
  if (profile.lockedPages?.length) out.lockedPages = [...profile.lockedPages];
  if (profile.note?.trim()) out.note = profile.note;
  if (profile.contact?.trim()) out.contact = profile.contact;
  return out;
}

/** `profile` with output `output`'s `key` set (`undefined`: no limit there). */
export function withOutputLimit(profile: LimitsProfile, output: number, key: Exclude<keyof OutputLimits, "output">, value: number | undefined): LimitsProfile {
  const outputs = (profile.outputs ?? []).map((o) => ({ ...o }));
  let o = outputs.find((x) => x.output === output);
  if (!o) {
    o = { output };
    outputs.push(o);
  }
  if (value === undefined) delete o[key];
  else o[key] = value;
  return tidyProfile({ ...profile, outputs });
}

/** `profile` with a device-wide field set (`undefined` or "": none). */
export function withField<K extends "maxPeqBoostDb" | "masterVolumeMaxDb" | "note" | "contact">(profile: LimitsProfile, key: K, value: LimitsProfile[K]): LimitsProfile {
  const next = { ...profile };
  if (value === undefined || value === "") delete next[key];
  else next[key] = value;
  return tidyProfile(next);
}

/** A number field's text as a limit: "" (or junk) is no limit; a number is kept within `range` (none: as typed). */
export function limitValue(text: string, range: { min: number; max: number } | null): number | undefined {
  const min = range?.min ?? -Infinity;
  const max = range?.max ?? Infinity;
  const t = text.trim().replace(",", ".");
  if (!t) return undefined;
  const v = Number(t);
  if (!Number.isFinite(v)) return undefined;
  return Math.min(max, Math.max(min, v));
}

/** One output's limit as the field shows it: "" for none. */
export function limitText(v: number | undefined): string {
  return v === undefined ? "" : String(Math.round(v * 100) / 100);
}

// ---------------------------------------------------------------- the page's own copy

const LIMITS_KEY = "dspi-web.limits";

function readAll(storage: KeyValueStore | null): Record<string, string> {
  try {
    const v = JSON.parse(storage?.getItem(LIMITS_KEY) ?? "{}") as unknown;
    return v && typeof v === "object" ? { ...(v as Record<string, string>) } : {};
  } catch {
    return {};
  }
}

/** The limits document this page keeps for `serial`, "" for none. */
export function loadProfile(storage: KeyValueStore | null, serial: string): string {
  const v = readAll(storage)[serial];
  return typeof v === "string" ? v : "";
}

/** Keep `text` as `serial`'s limits document; "" forgets it. Not kept where storage is refused. */
export function saveProfile(storage: KeyValueStore | null, serial: string, text: string): void {
  if (!serial) return;
  const all = readAll(storage);
  if (text) all[serial] = text;
  else delete all[serial];
  try {
    storage?.setItem(LIMITS_KEY, JSON.stringify(all));
  } catch {
    // Not kept: the profile holds for this visit (the core has it).
  }
}

// ---------------------------------------------------------------- power-on (S5)

/** `preset.startup`'s two bytes: mode 0 (`slot`) loads `slot`, 1 the last one used. */
export function startupBytes(lastUsed: boolean, slot: number): { bytes: number[] } {
  return { bytes: [lastUsed ? 1 : 0, Math.max(0, Math.min(9, Math.round(slot)))] };
}

/** The slot "Test power-on" loads: the startup slot, or the one now active when the device starts in the last one used. */
export function powerOnSlot(startupMode: number, defaultSlot: number, activeSlot: number): number {
  return startupMode === 1 ? activeSlot : defaultSlot;
}

/** The words for the storage modes (`vol.master.mode`, `preset.iomode`: 0 on the device, 1 with each preset). */
export const STORAGE_CHOICES = [
  { value: 0, label: "Kept on the device" },
  { value: 1, label: "Saved with each preset" },
];

// ---------------------------------------------------------------- the lock (S4)

// -- WP P1EFIX --
/** The core's notice row for the locked-user banner (`installer_lock.banner`); its reason is the line. */
export const LOCK_BANNER_KEY = "installer_lock.banner";

/** The locked-user banner's line: the core's row (read with the notice table), or its title before the table is read. */
export function bannerLine(): string {
  return noticeRow(LOCK_BANNER_KEY)?.reason || "Locked by the installer";
}

/** The lock in one line, for the Installer page and the locked front page. */
export function lockLine(lock: { pinSet: boolean; heldBy: string; locked: boolean; unlocked: boolean; covers: string[] } | null | undefined): string {
  if (!lock || !lock.pinSet) return "No installer PIN is set: nothing is locked.";
  const where = lock.heldBy === "hub" ? "the hub" : "this console";
  if (lock.unlocked) return `Unlocked on ${where} for this session.`;
  // -- WP P1EFIX -- the locked-user line is the core's row, the same in every client
  if (lock.locked) return bannerLine();
  return lock.covers.length ? `Locked on ${where}.` : `An installer PIN is set on ${where}; the lock covers nothing yet.`;
}
