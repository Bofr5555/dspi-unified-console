// -- WP WEB3 -- (DL1 on the web, REVD W9) The effects' words as the desktop console has them: each
// effect's intro row (its icon, name and what it does) and each parameter's title, caption and
// section (apps/desktop/qt/qml/tools/*Panel.qml). A parameter the table does not name keeps the
// registry's label and plain sentence. No I/O, so the unit tests reach it.

import type { GlyphName } from "../../ui/design";
import type { EffectId, MaskKind } from "./model";

/** An effect's intro row (§6). */
export interface EffectIntro {
  title: string;
  subtitle: string;
  icon: GlyphName;
}

export const EFFECT_INTROS: Record<EffectId, EffectIntro> = {
  crossfeed: { title: "Crossfeed", subtitle: "BS2B Bauer Stereophonic-to-Binaural", icon: "headphones" },
  loudness: { title: "Loudness Compensation", subtitle: "ISO 226:2003 Fletcher-Munson", icon: "ear" },
  leveller: { title: "Volume Leveller", subtitle: "Upward Dynamic Range Compression", icon: "activity" },
  psybass: { title: "Psychoacoustic Bass", subtitle: "Phantom fundamental bass enhancement", icon: "audio-lines" },
  subharm: { title: "Subharmonic Synthesizer", subtitle: "Generates a subharmonic at half the frequency of the source's bass", icon: "waves" },
  tube: { title: "Tube Modeller", subtitle: "Valve-style harmonic colour, supply sag and a tube amplifier's output stage", icon: "flame" },
  upmix: { title: "Stereo Upmixer", subtitle: "Derive Centre and Surround from stereo", icon: "speaker" },
};

/** A parameter's words: its title, the caption under it, and the section it is in (none: "Parameters"). */
export interface ParamText {
  title?: string;
  caption?: string;
  section?: string;
}

