// WP WB6i: the Settings page's rules that are the page's own: the app preferences (Graphing,
// Spectrum, Show Debug Information; the console's, kept in this page's storage, applied at once),
// the help links, and how the device's settings are put on screen. Every device change is the
// core's (`write`, the core-voltage calls); nothing here checks a pin, a voltage or a word.

import { useEffect, useState } from "preact/hooks";
import type { DspState, OutputLabel, OutputLatencyRecord, ParamValue } from "../../core/protocol";
import { safeLocalStorage, type KeyValueStore } from "../../fleet/local";
import type { Session } from "../../ui/session";

// ---------------------------------------------------------------- preferences

/** How the response graph draws (the Console's Graphing tab). */
export interface GraphPrefs {
  showGlow: boolean;
  lineWidth: number;
  showFreqGrid: boolean;
  showFreqLabels: boolean;
  showDbGrid: boolean;
  showDbLabels: boolean;
  showFreqReadout: boolean;
  showLevelReadout: boolean;
  /** 0..2; 0.5 is the theme's own grid. */
  gridOpacity: number;
  /** The vertical span in dB, 10..100. */
  dbRange: number;
  /** Its middle in dB, -40..20. */
  dbCenter: number;
  minFreq: number;
  maxFreq: number;
  // -- WP GAP1 -- Qt's Graphing switches: the edited channel's phase over the graph (the Phase chip)
  showPhase: boolean;
  /** The phase continuous rather than within ±180°. */
  phaseUnwrapped: boolean;
}

/** How the analyser draws and what it asks the device for (the Console's Spectrum tab). */
export interface SpectrumPrefs {
  graphOpacity: number;
  showPeakHold: boolean;
  smoothing: boolean;
  floorDb: number;
  ceilingDb: number;
  fftOrder: number;
  avgMs: number;
  peakDecay: number;
}

export interface Prefs {
  graph: GraphPrefs;
  spectrum: SpectrumPrefs;
  /** Show Debug Information (M62). */
  debug: boolean;
}

export const MIN_FREQS = [10, 15, 20, 50, 100];
export const MAX_FREQS = [5000, 10000, 20000];
export const FLOORS = [-60, -90, -120];
export const CEILINGS = [0, 6, 12];
export const AVERAGING = [0, 50, 125, 300, 1000, 3000];
export const PEAK_DECAYS = [0, 4, 12, 30];
/** The transform sizes as powers of two, the protocol's range. */
export const FFT_ORDERS = [8, 9, 10];

export const DEFAULT_PREFS: Prefs = {
  graph: {
    showGlow: false,
    lineWidth: 1.5,
    showFreqGrid: true,
    showFreqLabels: true,
    showDbGrid: true,
    showDbLabels: true,
    showFreqReadout: true,
    showLevelReadout: true,
    gridOpacity: 0.5,
    dbRange: 50,
    dbCenter: 0,
    minFreq: 20,
    maxFreq: 20000,
    showPhase: false, // -- WP GAP1 --
    phaseUnwrapped: false,
  },
  spectrum: { graphOpacity: 1, showPeakHold: true, smoothing: true, floorDb: -90, ceilingDb: 6, fftOrder: 10, avgMs: 125, peakDecay: 12 },
  debug: false,
};

const KEY = "dspi-web.prefs";

const num = (v: unknown, d: number, lo: number, hi: number): number => (typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d);
const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : d);
const oneOf = (v: unknown, list: number[], d: number): number => (typeof v === "number" && list.includes(v) ? v : d);

