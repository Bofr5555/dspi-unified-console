// WP WEBM3: the History page's words (Library › History, MODLAY 2): the core's journal of this
// connection (`history`, newest first) as rows a person reads. The paths' names are the
// registry's (`describe`), as the session's step notes name them.

import type { HistoryEntry, ParamValue } from "../../core/protocol";

/** How many changes the page shows (the core keeps the newest 1000). */
export const HISTORY_ROWS = 200;

/** One row of the page. */
export interface HistoryRow {
  key: string;
  time: string;
  what: string;
  values: string;
  /** Who made it, when not the person here: "Undo", "Redo", "Another writer". */
  source: string;
  undoable: boolean;
}

/** A value as the row shows it; "" for a trigger or a value the page cannot say shortly. */
export function valueWords(v: ParamValue | null): string {
  if (v === null || typeof v !== "object") return "";
  if ("bool" in v) return v.bool ? "On" : "Off";
  if ("float" in v) return String(Math.round(v.float * 100) / 100).replace("-", "−");
  if ("int" in v) return String(v.int);
  if ("choice" in v) return `#${v.choice}`;
  if ("mask" in v) return `0x${v.mask.toString(16)}`;
  if ("text" in v) return `“${v.text}”`;
  return "";
}

/** What the change was to: the path's name, then its indices counted from 1 ("Gain · 3"). */
export function whatWords(label: string, indices: readonly number[]): string {
  return [label, ...indices.map((i) => String(i + 1))].join(" · ");
}

/** Who made a change, in words; "" for the person's own. */
export function sourceWords(source: string): string {
  switch (source) {
    case "user":
    case "":
      return "";
    case "undo":
      return "Undo";
    case "redo":
      return "Redo";
    default:
      return source[0]!.toUpperCase() + source.slice(1).replace(/_/g, " ");
  }
}

/** The clock time today, the date and time before. */
export function timeWords(atMs: number, now: number): string {
  const d = new Date(atMs);
  const hm = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
  return new Date(now).toDateString() === d.toDateString() ? hm : `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")} ${hm}`;
}

/** The page's rows, newest first, with the names `label` gives each path. -- WP WEB4 -- With the
 * device's channel names (`names`), each row in Qt's words (DL3a's `HistoryController::describe`). */
export function historyRows(entries: readonly HistoryEntry[], label: (path: string) => string, now: number, names?: HistoryNames): HistoryRow[] {
  return entries.map((e, i) => {
    if (names) {
      const d = describeEntry(e, names, label);
      return { key: `${e.atMs}:${i}:${e.path}`, time: timeWords(e.atMs, now), what: d.title, values: d.detail, source: sourceWords(e.source), undoable: e.undoable };
    }
    const before = valueWords(e.before);
    const after = valueWords(e.after);
    return {
      key: `${e.atMs}:${i}:${e.path}`,
      time: timeWords(e.atMs, now),
      what: whatWords(label(e.path), e.indices),
      values: before && after ? `${before} → ${after}` : after,
      source: sourceWords(e.source),
      undoable: e.undoable,
    };
  });
}

// -- WP WEB4 -- Qt's history in words (DL3a): the glossary's names for the paths, the channel by its
// name ("Gain, Out 1, band 3"), a crossover band as a filter, a crosspoint "In 1 to Out 2", the
// presets by slot, the generator's start and stop, the values with their units.

/** The device's channels by name: a wire channel, an output's number, an input's number. */
export interface HistoryNames {
  wire: (i: number) => string;
  output: (i: number) => string;
  input: (i: number) => string;
}

/** The first crossover band's number in an EQ write (the firmware's XOVER_BAND_BASE). */
export const XOVER_BAND_BASE = 20;

