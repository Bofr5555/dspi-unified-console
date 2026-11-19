// WP SW3: the hub's audio-link system model as the web console shows it
// (docs/integrator/audio-link.md): the records of `system.*`, camelCase as the
// hub's serde writes them, and the pure logic of the System view (WB6d), the
// "Clock and audio links" panel, the scene bar (WB6e) and the status rail (WB6f).
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
  /** 8 audio link channels, one per matrix input; 255 is silent. Absent: the unit's own. */
  slots?: number[] | null;
  latencyUs?: number;
  measuredLatencyUs?: number | null;
  speakers?: Speaker[];
  /** -- WP AUSB1 -- `usb_direct`: fed by USB from the host, locked to its Start of Frame. Absent: the audio link.
   * -- WP ATWEB -- `spdif`: fed by S/PDIF from the source unit named as `upstream` (or, a source with no `upstream`, by nothing the model holds). */
  transport?: "alink" | "usb_direct" | "spdif";
  // -- WP ATWEB -- an S/PDIF time-base source (docs/integrator/audio-link.md, WP ATHUBM)
  /** It stamps its frame counter in the S/PDIF user bits of the slots in `spdifTxMask`. */
  spdifSource?: boolean;
  /** Bit n: output slot n (outputs 2n and 2n + 1) carries the time base; never 0 on a source. */
  spdifTxMask?: number;
  /** The domain's D, samples; absent: the engine's. */
  spdifD?: number | null;
  // -- end WP ATWEB --
  /** -- WP AUSB1 -- a USB-direct unit's mode, written with the alignment; absent: the unit's own. */
  usbMode?: UsbMode | null;
  /** -- WP AUSB2 -- what it does when it can't lock (the hub writes it); absent: mute. */
  onLockError?: OnLockError;
}
/** -- WP AUSB2 -- a USB-direct unit's lock-error action (`EXT_ALINK_USB` byte 4). */
export type OnLockError = "mute" | "play_unaligned";
export const ON_LOCK_ERROR: { id: OnLockError; label: string }[] = [
  { id: "mute", label: "Mute" },
  { id: "play_unaligned", label: "Play unaligned" },
];
/** -- WP AUSB1 -- a USB-direct unit's mode (`EXT_ALINK_USB.mode`). */
export type UsbMode = "off" | "rate_lock" | "phase_lock";
export const USB_MODES: { id: UsbMode; label: string }[] = [
  { id: "off", label: "Off" },
  { id: "rate_lock", label: "Rate lock" },
  { id: "phase_lock", label: "Phase lock" },
];
/** -- WP AUSB1 -- what the hub read of a USB-direct unit (`EXT_ALINK_USB`, `_STATUS`, `_DIAG`). */
export interface UsbSeen {
  mode: UsbMode | "unknown";
  d: number;
  autoReanchor: boolean;
  configFlags?: string[];
  state: string;
  phaseState: number;
  rateCode: number;
  hostPpm: number;
  vcxoPpm: number;
  phaseError: number;
  busFrame: number;
  sample: number;
  dMin: number;
  dMax: number;
  fill: number;
  stream: boolean;
  holdover: boolean;
  streamStartBusFrame?: number | null;
  streamShift: number;
  gapCount: number;
  anchors: number;
  reanchors: number;
  missedSamples: number;
  lateSamples: number;
  diag?: {
    lastCause?: string | null;
    ends: number;
    holdovers: number;
    rides: number;
    suspends: number;
    busResets: number;
    unplugs: number;
    longestRideUs: number;
    // -- WP AUSB2 -- layout 2 (0 from layout 1)
    longestRideMs?: number;
    lockErrors?: number;
    badSofReadings?: number;
    version?: number;
  } | null;
  // -- WP AUSB2 --
  /** What it does in lock error now (`mute` on firmware before the fork's WP USB3b). */
  onLockError?: OnLockError | "unknown";
  /** In lock error: asked to phase-lock, it can't be in step. */
  lockError?: boolean;
  /** Why: `host_rejected`, `config`, `pending`. */
  lockErrorCause?: string | null;
  /** It refused "play unaligned" (older firmware): it mutes. */
  playUnalignedUnsupported?: boolean;
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
  // -- WP FIXM1 --
  /** "Keep USB-direct units aligned" (Installer, off by default): the hub writes the USB-direct units' output delays again when their start offsets move. */
  keepUsbAligned?: boolean;
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
  stats?: AlinkStats | null;
  trimScan?: TrimScan | null;
  /** -- fixup sw3x -- a sender's superframe apply still waiting for its frame. */
  pendingApplyFrame?: number | null;
  /**
   * -- WP HWLK1F -- whether the firmware has the audio link: `none` (no 0x8000
   * block, or it stalls), `present`, or null when that could not be told.
   */
  alink?: "present" | "none" | null;
  /** -- WP AUSB1 -- a USB-direct unit: whether its firmware has the block, and what was read. */
  usbDirect?: "present" | "none" | null;
  usb?: UsbSeen | null;
  // -- WP ATWEB -- an S/PDIF unit or source (WP ATHUBM): its firmware's block, and what was read
  spdifBlock?: "present" | "none" | null;
  spdif?: SpdifSeen | null;
}

