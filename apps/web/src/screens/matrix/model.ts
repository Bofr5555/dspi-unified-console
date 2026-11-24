// -- WP MXW -- The matrix's rules without I/O (DESREV's redesign as MX1 built it in Qt): the tile's
// and the row tail's words, the inspector's one-line summary, the keyboard's moves, a gain's steps,
// and "Link L/R pairs": which crosspoint mirrors an edit and the per-device choice. The unit tests
// (test/unit/matrix_mxw.test.ts) reach all of it.

import type { KeyValueStore } from "../../fleet/local";
import { noNegZero, stepGrid } from "../outputs/model";

/** A crosspoint's gain inside its tile: "0", "-6", "+1.5" (the unit is the footer's, once). */
export function tileGain(db: number): string {
  const v = noNegZero(Math.round(db * 100) / 100);
  if (v === 0) return "0";
  const t = String(Math.abs(v));
  return `${v > 0 ? "+" : "-"}${t}`;
}

/** A row's tail: "→ Out 1, 2, 3" or "→ nothing" (outputs by number). */
export function tailText(outputs: readonly number[]): string {
  return outputs.length ? `→ Out ${outputs.map((o) => o + 1).join(", ")}` : "→ nothing";
}

/** Numbers 1-based as runs: [3, 5, 6, 7, 8] is "4, 6–9". */
export function runs(zeroBased: readonly number[]): string {
  const n = [...zeroBased].sort((a, b) => a - b).map((x) => x + 1);
  const out: string[] = [];
  for (let k = 0; k < n.length; ) {
    let e = k;
    while (e + 1 < n.length && n[e + 1] === n[e]! + 1) e++;
    out.push(e - k >= 2 ? `${n[k]}–${n[e]}` : e > k ? `${n[k]}, ${n[e]}` : `${n[k]}`);
    k = e + 1;
  }
  return out.join(", ");
}

/** `[input][output]`: the crosspoints that route (`DspState.matrixRouting`). */
export type Routing = readonly (readonly boolean[] | undefined)[];

/** The outputs input `i` feeds. */
export function feedsOf(routing: Routing, i: number, outputs: number): number[] {
  return Array.from({ length: outputs }, (_, o) => o).filter((o) => routing[i]?.[o] === true);
}

/** The inputs (of the first `inputs`) that feed output `o`. */
export function sourcesOf(routing: Routing, o: number, inputs: number): number[] {
  return Array.from({ length: inputs }, (_, i) => i).filter((i) => routing[i]?.[o] === true);
}

/** The inspector with nothing chosen: "4 crosspoints · 4 of 9 outputs on · Out 4, 6–9 fed by nothing". */
export function summaryText(routing: Routing, enabled: readonly boolean[], inputs: number, outputs: number): string {
  let n = 0;
  for (let i = 0; i < inputs; i++) n += feedsOf(routing, i, outputs).length;
  const on = Array.from({ length: outputs }, (_, o) => o).filter((o) => enabled[o] === true).length;
  const idle = Array.from({ length: outputs }, (_, o) => o).filter((o) => !sourcesOf(routing, o, inputs).length);
  const parts = [n === 1 ? "1 crosspoint" : `${n} crosspoints`, `${on} of ${outputs} outputs on`];
  if (idle.length) parts.push(`Out ${runs(idle)} fed by nothing`);
  return parts.join(" · ");
}

// ---------------------------------------------------------------- Link L/R pairs

/**
 * The crosspoint an edit of (i, o) is mirrored to while pairs are linked: the stereo partner row
 * one column over inside the output pair, `(i ^ 1, o ^ 1)` (L→Out 1 ⇄ R→Out 2, L→Out 2 ⇄ R→Out 1).
 * None for an input or output without a partner (an odd count's last row; the PDM output).
 */
export function mirrorOf(i: number, o: number, inputs: number, outputs: number): { i: number; o: number } | null {
  const pi = i ^ 1;
  const po = o ^ 1;
  return pi < inputs && po < outputs ? { i: pi, o: po } : null;
}

const LINK_KEY = "dspi.matrix.linkPairs.";

/** Whether this device's matrix links its pairs: off until chosen, then as chosen (per serial). */
export function linkPairsOf(store: KeyValueStore | null, serial: string): boolean {
  try {
    return store?.getItem(LINK_KEY + serial) === "1";
  } catch {
    return false;
  }
}

export function setLinkPairs(store: KeyValueStore | null, serial: string, on: boolean): void {
  try {
    store?.setItem(LINK_KEY + serial, on ? "1" : "0");
  } catch {
    // Not kept (a private window); the choice holds while the page is open.
  }
}

// ---------------------------------------------------------------- keys

export interface CellAt {
  i: number;
  o: number;
}

/** Where a key moves the current cell; null when it is not a move. */
export function moveCell(at: CellAt, key: string, inputs: number, outputs: number): CellAt | null {
  const clampI = (v: number) => Math.max(0, Math.min(inputs - 1, v));
  const clampO = (v: number) => Math.max(0, Math.min(outputs - 1, v));
  switch (key) {
    case "ArrowLeft":
      return { i: at.i, o: clampO(at.o - 1) };
    case "ArrowRight":
      return { i: at.i, o: clampO(at.o + 1) };
    case "ArrowUp":
      return { i: clampI(at.i - 1), o: at.o };
    case "ArrowDown":
      return { i: clampI(at.i + 1), o: at.o };
    case "Home":
      return { i: at.i, o: 0 };
    case "End":
      return { i: at.i, o: outputs - 1 };
    case "PageUp":
      return { i: 0, o: at.o };
    case "PageDown":
      return { i: inputs - 1, o: at.o };
    default:
      return null;
  }
}

/**
 * -- WP WEB2 -- (REVD G1) Which way a "+" or "−" key steps a crosspoint's gain: by the key's place
 * (`code`) as well as its character, since Alt turns "-" into a dead key or a dash on some layouts and
 * Shift is how "+" is typed on most. 0 for any other key. Stepping needs Alt or Ctrl; the plain keys
 * start a typed gain, so "-6" is −6 dB.
 */
export function stepSign(e: { key: string; code?: string }): 1 | -1 | 0 {
  if (e.key === "+" || e.key === "=" || e.code === "Equal" || e.code === "NumpadAdd") return 1;
  if (e.key === "-" || e.key === "_" || e.key === "\u2212" || e.code === "Minus" || e.code === "NumpadSubtract") return -1;
  return 0;
}

/** A crosspoint's gain one step on: 0.5 dB, or 3 dB (`big`: Ctrl on a key, Shift on the wheel; Alt on a key is 0.5). */
export function stepGain(db: number, dir: 1 | -1, big: boolean, range: readonly [number, number]): number {
  return stepGrid(db, dir, 0.5, range, big ? 6 : 1);
}

/** Many columns: the dense grid by itself (DESREV: above 12). */
export const DENSE_FROM = 13;

/** Clear asks first when more than this many crosspoints would go. */
export const CLEAR_ASKS_ABOVE = 4;
