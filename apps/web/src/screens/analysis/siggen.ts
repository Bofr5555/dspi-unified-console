// WP WB6j: the signal generator's page rules (M46; the Console's TestSignalsView, Qt's
// SiggenController): the draft the page edits, what stops it from playing, the status line. The
// catalogue and each parameter's range are the core's (`siggenCaps`); staging and the controls are
// the core's calls, so its read-only, lock and history rules apply. The generator only plays when
// the person presses Play or Identify; nothing here starts it by itself.

import type { SiggenCapsInfo, SiggenConfigRecord, SiggenStatusRecord, SiggenTypeInfo } from "../../core/tools-protocol";
import type { Session } from "../../ui/session";

export const SINE = 0;
export const TIMING_CONTINUOUS = 0;
export const TIMING_SWEEP = 1;
export const TIMING_PATTERN = 2;

export const FLAG_RAW = 1;
export const FLAG_DECORR = 2;
export const FLAG_WALK = 4;

/** The level's range, dBFS, and where the slider starts (the Console's). */
export const LEVEL_MIN = -120;
export const LEVEL_MAX = 0;
export const SLIDER_MIN = -80;
export const DEFAULT_LEVEL = -20;
/** How often the status is read while it plays, ms, and how long a change waits before it is sent. */
export const STATUS_POLL_MS = 300;
export const LIVE_APPLY_MS = 350;
export const NOT_WALKING = 0xff;

/** The draft before any device: Sine 1 kHz on outputs 1 and 2 at -20 dBFS. */
export function initialDraft(): SiggenConfigRecord {
  return { signalType: SINE, channelMask: 0b11, invertMask: 0, flags: 0, levelDb: DEFAULT_LEVEL, durationMs: 0, repeat: 0, gapMs: 0, p: [1000, 0, 0, 0] };
}

export function typeOf(caps: SiggenCapsInfo | null, id: number): SiggenTypeInfo | null {
  return caps?.types.find((t) => t.id === id) ?? null;
}

const clampTo = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** The draft with type `id` chosen: its parameters at their defaults; a sweep gets a length. */
export function withType(d: SiggenConfigRecord, t: SiggenTypeInfo): SiggenConfigRecord {
  const p = [0, 1, 2, 3].map((i) => {
    const x = t.params[i];
    return x && x.semantic !== 0 ? x.def : 0;
  });
  const durationMs = t.timing === TIMING_SWEEP ? d.durationMs || 5000 : t.timing === TIMING_CONTINUOUS ? 0 : d.durationMs;
  return { ...d, signalType: t.id, p, durationMs };
}

/** Parameter `i` set to `v`, held to its range. */
export function withParam(d: SiggenConfigRecord, t: SiggenTypeInfo, i: number, v: number): SiggenConfigRecord {
  const x = t.params[i];
  if (!x || x.semantic === 0 || !Number.isFinite(v)) return d;
  const p = [...d.p];
  p[i] = clampTo(x.semantic === 4 || x.semantic === 6 ? Math.round(v) : v, x.min, x.max);
  return { ...d, p };
}

export function withLevel(d: SiggenConfigRecord, db: number): SiggenConfigRecord {
  return Number.isFinite(db) ? { ...d, levelDb: Math.round(clampTo(db, LEVEL_MIN, LEVEL_MAX) * 10) / 10 } : d;
}

/** The outputs the device can play on, as a mask. */
export function playableMask(caps: SiggenCapsInfo | null, outputs: number): number {
  const all = outputs >= 16 ? 0xffff : (1 << outputs) - 1;
  return caps?.validMask ? caps.validMask & all : all;
}

/** Why Play is off, "" when it may play. */
export function startBlocker(open: boolean, caps: SiggenCapsInfo | null, d: SiggenConfigRecord, readOnly: string): string {
  if (!open) return "No device connected";
  if (caps && !caps.supported) return "Firmware has no signal generator";
  if (readOnly) return readOnly;
  if (d.channelMask === 0) return "Select at least one output";
  const t = typeOf(caps, d.signalType);
  if (t?.timing === TIMING_SWEEP && d.durationMs === 0) return "Sweep length must be greater than 0";
  return "";
}

export function playing(s: SiggenStatusRecord | null): boolean {
  return !!s && s.state !== 0;
}

export function phaseText(state: number): string {
  switch (state) {
    case 1:
      return "Fading in";
    case 2:
      return "Running";
    case 3:
      return "Gap";
    case 4:
      return "Fading out";
    default:
      return "Idle";
  }
}

export function formatFrequency(hz: number): string {
  if (!Number.isFinite(hz) || hz <= 0) return "";
  return hz >= 1000 ? `${(hz / 1000).toFixed(hz >= 10000 ? 1 : 2)} kHz` : `${Math.round(hz)} Hz`;
}

export function formatElapsed(ms: number): string {
  const s = Math.floor(Math.max(0, ms) / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** The transport's title: what plays, or why it cannot, or that it is ready. */
export function transportTitle(caps: SiggenCapsInfo | null, d: SiggenConfigRecord, s: SiggenStatusRecord | null, blocker: string): string {
  const nameOf = (id: number) => typeOf(caps, id)?.name ?? "";
  if (playing(s)) return `${phaseText(s!.state)} · ${nameOf(s!.signalType)}`;
  if (blocker && caps?.supported) return blocker;
  return `Ready · ${nameOf(d.signalType)}`;
}

/** The transport's detail while it plays: elapsed, the sweep's frequency, the walked output. */
export function transportDetail(s: SiggenStatusRecord | null, outputName: (o: number) => string): string {
  if (!playing(s)) return "";
  const parts = [formatElapsed(s!.elapsedMs)];
  const f = formatFrequency(s!.currentFreq);
  if (f) parts.push(f);
  if (s!.activeChannel !== NOT_WALKING) parts.push(outputName(s!.activeChannel));
  if (s!.cyclesDone) parts.push(`pass ${s!.cyclesDone + 1}`);
  return parts.join(" · ");
}

/** Identify (M46): channel-ID blips on `output` (0-based) alone, the core's call; the session's error line on refusal. */
export async function identifyOutput(session: Session, output: number): Promise<boolean> {
  if (session.readOnlyReason) {
    session.report(session.readOnlyReason);
    return false;
  }
  try {
    await session.core.call("siggenIdentify", { output });
    return true;
  } catch (e) {
    session.fail(e);
    return false;
  }
}