// -- WP ATWEB --
/** `EXT_ALINK_SPDIF` and `_STATUS` as the hub read them (`observed[serial].spdif`). */
export interface SpdifSeen {
  /** `off`, `follower` or `source`. */
  mode: string;
  d: number;
  onLockError: string;
  txMask: number;
  /** The contract state: `none`, `acquiring`, `anchoring`, `locked`, `holdover`, `lock_error`. */
  state: string;
  superframe: boolean;
  holdover: boolean;
  lockError: boolean;
  /** The receiver is locked to a stream without a DSPi time base. */
  foreign: boolean;
  sourceId: number;
  dMin: number;
  dMax: number;
  lockCount: number;
  lossCount: number;
}
export interface AlinkStats {
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
  /** -- WP AUSB1 -- `alink` or `usb_direct`; a USB-direct unit's D (samples), mode and start offset. -- WP ATWEB -- or `spdif`. */
  transport?: "alink" | "usb_direct" | "spdif";
  usbD?: number | null;
  usbMode?: UsbMode | null;
  startOffsetSamples?: number | null;
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
  /** -- fixup sw3x -- this session's role now, as the hub judges it. */
  role?: string;
  observed: Record<string, Observed>;
  alignment: Alignment;
  alarms: Alarm[];
}
export interface WriteResult {
  serial: string;
  element: string;
  what: string;
  result: string;
  /** -- WP HWLK1F -- why it was skipped or applied at once: `no audio link (firmware)`, `not connected`, ... */
  reason?: string;
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
  /** -- WP HWLK1F -- ops on units without the audio link, written at once (not at `frame`). */
  direct?: PlannedOp[];
  /** -- WP HWLK1F -- ops on units with the alink that no frame could be read for: not written. */
  unscheduled?: PlannedOp[];
  /** -- WP AUSB1 -- the bus frame the USB-direct units' ops act on, the ops, the lead in samples. */
  usbFrame?: number | null;
  usbOps?: PlannedOp[];
  usbLeadSamples?: number | null;
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
 * (alink configuration is an admin action), scenes, the system mute and an
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

/** The alink's state in the glossary's words (section 3.1). */
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

// -- WP HWLK1F --
/** A device whose firmware has no audio link, as every client says it. */
export const NO_ALINK = "No audio link (firmware)";

/** An element's lock word: "No audio link (firmware)" when its firmware has none, never "Unknown". */
export function elementLock(o: Observed | undefined, connected: (o: Observed) => string): string {
  if (!o?.online) return "Unknown";
  return o.alink === "none" ? NO_ALINK : connected(o);
}

/** The role on the alink in the glossary's words. */
export function alinkRoleWord(role: string | null | undefined): string {
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

/** An audio link channel: "Ch 3" (numbered from 0, as the glossary's audio link channels), or "Silent". */
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

/** The stream's rate from `EXT_ALINK_STATUS.rate_code`. */
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
    // -- WP HWLK1F --
    case "applied_now":
      return "Applied now";
    case "unsupported":
      return "Needs the audio link";
    case "no_frame":
      return "No audio link frame";
    default:
      return result.startsWith("clamped") ? "Clamped" : result;
  }
}