const EXACT: Record<string, string> = {
  "vol.user": "Volume",
  "vol.master": "Master volume",
  "vol.mute": "Mute",
  pre: "Input trim",
  bypass: "EQ bypass",
  "eq.band": "Filter band",
  "eq.type": "Filter type",
  "eq.freq": "Frequency",
  "eq.q": "Q",
  "eq.gain": "Gain",
  "eq.bypass": "Band bypass",
  "ch.delay": "Delay",
  "ch.name": "Channel name",
  mix: "Crosspoint",
  "out.gain": "Output gain",
  "out.mute": "Output mute",
  "out.delay": "Delay",
  "out.enable": "Output on",
  "in.source": "Input source",
  "dev.name": "Device name",
  "eq.fp": "Target frequency",
  "eq.qp": "Target Q",
  "in.name": "Source name",
  "out.latency": "Output latency",
  "ext.pairmap": "Crossfeed pairs",
  "ext.vreg": "Core voltage",
  "sig.config": "Signal generator settings",
  bulk: "All settings written",
  "dev.reset": "Factory reset",
  "preset.startup": "Startup preset",
  "preset.iomode": "I/O storage",
};

const PARAM: Record<string, string> = {
  "limit.threshold": "Threshold", "limit.release": "Release", "limit.link": "Link group",
  "loud.ref": "Reference level", "loud.intensity": "Intensity", "loud.mask": "Outputs",
  "lev.amount": "Amount", "lev.speed": "Speed", "lev.maxgain": "Maximum boost", "lev.lookahead": "Lookahead", "lev.gate": "Silence gate", "lev.masks": "Channels",
  "cf.preset": "Voicing", "cf.freq": "Cutoff frequency", "cf.feed": "Feed level", "cf.itd": "Interaural delay", "cf.outputs": "Outputs",
  "bass.cutoff": "Speaker limit", "bass.harmonics": "Harmonics", "bass.drive": "Drive", "bass.character": "Character", "bass.original": "Original bass", "bass.mask": "Outputs",
  "up.center_mode": "Centre mode", "up.surround_mode": "Surround mode", "up.strength": "Strength", "up.width": "Centre width", "up.threshold": "Correlation threshold",
  "up.attack": "Attack", "up.release": "Release", "up.det_hpf": "Detector high-pass", "up.sur_delay": "Surround delay", "up.sur_hpf": "Surround high-pass",
  "up.sur_lpf": "Surround low-pass", "up.decorr": "Decorrelation", "up.presence": "Centre presence", "up.config": "Settings",
  "sub.low": "24 - 36 Hz", "sub.high": "36 - 56 Hz", "sub.top": "56 - 80 Hz", "sub.boost": "70 Hz bell", "sub.mask": "Outputs", "sub.select": "Selectivity",
  "sub.depth": "Depth", "sub.hold": "Hold", "sub.ceiling": "Sub ceiling", "sub.link": "Link output pairs", "sub.solo": "Solo",
  "tube.mask": "Outputs", "tube.type": "Tube", "tube.drive": "Drive", "tube.bias": "Bias", "tube.asym": "Asymmetry", "tube.hardness": "Knee hardness",
  "tube.sag": "Sag", "tube.rectifier": "Rectifier", "tube.xfmr": "Output stage", "tube.damping": "Damping factor", "tube.resonance": "Speaker resonance",
  "tube.mix": "Mix", "tube.trim": "Output trim",
};

const FEATURE: [string, string][] = [
  ["limit.", "Output Limiter"],
  ["loud.", "Loudness"],
  ["lev.", "Leveller"],
  ["cf.", "Crossfeed"],
  ["ext.pairmap", "Crossfeed"],
  ["bass.", "Psychoacoustic Bass"],
  ["up.", "Upmixer"],
  ["sub.", "Subharmonic Synthesizer"],
  ["tube.", "Tube Modeller"],
  ["preset.", "Preset"],
  ["cs.", "Control surface"],
  ["ext.alink.", "Audio link"],
];

const XOVER: Record<string, string> = {
  "eq.band": "Crossover filter",
  "eq.type": "Crossover type",
  "eq.freq": "Crossover frequency",
  "eq.q": "Crossover Q",
  "eq.gain": "Crossover gain",
  "eq.bypass": "Crossover bypass",
};

