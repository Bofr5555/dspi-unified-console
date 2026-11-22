// The EQ screen's logic (WP WB6b), after the Console's graph editor
// (`PeqGraphModel.swift`, `PeqGraphEditor.swift`) and band list
// (`Components.swift` `FilterListView`): the filter types it offers, the
// shapes the graph creates, what a band's dot stands for, the graph's
// geometry, value text and entry, and the edits (drag, keys, wheel). No I/O
// and no filter maths: every level comes from the core's curves (`curves`,
// crates/dspi-wasm/src/curves.rs). The unit tests reach all of it.

import type { BandCurve, DspState, FilterParams } from "../../core/protocol";

/** The firmware's filter types the PEQ uses (config.h `enum FilterType`). */
export const T = {
  flat: 0,
  peaking: 1,
  lowShelf: 2,
  highShelf: 3,
  lowPass: 4,
  highPass: 5,
  notch: 6,
  allPass: 7,
  allPass1: 8,
  lowShelf1: 9,
  highShelf1: 10,
  linkwitz: 11,
  lowPass1: 12,
  highPass1: 13,
} as const;

/** The picker's order (the Console's `FilterType.allCases`, crossovers left out). */
const TYPE_ORDER = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13];

// -- WP CG34 --
/**
 * The filter types' names: the core's one table (`filterTypeNames`, the
 * Console's band-list names, "High Cut (12dB)"), which the EQ screen loads
 * once before it shows a band. The same for every session.
 */
const typeNames = new Map<number, string>();

export function setTypeNames(table: readonly { code: number; name: string }[]): void {
  for (const t of table) typeNames.set(t.code, t.name);
}

export function typeNamesLoaded(): boolean {
  return typeNames.size > 0;
}

export function typeName(t: number): string {
  return typeNames.get(t) ?? `Type ${t}`;
}

/**
 * Which types use Q and gain: `dspi_proto::FilterType::uses_q` / `uses_gain`
 * (the core answers the same per band in `curves`; a unit test holds the two
 * together). Needed before the core has seen a band of a new type.
 */
export function usesQ(t: number): boolean {
  return t === 1 || t === 2 || t === 3 || t === 4 || t === 5 || t === 6 || t === 7;
}
export function usesGain(t: number): boolean {
  return t === 1 || t === 2 || t === 3 || t === 9 || t === 10;
}

export function isCrossover(t: number): boolean {
  return t >= 32 && t <= 63;
}

/** A band the graph draws a dot for: not off, not a crossover. */
export function isGraphBand(p: FilterParams): boolean {
  return p.filterType !== T.flat && !isCrossover(p.filterType);
}

/**
 * The PEQ types this firmware takes (the Console's `availableFilterTypes`):
 * notch and the 2nd-order all-pass from 1.1.4, the 1st-order all-pass from
 * wire V13, 1st-order shelves V14, 1st-order passes V28. -- WP WB6c -- The
 * Linkwitz Transform on outputs only, from wire V22 (`includeLinkwitz`); its
 * target frequency and Q are the registry's `eq.fp` and `eq.qp` (CG34).
 */
export function availableTypes(s: Pick<DspState, "formatVersion" | "fwMajor" | "fwMinor" | "fwPatch">, output = false): number[] {
  const v114 = fwAtLeast(s, 1, 1, 4);
  const w = s.formatVersion;
  return TYPE_ORDER.filter((t) => {
    if (t === T.notch || t === T.allPass) return v114;
    if (t === T.allPass1) return w >= 13;
    if (t === T.lowShelf1 || t === T.highShelf1) return w >= 14;
    if (t === T.lowPass1 || t === T.highPass1) return w >= 28;
    if (t === T.linkwitz) return output && w >= 22;
    return true;
  });
}

/** Per-band bypass: firmware 1.1.4 (`firmwareSupportsBandBypass`). */
export function bypassSupported(s: Pick<DspState, "fwMajor" | "fwMinor" | "fwPatch">): boolean {
  return fwAtLeast(s, 1, 1, 4);
}

function fwAtLeast(s: Pick<DspState, "fwMajor" | "fwMinor" | "fwPatch">, a: number, b: number, c: number): boolean {
  return s.fwMajor !== a ? s.fwMajor > a : s.fwMinor !== b ? s.fwMinor > b : s.fwPatch >= c;
}

