// The main window's arithmetic (WP WB6a): the Console's volume tapers, preset
// slot labels and the channel layout. No I/O, so the unit tests reach it.

import type { DspState } from "../../core/protocol";

/** Preset slots on every DSPi. */
export const PRESET_SLOTS = 10;

/** The user volume's floor and the master volume's (-128 is muted). */
export const USER_MIN_DB = -60;
export const MASTER_MIN_DB = -128;

/** Slider positions of the user volume: 0..USER_STEPS. */
export const USER_STEPS = 200;
/** The master taper's units (0.1 dB to -10, 0.5 to -40, 1.0 to -128). */
export const MASTER_STEPS = 248;

/** The Console's user taper: square-root power over [-60, 0] dB (`UserVolumeSection`). */
export function userToPos(db: number): number {
  if (db <= USER_MIN_DB) return 0;
  const frac = Math.min(1, (db - USER_MIN_DB) / -USER_MIN_DB);
  return Math.round(frac * frac * USER_STEPS);
}

export function posToUser(pos: number): number {
  if (pos <= 0) return USER_MIN_DB;
  const s = Math.min(1, pos / USER_STEPS);
  return round1(USER_MIN_DB + -USER_MIN_DB * Math.sqrt(s));
}

/** The firmware's master taper (`MasterVolumeTaper`): a position is one of its steps. */
export function masterToPos(db: number): number {
  const d = Math.min(0, Math.max(MASTER_MIN_DB, db));
  const units = d >= -10 ? -d / 0.1 : d >= -40 ? 100 + (-10 - d) / 0.5 : 160 + (-40 - d);
  return Math.round(MASTER_STEPS - units);
}

export function posToMaster(pos: number): number {
  const units = MASTER_STEPS - Math.min(MASTER_STEPS, Math.max(0, Math.round(pos)));
  const db = units <= 100 ? -units * 0.1 : units <= 160 ? -10 - (units - 100) * 0.5 : -40 - (units - 160);
  return round1(db);
}

/** The readout: the master at its floor is muted. */
export function volumeText(db: number, master: boolean): string {
  if (master && db <= MASTER_MIN_DB) return "−∞";
  return `${db.toFixed(1)} dB`;
}

/** The Console's slot label: "3: Living room", "3: Preset 3", "3: Empty". */
export function presetLabel(s: Pick<DspState, "presetNames" | "presetOccupied">, slot: number): string {
  return `${slot + 1}: ${presetName(s, slot)}`;
}

export function presetName(s: Pick<DspState, "presetNames" | "presetOccupied">, slot: number): string {
  if (!occupied(s, slot)) return "Empty";
  const name = (s.presetNames[slot] ?? "").trim();
  return name || `Preset ${slot + 1}`;
}

export function occupied(s: Pick<DspState, "presetOccupied">, slot: number): boolean {
  return (s.presetOccupied & (1 << slot)) !== 0;
}

/** What the unsaved-changes prompt lists: at most `max` of the core's sentences. */
export function changeSummary(changes: string[], max = 15): string[] {
  if (changes.length <= max) return changes;
  const rest = changes.length - (max - 1);
  return [...changes.slice(0, max - 1), `…and ${rest} more changes`];
}

export interface ChannelRow {
  /** Wire channel: names and the channel's own parameters. */
  wire: number;
  /** The input's or output's own number: indices of `pre`, `out.*`. */
  index: number;
  /** "IN1", "OUT3", as the Console's sidebar. */
  descriptor: string;
  name: string;
}

/** Inputs every source has. */
const BASE_INPUTS = 2;

/**
 * How many inputs are live, as the Console's sidebar lists them
 * (`DSPViewModel.effectiveInputChannels`): two for S/PDIF, the configured
 * count for I2S, eight for ADAT, and for USB what the device reports streaming
 * (`activeInputChannels` of its status; two until it has been read). Never
 * fewer than two or more than the device has.
 */
export function liveInputs(s: Pick<DspState, "inputSource" | "i2sInputChannels" | "numInputChannels">, usbActive: number | null): number {
  let n: number;
  switch (s.inputSource) {
    case 1:
    case 4:
    case 5:
    case 6:
      n = BASE_INPUTS;
      break;
    case 2:
      n = s.i2sInputChannels;
      break;
    case 3:
      n = 8;
      break;
    default:
      n = usbActive ?? BASE_INPUTS;
  }
  return Math.max(BASE_INPUTS, Math.min(n, s.numInputChannels));
}

/** The live inputs (`liveInputs`): wire channels 0..count. */
export function inputRows(s: DspState, count: number): ChannelRow[] {
  const rows: ChannelRow[] = [];
  for (let i = 0; i < count; i++) {
    rows.push({ wire: i, index: i, descriptor: `IN${i + 1}`, name: s.channelNames[i] ?? `Input ${i + 1}` });
  }
  return rows;
}

/** The enabled outputs, as the Console's sidebar lists them: wire channel numInputChannels + output. */
export function outputRows(s: DspState): ChannelRow[] {
  const rows: ChannelRow[] = [];
  for (let o = 0; o < s.numOutputChannels; o++) {
    if (s.outputEnabled[o] === false) continue;
    const wire = s.numInputChannels + o;
    rows.push({ wire, index: o, descriptor: `OUT${o + 1}`, name: s.channelNames[wire] ?? `Output ${o + 1}` });
  }
  return rows;
}

/** UTF-8 bytes of a name, which the device's limits count. */
export function byteLength(s: string): number {
  return new TextEncoder().encode(s).length;
}

/** Arrays equal item by item (props of the memoised parts). */
export function sameItems<T>(a: readonly T[], b: readonly T[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function round1(v: number): number {
  return Math.round(v * 10) / 10;
}
