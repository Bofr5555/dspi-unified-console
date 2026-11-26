// WP P2WB1: the hub screen's logic. The hub's inventory and health as one row
// per device, its alarms as a board and as the health panel's conditions, the
// words for a restart and an uptime. Pure: what the hub said comes in, the time
// from the caller; nothing here talks to the hub (feed.ts does, through the
// core's typed calls).

import type { HubAlarmRecord, HubDeviceRecord, HubHealthRecord, HubHistoryRecord, HubResetRecord, NoticeRecord } from "../../core/protocol";
import type { Condition } from "../../ui/status-rail/health";

/** One row of the device table. */
export interface HubRow {
  serial: string;
  name: string;
  location: string;
  note: string;
  tags: string[];
  online: boolean;
  /** The firmware as shown ("1.1.6", "1.1.6 beta"); "" when unknown. */
  firmware: string;
  /** The busier core's load, %; null when not read (or offline). */
  cpu: number | null;
  /** °C; null when not read (or offline). */
  tempC: number | null;
  /** The last restart in words; "" when the hub has none. */
  restart: string;
  /** Not answering while attached. */
  unresponsive: boolean;
  /** Last seen (ISO-8601), for an offline device. */
  lastSeen: string;
  /** This session's own device. */
  isOpen: boolean;
}

/** A reset reason in words (the hub's reasons, hub-api.md section 5). */
export function resetWords(reason: string): string {
  switch (reason) {
    case "power_on":
      return "Power-on";
    case "brownout":
      return "Brown-out";
    case "run_pin":
      return "Run pin";
    case "watchdog":
      return "Watchdog";
    case "software":
      return "Software reset";
    case "debugger":
      return "Debugger";
    case "glitch":
      return "Glitch";
    default:
      return "Unknown cause";
  }
}

/** A restart for the table: its cause and, when the hub knows it, when. */
export function restartText(r: HubResetRecord | null): string {
  if (!r) return "";
  const when = r.at ? shortTime(r.at) : "";
  return when ? `${resetWords(r.reason)}, ${when}` : resetWords(r.reason);
}

const pad = (n: number) => String(n).padStart(2, "0");

