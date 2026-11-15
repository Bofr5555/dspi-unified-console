// WP WB6j: one session's spectrum analyser on the web (M22 the bar strip, M23 the overlay on the
// EQ graph). The device has one analyser; the core decides who drives it (the last choice made
// wins, `rta_get_ownership`) and runs its polling (`rtaStart`, the Worker's `rtaTick`). This only
// says what the views on screen want, reads the core's picture while one is shown, and turns it
// into bars. Nothing here talks to the device but through those calls.
//
// Views: each strip or overlay on screen registers what it wants (a tap and its channels). The
// request is the newest view's tap with every view's channels on that tap; with no view left the
// analyser is released (`rtaStop`). After the session reconnects the request is sent again, as the
// plan's rule for every client says (`rta_start` after a reconnect).

import type { AnalyserView } from "../../core/tools-protocol";
import type { KeyValueStore } from "../../fleet/local";
import { safeLocalStorage } from "../../fleet/local";
import type { Session } from "../../ui/session";

export const TAP_INPUT = 0;
export const TAP_OUTPUT = 1;

/** How often the picture is read while a view is shown, ms (the core's own tick is 60 ms). */
export const POLL_MS = 100;
/** The bar strip's columns at most (the Console's). */
export const MAX_COLUMNS = 4;
/** How much of the last level a bar keeps when smoothing is on (the Console's 0.35). */
export const SMOOTHING = 0.35;

// ------------------------------------------------------------------ preferences

/**
 * How the analyser draws and what it asks for: the Settings page's Spectrum tab (WB6i), kept in
 * this page's storage under `dspi-web.prefs` (`spectrum`). Read here, never written.
 */
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

export const DEFAULT_SPECTRUM: SpectrumPrefs = {
  graphOpacity: 1,
  showPeakHold: true,
  smoothing: true,
  floorDb: -90,
  ceilingDb: 6,
  fftOrder: 10,
  avgMs: 125,
  peakDecay: 12,
};

const PREFS_KEY = "dspi-web.prefs";

const num = (v: unknown, d: number, lo: number, hi: number): number => (typeof v === "number" && Number.isFinite(v) ? Math.min(hi, Math.max(lo, v)) : d);
const bool = (v: unknown, d: boolean): boolean => (typeof v === "boolean" ? v : d);

/** The Spectrum preferences from `raw` (the stored `dspi-web.prefs`), every field checked. */
export function spectrumFrom(raw: unknown): SpectrumPrefs {
  const s = ((raw && typeof raw === "object" ? (raw as { spectrum?: unknown }).spectrum : null) ?? {}) as Record<string, unknown>;
  const d = DEFAULT_SPECTRUM;
  const floorDb = num(s.floorDb, d.floorDb, -140, -20);
  return {
    graphOpacity: num(s.graphOpacity, d.graphOpacity, 0.3, 1),
    showPeakHold: bool(s.showPeakHold, d.showPeakHold),
    smoothing: bool(s.smoothing, d.smoothing),
    floorDb,
    ceilingDb: Math.max(floorDb + 10, num(s.ceilingDb, d.ceilingDb, -20, 24)),
    fftOrder: Math.round(num(s.fftOrder, d.fftOrder, 0, 16)),
    avgMs: Math.round(num(s.avgMs, d.avgMs, 0, 10000)),
    peakDecay: Math.round(num(s.peakDecay, d.peakDecay, 0, 255)),
  };
}

export function loadSpectrum(store: KeyValueStore | null): SpectrumPrefs {
  try {
    const t = store?.getItem(PREFS_KEY);
    return spectrumFrom(t ? JSON.parse(t) : null);
  } catch {
    return spectrumFrom(null);
  }
}

// ------------------------------------------------------------- what is shown

/** What this console shows of the analyser, kept in this page's storage. */
export interface AnalyserChoice {
  /** The bar strip on the main screen and the channel pages (M22). */
  bars: boolean;
  /** The spectrum on the EQ graph (M23). */
  graph: boolean;
  /** The main screen strip's tap and channels. */
  tap: number;
  mask: number;
}

export const DEFAULT_CHOICE: AnalyserChoice = { bars: false, graph: false, tap: TAP_INPUT, mask: 0b11 };
const CHOICE_KEY = "dspi-web.analyser";

export function tidyChoice(raw: unknown): AnalyserChoice {
  const o = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const tap = o.tap === TAP_OUTPUT ? TAP_OUTPUT : TAP_INPUT;
  const mask = typeof o.mask === "number" && Number.isInteger(o.mask) && o.mask >= 0 && o.mask <= 0xffff ? o.mask : DEFAULT_CHOICE.mask;
  return { bars: bool(o.bars, DEFAULT_CHOICE.bars), graph: bool(o.graph, DEFAULT_CHOICE.graph), tap, mask };
}

