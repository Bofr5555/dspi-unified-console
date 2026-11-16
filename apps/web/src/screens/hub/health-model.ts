// WP P3WHB: the health board's logic (PLAN-P3 F7, OD-P3m). One row per device of the
// hub: the online ones with what the hub's health poll read (CPU, temperature, clip
// events a minute or the clip latch, the stream counters, uptime, hang count, the
// last restart, the last change and who made it) and the alarms that are on about
// it; the offline ones below. A cell is coloured from the hub's alarm rules
// (`alarm.rules.get`, read by the feed): the board keeps no thresholds of its own.
// Pure: what the hub said comes in; nothing here talks to the hub.

import type { HubAlarmRecord, HubDeviceRecord, HubHealthRecord, HubLastChangeRecord } from "../../core/protocol";
import type { HubAlarmRuleRecord } from "../../core/hub-settings-protocol";
import { alarmOn, restartText } from "./model";

/** One row of the board. */
export interface BoardRow {
  serial: string;
  name: string;
  tags: string[];
  online: boolean;
  /** This session's own device. */
  isOpen: boolean;
  /** Not answering while attached. */
  unresponsive: boolean;
  cpu: number | null;
  tempC: number | null;
  /** Clip events in the last minute; null on firmware with only the latch (or not read). */
  clipPerMin: number | null;
  /** The sticky clip latch: true clipped, false not, null not read. */
  clipped: boolean | null;
  /** The stream counters as the hub gave them; null when it gave none. */
  underruns: number | null;
  starvation: number | null;
  hasCounters: boolean;
  uptimeS: number | null;
  hangCount: number | null;
  /** The last restart in words; "" when the hub has none. */
  restart: string;
  /** When it was (ms), for sorting; null when unknown. */
  restartAt: number | null;
  lastChange: HubLastChangeRecord | null;
  /** Last seen (ISO-8601), for an offline device. */
  lastSeen: string;
  /** The alarms that are on about this device (raised first, as the hub orders them). */
  alarms: HubAlarmRecord[];
}

/**
 * The board's rows: every device of the inventory with its health where the hub read one,
 * and a device the health list has that the inventory does not (yet). Online rows first,
 * offline below, each in the inventory's order.
 */
export function boardRows(
  inventory: readonly HubDeviceRecord[],
  health: readonly HubHealthRecord[],
  alarms: readonly HubAlarmRecord[],
  openSerial: string,
): BoardRow[] {
  const byHealth = new Map(health.map((h) => [h.serial, h]));
  const on = alarms.filter((a) => alarmOn(a) && a.source !== "alink" && !!a.serial);
  const alarmsOf = (serial: string) => on.filter((a) => a.serial === serial);
  const row = (serial: string, name: string, tags: string[], online: boolean, lastSeen: string, d: HubDeviceRecord | null): BoardRow => {
    const h = online ? byHealth.get(serial) : undefined;
    const reset = h?.reset ?? d?.lastReset ?? null;
    const at = reset?.at ? Date.parse(reset.at) : NaN;
    return {
      serial,
      name: name || serial,
      tags,
      online,
      isOpen: !!openSerial && serial === openSerial,
      unresponsive: !!h?.unresponsive,
      cpu: h?.cpuPct ?? null,
      tempC: h?.tempC ?? null,
      clipPerMin: h?.clipPerMin ?? null,
      clipped: h && h.clipMask !== null && h.clipMask !== undefined ? h.clipMask !== 0 : null,
      underruns: h?.counters?.underruns ?? null,
      starvation: h?.counters?.starvation ?? null,
      hasCounters: !!h?.counters,
      uptimeS: h?.uptimeS ?? null,
      hangCount: h?.hangCount ?? null,
      restart: restartText(reset),
      restartAt: Number.isFinite(at) ? at : null,
      lastChange: h?.lastChange ?? null,
      lastSeen,
      alarms: alarmsOf(serial),
    };
  };
  const rows = inventory.map((d) => row(d.serial, d.name, d.tags ?? [], d.online, d.lastSeen, d));
  const known = new Set(inventory.map((d) => d.serial));
  for (const h of health) if (!known.has(h.serial)) rows.push(row(h.serial, "", [], true, "", null));
  return [...rows.filter((r) => r.online), ...rows.filter((r) => !r.online)];
}

/** Which of the columns that depend on the hub show: the counters only when the hub gives them. */
export function boardColumns(rows: readonly BoardRow[]): { counters: boolean } {
  return { counters: rows.some((r) => r.hasCounters) };
}

/** The thresholds the hub alarms on, from its rules; null where a rule is off or not given. */
export interface Thresholds {
  /** `cpu`: at or above, %. */
  cpuPct: number | null;
  /** `temperature`: at or above, °C. */
  tempC: number | null;
  /** `clip`: events within a minute (firmware with clip events). */
  clipPerMin: number | null;
  /** `clip` on: on latch-only firmware a set latch is the alarm. */
  clipLatch: boolean;
}

