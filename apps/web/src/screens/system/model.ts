// WP SW3: the hub's audio-link system model as the web console shows it
// (docs/integrator/audio-link.md): the records of `system.*`, camelCase as the
// hub's serde writes them, and the pure logic of the System view (WB6d), the
// "Clock and links" panel, the scene bar (WB6e) and the status rail (WB6f).
// No I/O here: `feed.ts` asks the hub, the views draw.

import type { KeyValueStore } from "../../fleet/local";

// ---------------------------------------------------------------- records

export interface Room {
  id: string;
  name: string;
  lipSyncUs?: number | null;
}
export interface Zone {
  id: string;
  name: string;
  rooms: string[];
  source?: string | null;
}
export interface Group {
  id: string;
  name: string;
  units: string[];
  note?: string;
}
export interface SourceDef {
  id: string;
  name: string;
  sender?: string | null;
  channels: number[];
}
export interface Sender {
  id: string;
  name: string;
  serial?: string | null;
  senderId?: number | null;
}
export interface FanOut {
  id: string;
  name: string;
  serial?: string | null;
  upstream: string;
  output?: number | null;
  hopDelayFrames?: number | null;
}
export interface Speaker {
  output: number;
  distanceM: number;
}
export interface Unit {
  id: string;
  name: string;
  serial: string;
  room?: string | null;
  upstream: string;
  output?: number | null;
  repeater?: boolean;
  hopDelayFrames?: number | null;
  /** 8 link channels, one per matrix input; 255 is silent. Absent: the unit's own. */
  slots?: number[] | null;
  latencyUs?: number;
  measuredLatencyUs?: number | null;
  speakers?: Speaker[];
}
export interface SceneAction {
  target: string;
  op: string;
  arg?: number;
  value?: number;
}
export interface Scene {
  id: string;
  name: string;
  actions: SceneAction[];
  systemMute?: boolean;
}
export interface SystemModel {
  version?: number;
  dFrames?: number | null;
  rateHz?: number;
  lipSyncUs?: number;
  applyLeadFrames?: number;
  rooms: Room[];
  zones: Zone[];
  groups: Group[];
  sources: SourceDef[];
  senders: Sender[];
  fanOuts: FanOut[];
  units: Unit[];
  scenes: Scene[];
}

/** What the hub read from one device (`Observed`), by serial. */
export interface Observed {
  online: boolean;
  role?: string | null;
  dLocalMin?: number | null;
  dLocalMax?: number | null;
  state?: string | null;
  hops?: number | null;
  hopDelayFrames?: number | null;
  senderId?: number | null;
  rateCode?: number | null;
  crc16Errors?: number | null;
  lossCount?: number | null;
  phaseErrorFrames?: number | null;
  frame?: number | null;
  /** mode, d_local, hops_offset, flags */
  phase?: [number, number, number, number] | null;
  superframePath?: boolean;
  stateEvents?: boolean;
  systemMute?: boolean | null;
  stats?: LinkStats | null;
  trimScan?: TrimScan | null;
}
export interface LinkStats {
  lineClass: number;
  channels: number;
  lockCount: number;
  crc8Errors: number;
  bitSlips: number;
  superframeErrors: number;
  vcxoPpm: number;
  fillFrames: number;
}
export interface TrimScan {
  state: "none" | "running" | "done" | "no_clean_trim" | "unknown";
  trimLo: number;
  trimHi: number;
  trimUsed: number;
  cleanMask: number;
}
export interface UnitPlan {
  unit: string;
  serial: string;
  hops: number;
  pathDelayFrames: number;
  latencyFrames: number;
  lipSyncFrames: number;
  effective: number;
  dLocal: number;
  hopsOffset: number;
  outDelays: [number, number][];
  clampedOutputs?: number[];
  writable: boolean;
}
export interface Issue {
  subject?: string | null;
  code: string;
  message: string;
}
export interface Alignment {
  dFrames: number;
  dAuto: boolean;
  rateHz: number;
  latencyMaxFrames: number;
  units: UnitPlan[];
  issues: Issue[];
}
export interface Alarm {
  id: number;
  code: string;
  subject: string;
  severity: "warning" | "critical" | string;
  message: string;
  state: "raised" | "acknowledged" | "cleared";
  raisedAt: number;
  clearedAt?: number | null;
  count: number;
}
export interface SystemStatus {
  revision: number;
  observed: Record<string, Observed>;
  alignment: Alignment;
  alarms: Alarm[];
}
export interface WriteResult {
  serial: string;
  element: string;
  what: string;
  result: string;
}
export interface AlignReport {
  alignment: Alignment;
  applied: boolean;
  writes: WriteResult[];
}
export interface PlannedOp {
  serial: string;
  element: string;
  op: string;
  arg: number;
  value: number;
}
export interface ScenePlan {
  scene: string;
  frame: number;
  path: "superframe" | "per_unit" | "none" | string;
  ops: PlannedOp[];
  leadFrames: number;
  systemMute?: boolean | null;
  systemFlags?: number;
}
export interface SceneReport {
  plan: ScenePlan;
  writes: WriteResult[];
  frameAfterWrites?: number | null;
  onTime: boolean;
}

