// -- WP WEB7 -- Library › AutoEQ (Qt's AutoEqPanel.qml, Modern body): the pairs a profile may go to,
// in Qt's words, and the preview's curve as a path for the frame.

import type { AutoEqHit, AutoEqPreviewRecord } from "../../core/protocol";
import { autoEqPairs, type PairChannel } from "../eq/AutoEq";

export interface Pair {
  left: PairChannel;
  right: PairChannel;
  /** "Inputs 1 and 2", "Outputs 3 and 4". */
  title: string;
}

/** The pairs (neighbours among the live inputs, then among the enabled outputs), titled as Qt's combo. */
export function pairsOf(channels: PairChannel[], inputs: number): Pair[] {
  return autoEqPairs(channels).map((p) => {
    const n = (c: PairChannel) => (c.output ? c.wire - inputs + 1 : c.wire + 1);
    return { left: p.left, right: p.right, title: `${p.left.output ? "Outputs" : "Inputs"} ${n(p.left)} and ${n(p.right)}` };
  });
}

/** "Crinacle · in-ear": a headphone's measurement source and its form factor. */
export function hitLine(h: Pick<AutoEqHit, "source" | "formFactor">): string {
  return [h.source, h.formFactor].filter(Boolean).join(" · ");
}

/** In-ears and earbuds get the ear, the rest headphones (Qt's row icon). */
export function hitIcon(h: Pick<AutoEqHit, "formFactor">): "ear" | "headphones" {
  return h.formFactor === "in-ear" || h.formFactor === "earbud" ? "ear" : "headphones";
}

export const PLOT = { w: 600, h: 220, fMin: 20, fMax: 20000 };

/** The dB range the frame shows: ±12 dB, wider when a curve goes further (in 6 dB steps). */
export function dbSpan(curves: number[][]): number {
  let m = 12;
  for (const c of curves) for (const v of c) if (Number.isFinite(v)) m = Math.max(m, Math.abs(v));
  return Math.min(30, Math.ceil(m / 6) * 6);
}

export function xOf(f: number): number {
  return (Math.log10(f / PLOT.fMin) / Math.log10(PLOT.fMax / PLOT.fMin)) * PLOT.w;
}
export function yOf(db: number, span: number): number {
  return PLOT.h / 2 - (db / span) * (PLOT.h / 2);
}

/** An SVG path through `curve` (dB at each of `freqs`), offset by `levelDb`; "" when there is nothing to draw. */
export function curvePath(freqs: readonly number[], curve: readonly number[], levelDb: number, span: number): string {
  const pts: string[] = [];
  for (let i = 0; i < Math.min(freqs.length, curve.length); i++) {
    const f = freqs[i]!;
    const v = curve[i]! + levelDb;
    if (!(f >= PLOT.fMin && f <= PLOT.fMax) || !Number.isFinite(v)) continue;
    pts.push(`${pts.length ? "L" : "M"}${xOf(f).toFixed(1)} ${yOf(Math.max(-span, Math.min(span, v)), span).toFixed(1)}`);
  }
  return pts.length > 1 ? pts.join(" ") : "";
}

/** The preview's curves, per channel of the pair. */
export function previewCurves(p: AutoEqPreviewRecord | null): { channel: number; curve: number[]; levelDb: number }[] {
  return (p?.channels ?? []).filter((c) => c.curve.length).map((c) => ({ channel: c.channel, curve: c.curve, levelDb: c.levelDb }));
}