/** An off band, as the Console clears one (`clearChannelPEQ`). */
export const OFF_BAND: FilterParams = { filterType: 0, bypass: false, freq: 1000, q: 0.707, gain: 0, qp: 0.707 };

// ---------------------------------------------------------------- shapes

/** The shapes the graph creates and offers (`PeqShape`), with their orders. */
export const SHAPES = [
  { id: "bell", title: "Bell", types: { 2: T.peaking } },
  { id: "lowShelf", title: "Low Shelf", types: { 2: T.lowShelf, 1: T.lowShelf1 } },
  { id: "lowCut", title: "Low Cut", types: { 2: T.highPass, 1: T.highPass1 } },
  { id: "highShelf", title: "High Shelf", types: { 2: T.highShelf, 1: T.highShelf1 } },
  { id: "highCut", title: "High Cut", types: { 2: T.lowPass, 1: T.lowPass1 } },
  { id: "notch", title: "Notch", types: { 2: T.notch } },
  { id: "allPass", title: "All Pass", types: { 2: T.allPass, 1: T.allPass1 } },
] as const satisfies readonly { id: string; title: string; types: Partial<Record<1 | 2, number>> }[];

export type Shape = (typeof SHAPES)[number];

/** A type's shape and order; null for off, crossovers and the Linkwitz Transform. */
export function shapeOf(t: number): { shape: Shape; order: 1 | 2 } | null {
  for (const shape of SHAPES) {
    const types = shape.types as Partial<Record<1 | 2, number>>;
    if (types[2] === t) return { shape, order: 2 };
    if (types[1] === t) return { shape, order: 1 };
  }
  return null;
}

/** The slope names of a shape's two orders (`orderTitle`). */
export function orderTitle(shape: Shape, order: 1 | 2): string {
  if (shape.id === "allPass") return order === 1 ? "1st Order" : "2nd Order";
  return order === 1 ? "6 dB/oct" : "12 dB/oct";
}

/**
 * A band changed to another shape (`setShape`): frequency kept, gain and Q
 * carried over where the new type uses them (a Q it had none of starts at 1
 * for a bell or notch, 0.707 otherwise).
 */
export function retyped(p: FilterParams, t: number): FilterParams {
  // -- WP WB6c -- into a Linkwitz Transform: a neutral one, the target the driver
  // (fp = f0, Qp = Q0), as the core's `registry::retyped` and the Console do
  if (t === T.linkwitz && p.filterType !== T.linkwitz) {
    const freq = Math.max(p.freq, LT_FP[0]);
    const q = clamp(p.q, LT_Q);
    return { ...p, filterType: t, freq, q, gain: Math.min(freq, LT_FP[1]), qp: q };
  }
  const out = { ...p, filterType: t };
  if (!usesGain(t)) out.gain = 0;
  if (usesQ(t) && !usesQ(p.filterType)) out.q = t === T.peaking || t === T.notch ? 1 : 0.707;
  if (p.filterType === T.linkwitz && t !== T.linkwitz) out.gain = 0;
  return out;
}

// ---------------------------------------------------------------- limits

export interface Limits {
  freq: [number, number];
  q: [number, number];
  gain: [number, number];
}

/**
 * The registry's ranges (`eq.freq`, `eq.q`, `eq.gain`), which `write` checks;
 * these are its values until `describe` has answered.
 */
export const DEFAULT_LIMITS: Limits = { freq: [10, 24000], q: [0.1, 20], gain: [-24, 24] };

// -- WP WB6c --
/** A Linkwitz Transform's ranges: f0 and fp 10..14 400 Hz (`eq.fp`), Q0 and Qp 0.1..20 (`eq.qp`). */
export const LT_FP: [number, number] = [10, 14400];
export const LT_Q: [number, number] = [0.1, 20];

/** The Transform's low-frequency boost, dB (`40 log10(f0/fp)`); 0 for an unset one. */
export function dcBoost(f0: number, fp: number): number {
  return f0 > 0 && fp > 0 ? 40 * Math.log10(f0 / fp) : 0;
}

export function clamp(v: number, r: readonly [number, number]): number {
  return Math.min(Math.max(v, r[0]), r[1]);
}

// ---------------------------------------------------------------- geometry

/** Point ↔ frequency and dB, as the Console's `PeqGraphGeometry`: log frequency across, dB top to bottom. */
export interface Geometry {
  width: number;
  height: number;
  minFreq: number;
  maxFreq: number;
  dbTop: number;
  dbBottom: number;
}