/** How a result reads: good, a warning or a failure. */
export function resultTone(result: string): "ok" | "warn" | "bad" {
  if (["ok", "unchanged", "queued", "applied", "applied_now"].includes(result)) return "ok";
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
  if (path === "usb_frame") return "at a USB frame"; // -- WP AUSB1 --
  return path === "superframe" ? "through the sender" : path === "per_unit" ? "to each unit" : "system mute only";
}

// -- WP AUSB1 -- USB-direct units

/** A USB-direct unit. */
export function isUsbDirect(u: Unit): boolean {
  return u.transport === "usb_direct";
}

// -- WP ATWEB --
/** A unit fed by S/PDIF (a follower, or a source fed by nothing the model holds): not on the audio link. */
export function isSpdifFed(u: Unit): boolean {
  return u.transport === "spdif";
}

/** A USB-direct unit has no audio link firmware block: said so, never "Unknown". */
export const NO_USB_DIRECT = "No USB-direct (firmware)";

/** A USB-direct state (`EXT_ALINK_USB_STATUS.state`) in words. */
export function usbStateWord(state: string | null | undefined): string {
  switch (state) {
    case "phase_locked":
      return "Phase-locked";
    case "rate_locked":
      return "Rate-locked";
    case "pull_in":
      return "Pulling in";
    case "anchoring":
      return "Anchoring";
    case "no_sof":
      return "No USB time base";
    case "rejected":
      return "Host out of range";
    case "off":
      return "Off";
    default:
      return "Unknown";
  }
}

/** A USB-direct mode in words. */
export function usbModeWord(mode: string | null | undefined): string {
  return USB_MODES.find((m) => m.id === mode)?.label ?? "Unknown";
}

/** A USB-direct unit's lock as the System view says it. -- WP AUSB2 -- in lock error, what it does. */
export function usbLock(o: Observed | undefined): string {
  if (!o?.online) return "Unknown";
  if (o.usbDirect === "none") return NO_USB_DIRECT;
  if (o.usb?.lockError) return lockErrorState(o.usb);
  return usbStateWord(o.usb?.state);
}

// -- WP AUSB2 --
/** A lock error's cause in words. */
export function lockErrorWords(cause: string | null | undefined): string {
  switch (cause) {
    case "host_rejected":
      return "the host's rate is out of range";
    case "pending":
      return "a phase error is still being pulled in";
    case "config":
      return "its rate or D cannot phase-lock";
    default:
      return "cause unknown";
  }
}

/** A unit in lock error, in words: "Can't lock: muted" or "Can't lock: playing unaligned". */
export function lockErrorState(u: UsbSeen): string {
  return u.onLockError === "play_unaligned" ? "Can't lock: playing unaligned" : "Can't lock: muted";
}

/** `model` with USB-direct unit `unitId`'s lock-error action (mute is the hub's default: not written). */
export function withOnLockError(model: SystemModel, unitId: string, action: OnLockError): SystemModel {
  return {
    ...model,
    units: model.units.map((u) => {
      if (u.id !== unitId) return u;
      const { onLockError: _a, ...rest } = u;
      return action === "play_unaligned" ? { ...rest, onLockError: action } : rest;
    }),
  };
}