// ---------------------------------------------------------------- roles and modes

/** The hub's role for this session; "" for a device the console holds itself (no hub). */
export type Role = "admin" | "control" | "viewer" | "";

/** What a write needs (docs/integrator/audio-link.md, "On Link"). */
export type SystemWrite = "model" | "align" | "scene" | "mute" | "ack";

/**
 * Whether `role` may make `what`: the model and the alignment are admin-only
 * (link configuration is an admin action), scenes, the system mute and an
 * alarm's acknowledgement need control. The hub judges again; this only keeps
 * controls it would refuse from being offered.
 */
export function mayWrite(role: Role, what: SystemWrite): boolean {
  if (what === "model" || what === "align") return role === "admin";
  return role === "admin" || role === "control";
}

/** Why a write is not offered to `role`, or "" when it is. */
export function writeNote(role: Role, what: SystemWrite): string {
  if (mayWrite(role, what)) return "";
  return what === "model" || what === "align" ? "Changing the installation needs the hub's Admin role" : "This needs the hub's Control role";
}

/** The UI's modes (docs/UI-MODES.md). The web opens in Advanced. */
export type Mode = "simple" | "advanced" | "installer";
export const MODES: { id: Mode; label: string }[] = [
  { id: "simple", label: "Simple" },
  { id: "advanced", label: "Advanced" },
  { id: "installer", label: "Installer" },
];
export const DEFAULT_MODE: Mode = "advanced";

const MODE_KEY = "dspi-web.mode";

/**
 * The mode for the device `serial`: the one remembered for it, else the one
 * chosen for every device, else Advanced. No PIN exists on the web yet, so
 * every mode is reachable (UI-MODES section 1: nothing is locked unless an
 * installer locks it).
 */
export function loadMode(storage: KeyValueStore | null, serial: string): Mode {
  const all = readModes(storage);
  const m = all.devices[serial] ?? all.every;
  return MODES.some((x) => x.id === m) ? (m as Mode) : DEFAULT_MODE;
}

/** Remember `mode` for `serial`, or (`every`) for every device on this console. */
export function saveMode(storage: KeyValueStore | null, serial: string, mode: Mode, every: boolean): void {
  const all = readModes(storage);
  if (every) {
    all.every = mode;
    all.devices = {};
  } else all.devices[serial] = mode;
  try {
    storage?.setItem(MODE_KEY, JSON.stringify(all));
  } catch {
    // Not kept: the mode holds for this visit.
  }
}

function readModes(storage: KeyValueStore | null): { every?: string; devices: Record<string, string> } {
  try {
    const v = JSON.parse(storage?.getItem(MODE_KEY) ?? "{}") as { every?: unknown; devices?: unknown };
    const devices = v.devices && typeof v.devices === "object" ? (v.devices as Record<string, string>) : {};
    return { every: typeof v.every === "string" ? v.every : undefined, devices: { ...devices } };
  } catch {
    return { devices: {} };
  }
}

// ---------------------------------------------------------------- words

/** The link's state in the glossary's words (section 3.1). */
export function lockWord(state: string | null | undefined): string {
  switch (state) {
    case "locked":
      return "Locked";
    case "searching":
      return "Searching";
    case "holdover":
      return "Holdover";
    case "off":
      return "Off";
    default:
      return "Unknown";
  }
}

/** The role on the link in the glossary's words. */
export function linkRoleWord(role: string | null | undefined): string {
  switch (role) {
    case "sender":
      return "Sender";
    case "fan_out":
    case "fanout":
      return "Fan-out";
    case "repeater":
      return "Repeater";
    case "receiver":
      return "Receiver";
    default:
      return role && role !== "none" ? role : "No audio link";
  }
}