export function x(g: Geometry, freq: number): number {
  const lo = Math.log10(g.minFreq);
  const span = Math.log10(g.maxFreq) - lo;
  return ((Math.log10(Math.max(freq, 1e-3)) - lo) / span) * g.width;
}
export function freqAt(g: Geometry, px: number): number {
  const lo = Math.log10(g.minFreq);
  const span = Math.log10(g.maxFreq) - lo;
  return 10 ** (lo + (px / Math.max(g.width, 1)) * span);
}
export function y(g: Geometry, db: number): number {
  return ((g.dbTop - db) / (g.dbTop - g.dbBottom)) * g.height;
}
export function dbAt(g: Geometry, py: number): number {
  return g.dbTop - (py / Math.max(g.height, 1)) * (g.dbTop - g.dbBottom);
}
export function dbPerPx(g: Geometry): number {
  return (g.dbTop - g.dbBottom) / Math.max(g.height, 1);
}

/**
 * A curve's level at `freq`, between the core's grid points (log-frequency
 * interpolation of the core's own values; outside the grid, its end).
 */
export function curveAt(freqs: number[], curve: number[], freq: number): number {
  const n = Math.min(freqs.length, curve.length);
  if (n === 0) return 0;
  if (freq <= freqs[0]!) return curve[0]!;
  if (freq >= freqs[n - 1]!) return curve[n - 1]!;
  // The grid is log-spaced: find the cell by its index.
  const lo = Math.log10(freqs[0]!);
  const step = (Math.log10(freqs[n - 1]!) - lo) / (n - 1);
  const pos = (Math.log10(freq) - lo) / step;
  const i = Math.min(n - 2, Math.max(0, Math.floor(pos)));
  const t = pos - i;
  return curve[i]! + (curve[i + 1]! - curve[i]!) * t;
}

// ---------------------------------------------------------------- dots

/**
 * What a band's dot stands for (`PeqNodeRole`): where it sits and what a
 * vertical drag changes.
 * - `gain`: on the band's curve at its frequency, `scale` dB per dB of gain
 *   (the core's `unit`); a vertical drag is gain.
 * - `resonance`: a 2nd-order cut, at its level at the corner, 20 log10 Q; a
 *   vertical drag is Q.
 * - `fixed`: a notch or all-pass on 0 dB, a 1st-order cut at its corner level
 *   (the core's `at`); a drag moves frequency only.
 * - `locked`: shown and selectable, not dragged (the Linkwitz Transform).
 */
export type Role = { kind: "gain"; scale: number } | { kind: "resonance" } | { kind: "fixed"; db: number } | { kind: "locked"; db: number };

export function roleOf(p: FilterParams, c: BandCurve | undefined): Role {
  const t = p.filterType;
  if (usesGain(t)) return { kind: "gain", scale: Math.max(Math.abs(c?.unit ?? (t === T.peaking ? 1 : 0.5)), 0.05) };
  if (t === T.lowPass || t === T.highPass) return { kind: "resonance" };
  if (t === T.lowPass1 || t === T.highPass1) return { kind: "fixed", db: c?.at ?? -3 };
  if (t === T.linkwitz) return { kind: "locked", db: c?.at ?? 0 };
  return { kind: "fixed", db: 0 };
}

export function roleDb(r: Role, p: FilterParams): number {
  switch (r.kind) {
    case "gain":
      return p.gain * r.scale;
    case "resonance":
      return 20 * Math.log10(Math.max(p.q, 0.01));
    default:
      return r.db;
  }
}

/** Bands in frequency order (ties by number): Tab, Shift-click ranges. */
export function frequencyOrder(bands: FilterParams[], ids: number[]): number[] {
  return [...ids].sort((a, b) => bands[a]!.freq - bands[b]!.freq || a - b);
}

/** Which band a double click on empty graph makes (`PeqCreation`): a bell at the pointer, or pulled from the curve, a shelf near either end. */
export function newBand(g: Geometry, px: number, py: number, fromCurve: boolean, limits: Limits, available: number[]): FilterParams {
  const fx = px / Math.max(g.width, 1);
  const freq = clamp(freqAt(g, px), graphFreq(g, limits));
  const p: FilterParams = { filterType: T.peaking, bypass: false, freq, q: 1, gain: 0, qp: 0.707 };
  if (fromCurve) {
    if (fx < 0.12 && available.includes(T.lowShelf)) return { ...p, filterType: T.lowShelf, q: 0.707 };
    if (fx > 0.88 && available.includes(T.highShelf)) return { ...p, filterType: T.highShelf, q: 0.707 };
    return p;
  }
  return { ...p, gain: round2(clamp(dbAt(g, py), limits.gain)) };
}