export function thresholdsFrom(rules: readonly HubAlarmRuleRecord[]): Thresholds {
  const rule = (code: string) => rules.find((r) => r.code === code && r.on);
  const num = (v: number | null | undefined) => (typeof v === "number" && Number.isFinite(v) ? v : null);
  const clip = rule("clip");
  return {
    cpuPct: num(rule("cpu")?.abovePct),
    tempC: num(rule("temperature")?.aboveC),
    clipPerMin: num(clip?.perMinute),
    clipLatch: !!clip,
  };
}

export type Tone = "" | "warn" | "bad";

/** The columns a rule colours. */
export type ToneColumn = "cpu" | "temp" | "clip";

/**
 * A cell's colour from the rules: at or above a threshold is the alarm's severity
 * (temperature critical, CPU and clip warnings: hub-api.md section 7); "" otherwise,
 * when the rule is off, or when the cell has no reading.
 */
export function cellTone(col: ToneColumn, r: BoardRow, t: Thresholds): Tone {
  switch (col) {
    case "cpu":
      return t.cpuPct !== null && r.cpu !== null && r.cpu >= t.cpuPct ? "warn" : "";
    case "temp":
      return t.tempC !== null && r.tempC !== null && r.tempC >= t.tempC ? "bad" : "";
    case "clip":
      if (r.clipPerMin !== null) return t.clipPerMin !== null && r.clipPerMin >= t.clipPerMin ? "warn" : "";
      return t.clipLatch && r.clipped ? "warn" : "";
  }
}

/** The clip cell's words: events a minute, or the latch on firmware with only that. */
export function clipText(r: BoardRow): string {
  if (r.clipPerMin !== null) return `${r.clipPerMin}/min`;
  if (r.clipped === null) return "–";
  return r.clipped ? "Clipped" : "No";
}

export type SortKey =
  | "name"
  | "state"
  | "cpu"
  | "temp"
  | "clip"
  | "underruns"
  | "starvation"
  | "uptime"
  | "hangs"
  | "restart"
  | "change"
  | "alarms";

export interface Sort {
  key: SortKey;
  /** true: highest (or Z) first. */
  desc: boolean;
}

/** A row's value for a column; null sorts last whichever way. */
function sortValue(r: BoardRow, key: SortKey): number | string | null {
  switch (key) {
    case "name":
      return r.name.toLowerCase();
    case "state":
      return r.unresponsive ? 1 : 0;
    case "cpu":
      return r.cpu;
    case "temp":
      return r.tempC;
    case "clip":
      return r.clipPerMin ?? (r.clipped === null ? null : r.clipped ? 1 : 0);
    case "underruns":
      return r.underruns;
    case "starvation":
      return r.starvation;
    case "uptime":
      return r.uptimeS;
    case "hangs":
      return r.hangCount;
    case "restart":
      return r.restartAt;
    case "change": {
      const t = r.lastChange ? Date.parse(r.lastChange.at) : NaN;
      return Number.isFinite(t) ? t : null;
    }
    case "alarms":
      return r.alarms.filter((a) => a.state === "raised").length * 1000 + r.alarms.length;
  }
}

/**
 * The rows in the order asked, online above offline whatever the column (the board lists
 * offline devices below); a missing value last; ties by name, then serial.
 */
export function sortRows(rows: readonly BoardRow[], s: Sort): BoardRow[] {
  const byName = (a: BoardRow, b: BoardRow) => a.name.localeCompare(b.name) || (a.serial < b.serial ? -1 : a.serial > b.serial ? 1 : 0);
  const cmp = (a: BoardRow, b: BoardRow) => {
    if (a.online !== b.online) return a.online ? -1 : 1;
    const va = sortValue(a, s.key);
    const vb = sortValue(b, s.key);
    if (va === null && vb === null) return byName(a, b);
    if (va === null) return 1;
    if (vb === null) return -1;
    const c = typeof va === "string" && typeof vb === "string" ? va.localeCompare(vb) : (va as number) - (vb as number);
    return (s.desc ? -c : c) || byName(a, b);
  };
  return [...rows].sort(cmp);
}

/** The next sort after a click on `key`'s heading: the same column turns round; another starts ascending (names) or descending (numbers). */
export function nextSort(s: Sort, key: SortKey): Sort {
  if (s.key === key) return { key, desc: !s.desc };
  return { key, desc: key !== "name" };
}

/** The rows whose tags contain the text (any tag, ignoring case); all rows for an empty text. */
export function filterRows(rows: readonly BoardRow[], text: string): BoardRow[] {
  const q = text.trim().toLowerCase();
  if (!q) return [...rows];
  return rows.filter((r) => r.tags.some((t) => t.toLowerCase().includes(q)));
}

/** Every tag of the rows, once, sorted (the filter's suggestions). */
export function allTags(rows: readonly BoardRow[]): string[] {
  return [...new Set(rows.flatMap((r) => r.tags))].sort((a, b) => a.localeCompare(b));
}
