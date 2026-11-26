// WP WB6h: the effects of one device, as the effect screens show them:
// crossfeed, loudness, the leveller, psychoacoustic bass, the tube modeller,
// the subharmonic synthesizer and the upmixer. Each effect is a list of
// registry paths (`dspi_proto::registry`) with where the core's mirror
// (`DspState`) holds each value; labels, ranges and kinds come from the
// core's `describe`, the choices' words from `effectChoices`, and whether an
// effect is there at all from the probe's feature names (`features`), never
// from a firmware version. Every write is the core's `write` of the path.
// No I/O here, so the unit tests reach it.

import type { DspState, ParamInfo, ParamValue } from "../../core/protocol";
import { readNumber } from "../../ui/number";

/** An effect's stable id; the System view's blocks use the same ones (WB6d). */
export type EffectId = "crossfeed" | "loudness" | "leveller" | "psybass" | "tube" | "subharm" | "upmix";

/** The id of the screen that holds the effects. */
export const EFFECTS_SCREEN = "effects";

/**
 * Where a link to an effect opens: this screen at that effect. Another screen
 * (the System view) hands it to the session's `open` once that exists.
 */
export interface EffectTarget {
  screen: typeof EFFECTS_SCREEN;
  effect: EffectId;
}

export function effectTarget(effect: EffectId): EffectTarget {
  return { screen: EFFECTS_SCREEN, effect };
}

/** The DOM id of an effect's panel (`#effects-tube`). */
export function effectPanelId(effect: EffectId): string {
  return `effects-${effect}`;
}

export function isEffectId(v: unknown): v is EffectId {
  return typeof v === "string" && EFFECTS.some((e) => e.id === v);
}

/** What a bit-mask parameter's bits are: outputs, crossfeed's output pairs, or the leveller's inputs (each half of `lev.masks`). */
export type MaskKind = "outputs" | "pairs" | "detector" | "apply";

/** One parameter of an effect. */
export interface ParamDef {
  path: string;
  /** Its value in the core's mirror; null when the mirror does not hold it. */
  read: (s: DspState) => number | boolean | null;
  /** Shown only with the effect's Advanced view (the tube's split, M38). */
  advanced?: boolean;
  /** A bit mask: what its bits are. */
  mask?: MaskKind;
  /** The probe's feature name this parameter needs beyond its effect's. */
  feature?: string;
  /** Shown only when this holds (the leveller's channels: more than two inputs). */
  when?: (s: DspState) => boolean;
}

export interface EffectDef {
  id: EffectId;
  /** Its name in the screen's list. */
  label: string;
  /** The probe's feature name the effect needs; null for one every firmware has. */
  feature: string | null;
  /** The effect's own switch. */
  on: ParamDef;
  params: ParamDef[];
  /** Whether it has a Basic and an Advanced view. */
  split?: boolean;
}

function field(name: string): (s: DspState) => number | boolean | null {
  return (s) => {
    const v = s[name];
    return typeof v === "number" || typeof v === "boolean" ? v : null;
  };
}

function at(name: string, i: number): (s: DspState) => number | null {
  return (s) => {
    const a = s[name];
    const v = Array.isArray(a) ? a[i] : undefined;
    return typeof v === "number" && Number.isFinite(v) ? v : null;
  };
}

/** A parameter table indexed by the firmware's ids, as the core's `upmix`, `subharm` and `tube` lists are. */
function table(name: string, paths: readonly string[], extra: Partial<Record<string, Partial<ParamDef>>> = {}): ParamDef[] {
  return paths.map((path, i) => ({ path, read: at(name, i), ...extra[path] }));
}

/** `UPMIX_PARAM_*` order (crates/dspi-core/src/params_fx.rs `UPMIX`). */
export const UPMIX_PATHS = [
  "up.on",
  "up.center_mode",
  "up.surround_mode",
  "up.strength",
  "up.width",
  "up.threshold",
  "up.attack",
  "up.release",
  "up.det_hpf",
  "up.sur_delay",
  "up.sur_hpf",
  "up.sur_lpf",
  "up.decorr",
  "up.presence",
] as const;

/** `SUBHARM_PARAM_*` order (`SUBHARM`). */
export const SUBHARM_PATHS = ["sub.on", "sub.mask", "sub.low", "sub.high", "sub.top", "sub.boost", "sub.select", "sub.depth", "sub.hold", "sub.ceiling", "sub.link"] as const;