/** The glossary's name of `path`, else `fallback`'s (the registry's label). */
export function labelOf(path: string, fallback: (path: string) => string): string {
  if (EXACT[path]) return EXACT[path]!;
  for (const [prefix, name] of FEATURE) {
    if (!path.startsWith(prefix)) continue;
    return PARAM[path] ? `${name} › ${PARAM[path]}` : name;
  }
  return fallback(path);
}

/** The unit of a path's number, "" for none. */
export function unitOf(path: string): string {
  if (path === "eq.freq" || path === "cf.freq") return "Hz";
  if (path.endsWith(".delay") || path.endsWith(".release")) return "ms";
  if (path === "limit.threshold" || path === "lev.gate") return "dBFS";
  if (path === "loud.ref") return "dB SPL";
  if (path.startsWith("vol.") || path === "pre" || path.endsWith(".gain") || path === "mix" || path === "cf.feed") return "dB";
  return "";
}

/** A value with its unit, as Qt's history shows it ("1000 Hz", "−1.5 dB", "0.71"). */
export function valueText(v: ParamValue | null, path: string): string {
  if (v === null || typeof v !== "object") return "";
  if ("float" in v) {
    const unit = unitOf(path);
    const n = v.float.toFixed(unit === "Hz" ? 0 : path === "eq.q" ? 2 : 1).replace("-", "−");
    return unit ? `${n} ${unit}` : n;
  }
  if ("choice" in v) return String(v.choice);
  if ("text" in v) return v.text;
  return valueWords(v);
}

/** One change in Qt's words: its title ("Gain, Out 1, band 3") and its values ("0.0 dB → −1.5 dB"). */
export function describeEntry(e: HistoryEntry, names: HistoryNames, fallback: (path: string) => string): { title: string; detail: string } {
  const { path, indices } = e;
  const first = indices.length ? indices[0]! : -1;
  const crossover = path.startsWith("eq.") && indices.length > 1 && indices[1]! >= XOVER_BAND_BASE;
  let title = labelOf(path, fallback);
  const where: string[] = [];
  if (path.startsWith("out.") && indices.length) where.push(names.output(first));
  else if ((path.startsWith("eq.") || path.startsWith("ch.") || path === "pre" || path === "bypass") && indices.length) {
    where.push(names.wire(first));
    if (crossover) {
      title = XOVER[path] ?? title;
      where.push(`filter ${indices[1]! - XOVER_BAND_BASE + 1}`);
    } else if (path.startsWith("eq.") && indices.length > 1) where.push(`band ${indices[1]! + 1}`);
  } else if (path === "mix" && indices.length > 1) where.push(`${names.input(first)} to ${names.output(indices[1]!)}`);
  else if (path.startsWith("preset.") && indices.length) {
    const slot = first + 1;
    if (path === "preset.load") title = `Preset ${slot} loaded`;
    else if (path === "preset.save") title = `Saved to preset ${slot}`;
    else if (path === "preset.delete") title = `Preset ${slot} cleared`;
    else if (path === "preset.name") title = `Preset ${slot} name`;
    else where.push(`preset ${slot}`);
  }
  const worded = path === "sig.control";
  if (worded) {
    const a = e.after;
    const choice = a !== null && typeof a === "object" && "choice" in a ? a.choice : a !== null && typeof a === "object" && "int" in a ? a.int : -1;
    title = choice === 1 ? "Signal generator started" : choice === 2 ? "Signal generator stopped at once" : "Signal generator stopped";
  }
  if (where.length) title += `, ${where.join(", ")}`;
  const before = worded ? "" : valueText(e.before, path);
  const after = worded ? "" : valueText(e.after, path);
  return { title, detail: before && after ? `${before} → ${after}` : after };
}

// -- WP WEBM5 -- (review RC16) the page reads the journal again on every change of the device's
// state; when it is the same list the rows are not made again.
/** True when `b` is the same journal as `a` (the same changes, in the same order). */
export function sameHistory(a: readonly HistoryEntry[], b: readonly HistoryEntry[]): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i]!;
    const y = b[i]!;
    if (x.atMs !== y.atMs || x.path !== y.path || x.source !== y.source || x.undoable !== y.undoable || x.indices.join() !== y.indices.join()) return false;
  }
  return true;
}
