// -- WP WEB7 -- The Signal Generator's rules Qt Modern's SiggenPanel adds over WB6j's (siggen.ts):
// an output's chip steps off → on → inverted (ø) → off (SiggenController::stepOutput), None clears
// the inversions too, the timing rows by the type's timing model (Duration for a continuous signal
// such as Sine, a dwell and passes when it walks, a sweep's length, repeats and gap, a pattern's
// periods and gap), and the transport's idle line ("2 outputs · peak -20.0 dBFS").

import type { SiggenConfigRecord, SiggenTypeInfo } from "../../core/tools-protocol";
import { FLAG_WALK, TIMING_CONTINUOUS, TIMING_PATTERN, TIMING_SWEEP } from "./siggen";

export const WHITE = 2;
export const PINK = 3;
export const CHANNEL_ID = 14;

/** Qt's stepOutput: off → on → on and inverted → off. */
export function stepOutput(d: SiggenConfigRecord, o: number): SiggenConfigRecord {
  if (o < 0 || o >= 16) return d;
  const bit = 1 << o;
  if (!(d.channelMask & bit)) return { ...d, channelMask: d.channelMask | bit };
  if (!(d.invertMask & bit)) return { ...d, invertMask: d.invertMask | bit };
  return { ...d, channelMask: d.channelMask & ~bit, invertMask: d.invertMask & ~bit };
}

/** None: no output, and no inversion left behind. */
export function noOutputs(d: SiggenConfigRecord): SiggenConfigRecord {
  return { ...d, channelMask: 0, invertMask: 0 };
}

/** Whether the generator walks the outputs one at a time (Channel ID always does). */
export function walks(d: SiggenConfigRecord): boolean {
  return d.signalType === CHANNEL_ID || (d.flags & FLAG_WALK) !== 0;
}

export type TimingRowKind = "duration" | "repeat" | "gap";

export interface TimingRow {
  kind: TimingRowKind;
  label: string;
  caption: string;
  unit: string;
}

/** The timing rows Qt shows for this type and draft, in Qt's words. */
export function timingRows(t: SiggenTypeInfo | null, d: SiggenConfigRecord): TimingRow[] {
  if (!t) return [];
  const walk = t.timing === TIMING_CONTINUOUS && walks(d);
  if (t.timing === TIMING_SWEEP)
    return [
      { kind: "duration", label: "Sweep length", caption: "", unit: "s" },
      { kind: "repeat", label: "Repeat", caption: "0 = repeat forever", unit: "" },
      { kind: "gap", label: "Gap between sweeps", caption: "", unit: "ms" },
    ];
  if (t.timing === TIMING_PATTERN)
    return [
      { kind: "repeat", label: "Repeat", caption: t.id === CHANNEL_ID ? "Passes over the selected outputs. 0 = forever" : "Pattern periods. 0 = repeat forever", unit: "" },
      { kind: "gap", label: "Extra gap per period", caption: "", unit: "ms" },
    ];
  if (walk)
    return [
      { kind: "duration", label: "Dwell per channel", caption: "0 = 2 s default", unit: "s" },
      { kind: "repeat", label: "Passes", caption: "Full passes over the outputs. 0 = forever", unit: "" },
    ];
  return [{ kind: "duration", label: "Duration", caption: "0 = play until stopped", unit: "s" }];
}

/** A timing field's value as typed, held to Qt's ranges. */
export function withTiming(d: SiggenConfigRecord, kind: TimingRowKind, v: number, sweep: boolean): SiggenConfigRecord {
  if (!Number.isFinite(v)) return d;
  if (kind === "duration") return { ...d, durationMs: Math.round(Math.min(600, Math.max(sweep ? 0.01 : 0, v)) * 1000) };
  if (kind === "repeat") return { ...d, repeat: Math.round(Math.min(65535, Math.max(0, v))) };
  return { ...d, gapMs: Math.round(Math.min(65535, Math.max(0, v))) };
}

/** The transport's line while nothing plays: how many outputs, how many inverted, the peak level. */
export function idleDetail(d: SiggenConfigRecord): string {
  let on = 0;
  let inv = 0;
  for (let o = 0; o < 16; o++) {
    if (d.channelMask & (1 << o)) {
      on++;
      if (d.invertMask & (1 << o)) inv++;
    }
  }
  const outs = on === 1 ? "1 output" : `${on} outputs`;
  return `${outs}${inv ? ` (${inv} inverted)` : ""} · peak ${d.levelDb.toFixed(1)} dBFS`;
}
