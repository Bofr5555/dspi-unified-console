// WP P3OV: the Overview's logic (PLAN-P3 U1, OD-P3j): every output of the
// session's device in one row (routing sources, gain, delay, mute, limiter
// threshold and gain reduction, clip latch) and every live channel's meter with
// its peak hold. Rows come from the session's mirror (`DspState`, the core's
// output labels); the live parts from the core's meter feed (`meters`: peaks,
// the peak hold its ballistics decay, the clip hold) and `limiterMeter`. No
// I/O, so the unit tests reach all of it.

import { peakDb, type DspState, type OutputLabel, type SystemStatus } from "../../core/protocol";
import { inputRows, outputLabel } from "../main/model";
import { clamp, compactGain, delayText, levelText, limiterSupported, stepGrid } from "../outputs/model";

/** One input routed into an output (a matrix crosspoint that is on). */
export interface OverviewSource {
  /** "IN1". */
  descriptor: string;
  name: string;
  gainDb: number;
  invert: boolean;
}

export interface OverviewRow {
  /** The output's index (`out.*`). */
  index: number;
  /** Its wire channel (meters, names). */
  wire: number;
  /** "OUT1". */
  descriptor: string;
  /** The core's label, else the stored name. */
  name: string;
  /** The firmware's slot ("I2S 1 L", "PDM"), for a tooltip; "" when unknown. */
  slot: string;
  enabled: boolean;
  sources: OverviewSource[];
  gainDb: number;
  delayMs: number;
  muted: boolean;
  /** null: the firmware has no limiter (wire format below 32). */
  limiter: { on: boolean; thresholdDb: number } | null;
}

/** The registry's word on a parameter: its range and its unit. */
export interface OverviewUnits {
  gain: { range: [number, number]; unit: string };
  delay: { unit: string };
  threshold: { unit: string };
}

/** What the Overview uses until the registry has answered (`describe`). */
export const DEFAULT_UNITS: OverviewUnits = {
  gain: { range: [-60, 20], unit: "dB" },
  delay: { unit: "ms" },
  threshold: { unit: "dBFS" },
};

function num(v: unknown, d = 0): number {
  return typeof v === "number" && Number.isFinite(v) ? v : d;
}

/** Every output of the device in `s`, enabled or not, in output order. */
export function overviewRows(s: DspState, labels?: OutputLabel[]): OverviewRow[] {
  const rows: OverviewRow[] = [];
  const limiter = limiterSupported(s);
  const outs = Math.max(0, Math.min(num(s.numOutputChannels), 64));
  const ins = Math.max(0, Math.min(num(s.numInputChannels), 64));
  for (let o = 0; o < outs; o++) {
    const sources: OverviewSource[] = [];
    for (let i = 0; i < ins; i++) {
      if (s.matrixRouting?.[i]?.[o] !== true) continue;
      sources.push({
        descriptor: `IN${i + 1}`,
        name: s.channelNames?.[i] ?? `Input ${i + 1}`,
        gainDb: num(s.matrixGain?.[i]?.[o]),
        invert: s.matrixInvert?.[i]?.[o] === true,
      });
    }
    rows.push({
      index: o,
      wire: ins + o,
      descriptor: `OUT${o + 1}`,
      name: outputLabel(s, labels, o),
      slot: labels?.[o]?.slotName ?? "",
      enabled: s.outputEnabled?.[o] !== false,
      sources,
      gainDb: num(s.outputGainDb?.[o]),
      delayMs: num(s.outputDelayMs?.[o]),
      muted: s.outputMuted?.[o] === true,
      limiter: limiter ? { on: s.limiterEnabled?.[o] === true, thresholdDb: num(s.limiterThresholdDb?.[o]) } : null,
    });
  }
  return rows;
}

/** A routing source as the row says it: "USB 1 0 dB", "USB 2 -6 dB inv". */
export function sourceText(src: OverviewSource): string {
  return `${src.name} ${compactGain(src.gainDb)}${src.invert ? " inv" : ""}`;
}

/** The routing cell: each source, or "None". */
export function routingText(row: Pick<OverviewRow, "sources">): string {
  return row.sources.length ? row.sources.map(sourceText).join(", ") : "None";
}