/** `TUBE_PARAM_*` order (`TUBE`). */
export const TUBE_PATHS = [
  "tube.on",
  "tube.mask",
  "tube.type",
  "tube.drive",
  "tube.bias",
  "tube.asym",
  "tube.hardness",
  "tube.sag",
  "tube.rectifier",
  "tube.xfmr",
  "tube.damping",
  "tube.resonance",
  "tube.mix",
  "tube.trim",
] as const;

/** The crossfeed voicing that runs the custom pair (`CROSSFEED_PRESETS`: 0..2 are named). */
export const CROSSFEED_CUSTOM = 3;

const moreThanTwoInputs = (s: DspState) => s.numInputChannels > 2;

const upmix = table("upmix", UPMIX_PATHS);
const subharm = table("subharm", SUBHARM_PATHS, { "sub.mask": { mask: "outputs" } });
const tube = table("tube", TUBE_PATHS, {
  "tube.mask": { mask: "outputs" },
  "tube.bias": { advanced: true },
  "tube.asym": { advanced: true },
  "tube.hardness": { advanced: true },
  "tube.sag": { advanced: true },
  "tube.rectifier": { advanced: true },
  "tube.xfmr": { advanced: true },
  "tube.damping": { advanced: true },
  "tube.resonance": { advanced: true },
  "tube.trim": { advanced: true },
});

/** The effects, in the order the screen lists them. */
export const EFFECTS: readonly EffectDef[] = [
  {
    id: "crossfeed",
    label: "Crossfeed",
    feature: null,
    on: { path: "cf.on", read: field("crossfeedEnabled") },
    params: [
      { path: "cf.preset", read: field("crossfeedPreset") },
      // -- P1XF -- under a named voicing the mirror holds that voicing's values
      { path: "cf.freq", read: field("crossfeedFreq") },
      { path: "cf.feed", read: field("crossfeedFeed") },
      { path: "cf.itd", read: field("crossfeedItd") },
      { path: "cf.outputs", read: field("crossfeedOutputPairMask"), mask: "pairs", feature: "crossfeed_output_mask" },
    ],
  },
  {
    id: "loudness",
    label: "Loudness",
    feature: null,
    on: { path: "loud.on", read: field("loudnessEnabled") },
    params: [
      { path: "loud.ref", read: field("loudnessRefSpl") },
      { path: "loud.intensity", read: field("loudnessIntensity") },
      { path: "loud.mask", read: field("loudnessOutputMask"), mask: "outputs", feature: "loudness_output_mask" },
    ],
  },
  {
    id: "leveller",
    label: "Leveller",
    feature: null,
    on: { path: "lev.on", read: field("levellerEnabled") },
    params: [
      { path: "lev.amount", read: field("levellerAmount") },
      { path: "lev.speed", read: field("levellerSpeed") },
      { path: "lev.maxgain", read: field("levellerMaxGainDb") },
      { path: "lev.lookahead", read: field("levellerLookahead") },
      { path: "lev.gate", read: field("levellerGateDb") },
      { path: "lev.masks", read: field("levellerDetectorMask"), mask: "detector", feature: "leveller_masks", when: moreThanTwoInputs },
      { path: "lev.masks", read: field("levellerApplyMask"), mask: "apply", feature: "leveller_masks", when: moreThanTwoInputs },
    ],
  },
  {
    id: "psybass",
    label: "Psy Bass", // -- WP WEB3 -- (REVD W9) Qt's tab
    feature: "psychoacoustic_bass",
    on: { path: "bass.on", read: field("psybassEnabled") },
    params: [
      { path: "bass.cutoff", read: field("psybassCutoffHz") },
      { path: "bass.harmonics", read: field("psybassHarmonicsDb") },
      { path: "bass.drive", read: field("psybassDriveDb") },
      { path: "bass.character", read: field("psybassCharacterPct") },
      { path: "bass.original", read: field("psybassOriginalDb") },
      { path: "bass.mask", read: field("psybassOutputMask"), mask: "outputs" },
    ],
  },
  {
    id: "subharm",
    label: "Subharmonic",
    feature: "subharmonic_synth",
    on: subharm[0]!,
    params: [...subharm.slice(1), { path: "sub.solo", read: field("subharmSolo") }],
  },
  // -- WP WEB3 -- (REVD W9) Tube after Subharmonic, as Qt's tabs
  { id: "tube", label: "Tube", feature: "tube_preamp", on: tube[0]!, params: tube.slice(1), split: true },
  { id: "upmix", label: "Upmixer", feature: "upmixer", on: upmix[0]!, params: upmix.slice(1) },
];