export const PARAM_TEXTS: Record<string, ParamText> = {
  "cf.preset": { title: "Preset" },
  "cf.freq": { title: "Cutoff Frequency", caption: "Simulates head shadow lowpass cutoff. Lower = more bass crossfeed. Typical: 650-700 Hz." },
  "cf.feed": { title: "Feed Level", caption: "Crossfeed attenuation below direct signal. Higher = more crossfeed. Typical: 4.5-9.5 dB." },
  "cf.itd": { title: "Interaural Time Delay", caption: "Simulates ~220 µs path difference via all-pass filter" },
  "cf.outputs": { caption: "Crossfeed only the stereo output pairs feeding headphones. Speaker pairs stay bit-accurate. The mono sub is never crossfed." },

  "loud.ref": { title: "Reference SPL", caption: "SPL at 1 kHz when USB volume is 0 dB. Lower = more compensation per dB of volume reduction." },
  "loud.intensity": { title: "Intensity", caption: "Scales the ISO 226 compensation. 100% = standard curve. 0% = bypassed. >100% = exaggerated." },
  "loud.mask": { caption: "Compensate only the outputs feeding your low-level listening chain. Keep bass-managed pairs (mains + sub) together so the crossover stays coherent." },

  "lev.amount": { title: "Amount", caption: "Compression strength. Higher values reduce dynamic range more aggressively." },
  "lev.speed": { title: "Speed" },
  "lev.maxgain": { title: "Max Gain", caption: "Maximum boost for quiet passages. Higher values risk amplifying noise." },
  "lev.gate": { title: "Gate Threshold", caption: "Silence gate. Signals below this level are not boosted, preventing noise amplification." },
  "lev.lookahead": { title: "Lookahead", caption: "Adds 5ms latency. Improves transient handling." },

  "bass.cutoff": { title: "Cutoff Frequency", section: "Harmonics", caption: "The speaker's low-frequency limit. Content below this feeds the harmonic generator; generated harmonics span roughly this to 4x." },
  "bass.harmonics": { title: "Harmonics", section: "Harmonics", caption: "Level of the synthesized harmonics. The primary amount-of-effect control. Higher = more perceived bass." },
  "bass.drive": { title: "Drive", section: "Shaping", caption: "Pre-gain into the odd-harmonic soft clipper. Higher makes the effect audible on quieter passages. Mostly affects aggressive character." },
  "bass.character": { title: "Character", section: "Shaping", caption: "" },
  "bass.original": {
    title: "Original Bass",
    section: "Shaping",
    caption: "Level of the un-reproducible fundamental below the cutoff. Lower attenuates it, freeing driver excursion and headroom. -60 dB is full removal. Speaker protection.",
  },
  "bass.mask": { caption: "Enhance only the small-speaker outputs. Mask off the sub and any full-range outputs - synthesizing harmonics on a channel that can reproduce real bass is counterproductive." },

  "sub.low": { title: "24 - 36 Hz", section: "Levels", caption: "Derived from 48 - 72 Hz" },
  "sub.high": { title: "36 - 56 Hz", section: "Levels", caption: "Derived from 72 - 112 Hz" },
  "sub.top": { title: "56 - 80 Hz", section: "Levels", caption: "Derived from 112 - 160 Hz" },
  "sub.select": { title: "Selectivity", section: "Selectivity" },
  "sub.depth": { title: "Depth", section: "Selectivity" },
  "sub.hold": { title: "Hold", section: "Selectivity" },
  "sub.ceiling": { title: "Threshold", section: "Sub ceiling", caption: "" },
  "sub.boost": { title: "70 Hz bell", section: "LF boost", caption: "" },
  "sub.link": { title: "Link output pairs", section: "Outputs", caption: "One sub per pair, from its mono sum." },
  "sub.mask": {
    caption:
      "Select the outputs that can actually play 24 to 80 Hz. Subharm runs before the crossover, so a satellite with a highpass loses the sub again - mask it off and save the CPU instead.",
  },
  "sub.solo": { title: "Solo" },

  "tube.type": { title: "Tube", section: "Stage" },
  "tube.drive": { title: "Drive", section: "Stage", caption: "" },
  "tube.mix": { title: "Mix", section: "Stage", caption: "" },
  "tube.trim": { title: "Output Trim", section: "Stage", caption: "" },
  "tube.bias": { title: "Bias", section: "Character", caption: "" },
  "tube.asym": { title: "Asymmetry", section: "Character", caption: "" },
  "tube.hardness": { title: "Knee Hardness", section: "Character", caption: "" },
  "tube.sag": { title: "Sag", section: "Character", caption: "" },
  "tube.rectifier": {
    title: "Rectifier",
    section: "Character",
    caption: "The power-supply rectifier sets how deep and how slow the sag is. Solid state switches sag off entirely; the valve rectifiers get progressively softer and slower from GZ34 to 5Y3.",
  },
  "tube.xfmr": { title: "Output stage", section: "Output stage", caption: "A valve amplifier's loose grip on the speaker." },
  "tube.damping": { title: "Damping Factor", section: "Output stage", caption: "" },
  "tube.resonance": { title: "Speaker Resonance", section: "Output stage", caption: "" },
  "tube.mask": {
    caption: "Tube runs before the crossover and the per-output EQ, where a real preamp sits: a sub output saturates the full-band program and then low-passes the result.",
  },

  "up.center_mode": { title: "Centre", section: "Engines" },
  "up.surround_mode": {
    title: "Surround",
    section: "Engines",
    caption:
      "Logician centre gates extraction on running L/R correlation; Logician surround uses a Pro Logic II-style matrix decoder. Sinner modes are fixed (C = 0.7071(L+R), surround = L-R) - a Hafler-style passive matrix like the one in the Schiit Syn.",
  },
  "up.strength": {
    title: "Strength",
    section: "Centre",
    caption: "Centre extraction strength; scales both the C output and how much centre energy is removed from L/R. In Sinner mode this is the fixed centre gain.",
  },
  "up.width": {
    title: "Centre Width",
    section: "Centre",
    caption: "How much extracted centre stays in L/R. 0 = full removal (discrete centre); 100 = L/R untouched (expect combing if a real centre speaker plays).",
  },
  "up.presence": {
    title: "Presence",
    section: "Centre",
    caption: "Voice presence bell at 3 kHz (Q 0.6). Positive brings voices forward, negative pushes them back (Syn-style). Stored in 0.5 dB steps.",
  },
  "up.threshold": {
    title: "Correlation Threshold",
    section: "Centre",
    caption: "Correlation gate. Below this, nothing is extracted; above it, extraction scales up to full. Raise to extract only strongly-correlated content.",
  },
  "up.attack": { title: "Attack", section: "Centre", caption: "Centre gain rise time (Logician mode)." },
  "up.release": { title: "Release", section: "Centre", caption: "Centre gain fall time (Logician mode)." },
  "up.det_hpf": {
    title: "Detector HPF",
    section: "Centre",
    caption: "Detector bass-cut corner. Content below this is ignored by the steering detector (the audio itself is not filtered) so bass does not pump the centre.",
  },
  "up.sur_delay": { title: "Delay", section: "Surround", caption: "Haas delay on Ls/Rs (precedence effect). Rule of thumb ~1 ms per foot of listener distance." },
  "up.sur_hpf": { title: "Band-limit HPF", section: "Surround", caption: "Surround high-pass; keeps rumble out of the rears." },
  "up.sur_lpf": { title: "Band-limit LPF", section: "Surround", caption: "Surround low-pass. 7 kHz is the classic surround voicing; raise for full-band rears." },
  "up.decorr": { title: "Decorrelation", section: "Surround", caption: "Schroeder allpass decorrelator amount. 0 disables decorrelation." },
};

// -- WP WEB6 -- the rest of Qt's words on the effect pages

/** The crossfeed voicings as Qt lists them (EffectsController's kCrossfeedPresets), by value. */
export const CROSSFEED_PRESET_TEXTS: Record<number, { name: string; detail: string }> = {
  0: { name: "Default", detail: "700 Hz / 4.5 dB - Balanced, most popular" },
  1: { name: "Chu Moy", detail: "700 Hz / 6.0 dB - Stronger spatial effect" },
  2: { name: "Jan Meier", detail: "650 Hz / 9.5 dB - Natural speaker-like" },
  3: { name: "Custom", detail: "User-defined parameters" },
};