/** The frequencies a drag can reach: the registry's, inside the graph. */
export function graphFreq(g: Geometry, limits: Limits): [number, number] {
  return [Math.max(limits.freq[0], g.minFreq), Math.min(limits.freq[1], g.maxFreq)];
}

// ---------------------------------------------------------------- edits

export interface DragStart {
  grabbed: number;
  start: Map<number, FilterParams>;
  /** Ctrl- or Cmd-drag: Q of the bands. */
  qMode: boolean;
}

/** Q per 60 px of vertical travel, as on the Console. */
const Q_PX_PER_OCTAVE = 60;

/**
 * A drag's result (`dragResult`): the grabbed dot follows the pointer
 * (offset dx, dy in px, after fine scaling and any axis lock); the others
 * move by the same frequency ratio and gain step. Ctrl (scaleGains) scales
 * gains in proportion instead.
 */
export function dragResult(
  d: DragStart,
  dx: number,
  dy: number,
  g: Geometry,
  limits: Limits,
  roles: (b: number, p: FilterParams) => Role,
  scaleGains = false,
): Map<number, FilterParams> {
  const out = new Map<number, FilterParams>();
  const g0 = d.start.get(d.grabbed);
  if (!g0) return out;
  if (d.qMode) {
    const factor = 2 ** (-dy / Q_PX_PER_OCTAVE);
    for (const [b, s] of d.start) if (usesQ(s.filterType)) out.set(b, { ...s, q: round3(clamp(s.q * factor, limits.q)) });
    return out;
  }
  const range = graphFreq(g, limits);
  const newFreq = clamp(freqAt(g, x(g, g0.freq) + dx), range);
  const ratio = newFreq / g0.freq;
  const dbDelta = -dy * dbPerPx(g);
  const r0 = roles(d.grabbed, g0);
  const grabbed = { ...g0, freq: roundFreq(newFreq) };
  if (r0.kind === "locked") return out;
  if (r0.kind === "gain") grabbed.gain = round2(clamp(g0.gain + dbDelta / r0.scale, limits.gain));
  else if (r0.kind === "resonance") grabbed.q = round3(clamp(10 ** ((roleDb(r0, g0) + dbDelta) / 20), limits.q));
  out.set(d.grabbed, grabbed);
  const gainFactor = scaleGains && r0.kind === "gain" && Math.abs(g0.gain) >= 0.25 ? grabbed.gain / g0.gain : null;
  const gainOffset = r0.kind === "gain" ? grabbed.gain - g0.gain : null;
  for (const [b, s] of d.start) {
    if (b === d.grabbed) continue;
    const r = roles(b, s);
    if (r.kind === "locked") continue;
    const p = { ...s, freq: roundFreq(clamp(s.freq * ratio, range)) };
    if (usesGain(s.filterType)) {
      const gain = gainFactor !== null ? s.gain * gainFactor : s.gain + (gainOffset ?? dbDelta / (r.kind === "gain" ? r.scale : 1));
      p.gain = round2(clamp(gain, limits.gain));
    }
    out.set(b, p);
  }
  return out;
}

/**
 * The keys' steps (`nudge`): ←/→ a twelfth of an octave (Shift: 1/96), ↑/↓
 * half a dB (Shift: a tenth), Alt+↑/↓ Q by 2^0.1 (Shift: 2^0.02); a band
 * without gain takes ↑/↓ as Q.
 */
export function nudge(p: FilterParams, key: "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown", alt: boolean, fine: boolean, g: Geometry, limits: Limits): FilterParams | null {
  if (key === "ArrowLeft" || key === "ArrowRight") {
    const oct = (fine ? 1 / 96 : 1 / 12) * (key === "ArrowRight" ? 1 : -1);
    return { ...p, freq: roundFreq(clamp(p.freq * 2 ** oct, graphFreq(g, limits))) };
  }
  const up = key === "ArrowUp" ? 1 : -1;
  if (alt || !usesGain(p.filterType)) {
    if (!usesQ(p.filterType)) return null;
    return { ...p, q: round3(clamp(p.q * 2 ** ((fine ? 0.02 : 0.1) * up), limits.q)) };
  }
  return { ...p, gain: round2(clamp(p.gain + (fine ? 0.1 : 0.5) * up, limits.gain)) };
}

