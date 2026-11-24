// -- WP WEB3 -- (DL6 on the web) The Overview's channel cards, as Qt's `OverviewChannelCard`: one card
// per input in use (routed to an output, or with bands or a trim) and per output that is on, each with
// read-only chips for what is set ("3 bands", "HP LR24 3 kHz", "from C", "1.25 ms", "−5 dB",
// "Inverted", "Muted"), and the inputs not in use on one line. Qt's texts. No I/O, so the unit tests
// reach it.

import type { DspState, FilterParams, OutputLabel } from "../../core/protocol";
import { channelColour, T } from "../eq/model";
import { outputLabel } from "../main/model";
import { decodeXover } from "../outputs/model";

/** One read-only chip: "muted" when it says there is nothing, "warn" for an inverted polarity. */
export interface CardChip {
  text: string;
  kind?: "muted" | "warn";
}

export interface ChannelCard {
  wire: number;
  /** The input's or output's own number. */
  index: number;
  output: boolean;
  /** "IN3", "OUT1". */
  descriptor: string;
  name: string;
  /** "Input", "Output". */
  caption: string;
  colour: string;
  chips: CardChip[];
  /** What its curve is made of: its PEQ and, for an output, its crossover. */
  chain: FilterParams[];
  /** The level its curve is drawn at: an output's gain, an input's trim. */
  offsetDb: number;
}

const num = (v: unknown, d = 0): number => (typeof v === "number" && Number.isFinite(v) ? v : d);

function peq(s: DspState, wire: number): FilterParams[] {
  return (s.filters?.[wire] ?? []).slice(0, s.maxBands || 10);
}

/** The bands in use (not flat). */
export function bandsInUse(bands: readonly FilterParams[]): number {
  return bands.filter((b) => b.filterType !== T.flat).length;
}

/** "No bands", "1 band", "3 bands". */
export function bandsChip(n: number): CardChip {
  return n === 0 ? { text: "No bands", kind: "muted" } : { text: n === 1 ? "1 band" : `${n} bands` };
}

/** A frequency as a chip says it: "400 Hz", "3 kHz", "2.5 kHz". */
export function freqShort(f: number): string {
  if (f < 1000) return `${Math.round(f)} Hz`;
  const k = Math.round(f / 10) / 100;
  return `${Number.isInteger(k) ? k : k.toString()} kHz`;
}

const FAMILY_SHORT = ["", "LR", "BW", "BS"];

/** The first high pass and the first low pass of a crossover: "HP LR24 3 kHz", "LP BW12 400 Hz". */
export function xoverChips(xo: readonly FilterParams[]): CardChip[] {
  const out: CardChip[] = [];
  for (const high of [true, false]) {
    const b = xo.find((x) => !x.bypass && decodeXover(x.filterType).family !== 0 && decodeXover(x.filterType).highPass === high);
    if (!b) continue;
    const x = decodeXover(b.filterType);
    out.push({ text: `${high ? "HP" : "LP"} ${FAMILY_SHORT[x.family]}${x.order * 6} ${freqShort(b.freq)}` });
  }
  return out;
}

/** A level as a chip says it: "−5 dB", "+2 dB", "−1.5 dB". */
export function dbShort(db: number): string {
  const r = Math.round(db * 10) / 10;
  const t = Number.isInteger(r) ? String(Math.abs(r)) : Math.abs(r).toFixed(1);
  return `${r < 0 ? "−" : "+"}${t} dB`;
}

/** A delay as a chip says it: "1.25 ms", "5 ms". */
export function msShort(ms: number): string {
  const r = Math.round(ms * 100) / 100;
  return `${r} ms`;
}

/** The outputs input `i` feeds. */
function feeds(s: DspState, i: number): number[] {
  const out: number[] = [];
  for (let o = 0; o < num(s.numOutputChannels); o++) if (s.matrixRouting?.[i]?.[o] === true) out.push(o);
  return out;
}

/** Whether input `i` is in use: routed to an output, or with bands or a trim set. */
export function inputInUse(s: DspState, i: number): boolean {
  return feeds(s, i).length > 0 || bandsInUse(peq(s, i)) > 0 || Math.abs(num(s.inputPreampDb?.[i])) >= 0.05;
}

/** "to Out 1 + Out 2", "to 4 outputs", "To no output". */
export function feedsChip(outs: readonly number[]): CardChip {
  if (!outs.length) return { text: "To no output", kind: "muted" };
  if (outs.length > 3) return { text: `to ${outs.length} outputs` };
  return { text: `to ${outs.map((o) => `Out ${o + 1}`).join(" + ")}` };
}

