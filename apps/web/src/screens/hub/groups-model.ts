// WP P3WGR: the logic of the hub screen's groups (PLAN-P3 F3 to F5, F8): the groups as the
// hub keeps them (`hubGroups`, kept current by the hub's `group.changed`), a group's members
// joined to the inventory by serial (OD-P3b: the inventory has no groups field), the
// editor's bounds (hub-api.md 19.2), what each member will get from a volume with offsets
// (the value plus its offset, clamped into the registry's range, as the hub clamps it), the
// rows of a group request in words (kept until dismissed: a partial failure is shown, never
// hidden), the countdown of a guarded bulk apply, and the fleet history: the hub's history
// of every member (`hubEventsRead`, FLEET_HISTORY_PAGE rows each) merged by time, bounded.
// Pure functions first; `GroupStore` at the end is the one part that talks to the core.

import {
  CoreCallError,
  type HubDeviceRecord,
  type HubFleetTarget,
  type HubGroupRecord,
  type HubGroupRowRecord,
  type HubHistoryRecord,
  type HubServerEventRecord,
  type NoticeRecord,
  type ParamValue,
} from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeOf, noticeRow } from "../../ui/texts";

/** The hub's bounds (hub-api.md 19.2). */
export const GROUPS_MAX = 64;
export const GROUP_MEMBERS_MAX = 64;
export const GROUP_NAME_MAX = 48;
export const GROUP_NOTE_MAX = 256;
/** The fleet history: records read per member (PLAN-P3 F8). */
export const FLEET_HISTORY_PAGE = 50;
/** The fleet history: rows kept after the merge, newest first. */
export const FLEET_HISTORY_KEEP = GROUP_MEMBERS_MAX * FLEET_HISTORY_PAGE;
/** A guarded bulk apply's timeout, seconds: the hub's bounds and default (hub-api.md 20.3). */
export const APPLY_TIMEOUT_MIN_S = 1;
export const APPLY_TIMEOUT_MAX_S = 600;
export const APPLY_TIMEOUT_DEFAULT_S = 30;
/** The registry's paths of the action bar. */
export const VOLUME_PATH = "vol.user";
export const MUTE_PATH = "vol.mute";
/** Preset slots 0 to 9 (shown 1 to 10). */
export const PRESET_SLOTS = 10;

/** A group as the editor holds it while typed. */
export interface GroupDraft {
  /** "" for a new group. */
  id: string;
  name: string;
  note: string;
  members: string[];
}

/** One member of a group, joined to the inventory. */
export interface MemberRow {
  serial: string;
  /** The inventory's name, else the serial. */
  name: string;
  online: boolean;
  /** The hub's inventory has it (a member the hub forgot is dropped from the group by the hub). */
  known: boolean;
}

/** A draft of `g` (or a new group). */
export function draftOf(g: HubGroupRecord | null): GroupDraft {
  return g ? { id: g.id, name: g.name, note: g.note ?? "", members: [...g.members] } : { id: "", name: "", note: "", members: [] };
}

/** Why the hub would refuse `d`, in words; "" when it is within the bounds. */
export function draftProblem(d: GroupDraft): string {
  const name = d.name.trim();
  if (!name) return "Give the group a name.";
  if ([...name].length > GROUP_NAME_MAX) return `A group's name is at most ${GROUP_NAME_MAX} characters.`;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f\u007f]/.test(name)) return "A group's name cannot hold control characters.";
  if ([...d.note].length > GROUP_NOTE_MAX) return `A group's note is at most ${GROUP_NOTE_MAX} characters.`;
  if (d.members.length > GROUP_MEMBERS_MAX) return `A group has at most ${GROUP_MEMBERS_MAX} DSPis.`;
  if (new Set(d.members).size !== d.members.length) return "A DSPi is in the group twice.";
  return "";
}

/** `d` toggled: `serial` added at the end, or removed. */
export function toggleMember(d: GroupDraft, serial: string): GroupDraft {
  const has = d.members.includes(serial);
  return { ...d, members: has ? d.members.filter((s) => s !== serial) : [...d.members, serial] };
}