/** The wheel (`handleWheel`): Q, or with Ctrl gain; `delta` in px, already scaled for fine. */
export function wheel(p: FilterParams, delta: number, gain: boolean, limits: Limits): FilterParams | null {
  if (gain) return usesGain(p.filterType) ? { ...p, gain: round2(clamp(p.gain + delta * 0.05, limits.gain)) } : null;
  return usesQ(p.filterType) ? { ...p, q: round3(clamp(p.q * 2 ** (delta / 100), limits.q)) } : null;
}

/** A pinch (touch): Q of the band by how far the fingers moved apart (closer: narrower). */
export function pinch(p: FilterParams, ratio: number, limits: Limits): FilterParams | null {
  if (!usesQ(p.filterType) || !(ratio > 0)) return null;
  return { ...p, q: round3(clamp(p.q / ratio, limits.q)) };
}

/**
 * The writes that take a band from what the device has to `next`, by
 * registry path, type first (so the device takes the other fields for the
 * new type); bypass last. Nothing when they are the same.
 */
export function bandWrites(ch: number, b: number, now: FilterParams, next: FilterParams): { path: string; indices: number[]; value: { choice: number } | { float: number } | { bool: boolean } }[] {
  const at = [ch, b];
  const w: ReturnType<typeof bandWrites> = [];
  if (next.filterType !== now.filterType) w.push({ path: "eq.type", indices: at, value: { choice: next.filterType } });
  if (next.freq !== now.freq) w.push({ path: "eq.freq", indices: at, value: { float: next.freq } });
  if (next.q !== now.q) w.push({ path: "eq.q", indices: at, value: { float: next.q } });
  // -- WP WB6c -- a Linkwitz Transform's gain field is its target frequency, `eq.fp`, and its Qp `eq.qp`
  const lt = next.filterType === T.linkwitz;
  if (next.gain !== now.gain) w.push({ path: lt ? "eq.fp" : "eq.gain", indices: at, value: { float: next.gain } });
  if (lt && next.qp !== now.qp) w.push({ path: "eq.qp", indices: at, value: { float: next.qp } });
  if (next.bypass !== now.bypass) w.push({ path: "eq.bypass", indices: at, value: { bool: next.bypass } });
  return w;
}

// ---------------------------------------------------------------- text

/** The Console's value text (`PeqValueText`). */
export function freqText(hz: number): string {
  if (hz >= 10_000) return `${(hz / 1000).toFixed(2)} kHz`;
  if (hz >= 1000) return `${(hz / 1000).toFixed(3)} kHz`;
  if (hz >= 100) return `${hz.toFixed(1)} Hz`;
  return `${hz.toFixed(2)} Hz`;
}
export function shortFreq(hz: number): string {
  if (hz >= 1000) return `${(hz / 1000).toFixed(hz >= 10_000 ? 1 : 2)}k`;
  return hz.toFixed(hz >= 100 ? 0 : 1);
}
/** Short forms for a phone's band list, the units in its headings: "1.00k", "+6.0". */
export function shortGain(db: number): string {
  const v = Math.abs(db) < 0.05 ? 0 : db;
  return `${v >= 0 ? "+" : ""}${v.toFixed(1)}`;
}
export function gainText(db: number): string {
  const v = Math.abs(db) < 0.005 ? 0 : db;
  return `${v >= 0 ? "+" : ""}${v.toFixed(2)} dB`;
}
export function qText(q: number): string {
  return q.toFixed(3);
}

/** A typed frequency: "2k", "2 kHz", "440", or a note, "A4", "C#2+13" (cents). */
export function parseFreq(text: string): number | null {
  let s = text.trim().toLowerCase();
  const note = parseNote(s);
  if (note !== null) return note;
  s = s.replace(/\s+/g, "");
  let scale = 1;
  if (s.endsWith("khz")) {
    scale = 1000;
    s = s.slice(0, -3);
  } else if (s.endsWith("hz")) s = s.slice(0, -2);
  if (s.endsWith("k")) {
    scale = 1000;
    s = s.slice(0, -1);
  }
  return finite(s, scale);
}

/** A typed number, its unit ("dB") and a leading "q" or "+" allowed. */
export function parseNumber(text: string, unit: string): number | null {
  let s = text.trim().toLowerCase().replace(/\s+/g, "");
  if (unit && s.endsWith(unit)) s = s.slice(0, -unit.length);
  if (s.startsWith("q")) s = s.slice(1);
  if (s.startsWith("+")) s = s.slice(1);
  return finite(s, 1);
}