/** The gain, delay and threshold as the output card shows them, in the registry's units. */
export function valueTexts(row: OverviewRow, units: OverviewUnits = DEFAULT_UNITS): { gain: string; delay: string; threshold: string } {
  const delay = units.delay.unit === "ms" ? delayText(row.delayMs) : `${row.delayMs} ${units.delay.unit}`;
  return {
    gain: levelText(row.gainDb, units.gain.unit === "dBFS" ? "dBFS" : "dB"),
    delay,
    threshold: row.limiter ? (row.limiter.on ? levelText(row.limiter.thresholdDb, "dBFS") : "Off") : "—",
  };
}

/** The inline gain's − and + buttons: half a dB to the next half-dB step, inside the registry's range. */
export const GAIN_STEP = 0.5;

export function gainStep(v: number, dir: 1 | -1, range: readonly [number, number]): number {
  return stepGrid(v, dir, GAIN_STEP, range);
}

/** A typed gain, kept inside the registry's range (the core checks it again). */
export function gainTyped(v: number, range: readonly [number, number]): number {
  return clamp(v, range);
}

// ---------------------------------------------------------------- meters

/** One channel the Overview meters. */
export interface MeterChannel {
  wire: number;
  descriptor: string;
  name: string;
  kind: "input" | "output";
}

/** The live inputs (the Console's count, `liveInputs`) and every output, in wire order. */
export function meterChannels(s: DspState, labels: OutputLabel[] | undefined, liveIn: number): MeterChannel[] {
  const ins: MeterChannel[] = inputRows(s, liveIn).map((r) => ({ wire: r.wire, descriptor: r.descriptor, name: r.name, kind: "input" }));
  const outs: MeterChannel[] = overviewRows(s, labels).map((r) => ({ wire: r.wire, descriptor: r.descriptor, name: r.name, kind: "output" }));
  return [...ins, ...outs];
}

/** A channel's meter: its level and its held peak in dB (-90 floor), whether its clip marker is lit. */
export interface MeterReading {
  db: number;
  holdDb: number;
  clip: boolean;
}

export const SILENT: MeterReading = { db: -90, holdDb: -90, clip: false };

function bit(mask: unknown, i: number): boolean {
  return typeof mask === "number" && i >= 0 && i < 32 && ((mask >>> 0) & (2 ** i)) !== 0;
}

/**
 * The meter of `wire` from the core's feed. The hold marker is the core's
 * `peakHold` (its ballistics hold a peak, then let it fall), never below the
 * level itself; a core without ballistics holds nothing (the marker is the
 * level). The clip marker is the core's clip hold (not the raw sticky flags).
 */
export function meterReading(m: SystemStatus | null, wire: number): MeterReading {
  if (!m) return SILENT;
  const p = num(m.peaks?.[wire], 0);
  const h = Math.max(p, num(m.peakHold?.[wire], 0));
  return { db: peakDb(p), holdDb: peakDb(h), clip: bit(m.clipHold, wire) };
}

/** A meter's fill or marker position, 0..100 % over -90..0 dB. */
export function meterPct(db: number): number {
  return Math.max(0, Math.min(100, ((db + 90) / 90) * 100));
}

/** The core's display floor for gain reduction (`fetch_limiter_meter`): not a reading. */
export const GR_FLOOR_DB = 120;

/**
 * The gain reduction text from `limiterMeter` (dB, positive while limiting):
 * "-3.2 dB", "0 dB" when none; "—" with no reading, the display floor or no limiter on.
 */
export function grText(row: Pick<OverviewRow, "index" | "limiter">, gr: readonly number[] | null): string {
  if (!row.limiter?.on || !gr) return "—";
  const v = gr[row.index];
  if (typeof v !== "number" || !Number.isFinite(v) || v >= GR_FLOOR_DB) return "—";
  const db = Math.round(Math.max(0, v) * 10) / 10;
  return db === 0 ? "0 dB" : `-${db.toFixed(1)} dB`;
}

/** Whether any limiter is on (then the gain reduction is read). */
export function anyLimiter(rows: readonly OverviewRow[]): boolean {
  return rows.some((r) => r.limiter?.on === true);
}
