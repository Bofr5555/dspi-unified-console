// WP WB6d: the System view's signal chain of one device, read from today's
// `DspState`, the core's output labels and its meters (`SystemStatus`); no
// new core calls. Inputs (name, source, trim, meter, EQ), the routing summary
// (the core's label of what feeds each output), each output's processing
// blocks with their on or off state (EQ, crossover, limiter, the effects with
// an output mask) and the output itself (meter, gain, mute, delay), and the
// device-wide effects. A block names the screen that edits it (`open`). No
// I/O, so the unit tests reach it.

import { peakDb, type DspState, type FilterParams, type OutputLabel, type SystemStatus } from "../../core/protocol";
import { isCrossover } from "../eq/model";
import { inputRows, outputLabel } from "../main/model";
import { delayText, levelText, limiterSupported } from "../outputs/model";

/** Where a block opens: a screen, and on the EQ screen a channel and its PEQ or XO tab. */
export interface ChainTarget {
  screen: "main" | "eq" | "matrix";
  wire?: number;
  tab?: "peq" | "xo";
}

/** One processing block of a channel, or a device-wide effect. */
export interface ChainBlock {
  /** Stable id: "eq", "xo", "limiter", "loudness", "psybass", "tube", "subharm", "crossfeed", "leveller", "upmix". */
  id: string;
  label: string;
  on: boolean;
  /** What it does now, short ("3 bands", "-3.0 dBFS"); "" when nothing to say. */
  detail: string;
  /** The screen that edits it; null while no screen of this console has it. */
  open: ChainTarget | null;
}

export interface ChainInput {
  wire: number;
  descriptor: string;
  name: string;
  trimDb: number;
  blocks: ChainBlock[];
}

export interface ChainOutput {
  wire: number;
  index: number;
  descriptor: string;
  name: string;
  enabled: boolean;
  /** The core's label of what feeds it: "L", "R", "L+R", an input's name, "Mix" or "None". */
  from: string;
  /** The inputs routed to it, by descriptor ("IN1", "IN2"). */
  routedFrom: string[];
  blocks: ChainBlock[];
  gainDb: number;
  muted: boolean;
  delayMs: number;
  /** Its own screen: the EQ screen's output card when enabled, else the Matrix (where it is enabled). */
  open: ChainTarget;
}

export interface Chain {
  /** The whole DSP chain is bypassed (the Main window's Bypass). */
  bypass: boolean;
  userVolumeDb: number;
  userMute: boolean;
  inputs: ChainInput[];
  outputs: ChainOutput[];
  /** The effects that act on the device as a whole, not per output. */
  deviceEffects: ChainBlock[];
}

/**
 * The PEQ bands of `wire` that act: the device's own count (ten where the state does not
 * say, as the EQ screen takes it), not off, not bypassed, not a crossover.
 */
export function activePeq(s: Pick<DspState, "filters" | "maxBands">, wire: number): number {
  const bands: FilterParams[] = (s.filters?.[wire] ?? []).slice(0, s.maxBands || 10);
  return bands.filter((p) => p.filterType !== 0 && !p.bypass && !isCrossover(p.filterType)).length;
}

/** The crossover bands of `wire` that act: set and not bypassed. */
export function activeXover(s: Pick<DspState, "xover">, wire: number): number {
  return (s.xover?.[wire] ?? []).filter((p) => p.filterType !== 0 && !p.bypass).length;
}

function bands(n: number): string {
  return n === 0 ? "" : n === 1 ? "1 band" : `${n} bands`;
}

function bit(mask: unknown, i: number): boolean {
  return typeof mask === "number" && i >= 0 && i < 31 && (mask & (1 << i)) !== 0;
}

function num(v: unknown): number | null {
  return typeof v === "number" && Number.isFinite(v) ? v : null;
}

function param(v: unknown, i: number): number | null {
  return Array.isArray(v) ? num(v[i]) : null;
}

/** The firmware's parameter ids (consts.rs): enabled is 0, the output mask 1, for the tube and the subharmonic synth. */
const ENABLED = 0;
const MASK = 1;

/**
 * The effects with an output mask (bit k: output k, as the Console sets them):
 * whether each one is on for the device and its mask. Absent from a firmware
 * without the effect (no field, or an empty parameter list).
 */
function maskedEffects(s: DspState): { id: string; label: string; on: boolean; mask: number }[] {
  const fx: { id: string; label: string; on: boolean; mask: number }[] = [];
  if (typeof s.loudnessEnabled === "boolean") fx.push({ id: "loudness", label: "Loudness", on: s.loudnessEnabled, mask: num(s.loudnessOutputMask) ?? 0 });
  if (typeof s.psybassEnabled === "boolean") fx.push({ id: "psybass", label: "Psychoacoustic bass", on: s.psybassEnabled, mask: num(s.psybassOutputMask) ?? 0 });
  const tubeOn = param(s.tube, ENABLED);
  if (tubeOn !== null) fx.push({ id: "tube", label: "Tube", on: tubeOn >= 0.5, mask: param(s.tube, MASK) ?? 0 });
  const subOn = param(s.subharm, ENABLED);
  if (subOn !== null) fx.push({ id: "subharm", label: "Subharmonic", on: subOn >= 0.5, mask: param(s.subharm, MASK) ?? 0 });
  return fx;
}