export function loadChoice(store: KeyValueStore | null): AnalyserChoice {
  try {
    const t = store?.getItem(CHOICE_KEY);
    return tidyChoice(t ? JSON.parse(t) : null);
  } catch {
    return tidyChoice(null);
  }
}

export function saveChoice(store: KeyValueStore | null, c: AnalyserChoice): void {
  try {
    store?.setItem(CHOICE_KEY, JSON.stringify(c));
  } catch {
    // Refused storage: the choice lasts for this page only.
  }
}

// ----------------------------------------------------------------- the request

export interface ViewWant {
  tap: number;
  mask: number;
}

/** The analyser request for the views on screen, newest last: its tap, every view's channels on it. Null for none. */
export function combine(views: ViewWant[]): ViewWant | null {
  const newest = views[views.length - 1];
  if (!newest) return null;
  let mask = 0;
  for (const v of views) if (v.tap === newest.tap) mask |= v.mask;
  return { tap: newest.tap, mask: mask & 0xffff };
}

/** The bar strip's columns for `count` channels (the Console's: up to four). */
export function columnsFor(count: number): number {
  return Math.max(1, Math.min(MAX_COLUMNS, count));
}

/** The channel bits of `mask`, lowest first. */
export function channelsOf(mask: number): number[] {
  const out: number[] = [];
  for (let c = 0; c < 16; c++) if (mask & (1 << c)) out.push(c);
  return out;
}

/** How high a level is between the floor and the ceiling, 0..1. */
export function levelFrac(db: number, floorDb: number, ceilingDb: number): number {
  if (!Number.isFinite(db)) return 0;
  return Math.min(1, Math.max(0, (db - floorDb) / Math.max(1, ceilingDb - floorDb)));
}

/** Whether band `b` has data at this rate and transform size (`populated`, 64 bits as a decimal string). */
export function populated(bits: string, b: number): boolean {
  try {
    return ((BigInt(bits) >> BigInt(b)) & 1n) === 1n;
  } catch {
    return true;
  }
}

/** The bands of one channel to draw: centre, level, peak, only those with data. */
export interface Bar {
  hz: number;
  db: number;
  peak: number;
}

export function barsOf(v: AnalyserView, channel: number): Bar[] | null {
  const f = v.snapshot.frames.find((x) => x.channel === channel);
  if (!f) return null;
  const out: Bar[] = [];
  const n = Math.min(f.nBands, v.caps.centres.length, f.avgDb.length);
  for (let b = 0; b < n; b++) {
    if (!populated(v.snapshot.populated, b)) continue;
    const hz = v.caps.centres[b] ?? 0;
    if (hz <= 0) continue;
    out.push({ hz, db: f.avgDb[b] ?? -Infinity, peak: f.peakDb[b] ?? -Infinity });
  }
  return out;
}

/** `prev` eased toward `next` (a bar's smoothing): the Console keeps 0.35 of the last level. */
export function smooth(prev: number[] | undefined, next: number[], on: boolean): number[] {
  if (!on || !prev || prev.length !== next.length) return next;
  return next.map((v, i) => {
    const p = prev[i]!;
    return Number.isFinite(p) && Number.isFinite(v) ? p * SMOOTHING + v * (1 - SMOOTHING) : v;
  });
}

/** A channel's name on a tap, from the page's rows; "In 3" / "Out 3" where it has none. */
export function channelName(tap: number, channel: number, names: string[]): string {
  return names[channel] || `${tap === TAP_OUTPUT ? "Out" : "In"} ${channel + 1}`;
}

/** The line shown while another console drives the analyser. */
export function followingText(o: AnalyserView["ownership"], names: { inputs: string[]; outputs: string[] }): string {
  const list = o.tap === TAP_OUTPUT ? names.outputs : names.inputs;
  const which = channelsOf(o.mask).map((c) => channelName(o.tap, c, list));
  const side = o.tap === TAP_OUTPUT ? "outputs" : "inputs";
  return which.length
    ? `Another console is running the analyser: ${which.join(", ")}.`
    : `Another console is running the analyser on the ${side}.`;
}

// ------------------------------------------------------------------ the model

