// The outputs slice's logic (WP WB6c), after the Console's output page
// (`ChannelSettingsView`, `crossoverRowContent` in Components.swift), its
// Matrix Mixer (`MatrixMixerView.swift`) and its Output Limiter
// (`OutputLimiterView.swift`): the crossover's family, pass and slope, the
// level and delay controls' steps (owner rule SL1), the value text, the
// limiter's link groups and the Core 1 conflict of enabling an output. No
// I/O; the unit tests reach all of it.

import type { DspState } from "../../core/protocol";
import { readNumber } from "../../ui/number";

// ---------------------------------------------------------------- crossover

/** Crossover bands per output (wire bands 20..23). */
export const XOVER_BANDS = 4;
/** The first crossover band on the wire. */
export const XOVER_BASE = 20;

/** The crossover families, in the Console's order (`CrossoverFamilyOption`); 0 is off. */
export const FAMILIES = ["Off", "Linkwitz-Riley", "Butterworth", "Bessel"] as const;

/** The orders each family takes (`CrossoverFamily.orders`): LR and Bessel even, Butterworth 1..8. */
export const FAMILY_ORDERS: readonly (readonly number[])[] = [[], [2, 4, 6, 8], [1, 2, 3, 4, 5, 6, 7, 8], [2, 4, 6, 8]];

export interface Xover {
  /** 0 off, 1 Linkwitz-Riley, 2 Butterworth, 3 Bessel. */
  family: number;
  order: number;
  highPass: boolean;
}

/**
 * A crossover type's family, order and pass, from the firmware's numbering
 * (config.h: LR2..LR8 32..39, BW1..BW8 40..55, BES2..BES8 56..63; even low
 * pass, odd high pass). Anything else is off.
 */
export function decodeXover(t: number): Xover {
  if (t < 32 || t > 63) return { family: 0, order: 2, highPass: false };
  const rel = t - 32;
  const highPass = (rel & 1) === 1;
  const pair = rel >> 1;
  if (pair <= 3) return { family: 1, order: (pair + 1) * 2, highPass };
  if (pair <= 11) return { family: 2, order: pair - 3, highPass };
  return { family: 3, order: (pair - 11) * 2, highPass };
}

export function encodeXover(x: Xover): number {
  if (x.family === 0) return 0;
  const base = x.family === 1 ? 32 + (x.order / 2 - 1) * 2 : x.family === 2 ? 40 + (x.order - 1) * 2 : 56 + (x.order / 2 - 1) * 2;
  return base + (x.highPass ? 1 : 0);
}

/** A family chosen (`applyFamily`): the order kept if it has it, else the nearest; from off, 12 dB/oct low pass. */
export function withFamily(t: number, family: number): number {
  if (family === 0) return 0;
  const cur = decodeXover(t);
  const from = cur.family === 0 ? { order: 2, highPass: false } : cur;
  const orders = FAMILY_ORDERS[family] ?? [];
  let order = orders[0] ?? 2;
  for (const o of orders) if (Math.abs(o - from.order) < Math.abs(order - from.order)) order = o;
  return encodeXover({ family, order, highPass: from.highPass });
}

/** A slope's name (`"\(order*6) dB/oct"`). */
export function slopeText(order: number): string {
  return `${order * 6} dB/oct`;
}

// ---------------------------------------------------------------- level and delay steps

/** A value with -0 as 0 (the owner's rule: never send -0). */
export function noNegZero(v: number): number {
  return v === 0 ? 0 : v;
}

/** A slider's drag and release on a grid of `step` (the delay: whole ms), as the Console sends them. */
export function onGrid(v: number, step: number): number {
  return noNegZero(roundTo(Math.round(v / step) * step, step));
}

/**
 * The wheel or a key from `v` to the next grid value up (`dir` 1) or down
 * (-1): from 12.3 ms up is 13, down 12; from 12 up 13. Clamped to the range.
 */
export function stepGrid(v: number, dir: 1 | -1, step: number, range: readonly [number, number], steps = 1): number {
  const n = v / step;
  const k = dir > 0 ? Math.floor(n + 1e-6) + steps : Math.ceil(n - 1e-6) - steps;
  return noNegZero(roundTo(Math.min(Math.max(k * step, range[0]), range[1]), step));
}

function roundTo(v: number, step: number): number {
  const d = step >= 1 ? 0 : Math.ceil(-Math.log10(step) - 1e-9);
  const f = 10 ** d;
  return Math.round(v * f) / f;
}

/** A delay as it is, to 0.0001 ms (the core keeps that), with at least one decimal: "12.0 ms", "12.3456 ms". */
export function delayText(ms: number): string {
  return `${trimmed(ms, 4, 1)} ms`;
}