/** `raw` as preferences: every field checked, anything missing or wrong the default. */
export function tidyPrefs(raw: unknown): Prefs {
  const o = (raw && typeof raw === "object" ? raw : {}) as { graph?: Record<string, unknown>; spectrum?: Record<string, unknown>; debug?: unknown };
  const g = o.graph ?? {};
  const s = o.spectrum ?? {};
  const dg = DEFAULT_PREFS.graph;
  const ds = DEFAULT_PREFS.spectrum;
  const graph: GraphPrefs = {
    showGlow: bool(g.showGlow, dg.showGlow),
    lineWidth: num(g.lineWidth, dg.lineWidth, 1, 4),
    showFreqGrid: bool(g.showFreqGrid, dg.showFreqGrid),
    showFreqLabels: bool(g.showFreqLabels, dg.showFreqLabels),
    showDbGrid: bool(g.showDbGrid, dg.showDbGrid),
    showDbLabels: bool(g.showDbLabels, dg.showDbLabels),
    showFreqReadout: bool(g.showFreqReadout, dg.showFreqReadout),
    showLevelReadout: bool(g.showLevelReadout, dg.showLevelReadout),
    gridOpacity: num(g.gridOpacity, dg.gridOpacity, 0, 2),
    dbRange: Math.round(num(g.dbRange, dg.dbRange, 10, 100)),
    dbCenter: Math.round(num(g.dbCenter, dg.dbCenter, -40, 20)),
    minFreq: oneOf(g.minFreq, MIN_FREQS, dg.minFreq),
    maxFreq: oneOf(g.maxFreq, MAX_FREQS, dg.maxFreq),
    showPhase: bool(g.showPhase, dg.showPhase), // -- WP GAP1 --
    phaseUnwrapped: bool(g.phaseUnwrapped, dg.phaseUnwrapped),
  };
  const spectrum: SpectrumPrefs = {
    graphOpacity: num(s.graphOpacity, ds.graphOpacity, 0.3, 1),
    showPeakHold: bool(s.showPeakHold, ds.showPeakHold),
    smoothing: bool(s.smoothing, ds.smoothing),
    floorDb: oneOf(s.floorDb, FLOORS, ds.floorDb),
    ceilingDb: oneOf(s.ceilingDb, CEILINGS, ds.ceilingDb),
    fftOrder: oneOf(s.fftOrder, FFT_ORDERS, ds.fftOrder),
    avgMs: oneOf(s.avgMs, AVERAGING, ds.avgMs),
    peakDecay: oneOf(s.peakDecay, PEAK_DECAYS, ds.peakDecay),
  };
  return { graph, spectrum, debug: bool(o.debug, DEFAULT_PREFS.debug) };
}

export function loadPrefs(store: KeyValueStore | null): Prefs {
  try {
    const t = store?.getItem(KEY);
    return tidyPrefs(t ? JSON.parse(t) : null);
  } catch {
    return tidyPrefs(null);
  }
}

export function savePrefs(store: KeyValueStore | null, p: Prefs): void {
  try {
    store?.setItem(KEY, JSON.stringify(p));
  } catch {
    // A refused storage keeps nothing; the page goes on with what it shows.
  }
}

/**
 * The console's preferences, one copy for the page: every graph and the Settings page see a
 * change at once. Kept in this page's storage (the console's, not the device's).
 */
export class PrefsStore {
  private value: Prefs;
  private readonly listeners = new Set<() => void>();
  constructor(private readonly store: KeyValueStore | null) {
    this.value = loadPrefs(store);
  }
  get prefs(): Prefs {
    return this.value;
  }
  set(patch: { graph?: Partial<GraphPrefs>; spectrum?: Partial<SpectrumPrefs>; debug?: boolean }): void {
    this.value = tidyPrefs({
      graph: { ...this.value.graph, ...patch.graph },
      spectrum: { ...this.value.spectrum, ...patch.spectrum },
      debug: patch.debug ?? this.value.debug,
    });
    savePrefs(this.store, this.value);
    for (const l of this.listeners) l();
  }
  reset(part: "graph" | "spectrum"): void {
    this.set(part === "graph" ? { graph: DEFAULT_PREFS.graph } : { spectrum: DEFAULT_PREFS.spectrum });
  }
  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }
}

let shared: PrefsStore | null = null;
/** The page's one store. */
export function prefsStore(): PrefsStore {
  shared ??= new PrefsStore(safeLocalStorage());
  return shared;
}

/** The preferences, re-rendering on a change. */
export function usePrefs(store: PrefsStore = prefsStore()): Prefs {
  const [p, setP] = useState(store.prefs);
  useEffect(() => {
    setP(store.prefs);
    return store.watch(() => setP(store.prefs));
  }, [store]);
  return p;
}

/** The graph's vertical extent from the preferences: `[bottom, top]` in dB. */
export function dbSpan(g: GraphPrefs): [number, number] {
  return [g.dbCenter - g.dbRange / 2, g.dbCenter + g.dbRange / 2];
}

export function signedDb(db: number): string {
  const n = Math.round(db);
  return n > 0 ? `+${n}` : String(n);
}

export function freqText(hz: number): string {
  return hz >= 1000 ? `${hz / 1000} kHz` : `${hz} Hz`;
}

/** The graph's cursor readout: "1.25 kHz", "-3.5 dB", or both; "" with neither on. */
export function readoutText(g: GraphPrefs, hz: number, db: number): string {
  const parts: string[] = [];
  if (g.showFreqReadout) parts.push(hz >= 1000 ? `${(hz / 1000).toFixed(hz >= 10000 ? 1 : 2)} kHz` : `${Math.round(hz)} Hz`);
  if (g.showLevelReadout) parts.push(`${db > 0 ? "+" : ""}${db.toFixed(1)} dB`);
  return parts.join("  ");
}

// ---------------------------------------------------------------- help links (M61)