/** A group's members in its order, each with the inventory's name and state. */
export function memberRows(g: HubGroupRecord, devices: readonly HubDeviceRecord[]): MemberRow[] {
  const by = new Map(devices.map((d) => [d.serial, d]));
  return g.members.slice(0, GROUP_MEMBERS_MAX).map((serial) => {
    const d = by.get(serial);
    return { serial, name: d?.name || serial, online: !!d?.online, known: !!d };
  });
}

/** The groups `serial` is in (the health board's filter, the device's own word). */
export function groupsOf(groups: readonly HubGroupRecord[], serial: string): HubGroupRecord[] {
  return groups.filter((g) => g.members.includes(serial));
}

/** A group's request target. */
export function targetOf(g: HubGroupRecord): HubFleetTarget {
  return { group: g.id };
}

/** The groups after a `group.changed` (`change` `delete`: gone; else set as it now is), in id order. */
export function withGroup(groups: readonly HubGroupRecord[], g: HubGroupRecord, change: string): HubGroupRecord[] {
  const rest = groups.filter((x) => x.id !== g.id);
  const out = change === "delete" ? rest : [...rest, g];
  return out.sort(byId).slice(0, GROUPS_MAX);
}

/** Ids in the hub's order: `g2` before `g10`. */
function byId(a: HubGroupRecord, b: HubGroupRecord): number {
  const n = (id: string) => Number(/^g(\d+)$/.exec(id)?.[1] ?? Number.MAX_SAFE_INTEGER);
  return n(a.id) - n(b.id) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
}

/** Sorted, bounded: the hub's list as the page keeps it. */
export function sortedGroups(list: readonly HubGroupRecord[]): HubGroupRecord[] {
  return [...list].sort(byId).slice(0, GROUPS_MAX);
}

// -- the action bar --

/** A value kept inside [min, max]; unknown bounds leave it as it is. */
export function clampTo(v: number, min: number | undefined, max: number | undefined): number {
  let x = v;
  if (typeof min === "number" && x < min) x = min;
  if (typeof max === "number" && x > max) x = max;
  return x;
}

/** What one member will get from a volume with offsets. */
export interface OffsetPreview {
  serial: string;
  offset: number;
  /** The value plus the offset, clamped into the range: what the hub sends. */
  value: number;
  /** The clamp moved it. */
  clamped: boolean;
}

/**
 * Each member's value: `value` plus its offset (0 without one), clamped into `range` as the hub
 * clamps it (hub-api.md 20.2; the device's own range may be narrower, the hub's row says so).
 */
export function offsetPreview(value: number, members: readonly string[], offsets: Readonly<Record<string, number>>, range: { min?: number; max?: number }): OffsetPreview[] {
  return members.map((serial) => {
    const offset = Number.isFinite(offsets[serial]) ? offsets[serial]! : 0;
    const sum = value + offset;
    const v = clampTo(sum, range.min, range.max);
    return { serial, offset, value: v, clamped: v !== sum };
  });
}

/** The offsets the hub takes: the members' non-zero ones. */
export function offsetList(members: readonly string[], offsets: Readonly<Record<string, number>>): { serial: string; db: number }[] {
  return members.filter((s) => Number.isFinite(offsets[s]) && offsets[s] !== 0).map((serial) => ({ serial, db: offsets[serial]! }));
}

/** A dB number as typed ("-3", "−3,5"); null when it is not one. */
export function dbFrom(text: string): number | null {
  const t = text.trim().replace("−", "-").replace(",", ".");
  if (!t) return null;
  const n = Number(t);
  return Number.isFinite(n) ? n : null;
}

/** dB as the page shows it: "-20.0 dB". */
export function dbText(v: number): string {
  return `${(Math.round(v * 10) / 10).toFixed(1)} dB`;
}

/** A preset slot (0 to 9) as the page shows it. */
export function slotText(slot: number): string {
  return `Preset ${slot + 1}`;
}

/** A row's value in words: a dB number, Muted / Not muted, a slot, else "". */
export function valueText(v: ParamValue | null, path: string): string {
  if (!v || typeof v !== "object") return "";
  if ("float" in v) return path === VOLUME_PATH || path.startsWith("vol.") ? dbText(v.float) : String(v.float);
  if ("bool" in v) return path === MUTE_PATH ? (v.bool ? "Muted" : "Not muted") : v.bool ? "On" : "Off";
  if ("int" in v) return String(v.int);
  if ("choice" in v) return String(v.choice);
  return "";
}