export function effectDef(id: EffectId): EffectDef {
  return EFFECTS.find((e) => e.id === id)!;
}

/** The feature names the effects ask the core about. */
export function featureNames(): string[] {
  const names = new Set<string>();
  for (const e of EFFECTS) {
    if (e.feature) names.add(e.feature);
    for (const p of e.params) if (p.feature) names.add(p.feature);
  }
  return [...names];
}

/** Every registry path the effects use, once. */
export function effectPaths(): string[] {
  return [...new Set(EFFECTS.flatMap((e) => [e.on.path, ...e.params.map((p) => p.path)]))];
}

/** What the probe found; null before it has answered (or when it could not). */
export type Features = Record<string, boolean> | null;

function has(features: Features, name: string | null | undefined): boolean {
  return !name || features?.[name] === true;
}

/**
 * The effects this device has: each one whose feature the probe found (an
 * effect every firmware has needs none) and whose switch the mirror holds.
 */
export function availableEffects(s: DspState, features: Features): EffectDef[] {
  return EFFECTS.filter((e) => has(features, e.feature) && e.on.read(s) !== null);
}

/** Whether effect `e` is on, as the mirror says. */
export function isOn(e: EffectDef, s: DspState): boolean {
  const v = e.on.read(s);
  return typeof v === "boolean" ? v : typeof v === "number" ? v >= 0.5 : false;
}

/** The parameters of `e` this device shows: their features found, their conditions met; with `advanced` false, the Basic view's. */
export function shownParams(e: EffectDef, s: DspState, features: Features, advanced: boolean): ParamDef[] {
  return e.params.filter((p) => has(features, p.feature) && (!p.when || p.when(s)) && (advanced || !p.advanced) && p.read(s) !== null);
}

// ---------------------------------------------------------------- masks

/** One bit of a mask row: its bit, its short name and a longer one for a tooltip. */
export interface MaskBit {
  bit: number;
  short: string;
  long: string;
}

/** Crossfeed's stereo output pairs: two on RP2040, four on RP2350 (the PDM sub is in none). */
export function crossfeedPairs(s: Pick<DspState, "platformId">): number {
  return s.platformId === 0 ? 2 : 4;
}

/**
 * The bits of a mask of kind `kind`; `outputName` names output `o`. -- WP GAP1 -- `pairs`: the
 * crossfeed pair map's outputs per pair (the extension's), else each pair is 2p and 2p+1.
 */
export function maskBits(kind: MaskKind, s: DspState, outputName: (o: number) => string, pairs?: readonly { left: number; right: number }[]): MaskBit[] {
  switch (kind) {
    case "outputs":
      return Array.from({ length: Math.min(s.numOutputChannels, 16) }, (_, o) => ({ bit: o, short: `OUT${o + 1}`, long: outputName(o) }));
    case "pairs":
      return Array.from({ length: crossfeedPairs(s) }, (_, p) => {
        const { left, right } = pairs?.[p] ?? { left: 2 * p, right: 2 * p + 1 }; // -- WP GAP1 --
        return { bit: p, short: `OUT${left + 1}+${right + 1}`, long: `${outputName(left)} and ${outputName(right)}` };
      });
    case "detector":
    case "apply":
      return Array.from({ length: Math.min(s.numInputChannels, 8) }, (_, i) => ({ bit: i, short: `IN${i + 1}`, long: s.channelNames[i] ?? `Input ${i + 1}` }));
  }
}

/** A row's heading for a mask of kind `kind`; `label` is the registry's. */
export function maskTitle(kind: MaskKind, label: string): string {
  if (kind === "detector") return "Listens to";
  if (kind === "apply") return "Acts on";
  return kind === "pairs" ? "Output pairs" : label.replace(/^\S+ outputs$/, "Outputs");
}

/** `mask` with bit `bit` flipped. */
export function toggled(mask: number, bit: number): number {
  return (mask ^ (1 << bit)) >>> 0;
}

/**
 * The value a mask row writes: `lev.masks` carries both halves (the detector
 * in the low byte, apply in the high one), so a change to one keeps the other.
 */
export function maskValue(kind: MaskKind, s: DspState, mask: number): number {
  const det = typeof s.levellerDetectorMask === "number" ? s.levellerDetectorMask : 0xff;
  const app = typeof s.levellerApplyMask === "number" ? s.levellerApplyMask : 0xff;
  if (kind === "detector") return ((mask & 0xff) | (app << 8)) >>> 0;
  if (kind === "apply") return ((det & 0xff) | ((mask & 0xff) << 8)) >>> 0;
  return mask >>> 0;
}