/** A link channel: "Ch 3" (numbered from 0, as the glossary's link channels), or "Silent". */
export function slotWord(ch: number): string {
  return ch === 255 ? "Silent" : `Ch ${ch}`;
}

export const SILENT = 255;
export const UNIT_INPUTS = 8;

/** Frames as milliseconds at `rateHz`, "176 (3.67 ms)". */
export function framesMs(frames: number, rateHz: number): string {
  const ms = rateHz > 0 ? (frames * 1000) / rateHz : 0;
  return `${frames} (${ms.toFixed(2)} ms)`;
}

/** The stream's rate from `EXT_LINK_STATUS.rate_code`. */
export function rateOf(code: number | null | undefined): string {
  return code === 0 ? "44.1 kHz" : code === 1 ? "48 kHz" : code === 2 ? "96 kHz" : "Unknown";
}

/** The line class, L1..L3. */
export function classWord(c: number | null | undefined): string {
  return c && c >= 1 && c <= 3 ? `L${c}` : "Unknown";
}

/**
 * One write's outcome for people (a scene's per-unit result, or an alignment
 * write): `queued`, `applied`, `late`, `refused`, ... (docs/integrator/audio-link.md).
 */
export function resultWord(result: string): string {
  switch (result) {
    case "ok":
      return "Done";
    case "unchanged":
      return "Unchanged";
    case "queued":
      return "Queued";
    case "queued_unverified":
      return "Queued, not verified";
    case "applied":
      return "Applied";
    case "late":
      return "Late";
    case "cancelled":
      return "Cancelled";
    case "refused_at_apply":
    case "refused":
      return "Refused";
    case "skipped":
      return "Skipped";
    case "locked":
      return "Locked by a client";
    case "denied":
      return "Denied";
    case "timeout":
      return "No answer";
    case "no_device":
      return "Not connected";
    default:
      return result.startsWith("clamped") ? "Clamped" : result;
  }
}

/** How a result reads: good, a warning or a failure. */
export function resultTone(result: string): "ok" | "warn" | "bad" {
  if (["ok", "unchanged", "queued", "applied"].includes(result)) return "ok";
  if (["late", "queued_unverified", "skipped", "cancelled"].includes(result) || result.startsWith("clamped")) return "warn";
  return "bad";
}

/** An op for people: `master_volume` -12.00 dB, `mute`, `preset_load` 3. */
export function opWord(op: string, arg = 0, value = 0): string {
  switch (op) {
    case "mute":
      return "Mute";
    case "unmute":
      return "Unmute";
    case "preset_load":
      return `Preset ${arg + 1}`;
    case "master_volume":
      return `Master volume ${(value / 100).toFixed(1)} dB`;
    case "out_delay":
      return `Delay OUT${arg + 1} ${value} samples`;
    case "click":
      return `Test click OUT${arg + 1}`;
    case "sync_pulse":
      return "Sync pulse";
    default:
      return op;
  }
}

/** The path a scene took. */
export function pathWord(path: string): string {
  return path === "superframe" ? "through the sender" : path === "per_unit" ? "to each unit" : "system mute only";
}

// ---------------------------------------------------------------- the installation

/** What one unit shows: connected, the audio link's lock, its alarms. */
export interface UnitView {
  id: string;
  name: string;
  serial: string;
  connected: boolean;
  lock: string;
  alarms: Alarm[];
  slots: number[];
}

/** Alarms still on (raised or acknowledged) for `subject`. */
export function activeAlarms(alarms: Alarm[], subject?: string): Alarm[] {
  return alarms.filter((a) => a.state !== "cleared" && (subject === undefined || a.subject === subject));
}

export function unitView(u: Unit, status: SystemStatus | null): UnitView {
  const o = status?.observed[u.serial];
  return {
    id: u.id,
    name: u.name || u.id,
    serial: u.serial,
    connected: !!o?.online,
    lock: o?.online ? lockWord(o.state) : "Unknown",
    alarms: activeAlarms(status?.alarms ?? [], u.id),
    slots: unitSlots(u),
  };
}

/** The unit's 8 slots as the model has them (silent where it has none). */
export function unitSlots(u: Unit): number[] {
  const s = u.slots ?? [];
  return Array.from({ length: UNIT_INPUTS }, (_, i) => s[i] ?? SILENT);
}