// -- the rows of a request --

/** One device's line of a request's answer. */
export interface ResultLine {
  serial: string;
  name: string;
  ok: boolean;
  /** What happened, in words: the value sent, the slot loaded, the reason it was not. */
  text: string;
  /** The notice row's key for a failure ("" when ok). */
  key: string;
  /** `bad`: refused or failed; "" ok. */
  tone: "" | "bad" | "warn";
}

/** A request's answer, kept until dismissed. */
export interface ResultSet {
  /** What was asked, in words ("Volume -20.0 dB on Bar"). */
  title: string;
  lines: ResultLine[];
  /** The hub's word for the whole run (a bulk apply's `msg`); "". */
  note: string;
}

/** Why a row failed, in the core's words (its notice row), else the hub's. */
export function failureText(r: HubGroupRowRecord, row: (key: string) => NoticeRecord | null = noticeRow): string {
  const n = r.noticeKey ? row(r.noticeKey) : null;
  const title = n?.title ?? "";
  const msg = r.msg?.trim() ?? "";
  if (title && msg && msg !== title) return `${title}: ${msg}`;
  return title || msg || r.code || "Failed";
}

/** A request's rows in words; `ok` says what the row did. */
export function resultLines(rows: readonly HubGroupRowRecord[], name: (serial: string) => string, ok: (r: HubGroupRowRecord) => string, row: (key: string) => NoticeRecord | null = noticeRow): ResultLine[] {
  return rows.map((r) => {
    if (r.ok) return { serial: r.serial, name: name(r.serial), ok: true, text: ok(r), key: "", tone: "" };
    return { serial: r.serial, name: name(r.serial), ok: false, text: failureText(r, row), key: r.noticeKey || "", tone: r.code === "offline" ? "warn" : "bad" };
  });
}

/** A parameter row that went: the value the device got (and that it was clamped). */
export function paramOk(path: string): (r: HubGroupRowRecord) => string {
  return (r) => {
    const v = valueText(r.value, path);
    return `${v || "Done"}${r.clamped ? " (clamped to its range)" : ""}`;
  };
}

/** A preset row that went. */
export function presetOk(r: HubGroupRowRecord): string {
  const slot = typeof r.slot === "number" ? slotText(r.slot) : "Preset";
  return r.name ? `${slot} loaded (${r.name})` : `${slot} loaded`;
}

/** A bulk apply's row that went. */
export function applyOk(r: HubGroupRowRecord): string {
  if (r.reverted) return "Reverted";
  return r.guard !== null && r.guard !== undefined ? "Applied, waiting for Keep" : "Applied";
}

/** A confirm's row that went. */
export function confirmOk(r: HubGroupRowRecord): string {
  return r.reverted ? "Reverted" : "Kept";
}

/** How many of the lines failed. */
export function failures(lines: readonly ResultLine[]): number {
  return lines.filter((l) => !l.ok).length;
}

// -- bulk apply --

/** Seconds left until `confirmBy` (ISO); null without one. Never below 0. */
export function secondsLeft(confirmBy: string, nowMs: number): number | null {
  if (!confirmBy) return null;
  const t = Date.parse(confirmBy);
  if (!Number.isFinite(t)) return null;
  return Math.max(0, Math.ceil((t - nowMs) / 1000));
}

/** The countdown in words: "1:05". */
export function countdownText(s: number): string {
  const m = Math.floor(s / 60);
  return `${m}:${String(s % 60).padStart(2, "0")}`;
}

/** A timeout typed in seconds, inside the hub's bounds; null when it is not a number. */
export function timeoutFrom(text: string): number | null {
  const n = Number(text.trim());
  if (!Number.isFinite(n) || !text.trim()) return null;
  return Math.round(clampTo(n, APPLY_TIMEOUT_MIN_S, APPLY_TIMEOUT_MAX_S));
}

/** A guarded run has guards still to settle: any `ok` row with a guard that has not reverted. */
export function runPending(rows: readonly HubGroupRowRecord[]): boolean {
  return rows.some((r) => r.ok && r.guard !== null && r.guard !== undefined && !r.reverted);
}

// -- the fleet history --