/** A timeline end's cause in words (`EXT_ALINK_USB_DIAG`). */
export function endWord(cause: string | null | undefined): string {
  switch (cause) {
    case "sof_lost":
      return "SOF lost";
    case "long_gap":
      return "a gap over 1 s";
    case "discontinuity":
      return "a SOF discontinuity";
    case "host_rejected":
      return "host out of range";
    case "rate_or_config":
      return "rate or configuration";
    case "reanchor_phase_error":
      return "re-anchor (phase error)";
    case "reanchor_output_reset":
      return "re-anchor (output reset)";
    case "reanchor_other":
      return "re-anchor";
    case "stop":
      return "switched off";
    default:
      return "none";
  }
}

/** A USB-direct unit's statistics in one line: host and VCXO ppm, phase error, gaps, missed and late samples. */
export function usbStats(u: UsbSeen): string {
  const sign = (v: number, d: number) => `${v >= 0 ? "+" : ""}${v.toFixed(d)}`;
  const parts = [`host ${sign(u.hostPpm, 1)} ppm`, `VCXO ${sign(u.vcxoPpm, 1)} ppm`];
  if (u.state === "phase_locked") parts.push(`phase error ${sign(u.phaseError, 2)} samples`);
  parts.push(`gaps ${u.gapCount}, missed ${u.missedSamples}, late ${u.lateSamples} samples`);
  // -- WP AUSB2 -- DIAG layout 2's counters, and a lock error's cause
  if (u.diag?.version === 2) {
    parts.push(`lock errors ${u.diag.lockErrors ?? 0}`);
    if (u.diag.badSofReadings) parts.push(`SOF readings repaired ${u.diag.badSofReadings}`);
  }
  if (u.diag?.longestRideMs) parts.push(`longest ride ${u.diag.longestRideMs} ms`);
  if (u.lockError) parts.push(`can't lock: ${lockErrorWords(u.lockErrorCause)}`);
  return parts.join(" · ");
}

/** `model` with USB-direct unit `unitId`'s mode (null: the unit's own is kept). */
export function withUsbMode(model: SystemModel, unitId: string, mode: UsbMode | null): SystemModel {
  return {
    ...model,
    units: model.units.map((u) => {
      if (u.id !== unitId) return u;
      const { usbMode: _m, ...rest } = u;
      return mode ? { ...rest, usbMode: mode } : rest;
    }),
  };
}

// -- WP FIXM1 --
/** The hub's issue when USB-direct start offsets moved since it last wrote them: an alignment is due. */
export const USB_OFFSETS_CHANGED = "usb_start_offsets_changed";

/** `model` with "Keep USB-direct units aligned" on or off (off is the hub's default: not written). */
export function withKeepUsbAligned(model: SystemModel, on: boolean): SystemModel {
  const { keepUsbAligned: _k, ...rest } = model;
  return on ? { ...rest, keepUsbAligned: true } : rest;
}

/** Whether the hub's status says the USB-direct units need aligning again. */
export function usbAlignDue(status: SystemStatus | null | undefined): boolean {
  return status?.alignment.issues.some((i) => i.code === USB_OFFSETS_CHANGED) ?? false;
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
  /** -- WP AUSB1 -- a USB-direct unit: its statistics line ("" until read). */
  usb?: { stats: string };
}

/** Alarms still on (raised or acknowledged) for `subject`. */
export function activeAlarms(alarms: Alarm[], subject?: string): Alarm[] {
  return alarms.filter((a) => a.state !== "cleared" && (subject === undefined || a.subject === subject));
}

