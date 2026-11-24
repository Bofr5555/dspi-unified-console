// -- WP WEBM2 -- The Equaliser's Crossover and Delay & gain tabs (MODLAY 2), as data: an output's
// high and low pass, taken from its four crossover bands, and each output's distance and arrival
// from its delay. The tabs and the inspector edit through these, so both say the same thing.

import type { FilterParams } from "../../core/protocol";
import { FAMILIES, FAMILY_ORDERS, decodeXover, encodeXover } from "../outputs/model";
import { plainNumber } from "../../ui/number";

/** The speed of sound the distances use, m/s (as the mockup and Measure's alignment). */
export const SOUND_MPS = 343;

/** One pass of an output's crossover: the band that has it, its type and frequency. */
export interface Pass {
  /** The crossover band (0..3). */
  band: number;
  filterType: number;
  freq: number;
  bypass: boolean;
}

/** An output's high and low pass: the first band of each kind that is on. */
export function passes(bands: readonly FilterParams[]): { hp: Pass | null; lp: Pass | null } {
  let hp: Pass | null = null;
  let lp: Pass | null = null;
  bands.forEach((p, band) => {
    const x = decodeXover(p.filterType);
    if (x.family === 0) return;
    const pass = { band, filterType: p.filterType, freq: p.freq, bypass: !!p.bypass };
    if (x.highPass) hp ??= pass;
    else lp ??= pass;
  });
  return { hp, lp };
}

/** The band a new pass goes in: the first one that is off; null when all four are in use. */
export function freeBand(bands: readonly FilterParams[]): number | null {
  const at = bands.findIndex((p) => decodeXover(p.filterType).family === 0);
  return at < 0 ? null : at;
}

/** A pass's choices: Off, then every family's slopes ("LR4 24 dB/oct"); the value is the type. */
export function passChoices(highPass: boolean): { value: number; label: string }[] {
  const out = [{ value: 0, label: "Off" }];
  const short = ["", "LR", "BW", "BES"];
  FAMILIES.forEach((_, family) => {
    for (const order of FAMILY_ORDERS[family] ?? [])
      out.push({ value: encodeXover({ family, order, highPass }), label: `${short[family]}${order} ${order * 6} dB/oct` });
  });
  return out;
}

/** A pass in short ("LR4 2.40 kHz" style is the caller's; this is "LR4"), "Off" without one. */
export function passName(pass: Pass | null): string {
  if (!pass) return "Off";
  const x = decodeXover(pass.filterType);
  return `${["", "LR", "BW", "BES"][x.family]}${x.order}`;
}

/**
 * What choosing `type` for an output's high (or low) pass changes: the band it goes to and its new
 * type, or null when there is nothing to do (no free band for a new one). Off clears the band.
 */
export function setPass(bands: readonly FilterParams[], highPass: boolean, type: number): { band: number; filterType: number } | null {
  const cur = passes(bands)[highPass ? "hp" : "lp"];
  if (cur) return cur.filterType === type ? null : { band: cur.band, filterType: type };
  if (type === 0) return null;
  const band = freeBand(bands);
  return band === null ? null : { band, filterType: type };
}

// ---------------------------------------------------------------- delay and distance

/** The distance sound travels in `ms`, metres. */
export function delayToMetres(ms: number): number {
  return (ms * SOUND_MPS) / 1000;
}

export function metresToDelay(m: number): number {
  return (m * 1000) / SOUND_MPS;
}

/** A distance to read: "34.3 cm" under a metre, "1.72 m" from one. */
export function distanceText(m: number): string {
  return m < 1 ? `${(m * 100).toFixed(1)} cm` : `${m.toFixed(2)} m`;
}

/** Typed distance to metres: "1.2", "1.2 m", "35 cm"; null when it is not one or below 0. */
export function parseDistance(text: string): number | null {
  // -- WP WEB -- spaces, a decimal comma and U+2212 as Qt's fields take them (USR1)
  const t = plainNumber(text).toLowerCase();
  const m = /^([0-9]*\.?[0-9]+)(cm|mm|m)?$/.exec(t);
  if (!m) return null;
  const v = Number(m[1]);
  if (!Number.isFinite(v)) return null;
  return m[2] === "cm" ? v / 100 : m[2] === "mm" ? v / 1000 : v;
}

/** Each output's arrival bar, 0..1 of the longest delay (all 0 with no delay anywhere). */
export function arrivals(delaysMs: readonly number[]): number[] {
  const longest = Math.max(0, ...delaysMs);
  return delaysMs.map((d) => (longest > 0 ? Math.max(0, d) / longest : 0));
}