// ---------------------------------------------------------------- values

/** The grid a float's slider, wheel and keys land on, from its unit and range. */
export function stepFor(info: Pick<ParamInfo, "unit" | "min" | "max">): number {
  const span = (info.max ?? 1) - (info.min ?? 0);
  switch (info.unit) {
    case "Hz":
      return span >= 1000 ? 10 : 1;
    case "%":
      return 1;
    case "ms":
      return span >= 100 ? 1 : 0.1;
    default:
      return span <= 2 ? 0.01 : 0.1;
  }
}

function decimals(step: number): number {
  return step >= 1 ? 0 : Math.ceil(-Math.log10(step) - 1e-9);
}

/** A value with its unit, on its step's decimals: "700 Hz", "4.5 dB", "50 %", "83.0 dB SPL"; never "-0". */
export function valueText(v: number, info: Pick<ParamInfo, "unit">, step: number): string {
  const d = decimals(step);
  const f = 10 ** d;
  let r = Math.round(v * f) / f;
  if (r === 0) r = 0;
  const unit = info.unit ? ` ${info.unit}` : "";
  return `${r.toFixed(d)}${unit}`;
}

/** A typed value: a number, its unit allowed ("650", "650 Hz", "-3dB", "+2"). Null when it is not one. */
export function parseValue(text: string, info: Pick<ParamInfo, "unit">): number | null {
  // -- WP WEB -- as Qt's fields (USR1): a decimal comma, U+2212, the unit after it
  const unit = (info.unit ?? "").toLowerCase().replace(/\s+/g, "");
  return readNumber(text, unit === "dbfs" ? ["dbfs", "db"] : unit ? [unit] : []);
}

/** The registry kind's value for `v` (`{float: 700}`, `{choice: 2}`, `{int: 5}`, `{mask: 3}`, `{bool: true}`). */
export function paramValue(info: Pick<ParamInfo, "kind">, v: number | boolean): ParamValue {
  switch (info.kind) {
    case "bool":
      return { bool: typeof v === "boolean" ? v : v >= 0.5 };
    case "choice":
      return { choice: Math.round(Number(v)) };
    case "int":
      return { int: Math.round(Number(v)) };
    case "mask":
      return { mask: Number(v) >>> 0 };
    default:
      return { float: Number(v) };
  }
}

/** A mirror value as a number (a bool as 0 or 1). */
export function asNumber(v: number | boolean | null): number {
  return typeof v === "boolean" ? (v ? 1 : 0) : (v ?? 0);
}

/** A mirror value as a switch's state. */
export function asBool(v: number | boolean | null): boolean {
  return typeof v === "boolean" ? v : typeof v === "number" ? v >= 0.5 : false;
}

/** One choice's words, from `effectChoices`. */
export interface ChoiceWords {
  value: number;
  label: string;
  detail: string;
}

/**
 * The options of a choice (or the tube's numbered type): the core's words
 * where `effectChoices` has them, else the registry's choice names, else the
 * numbers in the range.
 */
export function choiceOptions(info: ParamInfo, words: ChoiceWords[] | undefined): ChoiceWords[] {
  if (words?.length) return words;
  if (info.choices?.length) return info.choices.map((c) => ({ value: c.value, label: c.label, detail: "" }));
  const lo = Math.ceil(info.min ?? 0);
  const hi = Math.floor(info.max ?? lo);
  return Array.from({ length: Math.max(0, hi - lo + 1) }, (_, i) => ({ value: lo + i, label: String(lo + i), detail: "" }));
}

/**
 * The writes of a crossfeed frequency or feed change (P1XF): under Custom the
 * value alone. Under a named voicing the mirror shows that voicing's own
 * values, so the change makes the voicing Custom, starting from what is shown:
 * Custom, then the other value as shown, then this one (one undo step, the
 * caller groups them).
 */
export function crossfeedWrites(s: DspState, path: "cf.freq" | "cf.feed", v: number): { path: string; value: ParamValue }[] {
  const own = { path, value: { float: v } as ParamValue };
  if (s.crossfeedPreset === CROSSFEED_CUSTOM || typeof s.crossfeedPreset !== "number") return [own];
  const otherPath = path === "cf.freq" ? "cf.feed" : "cf.freq";
  const other = Number(path === "cf.freq" ? s.crossfeedFeed : s.crossfeedFreq);
  return [{ path: "cf.preset", value: { choice: CROSSFEED_CUSTOM } }, { path: otherPath, value: { float: other } }, own];
}