/** The effects that act on the device as a whole: crossfeed, the leveller, the upmixer. */
function deviceEffects(s: DspState, masked: ReturnType<typeof maskedEffects>): ChainBlock[] {
  const out: ChainBlock[] = [];
  if (typeof s.crossfeedEnabled === "boolean") out.push({ id: "crossfeed", label: "Crossfeed", on: s.crossfeedEnabled, detail: "", open: null });
  if (typeof s.levellerEnabled === "boolean") out.push({ id: "leveller", label: "Leveller", on: s.levellerEnabled, detail: "", open: null });
  const up = param(s.upmix, ENABLED);
  if (up !== null) out.push({ id: "upmix", label: "Upmixer", on: up >= 0.5, detail: "", open: null });
  // The per-output effects are listed here too, with the outputs they act on.
  for (const f of masked) {
    const on: string[] = [];
    for (let o = 0; o < s.numOutputChannels; o++) if (bit(f.mask, o)) on.push(`OUT${o + 1}`);
    out.push({ id: f.id, label: f.label, on: f.on, detail: f.on ? (on.length ? on.join(", ") : "No outputs") : "", open: null });
  }
  return out;
}

/** The chain of the device in `s`: `liveIn` inputs (the Console's count), every output. */
export function chainView(s: DspState, labels: OutputLabel[] | undefined, liveIn: number): Chain {
  const inputs: ChainInput[] = inputRows(s, liveIn).map((r) => {
    const n = activePeq(s, r.wire);
    return {
      wire: r.wire,
      descriptor: r.descriptor,
      name: r.name,
      trimDb: s.inputPreampDb?.[r.index] ?? 0,
      blocks: [{ id: "eq", label: "EQ", on: n > 0, detail: bands(n), open: { screen: "eq", wire: r.wire, tab: "peq" } }],
    };
  });
  const masked = maskedEffects(s);
  const limiter = limiterSupported(s);
  const outputs: ChainOutput[] = [];
  for (let o = 0; o < s.numOutputChannels; o++) {
    const wire = s.numInputChannels + o;
    const enabled = s.outputEnabled?.[o] !== false;
    // A disabled output is not on the EQ screen: its blocks open the Matrix, where it is enabled.
    const at = (tab?: "peq" | "xo"): ChainTarget => (enabled ? { screen: "eq", wire, tab } : { screen: "matrix" });
    const peq = activePeq(s, wire);
    const xo = activeXover(s, wire);
    const blocks: ChainBlock[] = [
      { id: "eq", label: "EQ", on: peq > 0, detail: bands(peq), open: at("peq") },
      { id: "xo", label: "Crossover", on: xo > 0, detail: xo > 0 ? (labels?.[o]?.band ?? bands(xo)) : "", open: at("xo") },
    ];
    if (limiter) {
      const on = s.limiterEnabled?.[o] === true;
      const th = num(s.limiterThresholdDb?.[o]);
      blocks.push({ id: "limiter", label: "Limiter", on, detail: on && th !== null ? levelText(th, "dBFS") : "", open: at() });
    }
    for (const f of masked) if (f.on) blocks.push({ id: f.id, label: f.label, on: bit(f.mask, o), detail: "", open: null });
    const routedFrom: string[] = [];
    for (let i = 0; i < s.numInputChannels; i++) if (s.matrixRouting?.[i]?.[o]) routedFrom.push(`IN${i + 1}`);
    outputs.push({
      wire,
      index: o,
      descriptor: `OUT${o + 1}`,
      name: outputLabel(s, labels, o),
      enabled,
      from: labels?.[o]?.channel || (routedFrom.length ? routedFrom.join("+") : "None"),
      routedFrom,
      blocks,
      gainDb: s.outputGainDb?.[o] ?? 0,
      muted: s.outputMuted?.[o] === true,
      delayMs: s.outputDelayMs?.[o] ?? 0,
      open: at(),
    });
  }
  return {
    bypass: s.bypass === true,
    userVolumeDb: s.userVolumeDb,
    userMute: s.userMute === true,
    inputs,
    outputs,
    deviceEffects: deviceEffects(s, masked),
  };
}

/** A channel's meter from the status: its level in dB (-90 for none) and whether its clip marker is lit. */
export function meterOf(m: SystemStatus | null, wire: number): { db: number; clip: boolean } {
  if (!m) return { db: -90, clip: false };
  const p = m.peaks?.[wire];
  return { db: typeof p === "number" ? peakDb(p) : -90, clip: bit(m.clipHold, wire) };
}

/** A meter's fill, 0..100 % over -90..0 dB. */
export function meterPct(db: number): number {
  return Math.max(0, Math.min(100, ((db + 90) / 90) * 100));
}

/** An output's level text: "-3.0 dB", its delay "1.5 ms", as the output card shows them. */
export function outputText(o: Pick<ChainOutput, "gainDb" | "delayMs">): { gain: string; delay: string } {
  return { gain: levelText(o.gainDb), delay: delayText(o.delayMs) };
}

/** What a block says: "EQ: on, 3 bands", "Limiter: off". For its accessible name and tooltip. */
export function blockText(b: ChainBlock): string {
  return `${b.label}: ${b.on ? "on" : "off"}${b.on && b.detail ? `, ${b.detail}` : ""}`;
}