/** The two GitHub links of the Console's Help menu: this console's project, and the firmware. */
export const HELP_LINKS = [
  { id: "project", title: "DSPi Unified Console on GitHub", url: "https://github.com/Bofr5555/dspi-unified-console" },
  { id: "firmware", title: "DSPi Firmware on GitHub", url: "https://github.com/WeebLabs/DSPi" },
];

// ---------------------------------------------------------------- names (M34, M61)

/**
 * The names Reset Channel Names writes, one per wire channel (the Console's
 * `defaultChannelNames`): the inputs "USB L", "USB R", then "USB 3".., the outputs the core's
 * own slot name (`OutputLabel.slotName`: "SPDIF 1 L", "I2S 2 R", "PDM"). The input rule is the
 * core's too (`presets.rs` `firmware_name`) but not exported; see the WP report.
 */
export function defaultChannelNames(s: Pick<DspState, "numInputChannels" | "numOutputChannels">, labels: OutputLabel[] | undefined): string[] | null {
  const names: string[] = [];
  for (let ch = 0; ch < s.numInputChannels; ch++) names.push(ch === 0 ? "USB L" : ch === 1 ? "USB R" : `USB ${ch + 1}`);
  for (let o = 0; o < s.numOutputChannels; o++) {
    const l = labels?.find((x) => x.output === o);
    if (!l?.slotName) return null;
    names.push(l.slotName);
  }
  return names;
}

/** The writes that put `names` on the device, skipping a channel that already has its name. */
export function nameWrites(current: string[], names: string[]): { path: string; indices: number[]; value: ParamValue }[] {
  return names.flatMap((n, ch) => (current[ch] === n ? [] : [{ path: "ch.name", indices: [ch], value: { text: n } as ParamValue }]));
}

/**
 * Reset Channel Names (M61): every channel to its default name, as one undo step, only the
 * channels that differ. False when the names cannot be known yet or a write was refused.
 */
export async function resetChannelNames(session: Session): Promise<boolean> {
  const snap = session.state.snapshot;
  if (!snap) return false;
  const names = defaultChannelNames(snap.state, snap.outputLabels);
  if (!names) return false;
  const writes = nameWrites(snap.state.channelNames, names);
  if (!writes.length) return true;
  return session.group("Reset Channel Names", () => session.writeAll(writes));
}

/** The device name's field, trimmed; null when it is not a name the core can take (over 31 bytes). */
export function deviceNameValue(text: string): string | null {
  const t = text.trim();
  return new TextEncoder().encode(t).length > 31 ? null : t;
}

// ---------------------------------------------------------------- output latency (M35)

/** The default option (the Console's OUTPUT_LATENCY_DEFAULT_ID). */
export const LATENCY_DEFAULT_ID = 5;

export function latencyMs(us: number): string {
  const ms = us / 1000;
  return `${Number.isInteger(ms) ? ms.toFixed(0) : ms.toFixed(2).replace(/0$/, "")} ms`;
}

/** The latency choices to show: the device's options, the default marked; the request kept if unknown. */
export function latencyChoices(r: OutputLatencyRecord): { id: number; text: string; available: boolean }[] {
  const out = r.options.map((o) => ({
    id: o.id,
    text: o.id === LATENCY_DEFAULT_ID ? `${latencyMs(o.latencyUs)} (default)` : latencyMs(o.latencyUs),
    available: o.available,
  }));
  if (r.options.length && !r.options.some((o) => o.id === r.requestedId)) out.push({ id: r.requestedId, text: "-", available: false });
  return out;
}

/** The words under the latency choice (the Console's). */
export function latencyCaption(r: OutputLatencyRecord): string {
  let text = "Lower values reduce the delay from input to output. Higher values may benefit sources exhibiting high jitter in the event of audio stutters.";
  const o = r.options.find((x) => x.id === r.requestedId);
  if (o && o.available && o.effectiveUs !== o.latencyUs)
    text += ` At the current sample rate the output buffer is smaller than this, so it runs as ${latencyMs(o.effectiveUs)}.`;
  if (r.pending) text += " The device is still applying the last change.";
  return `${text} Values less than 2ms have no effect upon USB Audio.`;
}

// ---------------------------------------------------------------- pins (E1)

/** A pin the device reports, and the registry path that moves it. */
export interface PinRow {
  key: string;
  /** What uses the pin, from the device's own lists. */
  title: string;
  path: string;
  indices: number[];
  pin: number;
}

/** No pin assigned (the firmware's 0xFF). */
export const NO_PIN = 255;

const OUTPUT_TYPE = ["S/PDIF", "I2S", "PDM"];