export function inputCard(s: DspState, i: number): ChannelCard {
  const bands = peq(s, i);
  const chips: CardChip[] = [bandsChip(bandsInUse(bands)), feedsChip(feeds(s, i))];
  const pre = num(s.inputPreampDb?.[i]);
  if (Math.abs(pre) >= 0.05) chips.push({ text: `Preamp ${dbShort(pre)}` });
  return {
    wire: i,
    index: i,
    output: false,
    descriptor: `IN${i + 1}`,
    name: s.channelNames?.[i] ?? `Input ${i + 1}`,
    caption: "Input",
    colour: channelColour(s, i),
    chips,
    chain: bands,
    offsetDb: pre,
  };
}

export function outputCard(s: DspState, labels: OutputLabel[] | undefined, o: number): ChannelCard {
  const ins = num(s.numInputChannels);
  const wire = ins + o;
  const bands = peq(s, wire);
  const xo = s.xover?.[wire] ?? [];
  const chips: CardChip[] = [bandsChip(bandsInUse(bands)), ...xoverChips(xo)];
  const from: string[] = [];
  let inverted = false;
  for (let i = 0; i < ins; i++) {
    if (s.matrixRouting?.[i]?.[o] !== true) continue;
    from.push(s.channelNames?.[i] ?? `Input ${i + 1}`);
    if (s.matrixInvert?.[i]?.[o] === true) inverted = true;
  }
  chips.push(from.length ? { text: `from ${from.join(" + ")}` } : { text: "No input", kind: "muted" });
  const delay = num(s.outputDelayMs?.[o]);
  if (delay >= 0.005) chips.push({ text: msShort(delay) });
  const gain = num(s.outputGainDb?.[o]);
  if (Math.abs(gain) >= 0.05) chips.push({ text: dbShort(gain) });
  if (inverted) chips.push({ text: "Inverted", kind: "warn" });
  if (s.outputMuted?.[o] === true) chips.push({ text: "Muted" });
  return {
    wire,
    index: o,
    output: true,
    descriptor: `OUT${o + 1}`,
    name: outputLabel(s, labels, o),
    caption: "Output",
    colour: channelColour(s, wire),
    chips,
    chain: [...bands, ...xo],
    offsetDb: gain,
  };
}

/** The Overview's cards: the live inputs in use, the names of the others, the outputs that are on. */
export function overviewCards(s: DspState, labels: OutputLabel[] | undefined, liveIn: number): { inputs: ChannelCard[]; idle: string[]; outputs: ChannelCard[] } {
  const inputs: ChannelCard[] = [];
  const idle: string[] = [];
  for (let i = 0; i < Math.min(liveIn, num(s.numInputChannels)); i++) {
    if (inputInUse(s, i)) inputs.push(inputCard(s, i));
    else idle.push(s.channelNames?.[i] ?? `Input ${i + 1}`);
  }
  const outputs: ChannelCard[] = [];
  for (let o = 0; o < num(s.numOutputChannels); o++) if (s.outputEnabled?.[o] !== false) outputs.push(outputCard(s, labels, o));
  return { inputs, idle, outputs };
}

/** The mini response's span: +12..−24 dB (Qt's card). */
export const MINI = { top: 12, bottom: -24, minFreq: 20, maxFreq: 20000 };

/** A curve as the card's SVG path in a 100 × 48 box; the fill closes it to the bottom. */
export function miniPath(freqs: readonly number[], curve: readonly number[], offset: number): { line: string; fill: string } {
  const xs: string[] = [];
  const span = Math.log10(MINI.maxFreq / MINI.minFreq);
  for (let i = 0; i < freqs.length && i < curve.length; i++) {
    const f = freqs[i]!;
    const v = curve[i]! + offset;
    if (f < MINI.minFreq || f > MINI.maxFreq || !Number.isFinite(v)) continue;
    const x = (Math.log10(f / MINI.minFreq) / span) * 100;
    const y = ((MINI.top - Math.max(MINI.bottom, Math.min(MINI.top, v))) / (MINI.top - MINI.bottom)) * 48;
    xs.push(`${x.toFixed(2)} ${y.toFixed(2)}`);
  }
  if (!xs.length) return { line: "", fill: "" };
  const line = `M${xs.join("L")}`;
  const first = xs[0]!.split(" ")[0];
  const last = xs[xs.length - 1]!.split(" ")[0];
  return { line, fill: `${line}L${last} 48L${first} 48Z` };
}

/** Where 0 dB sits in the card's box. */
export const MINI_ZERO_Y = ((MINI.top - 0) / (MINI.top - MINI.bottom)) * 48;