/** An ISO time as the page shows it (local): "2026-11-15 14:03". "" stays "". */
export function shortTime(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return iso;
  const d = new Date(t);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** The local clock of an ISO time: "14:03:09"; the text itself when it is not one. */
export function clockTime(iso: string): string {
  const t = Date.parse(iso);
  if (!Number.isFinite(t)) return iso;
  const d = new Date(t);
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

/** Seconds as "3 d 4 h", "2 h 5 min", "40 s". */
export function durationText(s: number): string {
  if (!Number.isFinite(s) || s < 0) return "";
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d} d ${h} h`;
  if (h) return `${h} h ${m} min`;
  if (m) return `${m} min`;
  return `${Math.floor(s)} s`;
}

/**
 * The table: every device of the inventory (online first, as the hub orders them),
 * with its health where the hub has read one. A device in the health list but not
 * (yet) in the inventory is listed too, from its health alone.
 */
export function deviceRows(inventory: readonly HubDeviceRecord[], health: readonly HubHealthRecord[], openSerial: string): HubRow[] {
  const byHealth = new Map(health.map((h) => [h.serial, h]));
  const rows: HubRow[] = inventory.map((d) => {
    const h = d.online ? byHealth.get(d.serial) : undefined;
    return {
      serial: d.serial,
      name: d.name || d.serial,
      location: d.location,
      note: d.note,
      tags: d.tags ?? [],
      online: d.online,
      firmware: d.fw ? (d.fwBeta ? `${d.fw} beta` : d.fw) : "",
      cpu: h?.cpuPct ?? null,
      tempC: h?.tempC ?? null,
      restart: restartText(h?.reset ?? d.lastReset ?? null),
      unresponsive: !!h?.unresponsive,
      lastSeen: d.lastSeen,
      isOpen: !!openSerial && d.serial === openSerial,
    };
  });
  const known = new Set(inventory.map((d) => d.serial));
  for (const h of health) {
    if (known.has(h.serial)) continue;
    rows.push({
      serial: h.serial,
      name: h.serial,
      location: "",
      note: "",
      tags: [],
      online: true,
      firmware: "",
      cpu: h.cpuPct,
      tempC: h.tempC,
      restart: restartText(h.reset),
      unresponsive: h.unresponsive,
      lastSeen: "",
      isOpen: !!openSerial && h.serial === openSerial,
    });
  }
  return rows;
}

/** One inventory record changed (`inventory.changed`): the list with it replaced, added or (null: forgotten) removed. */
export function withDevice(list: readonly HubDeviceRecord[], serial: string, device: HubDeviceRecord | null): HubDeviceRecord[] {
  const rest = list.filter((d) => d.serial !== serial);
  if (!device) return rest;
  const at = list.findIndex((d) => d.serial === serial);
  if (at < 0) return device.online ? [device, ...rest] : [...rest, device];
  const out = [...list];
  out[at] = device;
  return out;
}

/** An alarm is on until it is cleared (raised, or acknowledged and still on). */
export function alarmOn(a: HubAlarmRecord): boolean {
  return a.state !== "cleared";
}

/** One alarm changed (`alarm.changed`, an acknowledgement's answer): the list with it replaced or added. */
export function withAlarm(list: readonly HubAlarmRecord[], a: HubAlarmRecord): HubAlarmRecord[] {
  const at = list.findIndex((x) => x.id === a.id);
  if (at < 0) return sortAlarms([a, ...list]);
  const out = [...list];
  out[at] = a;
  return sortAlarms(out);
}

/** The board's order (the hub's): raised, then acknowledged, then cleared; newest first within each. */
export function sortAlarms(list: readonly HubAlarmRecord[]): HubAlarmRecord[] {
  const rank = (a: HubAlarmRecord) => (a.state === "raised" ? 0 : a.state === "acknowledged" ? 1 : 2);
  const time = (a: HubAlarmRecord) => Date.parse(a.state === "cleared" ? a.clearedAt || a.raisedAt : a.raisedAt) || 0;
  return [...list].sort((a, b) => rank(a) - rank(b) || time(b) - time(a) || (a.id < b.id ? 1 : a.id > b.id ? -1 : 0));
}

/** How many alarms still ask to be acknowledged, and whether one is critical. */
export function alarmSummary(list: readonly HubAlarmRecord[]): { raised: number; critical: boolean } {
  const raised = list.filter((a) => a.state === "raised");
  return { raised: raised.length, critical: raised.some((a) => a.severity === "critical") };
}

// -- WP UB6 -- (UXP B6)
/** The Fleet's count: the DSPis online and offline, and the alarms still to acknowledge (one critical among them). */
export function fleetSummary(devices: readonly HubDeviceRecord[], alarms: readonly HubAlarmRecord[]): { online: number; offline: number; raised: number; critical: boolean } {
  const online = devices.filter((d) => d.online).length;
  const s = alarmSummary(alarms.filter((a) => a.source !== "alink"));
  return { online, offline: devices.length - online, raised: s.raised, critical: s.critical };
}

/** The alarms that are on about device `serial` (the audio link's are the System view's). */
export function deviceAlarms(alarms: readonly HubAlarmRecord[], serial: string): HubAlarmRecord[] {
  return alarms.filter((a) => alarmOn(a) && a.source !== "alink" && a.serial === serial);
}

/** "2 alarms", "1 alarm"; "" for none. */
export function alarmCountText(n: number): string {
  return n ? `${n} ${n === 1 ? "alarm" : "alarms"}` : "";
}

/** The health panel's key for a hub alarm. */
export function alarmKey(a: HubAlarmRecord): string {
  return `hub:${a.id}`;
}

/** The alarm's id from the panel's key; "" for another key. */
export function alarmIdOf(key: string): string {
  return key.startsWith("hub:") ? key.slice(4) : "";
}

/**
 * The health panel's conditions: the device alarms that are on about `serial` (and
 * those about no one device, the hub's own), in the core's notice words. The audio
 * link's alarms stay the System view's (its status rail lists them).
 */
export function alarmConditions(
  list: readonly HubAlarmRecord[],
  serial: string,
  notice: (a: HubAlarmRecord) => NoticeRecord | null,
  deviceName: (serial: string) => string,
): Condition[] {
  return list
    .filter((a) => alarmOn(a) && a.source !== "alink" && (!a.serial || a.serial === serial))
    .map((a) => ({
      key: alarmKey(a),
      kind: "hub" as const,
      severity: a.severity === "critical" ? ("error" as const) : ("warning" as const),
      text: alarmTitle(a, notice(a), a.serial ? deviceName(a.serial) : ""),
    }));
}

/** An alarm's line: the notice's title (the hub's message without one), and what it is about. */
export function alarmTitle(a: HubAlarmRecord, n: NoticeRecord | null, about: string): string {
  const title = n?.title || a.message || a.code;
  return about ? `${title}: ${about}` : title;
}

/** The kind of a history record in words (the event log's, and the hub's own kinds). */
const HUB_KIND_WORDS: Record<string, string> = {
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
  identify: "Identify",
  unresponsive: "Not answering",
  alarm: "Alarm",
  schedule: "Schedule",
  access: "Access",
  events_dropped: "Records lost",
};

export function hubKindWord(kind: string): string {
  return HUB_KIND_WORDS[kind] ?? "Event";
}

/** A row of the hub's history as the events view shows it. */
export interface HubHistoryRow {
  key: string;
  t: number;
  kind: string;
  text: string;
  /** Who caused it, by name; "" when nobody did. */
  by: string;
  tone: "" | "warn" | "bad";
}

export function historyRow(r: HubHistoryRecord): HubHistoryRow {
  return {
    key: `h${r.serial}:${r.id}`,
    t: r.timeMs,
    kind: hubKindWord(r.kind),
    text: r.count > 1 ? `${r.text} (${r.count}×)` : r.text,
    by: r.by,
    tone: r.kind === "clip" || r.kind === "unresponsive" ? "bad" : r.kind === "reset" || r.kind === "alarm" ? "warn" : "",
  };
}

/** At most this many of the hub's records are kept in the view. */
export const HISTORY_KEEP = 500;

/**
 * The records kept after a read: `old` and `fresh` without repeats, oldest first, the newest
 * `HISTORY_KEEP`. `restarted`: the hub's numbering started again, so `old` is dropped.
 */
export function mergeHistory(old: readonly HubHistoryRecord[], fresh: readonly HubHistoryRecord[], restarted: boolean): HubHistoryRecord[] {
  const byId = new Map((restarted ? [] : old).map((r) => [r.id, r]));
  for (const r of fresh) byId.set(r.id, r);
  const all = [...byId.values()].sort((a, b) => a.id - b.id);
  return all.slice(Math.max(0, all.length - HISTORY_KEEP));
}

// -- WP P2WB3 --
/** Tags as typed ("main, stage"): trimmed, empty ones and repeats dropped (the hub bounds them). */
export function tagsFrom(text: string): string[] {
  const out: string[] = [];
  for (const t of text.split(",")) {
    const v = t.trim();
    if (v && !out.includes(v)) out.push(v);
  }
  return out;
}

/**
 * Which of the row's actions this session is offered: Identify on every online row (off, with
 * the reason, for a Viewer or a read-only session), Open on the others online, and for an Admin
 * Edit on every row and Forget on an offline one. The hub refuses below in its words.
 */
export function rowActions(r: HubRow, o: { role: string; identify: boolean; inventory: boolean; readOnly: string; canOpen: boolean }) {
  const admin = o.role === "admin" && !o.readOnly;
  return {
    identify: r.online && o.identify,
    identifyOff: o.readOnly || (o.role === "viewer" ? "A Viewer cannot identify a DSPi" : ""),
    open: r.online && !r.isOpen && o.canOpen,
    edit: admin && o.inventory,
    forget: admin && o.inventory && !r.online,
  };
}