export function unitView(u: Unit, status: SystemStatus | null): UnitView {
  const o = status?.observed[u.serial];
  // -- WP AUSB1 --
  if (isUsbDirect(u)) {
    return {
      id: u.id,
      name: u.name || u.id,
      serial: u.serial,
      connected: !!o?.online,
      lock: usbLock(o),
      alarms: activeAlarms(status?.alarms ?? [], u.id),
      slots: [],
      usb: { stats: o?.usb ? usbStats(o.usb) : "" },
    };
  }
  return {
    id: u.id,
    name: u.name || u.id,
    serial: u.serial,
    connected: !!o?.online,
    lock: elementLock(o, (x) => lockWord(x.state)),
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
  /** -- WP AUSB1 -- `usb_direct`: a unit fed by USB, beside the audio link's tree. */
  kind: "sender" | "fan_out" | "unit" | "usb_direct";
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
      connected: !!o?.online, lock: elementLock(o, () => "Clock"), hopDelayFrames: null, slots: null,
      alarms: activeAlarms(alarms, s.id).length, children: [], upstream: null,
    });
  }
  for (const f of model.fanOuts) {
    const o = obs(f.serial);
    nodes.set(f.id, {
      id: f.id, name: f.name || f.id, kind: "fan_out", serial: f.serial ?? "", output: f.output ?? null,
      connected: !!o?.online, lock: elementLock(o, (x) => lockWord(x.state)),
      hopDelayFrames: o?.hopDelayFrames ?? f.hopDelayFrames ?? null, slots: null,
      alarms: activeAlarms(alarms, f.id).length, children: [], upstream: f.upstream,
    });
  }
  for (const u of model.units) {
    const v = unitView(u, status);
    const o = obs(u.serial);
    const usb = isUsbDirect(u);
    nodes.set(u.id, {
      id: u.id, name: v.name, kind: usb ? "usb_direct" : "unit", serial: u.serial, output: u.output ?? null,
      connected: v.connected, lock: v.lock,
      hopDelayFrames: u.repeater && !usb ? (o?.hopDelayFrames ?? u.hopDelayFrames ?? null) : null,
      slots: usb ? null : v.slots, alarms: v.alarms.length, children: [], upstream: usb ? null : u.upstream,
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

/** The audio link channels a sender's sources carry, 0..63, for the slot matrix's rows. */
export function senderChannels(model: SystemModel): number[] {
  const used = new Set<number>();
  for (const s of model.sources) for (const c of s.channels) used.add(c);
  for (const u of model.units) if (!isUsbDirect(u)) for (const c of unitSlots(u)) if (c !== SILENT) used.add(c);
  const max = Math.max(7, ...used);
  return Array.from({ length: Math.min(64, max + 1) }, (_, i) => i);
}

/** The source carrying audio link channel `ch`, if one names it. */
export function sourceOf(model: SystemModel, ch: number): SourceDef | undefined {
  return model.sources.find((s) => s.channels.includes(ch));
}

// ---------------------------------------------------------------- model edits (admin)

/** `model` with unit `unitId`'s matrix input `input` fed by audio link channel `ch` (255: silent). */
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

/** `model` with its audio link delay D (frames; null: the smallest every unit takes). */
export function withAlinkDelay(model: SystemModel, frames: number | null): SystemModel {
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
 * so the System view works without a hub. Its alink state comes from the
 * device's own audio link calls when it has the alink.
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

// -- fixup sw3x -- a recall's outcome

/** Results that mean a unit (or the sender) took the op. */
const TAKEN = new Set(["ok", "unchanged", "queued", "queued_unverified", "applied", "late", "applied_now"]);

/** Whether any write of a recall was taken: if none was, the scene was not recalled. */
export function sceneTaken(report: SceneReport): boolean {
  return report.writes.some((w) => TAKEN.has(w.result) || w.result.startsWith("clamped"));
}

// -- WP WB8 -- why a recall was refused is the core's sentence now (`sceneOutcome`,
// dspi_core::link::alink_system::scene_recall_outcome), not composed here.

/** -- fixup sw3x -- an alignment write in the glossary's words: `phase` is the audio link delay, `slots` the audio link slots. */
export function writeWhat(what: string): string {
  if (what === "phase") return "Audio link delay";
  if (what === "slots") return "Audio link slots";
  if (what === "usb_direct") return "USB-direct mode and delay"; // -- WP AUSB1 --
  const out = /^out_delay:(\d+)$/.exec(what);
  if (out) return `Delay OUT${Number(out[1]) + 1}`;
  return what;
}
