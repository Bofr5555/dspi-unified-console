// -- WP WEB6 -- Where each effect's parts sit, as Qt's Modern panels place them
// (apps/desktop/qt/qml/tools/*Panel.qml): the graph and what the effect acts on or starts from at the
// left, its parameters and outputs at the right, in Qt's order; and the masks' "Presets ▾" items with
// Qt's words and masks. No I/O, so the unit tests reach it.

import type { DspState } from "../../core/protocol";
import { pdmOutput } from "../outputs/model";
import { crossfeedPairs, type EffectId } from "./model";

/** One block of a column. */
export type Block =
  /** The effect's graph under its section label; `points` names its starting points ("Apply preset ▾"). */
  | { kind: "graph"; title: string; points?: "psybass" | "subharm" | "tube" }
  /** A section of parameters, in this order; `title` "" for rows without a label (the tube's drive and mix). */
  | { kind: "section"; title: string; paths: string[] }
  /** The upmixer's status, its routing note; the tube's picture and its type chips. */
  | { kind: "status" }
  | { kind: "routing" }
  | { kind: "picture" }
  | { kind: "tube-types" };

export interface Columns {
  left: Block[];
  right: Block[];
}

const section = (title: string, ...paths: string[]): Block => ({ kind: "section", title, paths });

/** The columns of effect `id` (the tube's view by `advanced`). */
export function columnsOf(id: EffectId, advanced: boolean): Columns {
  switch (id) {
    case "crossfeed":
      return {
        left: [{ kind: "graph", title: "Frequency Response" }, section("Preset", "cf.preset")],
        right: [section("Parameters", "cf.freq", "cf.feed", "cf.itd"), section("Output Pairs", "cf.outputs")],
      };
    case "loudness":
      return {
        left: [{ kind: "graph", title: "Compensation Curve" }],
        right: [section("Parameters", "loud.ref", "loud.intensity"), section("Outputs", "loud.mask")],
      };
    case "leveller":
      return {
        left: [section("Channels", "lev.masks")],
        right: [section("Parameters", "lev.amount", "lev.speed", "lev.maxgain", "lev.gate", "lev.lookahead")],
      };
    case "psybass":
      return {
        left: [{ kind: "graph", title: "Spectrum", points: "psybass" }, section("Harmonics", "bass.cutoff", "bass.harmonics")],
        right: [section("Outputs", "bass.mask"), section("Shaping", "bass.drive", "bass.character", "bass.original")],
      };
    case "subharm":
      return {
        left: [{ kind: "graph", title: "Bands", points: "subharm" }, section("Levels", "sub.low", "sub.high", "sub.top")],
        right: [
          section("Selectivity", "sub.select", "sub.depth", "sub.hold"),
          section("Sub ceiling", "sub.ceiling"),
          section("LF boost", "sub.boost"),
          section("Outputs", "sub.mask", "sub.link"),
        ],
      };
    case "tube":
      return advanced
        ? {
            left: [{ kind: "graph", title: "Transfer curve", points: "tube" }, section("Stage", "tube.type", "tube.drive", "tube.mix", "tube.trim"), section("Outputs", "tube.mask")],
            right: [section("Character", "tube.bias", "tube.asym", "tube.hardness", "tube.sag", "tube.rectifier"), section("Output stage", "tube.xfmr", "tube.damping", "tube.resonance")],
          }
        : {
            left: [{ kind: "picture" }],
            right: [{ kind: "tube-types" }, section("", "tube.drive", "tube.mix"), section("Outputs", "tube.mask")],
          };
    case "upmix":
      return {
        left: [{ kind: "status" }, section("Engines", "up.center_mode", "up.surround_mode"), { kind: "routing" }],
        right: [
          section("Centre", "up.strength", "up.width", "up.presence", "up.threshold", "up.attack", "up.release", "up.det_hpf"),
          section("Surround", "up.sur_delay", "up.sur_hpf", "up.sur_lpf", "up.decorr"),
        ],
      };
  }
}

/** Every path a column names (a parameter none names goes to a last "Parameters" section). */
export function placedPaths(c: Columns): Set<string> {
  const out = new Set<string>();
  for (const b of [...c.left, ...c.right]) {
    if (b.kind === "section") for (const p of b.paths) out.add(p);
    if (b.kind === "tube-types") out.add("tube.type");
  }
  return out;
}