function finite(s: string, scale: number): number | null {
  if (!/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/.test(s)) return null;
  const v = Number(s) * scale;
  return Number.isFinite(v) ? v : null;
}

/** "A4" is 440 Hz; "C#2+13" is C sharp 2 raised 13 cents (C4 is middle C). */
export function parseNote(s: string): number | null {
  const m = /^([a-g])(#|b(?=-?\d))?(-?\d+)([-+]\d+(\.\d+)?)?$/.exec(s);
  if (!m) return null;
  const base: Record<string, number> = { c: 0, d: 2, e: 4, f: 5, g: 7, a: 9, b: 11 };
  const semitone = base[m[1]!]! + (m[2] === "#" ? 1 : m[2] === "b" ? -1 : 0);
  const midi = (Number(m[3]) + 1) * 12 + semitone + (m[4] ? Number(m[4]) / 100 : 0);
  return 440 * 2 ** ((midi - 69) / 12);
}

/** What a screen reader hears for a band: "Band 3: Peaking, 1.000 kHz, +3.00 dB, Q 1.000, bypassed". */
export function bandLabel(b: number, p: FilterParams): string {
  const parts = [typeName(p.filterType)];
  if (p.filterType !== T.flat) {
    parts.push(freqText(p.freq));
    if (usesGain(p.filterType)) parts.push(gainText(p.gain));
    if (usesQ(p.filterType)) parts.push(`Q ${qText(p.q)}`);
    // -- WP WB6c --
    if (p.filterType === T.linkwitz) parts.push(`Q0 ${qText(p.q)}`, `fp ${freqText(p.gain)}`, `Qp ${qText(p.qp)}`);
    if (p.bypass) parts.push("bypassed");
  }
  return `Band ${b + 1}: ${parts.join(", ")}`;
}

// -- WP WEBM2 --
/** The on-graph label of the chosen band, in short: "3 · Bell · 1.20kHz · +3.0 dB · Q 1.410". */
export function bandTag(b: number, p: FilterParams): string {
  const s = shapeOf(p.filterType);
  const parts = [String(b + 1), s ? s.shape.title : typeName(p.filterType), `${shortFreq(p.freq)}Hz`];
  if (usesGain(p.filterType)) parts.push(`${shortGain(p.gain)} dB`);
  if (usesQ(p.filterType)) parts.push(`Q ${qText(p.q)}`);
  if (p.bypass) parts.push("off");
  return parts.join(" · ");
}

// ---------------------------------------------------------------- colours

/** One hue per band (`PeqBandPalette`), so a band keeps its colour in the list. */
export const BAND_COLOURS = ["#ed7873", "#f2a35c", "#ebc966", "#8ccc85", "#5cc4ad", "#70adf0", "#949af0", "#ba91eb", "#e38ab8", "#cc9e80"];

export function bandColour(b: number): string {
  return BAND_COLOURS[((b % BAND_COLOURS.length) + BAND_COLOURS.length) % BAND_COLOURS.length]!;
}

/** One hue per channel on the graph. */
export const CHANNEL_COLOURS = ["#5ac8fa", "#ff9f0a", "#30d158", "#ff375f", "#bf5af2", "#ffd60a", "#64d2ff", "#ff6961", "#a2845e", "#8e8e93"];

export function channelColour(i: number): string {
  return CHANNEL_COLOURS[((i % CHANNEL_COLOURS.length) + CHANNEL_COLOURS.length) % CHANNEL_COLOURS.length]!;
}

// ---------------------------------------------------------------- rounding

/** Values as the fields show them, so a drag writes no noise digits. */
export function roundFreq(hz: number): number {
  return hz >= 1000 ? Math.round(hz) : hz >= 100 ? Math.round(hz * 10) / 10 : Math.round(hz * 100) / 100;
}
export function round2(v: number): number {
  return Math.round(v * 100) / 100;
}
export function round3(v: number): number {
  return Math.round(v * 1000) / 1000;
}

/** Bands equal field by field. */
export function sameBand(a: FilterParams, b: FilterParams): boolean {
  return a.filterType === b.filterType && a.bypass === b.bypass && a.freq === b.freq && a.q === b.q && a.gain === b.gain && a.qp === b.qp;
}