/** Rooms with their zones and units; units without a room last, under "No room". */
export function roomsView(model: SystemModel, status: SystemStatus | null): { id: string; name: string; zones: string[]; units: UnitView[] }[] {
  const rooms = model.rooms.map((r) => ({
    id: r.id,
    name: r.name || r.id,
    zones: model.zones.filter((z) => z.rooms.includes(r.id)).map((z) => z.name || z.id),
    units: model.units.filter((u) => u.room === r.id).map((u) => unitView(u, status)),
  }));
  const loose = model.units.filter((u) => !u.room || !model.rooms.some((r) => r.id === u.room));
  if (loose.length) rooms.push({ id: "", name: "No room", zones: [], units: loose.map((u) => unitView(u, status)) });
  return rooms;
}

/** One node of the topology: the sender, a fan-out, a unit (a repeater is a unit that passes the stream on). */
export interface TopoNode {
  id: string;
  name: string;
  kind: "sender" | "fan_out" | "unit";
  serial: string;
  /** The output of its upstream it hangs on, when the model says. */
  output: number | null;
  connected: boolean;
  lock: string;
  /** Frames the box adds re-emitting (fan-outs, repeaters). */
  hopDelayFrames: number | null;
  slots: number[] | null;
  alarms: number;
  children: TopoNode[];
}

/**
 * The topology as a tree under each sender: fan-outs and units by their
 * `upstream`. A node whose upstream the model does not have is listed at the
 * top level, so nothing is hidden. Cycles cannot occur (the hub validates the
 * model), but each node is placed once whatever it says.
 */
export function topology(model: SystemModel, status: SystemStatus | null): TopoNode[] {
  const alarms = status?.alarms ?? [];
  const obs = (serial: string | null | undefined) => (serial ? status?.observed[serial] : undefined);
  const nodes = new Map<string, TopoNode & { upstream: string | null }>();
  for (const s of model.senders) {
    const o = obs(s.serial);
    nodes.set(s.id, {
      id: s.id, name: s.name || s.id, kind: "sender", serial: s.serial ?? "", output: null,
      connected: !!o?.online, lock: o?.online ? "Clock" : "Unknown", hopDelayFrames: null, slots: null,
      alarms: activeAlarms(alarms, s.id).length, children: [], upstream: null,
    });
  }
  for (const f of model.fanOuts) {
    const o = obs(f.serial);
    nodes.set(f.id, {
      id: f.id, name: f.name || f.id, kind: "fan_out", serial: f.serial ?? "", output: f.output ?? null,
      connected: !!o?.online, lock: o?.online ? lockWord(o.state) : "Unknown",
      hopDelayFrames: o?.hopDelayFrames ?? f.hopDelayFrames ?? null, slots: null,
      alarms: activeAlarms(alarms, f.id).length, children: [], upstream: f.upstream,
    });
  }
  for (const u of model.units) {
    const v = unitView(u, status);
    const o = obs(u.serial);
    nodes.set(u.id, {
      id: u.id, name: v.name, kind: "unit", serial: u.serial, output: u.output ?? null,
      connected: v.connected, lock: v.lock,
      hopDelayFrames: u.repeater ? (o?.hopDelayFrames ?? u.hopDelayFrames ?? null) : null,
      slots: v.slots, alarms: v.alarms.length, children: [], upstream: u.upstream,
    });
  }
  const roots: TopoNode[] = [];
  for (const n of nodes.values()) {
    const up = n.upstream !== null && n.upstream !== n.id ? nodes.get(n.upstream) : undefined;
    if (up) up.children.push(n);
    else roots.push(n);
  }
  const strip = (n: TopoNode & { upstream?: string | null }): TopoNode => {
    const { upstream: _u, ...rest } = n;
    return { ...rest, children: n.children.map(strip) };
  };
  return roots.map(strip);
}

/** The link channels a sender's sources carry, 0..63, for the slot matrix's rows. */
export function senderChannels(model: SystemModel): number[] {
  const used = new Set<number>();
  for (const s of model.sources) for (const c of s.channels) used.add(c);
  for (const u of model.units) for (const c of unitSlots(u)) if (c !== SILENT) used.add(c);
  const max = Math.max(7, ...used);
  return Array.from({ length: Math.min(64, max + 1) }, (_, i) => i);
}