/** One row of the fleet history. */
export interface FleetRow {
  key: string;
  t: number;
  serial: string;
  name: string;
  kind: string;
  text: string;
  by: string;
}

/**
 * The members' histories merged by time, newest first, without repeats, at most `keep` rows
 * (each page is at most FLEET_HISTORY_PAGE records already).
 */
export function mergeFleet(pages: readonly { serial: string; records: readonly HubHistoryRecord[] }[], name: (serial: string) => string, kind: (k: string) => string, keep = FLEET_HISTORY_KEEP): FleetRow[] {
  const seen = new Set<string>();
  const out: FleetRow[] = [];
  for (const p of pages) {
    for (const r of p.records.slice(-FLEET_HISTORY_PAGE)) {
      const serial = r.serial || p.serial;
      const key = `${serial}:${r.id}`;
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ key, t: r.timeMs, serial, name: name(serial), kind: kind(r.kind), text: r.count > 1 ? `${r.text} (${r.count}×)` : r.text, by: r.by });
    }
  }
  out.sort((a, b) => b.t - a.t || (a.key < b.key ? 1 : -1));
  return out.slice(0, Math.max(0, keep));
}

// -- the store --

/** What the page knows of the hub's groups. */
export interface GroupsView {
  groups: HubGroupRecord[];
  /** The last read's failure, "" when it went. */
  error: string;
  /** Read at least once. */
  read: boolean;
}

/** A failure in the core's words (its notice's title, and the core's message). */
export function failureWords(e: unknown): { text: string; key: string } {
  const n = noticeOf(e);
  const msg = e instanceof Error ? e.message : String(e);
  if (n) return { text: n.title && msg && msg !== n.title ? `${n.title}: ${msg}` : n.title || msg, key: n.key };
  return { text: msg, key: e instanceof CoreCallError ? e.noticeKey || "" : "" };
}

/**
 * The hub's groups for one session: read when started (and when the DSPi Link binds again),
 * then kept by the hub's `group.changed`. Only while `on` (the hub has `groups`).
 */
export class GroupStore {
  view: GroupsView = { groups: [], error: "", read: false };
  private readonly listeners = new Set<() => void>();
  private unhub: (() => void) | null = null;
  private unwatch: (() => void) | null = null;
  private phase: string | null = null;
  private running = false;
  private reading: Promise<void> | null = null;

  constructor(private readonly session: Session) {}

  start(): void {
    if (this.running) return;
    this.running = true;
    this.unhub = this.session.watchHub((events) => this.onEvents(events));
    this.phase = this.session.state.link?.phase ?? null;
    this.unwatch = this.session.watch(() => {
      const p = this.session.state.link?.phase ?? null;
      if (p === this.phase) return;
      this.phase = p;
      if (p === "bound") void this.refresh();
    });
    void this.refresh();
  }

  stop(): void {
    this.running = false;
    this.unhub?.();
    this.unhub = null;
    this.unwatch?.();
    this.unwatch = null;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** Read the hub's list now (one read at a time). */
  refresh(): Promise<void> {
    if (!this.running) return Promise.resolve();
    this.reading ??= (async () => {
      try {
        const list = await this.session.core.call("hubGroups", undefined);
        if (this.running) this.set({ groups: sortedGroups(Array.isArray(list) ? list : []), error: "", read: true });
      } catch (e) {
        if (this.running) this.set({ error: failureWords(e).text, read: true });
      } finally {
        this.reading = null;
      }
    })();
    return this.reading;
  }

  /** The hub's answer to a set, taken at once (the event follows). */
  took(g: HubGroupRecord): void {
    this.set({ groups: withGroup(this.view.groups, g, "set") });
  }

  /** A delete the hub answered. */
  dropped(id: string): void {
    this.set({ groups: this.view.groups.filter((g) => g.id !== id) });
  }

  private onEvents(events: HubServerEventRecord[]): void {
    let groups = this.view.groups;
    let dirty = false;
    for (const e of events) {
      if (e.kind === "group.changed" && e.group && typeof e.group.id === "string") {
        groups = withGroup(groups, e.group, e.change);
        dirty = true;
      }
    }
    if (dirty) this.set({ groups });
  }

  private set(patch: Partial<GroupsView>): void {
    this.view = { ...this.view, ...patch };
    for (const l of [...this.listeners]) l();
  }
}