/** One session's analyser: the views on screen, the request, the picture. */
export class Analyser {
  /** The latest picture, null until read (or while nothing is shown). */
  view: AnalyserView | null = null;
  /** The last refusal of the core, "" when none. */
  error = "";
  choice: AnalyserChoice;
  private readonly views = new Map<string, ViewWant>();
  private readonly listeners = new Set<() => void>();
  private sent: string | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private unwatch: (() => void) | null = null;
  private lastStatus = "";
  private stopped = false;

  constructor(
    private readonly session: Session,
    private readonly store: KeyValueStore | null = safeLocalStorage(),
    private readonly pollMs = POLL_MS,
  ) {
    this.choice = loadChoice(store);
  }

  get prefs(): SpectrumPrefs {
    return loadSpectrum(this.store);
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(): void {
    for (const l of this.listeners) l();
  }

  setChoice(patch: Partial<AnalyserChoice>): void {
    this.choice = tidyChoice({ ...this.choice, ...patch });
    saveChoice(this.store, this.choice);
    this.emit();
  }

  /** Show (`want`) or take away (null) view `id`. */
  setView(id: string, want: ViewWant | null): void {
    const had = this.views.get(id);
    if (want && had && had.tap === want.tap && had.mask === want.mask) return;
    if (!want && !had) return;
    this.views.delete(id);
    if (want) this.views.set(id, want);
    this.apply();
  }

  /** The request the views make now. */
  request(): ViewWant | null {
    return combine([...this.views.values()]);
  }

  /** Stop following another console: this console's request runs the analyser again. */
  async takeControl(): Promise<void> {
    try {
      await this.session.core.call("rtaTakeControl", undefined);
      await this.read();
    } catch (e) {
      this.session.fail(e);
    }
  }

  private apply(): void {
    if (this.stopped) return;
    const want = this.request();
    if (!this.unwatch && want) {
      // A reconnect: the request is sent again once the session is open once more.
      this.unwatch = this.session.watch(() => {
        const st = this.session.state.status;
        if (st !== this.lastStatus) {
          this.lastStatus = st;
          if (st === "open") {
            this.sent = null;
            this.apply();
          }
        }
      });
      this.lastStatus = this.session.state.status;
    }
    const key = want ? `${want.tap}:${want.mask}:${JSON.stringify(this.prefs)}` : "off";
    // Released only when this console asked for something: a view that shows nothing never stops another's request.
    if (!want && this.sent === null) this.sent = "off";
    if (key !== this.sent && this.session.state.status === "open") {
      this.sent = key;
      void this.send(want);
    }
    if (want) this.schedule(0);
    else {
      clearTimeout(this.timer);
      this.timer = undefined;
      this.unwatch?.();
      this.unwatch = null;
      this.view = null;
      this.emit();
    }
  }

  private async send(want: ViewWant | null): Promise<void> {
    try {
      if (!want) {
        await this.session.core.call("rtaStop", undefined);
        return;
      }
      const p = this.prefs;
      await this.session.core.call("rtaStart", {
        tap: want.tap,
        mask: want.mask,
        fftOrder: p.fftOrder,
        avgMs: p.avgMs,
        peakDecayDbS: p.peakDecay,
      });
      this.error = "";
    } catch (e) {
      this.sent = null;
      this.error = e instanceof Error ? e.message : String(e);
      this.emit();
    }
  }

  private schedule(ms: number): void {
    if (this.timer !== undefined || this.stopped) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      void this.tick();
    }, ms);
  }

  private async tick(): Promise<void> {
    if (!this.request() || this.stopped) return;
    const hidden = typeof document !== "undefined" && document.visibilityState === "hidden";
    if (!hidden && this.session.state.status === "open") await this.read();
    this.schedule(this.pollMs);
  }

  private smoothed = new Map<number, number[]>();

  private async read(): Promise<void> {
    try {
      const v = await this.session.core.call("analyserView", undefined);
      if (!this.request()) return;
      const on = this.prefs.smoothing;
      for (const f of v.snapshot.frames) {
        const s = smooth(this.smoothed.get(f.channel), f.avgDb, on);
        this.smoothed.set(f.channel, s);
        f.avgDb = s;
      }
      this.view = v;
      this.error = "";
    } catch (e) {
      this.error = e instanceof Error ? e.message : String(e);
    }
    this.emit();
  }

  /** The session ends: release the analyser, stop reading. */
  stop(): void {
    this.views.clear();
    this.apply();
    this.stopped = true;
  }
}

const analysers = new WeakMap<Session, Analyser>();

/** The session's one analyser, shared by every view of it. */
export function analyserFor(session: Session): Analyser {
  let a = analysers.get(session);
  if (!a) {
    a = new Analyser(session);
    analysers.set(session, a);
  }
  return a;
}