type PinState = DspState & {
  numPinOutputs?: number;
  outputPins?: number[];
  outputTypes?: number[];
  spdifRxPins?: number[];
  spdifInputsEnabled?: number;
  i2sRxPins?: number[];
  i2sInputChannels?: number;
  adatInputPin?: number;
  adatInputEnabled?: boolean;
  i2sBckPin?: number;
  i2sBckPinSlave?: number;
  i2sClockPinMode?: number;
  mckEnabled?: boolean;
  mckPin?: number;
  adatOutEnabled?: boolean;
  adatOutPin?: number;
};

/**
 * Every pin the device's state lists, in the device's order: the output slots, the I2S clocks,
 * the master clock and ADAT out, then the inputs. Which pins are free or can do a job is not
 * judged here (the core does not export those rules yet): the device refuses a pin it cannot use.
 */
export function pinRows(state: DspState): PinRow[] {
  const s = state as PinState;
  const rows: PinRow[] = [];
  const outs = s.outputPins ?? [];
  const n = Math.min(s.numPinOutputs ?? outs.length, outs.length);
  for (let i = 0; i < n; i++) {
    const kind = OUTPUT_TYPE[s.outputTypes?.[i] ?? 0] ?? "Output";
    rows.push({ key: `out${i}`, title: `Output ${i + 1} (${kind})`, path: "out.pin", indices: [i], pin: outs[i]! });
  }
  if (typeof s.i2sBckPin === "number") rows.push({ key: "bck", title: "I2S bit clock (word clock on the next pin)", path: "i2s.bck", indices: [], pin: s.i2sBckPin });
  if (typeof s.i2sBckPinSlave === "number" && s.i2sClockPinMode === 1)
    rows.push({ key: "bck-slave", title: "I2S slave bit clock", path: "i2s.bck.slave", indices: [], pin: s.i2sBckPinSlave });
  if (typeof s.mckPin === "number" && s.mckEnabled) rows.push({ key: "mck", title: "Master clock", path: "i2s.mck.pin", indices: [], pin: s.mckPin });
  if (typeof s.adatOutPin === "number" && s.adatOutEnabled) rows.push({ key: "adat-out", title: "ADAT output", path: "adat.pin", indices: [], pin: s.adatOutPin });
  (s.spdifRxPins ?? []).forEach((p, i) => {
    if (i < Math.max(1, s.spdifInputsEnabled ?? 1)) rows.push({ key: `spdif${i}`, title: `S/PDIF input ${i + 1}`, path: "in.spdif.pin", indices: [i], pin: p });
  });
  const pairs = Math.ceil((s.i2sInputChannels ?? 0) / 2);
  (s.i2sRxPins ?? []).slice(0, pairs).forEach((p, i) => rows.push({ key: `i2s${i}`, title: `I2S input pair ${i + 1}`, path: "in.i2s.pin", indices: [i], pin: p }));
  if (typeof s.adatInputPin === "number" && s.adatInputEnabled) rows.push({ key: "adat-in", title: "ADAT input", path: "in.adat.pin", indices: [], pin: s.adatInputPin });
  return rows;
}

export function pinText(pin: number): string {
  return pin === NO_PIN ? "none" : `GPIO ${pin}`;
}

/** A typed pin as an int write's value, or null when it is not a whole number. */
export function pinValue(text: string): ParamValue | null {
  const t = text.trim();
  if (!/^\d{1,3}$/.test(t)) return null;
  return { int: Number(t) };
}

// ---------------------------------------------------------------- core voltage (M56)

export function voltsText(mv: number): string {
  return mv > 0 ? `${(mv / 1000).toFixed(2)} V` : "-";
}

export function durationText(s: number): string {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (d > 0) return `${d} d ${h} h`;
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min ${sec} s`;
  return `${sec} s`;
}

/** The soak choices in minutes: the core's minimum, its suggestion, and a few longer. */
export function soakMinutes(minS: number, defaultS: number): number[] {
  const lo = Math.max(1, Math.ceil(minS / 60));
  const set = new Set([lo, Math.round(defaultS / 60), 60, 120, 240].filter((m) => m >= lo));
  return [...set].sort((a, b) => a - b);
}

/**
 * Whether the operation `op` of the core's mode manifest belongs to `mode` (the core voltage is
 * the Installer mode's: `operations[].modes`). Without the manifest, nothing extra is shown.
 */
export function operationIn(manifest: { operations?: { id: string; modes: string[] }[] } | null, op: string, mode: string): boolean {
  return !!manifest?.operations?.some((o) => o.id === op && o.modes.includes(mode));
}

/** Whether the manifest shows screen `id` in `mode` (the Pins section: "hardware"). */
export function screenIn(manifest: { screens?: { id: string; modes: string[] }[] } | null, id: string, mode: string): boolean {
  return !!manifest?.screens?.some((x) => x.id === id && x.modes.includes(mode));
}