/** The source carrying link channel `ch`, if one names it. */
export function sourceOf(model: SystemModel, ch: number): SourceDef | undefined {
  return model.sources.find((s) => s.channels.includes(ch));
}

// ---------------------------------------------------------------- model edits (admin)

/** `model` with unit `unitId`'s matrix input `input` fed by link channel `ch` (255: silent). */
export function withSlot(model: SystemModel, unitId: string, input: number, ch: number): SystemModel {
  return {
    ...model,
    units: model.units.map((u) => {
      if (u.id !== unitId) return u;
      const slots = unitSlots(u);
      slots[input] = ch;
      return { ...u, slots };
    }),
  };
}

/** `model` with a unit's measured latency (µs; null clears it, so `latencyUs` counts). */
export function withMeasuredLatency(model: SystemModel, unitId: string, us: number | null): SystemModel {
  return { ...model, units: model.units.map((u) => (u.id === unitId ? { ...u, measuredLatencyUs: us } : u)) };
}

/** `model` with a room's lip-sync offset (µs; null: the model's own). */
export function withRoomLipSync(model: SystemModel, roomId: string, us: number | null): SystemModel {
  return { ...model, rooms: model.rooms.map((r) => (r.id === roomId ? { ...r, lipSyncUs: us } : r)) };
}

/** `model` with its link delay D (frames; null: the smallest every unit takes). */
export function withLinkDelay(model: SystemModel, frames: number | null): SystemModel {
  return { ...model, dFrames: frames };
}

/** A whole number in `lo..hi` from typed text, or null for empty; undefined when not a number in range. */
export function parseWhole(text: string, lo: number, hi: number): number | null | undefined {
  const t = text.trim();
  if (!t) return null;
  if (!/^-?\d+$/.test(t)) return undefined;
  const n = Number(t);
  return n >= lo && n <= hi ? n : undefined;
}

// ---------------------------------------------------------------- one device without a hub

/**
 * A device the console holds itself (USB, the demo) as a system of one unit,
 * so the System view works without a hub. Its link state comes from the
 * device's own audio link calls when it has the link.
 */
export function singleUnit(
  device: { name: string; serial: string },
  link: { role: string; state: string; slots: number[]; dLocalMin: number; dLocalMax: number } | null,
  connected: boolean,
): { model: SystemModel; status: SystemStatus } {
  const serial = device.serial || "this-device";
  const unit: Unit = { id: "this", name: device.name || "DSPi", serial, upstream: "", slots: link?.slots ?? null };
  return {
    model: { rooms: [], zones: [], groups: [], sources: [], senders: [], fanOuts: [], units: [unit], scenes: [] },
    status: {
      revision: 0,
      observed: {
        [serial]: {
          online: connected,
          role: link?.role ?? null,
          state: link ? link.state : null,
          dLocalMin: link?.dLocalMin ?? null,
          dLocalMax: link?.dLocalMax ?? null,
        },
      },
      alignment: { dFrames: 0, dAuto: true, rateHz: 48000, latencyMaxFrames: 0, units: [], issues: [] },
      alarms: [],
    },
  };
}

/** Alarms for the rail: on first (critical before warning, newest first), then the last cleared few. */
export function railAlarms(alarms: Alarm[], cleared = 3): Alarm[] {
  const on = alarms
    .filter((a) => a.state !== "cleared")
    .sort((a, b) => (a.severity === b.severity ? b.raisedAt - a.raisedAt : a.severity === "critical" ? -1 : 1));
  const off = alarms.filter((a) => a.state === "cleared").sort((a, b) => (b.clearedAt ?? 0) - (a.clearedAt ?? 0));
  return [...on, ...off.slice(0, cleared)];
}

/** The element's name for an alarm's subject (a unit's or fan-out's id). */
export function subjectName(model: SystemModel | null, subject: string): string {
  if (!model) return subject;
  const el = [...model.units, ...model.fanOuts, ...model.senders].find((e) => e.id === subject);
  return el?.name || subject;
}

/** Whether a sender of the model reports its system mute set. */
export function systemMuted(model: SystemModel, status: SystemStatus | null): boolean | null {
  const seen = model.senders.map((s) => (s.serial ? status?.observed[s.serial]?.systemMute : null)).filter((m) => m != null);
  return seen.length ? seen.some(Boolean) : null;
}