/** The leveller's speeds, what each is for (LevellerPanel's speedTexts), by value. */
export const LEVELLER_SPEED_TEXTS = [
  "Slow — Gentle response for music and wide dynamic range content.",
  "Medium — Balanced response for general purpose use.",
  "Fast — Tight response for speech, dialogue, and podcasts.",
];

/** The leveller's two channel rows' hints (LevellerPanel). */
export const LEVELLER_MASK_HINTS = { detector: "sets the shared gain", apply: "receives the gain" } as const;

/** The subharmonic selectivity's line under its segments, by mode (SubharmonicPanel). */
export const SELECTIVITY_TEXTS = [
  "Every band signal is treated alike.",
  "A short sub burst after each attack - extends kicks, not the bass line.",
  "The sub opens once a band has been ringing - extends bass notes, not kicks.",
];

/** Depth and hold, shown while a selectivity mode is chosen (SubharmonicPanel). */
export const SELECTIVITY_PARAM_TEXTS = {
  depth: "How far the material this mode does not favour is gated down. At 0% the selectivity is inaudible whatever the mode is set to; 100% is full gating.",
  holdPercussive: "The length of the sub burst after each attack.",
  holdSustained: "How long a band must ring before its sub opens. Every note's first hold period has no sub, so staccato bass lines get little.",
};

/** The headroom cost's tooltip (SubharmonicPanel). */
export function headroomTip(db: number | null): string {
  return db !== null && db > 0
    ? `This setting can add up to ${db.toFixed(1)} dB. Lower the preamp on the inputs feeding the selected outputs by that much, or a loud passage will clip.`
    : "This setting cannot push the signal past full scale.";
}

/** The harmonics at full scale: their tooltip (TubePanel). */
export const HARMONICS_TIP =
  "Level of the second and third harmonic relative to the fundamental, for a full-scale sine through the static curve after mix and trim. Sag lowers the drive on sustained loud passages and the output stage adds its own low-frequency lift, so the running figures sit somewhat lower.";

/** The tube types' shelf tooltip (TubePanel). */
export const TUBE_SHELF_TIP = "Loads the character of a real tube: its bias, asymmetry, knee hardness and sag. Drive, mix and everything in Advanced keep their values.";

/** The picture tile's caption under the tube's name (Basic) and the type's caption in Advanced (TubePanel). */
export function tubeCaption(style: string | null, pushPull: boolean, xfmr: boolean, advanced: boolean): string {
  if (style === null) return advanced ? "Character controls as set, no tube row applied." : "Character set by hand. Pick a tube to load one, or fine-tune it in Advanced.";
  if (pushPull && !xfmr) return advanced ? `${style}. Meant for use with the output stage on.` : `${style}. Meant for use with the output stage, in Advanced.`;
  return `${style}.`;
}

/** The output stage's lift at its damping (TubePanel). */
export function liftText(lift: { bell: number; top: number }): string {
  return `+${lift.bell.toFixed(1)} dB at resonance, +${lift.top.toFixed(1)} dB at the top.`;
}

/** The upmixer's note under its engines (Qt's Routing section). */
export const UPMIX_ROUTING =
  "The derived channels appear as matrix source rows: row 2 = Centre, row 3 = Left Surround, row 4 = Right Surround. Open the Matrix Mixer to route them to your output slots (a centre crosspoint gain of -3 dB is a safe start, since the centre row can reach +3 dBFS).";

/** A parameter's title: the table's, else the registry's label. */
export function paramTitle(path: string, label: string): string {
  return PARAM_TEXTS[path]?.title ?? label;
}

/** The caption under a parameter: the table's, else the registry's plain sentence. */
export function paramCaption(path: string, plain: string | undefined): string {
  return PARAM_TEXTS[path]?.caption ?? plain ?? "";
}

/** A mask row's section title, as Qt's (Outputs, Output Pairs; the leveller's Detector and Apply in Channels). */
export function maskSection(kind: MaskKind): string {
  if (kind === "pairs") return "Output Pairs";
  if (kind === "detector" || kind === "apply") return "Channels";
  return "Outputs";
}

/** A mask chip group's own label: the leveller's two rows are named; the others are their section. */
export function maskLabel(kind: MaskKind): string {
  if (kind === "detector") return "Detector";
  if (kind === "apply") return "Apply";
  return maskSection(kind);
}

/** One group of a panel's parameters under one section label, in the order they come. */
export interface ParamGroup<T> {
  section: string;
  items: T[];
}

/** `items` grouped by their section, consecutive ones together, in their order; unnamed: `fallback`. */
export function groupBySection<T>(items: readonly T[], sectionOf: (t: T) => string | undefined, fallback = "Parameters"): ParamGroup<T>[] {
  const out: ParamGroup<T>[] = [];
  for (const t of items) {
    const section = sectionOf(t) ?? fallback;
    const last = out[out.length - 1];
    if (last && last.section === section) last.items.push(t);
    else out.push({ section, items: [t] });
  }
  return out;
}