/** One "Presets ▾" item: its words and the mask it writes (the registry value). */
export interface MaskPreset {
  label: string;
  value: number;
}

/** Every output's bit (Qt's `allOutputsMask`). */
export function allOutputs(s: Pick<DspState, "numOutputChannels">): number {
  const n = s.numOutputChannels;
  return n >= 16 ? 0xffff : ((1 << Math.max(0, n)) - 1) >>> 0;
}

/** The PDM sub's bit alone (Qt's `subOnlyMask`). */
export function subOnly(s: Pick<DspState, "numOutputChannels">): number {
  const pdm = pdmOutput(s);
  return pdm >= 0 ? (1 << pdm) >>> 0 : 0;
}

/** The items of a mask's "Presets ▾" (Qt's per panel); none for a path without. */
export function maskPresets(path: string, s: DspState): MaskPreset[] {
  const all = allOutputs(s);
  switch (path) {
    case "cf.outputs": {
      const n = crossfeedPairs(s);
      return [
        { label: "All pairs", value: n >= 8 ? 0xff : (1 << n) - 1 },
        { label: "Pair 1 only (Headphones)", value: 0x01 },
        { label: "None", value: 0 },
      ];
    }
    case "loud.mask":
      return [
        { label: "All outputs", value: 0xffff },
        { label: "Slot 1 only (Headphones)", value: 0x0003 },
        { label: "None", value: 0 },
      ];
    // The detector in the low byte, apply in the high one: both set at once
    case "lev.masks":
      return [
        { label: "All channels (Night mode)", value: 0xff | (0xff << 8) },
        { label: "Center only (Dialog boost)", value: 0x04 | (0x04 << 8) },
        { label: "Front L / R only", value: 0x03 | (0x03 << 8) },
      ];
    case "bass.mask":
      return [
        { label: "All outputs", value: all },
        { label: "Exclude sub (recommended)", value: (all & ~subOnly(s)) >>> 0 },
        { label: "None", value: 0 },
      ];
    case "sub.mask":
      return [
        { label: "Sub only (recommended)", value: subOnly(s) },
        { label: "All outputs", value: all },
        { label: "None", value: 0 },
      ];
    case "tube.mask":
      return [
        { label: "All outputs", value: all },
        { label: "Exclude sub", value: (all & ~subOnly(s)) >>> 0 },
        { label: "None", value: 0 },
      ];
    default:
      return [];
  }
}

/** A floor or ceiling that switches a stage off reads "Off" (Qt's `offAt`): the levels at their floor, the ceiling at its top. */
export function offAt(path: string): "floor" | "top" | null {
  if (path === "sub.low" || path === "sub.high" || path === "sub.top") return "floor";
  if (path === "sub.ceiling") return "top";
  return null;
}

/** The words under a slider's two ends, where Qt has them. */
export const ENDS: Record<string, [string, string]> = {
  "sub.low": ["Off", "+12 dB"],
  "sub.high": ["Off", "+12 dB"],
  "sub.top": ["Off", "+12 dB"],
  "sub.ceiling": ["-40 dBFS", "Off"],
  "sub.boost": ["Off", "+6 dB"],
  "bass.character": ["Warm", "Aggressive"],
  "tube.drive": ["Clean", "Overdrive"],
  "tube.mix": ["Dry", "All tube"],
  "tube.damping": ["1 (loose)", "20 (tight)"],
  "tube.resonance": ["30 Hz", "150 Hz"],
};

/** The headroom cost's words (Qt's): "+3.5 dB" when the setting can add gain, else "none". */
export function headroomText(db: number | null): string {
  return db !== null && db > 0 ? `+${db.toFixed(1)} dB` : "none";
}

/** A harmonic's level at full scale (Qt's): "-39 dB", "none" at or under -100. */
export function harmonicText(db: number): string {
  return db <= -100 ? "none" : `${Math.round(db)} dB`;
}

/** The upmixer's status line (Qt's): why it is idle, or that it runs. */
export function upmixStatusText(on: boolean, status: { active: boolean; parkedReason: number } | null): string {
  if (status?.active) return "Active - processing audio";
  if (!on || status?.parkedReason === 1) return "Idle: upmixer disabled";
  if (status?.parkedReason === 2) return "Idle: input is not stereo";
  if (status?.parkedReason === 3) return "Idle: sample rate above 48 kHz";
  return status ? "Idle" : "On";
}
