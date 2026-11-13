// The event log's logic (WP WB8): the core's records (P0M, `eventLogRead`) and,
// on a hub's installation, its alarms, as one list of rows with a time, a unit
// and the text, newest first; filtered by unit; and as plain text to copy.

import type { EventRecord } from "../../core/protocol";
import type { Alarm, SystemModel } from "../system/model";
import { subjectName } from "../system/model";

export interface EventRow {
  /** Unique in the list. */
  key: string;
  /** ms since the epoch. */
  t: number;
  /** The unit's serial (a record's) or element id (an alarm's subject); "" for none. */
  unitId: string;
  /** The unit as shown. */
  unit: string;
  /** What kind, in words: "Clip", "Audio link", "Alarm". */
  kind: string;
  text: string;
  /** Colour for meaning only: a clip or a critical alarm is `bad`, a reset or a warning `warn`. */
  tone: "" | "warn" | "bad";
}

const KIND_WORDS: Record<string, string> = {
  notification: "Change",
  connected: "Connected",
  disconnected: "Disconnected",
  clip: "Clip",
  temperature: "Temperature",
  counter: "Counter",
  reset: "Reset",
  firmware: "Firmware",
  import: "Import",
  recall: "Recall",
  audio_link: "Audio link",
};

/** A record's kind in words. */
export function kindWord(kind: string): string {
  return KIND_WORDS[kind] ?? "Event";
}

/** One of the core's records. `unitName` names a serial. */
export function recordRow(r: EventRecord, unitName: (serial: string) => string): EventRow {
  const serial = r.serial ?? "";
  return {
    key: `e${serial}:${r.id}`,
    t: r.t,
    unitId: serial,
    unit: serial ? unitName(serial) : "",
    kind: kindWord(r.kind),
    text: r.text,
    tone: r.kind === "clip" ? "bad" : r.kind === "reset" ? "warn" : "",
  };
}

/** One of the hub's alarms (raised at `raisedAt`, in seconds). */
export function alarmRow(a: Alarm, model: SystemModel | null): EventRow {
  const cleared = a.state === "cleared" ? " (cleared)" : a.state === "acknowledged" ? " (acknowledged)" : "";
  return {
    key: `a${a.id}`,
    t: a.raisedAt * 1000,
    unitId: a.subject,
    unit: subjectName(model, a.subject),
    kind: "Alarm",
    text: `${a.message}${a.count > 1 ? ` (${a.count}×)` : ""}${cleared}`,
    tone: a.severity === "critical" ? "bad" : "warn",
  };
}

/** The units to filter by, as `{id, name}`, in the order first seen. */
export function unitsOf(rows: EventRow[]): { id: string; name: string }[] {
  const seen = new Map<string, string>();
  for (const r of rows) if (r.unitId && !seen.has(r.unitId)) seen.set(r.unitId, r.unit);
  return [...seen].map(([id, name]) => ({ id, name }));
}

/** Newest first; only `unit`'s when one is chosen ("" for every unit). */
export function shownRows(rows: EventRow[], unit: string): EventRow[] {
  return rows.filter((r) => !unit || r.unitId === unit).sort((a, b) => b.t - a.t || (a.key < b.key ? 1 : -1));
}

const pad = (n: number, w = 2) => String(n).padStart(w, "0");

/** The time as shown: the clock today, the date in front otherwise (local time). */
export function timeText(t: number, now: Date): string {
  const d = new Date(t);
  const clock = `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
  const today = d.getFullYear() === now.getFullYear() && d.getMonth() === now.getMonth() && d.getDate() === now.getDate();
  return today ? clock : `${dateText(d)} ${clock}`;
}

function dateText(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The full date and time (local). */
export function fullTime(t: number): string {
  const d = new Date(t);
  return `${dateText(d)} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** The rows as text to paste into a report: one line each, tab-separated, the full date and time. */
export function rowsText(rows: EventRow[]): string {
  return rows.map((r) => [fullTime(r.t), r.unit || "-", r.kind, r.text].join("\t")).join("\n");
}

/** At most this many rows are kept from the core's log. */
export const KEEP = 500;

/** `old` and the new records, without repeats, the newest `KEEP`. */
export function merged(old: EventRecord[], fresh: EventRecord[]): EventRecord[] {
  const byKey = new Map(old.map((r) => [`${r.serial ?? ""}:${r.id}`, r]));
  for (const r of fresh) byKey.set(`${r.serial ?? ""}:${r.id}`, r);
  const all = [...byKey.values()].sort((a, b) => a.t - b.t || a.id - b.id);
  return all.slice(Math.max(0, all.length - KEEP));
}