/** A level on the output card: "-3.0 dB"; a limiter's threshold is "-3.0 dBFS" (`docs/GLOSSARY.md`). */
export function levelText(db: number, unit: "dB" | "dBFS" = "dB"): string {
  return `${noNegZero(Math.round(db * 10) / 10).toFixed(1)} ${unit}`;
}

/** A crosspoint's or the matrix's level (`CompactGainField`): "0 dB", "+3.5 dB", "-12.25 dB". */
export function compactGain(db: number): string {
  const v = noNegZero(Math.round(db * 100) / 100);
  if (v === 0) return "0 dB";
  return `${v > 0 ? "+" : ""}${trimmed(v, 2, 0)} dB`;
}

function trimmed(v: number, max: number, min: number): string {
  let s = noNegZero(Math.round(v * 10 ** max) / 10 ** max).toFixed(max);
  while (s.includes(".") && s.length > 0 && s.endsWith("0") && s.split(".")[1]!.length > min) s = s.slice(0, -1);
  if (s.endsWith(".")) s = s.slice(0, -1);
  return s;
}

/** A typed level or delay: a number with its unit allowed ("-3", "12.5 ms", "+2 dB"). */
export function parseLevel(text: string, unit: string): number | null {
  // -- WP WEB -- as Qt's fields (USR1): a decimal comma, U+2212, the unit after it; a dBFS field takes "-6 dB" too
  return readNumber(text, unit === "dbfs" ? ["dbfs", "db"] : unit ? [unit] : []);
}

export function clamp(v: number, r: readonly [number, number]): number {
  return Math.min(Math.max(v, r[0]), r[1]);
}

// ---------------------------------------------------------------- limiter

/** The limiter: wire format 32 (`firmwareSupportsLimiter`). */
export function limiterSupported(s: Pick<DspState, "formatVersion">): boolean {
  return s.formatVersion >= 32;
}

/** Link groups 1..4 (`LIMITER_LINK_GROUP_MAX`). */
export const LINK_GROUPS = 4;

/** The PDM output: the last one (`pdmOutputIndex`). */
export function pdmOutput(s: Pick<DspState, "numOutputChannels">): number {
  return s.numOutputChannels - 1;
}

/** "Link all stereo pairs" (`stereoPairGroups`): 1+2 group 1, 3+4 group 2, as far as four go; PDM unlinked. */
export function stereoPairGroups(s: Pick<DspState, "numOutputChannels">): number[] {
  const pdm = pdmOutput(s);
  return Array.from({ length: s.numOutputChannels }, (_, o) => {
    const pair = Math.floor(o / 2) + 1;
    return o < pdm && pair <= LINK_GROUPS ? pair : 0;
  });
}

/** The link summary (`linkSummary`): "Not linked.", "No other outputs in group 2.", "Linked with A, B and C." */
export function linkSummary(group: number, others: string[]): string {
  if (group === 0) return "Not linked.";
  if (!others.length) return `No other outputs in group ${group}.`;
  const last = others[others.length - 1]!;
  return `Linked with ${others.length === 1 ? last : `${others.slice(0, -1).join(", ")} and ${last}`}.`;
}

// ---------------------------------------------------------------- outputs and Core 1

/** The EQ-worker outputs that share Core 1 with PDM (`eqWorkerRange`): 3-4 on RP2040, 3-8 on RP2350 (0-based 2..3, 2..7). */
export function eqWorkerRange(s: Pick<DspState, "platformId">): [number, number] {
  return s.platformId === 1 ? [2, 7] : [2, 3];
}

/**
 * Whether enabling `output` collides on Core 1 (`outputEnableWouldConflict`):
 * PDM while an EQ-worker output is on, or one of those while PDM is. The
 * core refuses such an enable by itself; the Console asks to switch over.
 */
export function enableConflict(s: Pick<DspState, "platformId" | "numOutputChannels" | "outputEnabled">, output: number): "pdm" | "worker" | null {
  const [a, b] = eqWorkerRange(s);
  const pdm = pdmOutput(s);
  if (output === pdm) {
    for (let o = a; o <= b; o++) if (s.outputEnabled[o]) return "pdm";
    return null;
  }
  if (output >= a && output <= b && s.outputEnabled[pdm]) return "worker";
  return null;
}

/** A matrix column's slot name (`MatrixOutput.visible`): "SPDIF 1 L" .. "PDM". */
export function outputSlotName(s: Pick<DspState, "numOutputChannels"> & { outputTypes?: unknown }, o: number): string {
  if (o === pdmOutput(s)) return "PDM";
  const types = Array.isArray(s.outputTypes) ? (s.outputTypes as number[]) : [];
  const slot = Math.floor(o / 2);
  return `${types[slot] === 1 ? "I2S" : "SPDIF"} ${slot + 1} ${o % 2 === 0 ? "L" : "R"}`;
}
