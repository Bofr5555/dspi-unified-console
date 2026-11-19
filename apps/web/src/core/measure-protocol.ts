// WP P5WMS: the measurement calls of the web console (PLAN-P5 sections 1.1 to 1.6, T9, T11):
// their records as TypeScript, the typed way to call them, and the Worker's part of a run.
//
// The other side is crates/dspi-wasm: measure.rs (P5SES: `measure*`), measure_live.rs (P5LIVE:
// `measureLive*`), measure_docs.rs (P5DOCK), measure_sim.rs (P5DEMO), hub_server_measure.rs
// (P5HUB). Shapes are the core's records in camelCase (`dspi_core::MeasureSetup`, ...); a field
// the core may leave out is optional here.
//
// The Worker's part (`MeasureWorkerRoute`, wired in worker.ts):
// - `measureCaptureOpen` makes a MessageChannel in the Worker and answers with one end (and where
//   capture-worklet.js is, beside the Worker). The page hands that end to the capture worklet
//   (capture.ts), so the microphone's blocks go from the worklet straight to the Worker, which
//   pushes each into `measureCapturePush` in order. Raw audio goes nowhere else (S12).
// - After a `measureStart`, `measureNextPosition` or `measureLiveStart` the Worker turns the
//   session (`measureTick`, or `measureLiveTick` while live) about every 50 ms until it is no
//   longer active, and pushes each state to the page as a `measure` or `measureLive` event. A
//   Worker's timers are not slowed in a background tab as a page's are, so the device lock's
//   lease and the capture keep up while the console is not in front.
// Neither method reaches the core; nothing here plays anything.

import { bindCapturePort, measurePushParams } from "./capture";

// ---- the session (P5SES) ----

export type MeasureRunKind = "walked" | "summed" | "liveSweep" | "pink";
export type MeasureRole = "full" | "sub" | "low" | "mid" | "high";
export type MeasurePlacement = "listeningPosition" | "multiPosition" | "nearField" | "headphones";
export type MeasureCoupler = "iec60318_4" | "flatPlate" | "hats";

export interface MeasureOutputRecord {
  output: number;
  role: MeasureRole;
  speaker?: string;
  group?: number;
  inverted?: boolean;
}

/** `dspi_core::MeasureSetup`: every number 0 (or absent) is the plan's default. */
export interface MeasureSetupRecord {
  kind?: MeasureRunKind;
  outputs: MeasureOutputRecord[];
  placement?: MeasurePlacement;
  count?: number;
  coupler?: MeasureCoupler | null;
  micDocument?: string;
  sweepMs?: number;
  gapMs?: number;
  passes?: number;
  pinkMs?: number;
  ceilingDb?: number | null;
  splCapDb?: number | null;
  captureRateHz?: number;
  name?: string;
  startLevelOnly?: boolean;
}

export interface MeasureIssueRecord {
  key: string;
  output?: number | null;
  message: string;
}

export interface MeasurePreflightRecord {
  clear: boolean;
  blockers: MeasureIssueRecord[];
  warnings: MeasureIssueRecord[];
}

export type MeasurePhase = "idle" | "preRoll" | "ramp" | "run" | "analysing" | "waitPosition" | "done" | "aborted" | "failed";

export interface MeasureTimingRecord {
  reliable: boolean;
  ratioPpm: number;
  ratioFitted: boolean;
  maxResidualUs: number;
  residualBoundUs: number;
  maxPhaseSpreadDeg: number;
  passes: number;
  reasons: string[];
  cycleOutputs: number[];
  cycleResidualsUs: number[];
}

export interface MeasureStateRecord {
  phase: MeasurePhase;
  kind: MeasureRunKind;
  active: boolean;
  wantsCapture: boolean;
  captureRateHz: number;
  position: number;
  positions: number;
  run: number;
  runs: number;
  output?: number | null;
  levelDb?: number | null;
  rampLevels: { output: number; levelDb: number }[];
  runLevelDb?: number | null;
  frames: number;
  generatorStartFrame?: number | null;
  generatorStarts: number;
  progress: number;
  issue?: MeasureIssueRecord | null;
  timingReliable?: boolean | null;
  resultReady: boolean;
  timing?: MeasureTimingRecord | null;
  frameFaults: number;
}

// ---- the live view (P5LIVE) ----

export type MeasureLiveMode = "sweep" | "pinkRta";
export type MeasureRtaAveraging = "fast" | "slow" | "infinite";

export interface MeasureLiveSetupRecord {
  output: number;
  mode?: MeasureLiveMode;
  role?: MeasureRole;
  placement?: MeasurePlacement;
  coupler?: MeasureCoupler | null;
  micDocument?: string;
  targetDocument?: string;
  smoothing?: number;
  sweepMs?: number;
  gapMs?: number;
  pinkMs?: number;
  ceilingDb?: number | null;
  splCapDb?: number | null;
  captureRateHz?: number;
  averaging?: MeasureRtaAveraging;
  bandsPerOctave?: number;
}

/** A curve as a graph draws it. */
export interface MeasureCurveRecord {
  freqsHz: number[];
  magDb: number[];
  valid: boolean[];
  spl: boolean;
}

export interface MeasureLiveBandRecord {
  band: number;
  filterType: number;
  bypass: boolean;
  freq: number;
  q: number;
  gain: number;
  qp: number;
}

export interface MeasureLiveChangeRecord {
  band: number;
  before: MeasureLiveBandRecord;
  after: MeasureLiveBandRecord;
  words: string;
}

export interface MeasureLiveProposalRecord {
  id: number;
  adjust: "cutPeak" | "fitRange" | "edit";
  output: number;
  label: string;
  changes: MeasureLiveChangeRecord[];
  gainBeforeDb: number;
  gainAfterDb: number;
  rmsBeforeDb: number;
  rmsAfterDb: number;
  notes: string[];
  issue?: MeasureIssueRecord | null;
  predicted?: MeasureCurveRecord | null;
}

export interface MeasureLiveRtaRecord {
  current?: MeasureCurveRecord | null;
  held?: MeasureCurveRecord | null;
  compare?: MeasureCurveRecord | null;
  frames: number;
  averaging: MeasureRtaAveraging;
}

export interface MeasureLiveFrameRecord {
  active: boolean;
  mode: MeasureLiveMode;
  output: number;
  seq: number;
  measured?: MeasureCurveRecord | null;
  target?: MeasureCurveRecord | null;
  predicted?: MeasureCurveRecord | null;
  rta?: MeasureLiveRtaRecord | null;
  pending?: MeasureLiveProposalRecord | null;
  issue?: MeasureIssueRecord | null;
  session: MeasureStateRecord;
}

export interface MeasureLiveStateRecord {
  active: boolean;
  mode: MeasureLiveMode;
  output: number;
  seq: number;
  runs: number;
  levelDb?: number | null;
  pending?: number | null;
  issue?: MeasureIssueRecord | null;
  session: MeasureStateRecord;
}

// ---- documents (P5DOCK, P5HUB) and the simulated microphone (P5DEMO) ----

export interface MeasureDocInfoRecord {
  id: string;
  kind: string;
  name: string;
  note: string;
  serial: string;
  savedMs: number;
  bytes: number;
}

export interface TargetInfoRecord {
  id: string;
  name: string;
  targetUse: string;
  coupler: string;
  params: { name: string; value: number }[];
}

export interface MeasureSimInfoRecord {
  available: boolean;
  on: boolean;
  label: string;
  world: string;
  seed: number;
  issue?: string | null;
}

/** The measurement calls, method by method. */
export interface MeasureMethods {
  measurePreflight: { params: { setup: MeasureSetupRecord }; result: MeasurePreflightRecord };
  measureStart: { params: { setup: MeasureSetupRecord }; result: MeasureStateRecord };
  measureCapturePush: { params: { firstFrame: number; samples: number[]; rateHz: number; clipped: boolean }; result: MeasureStateRecord };
  measureTick: { params: undefined; result: MeasureStateRecord };
  measureNextPosition: { params: undefined; result: MeasureStateRecord };
  measureStop: { params: undefined; result: MeasureStateRecord };
  measureState: { params: undefined; result: MeasureStateRecord };
  /** The `measurement` document's text. */
  measureResult: { params: undefined; result: string };
  measureClear: { params: undefined; result: null };

  measureLiveStart: { params: { setup: MeasureLiveSetupRecord }; result: MeasureLiveStateRecord };
  measureLiveTick: { params: undefined; result: MeasureLiveStateRecord };
  measureLiveFrame: { params: undefined; result: MeasureLiveFrameRecord };
  measureLiveState: { params: undefined; result: MeasureLiveStateRecord };
  measureLiveStop: { params: undefined; result: MeasureLiveStateRecord };
  measureLivePredict: { params: { output: number; bands: MeasureLiveBandRecord[] }; result: MeasureLiveProposalRecord };
  measureLiveCutPeak: { params: { output: number; freqHz: number }; result: MeasureLiveProposalRecord };
  measureLiveFitRange: { params: { output: number; f1Hz: number; f2Hz: number }; result: MeasureLiveProposalRecord };
  measureLiveApply: { params: { proposalId: number }; result: MeasureLiveStateRecord };
  measureLiveDiscard: { params: undefined; result: MeasureLiveStateRecord };
  measureLiveHold: { params: { on: boolean }; result: MeasureLiveStateRecord };
  measureLiveAveraging: { params: { averaging: MeasureRtaAveraging }; result: MeasureLiveStateRecord };

  measureDocInfo: { params: { text: string }; result: MeasureDocInfoRecord };
  micImport: { params: { name: string; fileName: string; text: string }; result: { text: string } };
  targetImport: { params: { name: string; text: string; use: "speakers" | "headphones"; coupler: MeasureCoupler | "" }; result: { text: string } };
  targetBuiltins: { params: undefined; result: TargetInfoRecord[] };
  measurementExportRew: { params: { text: string; output: number }; result: { text: string } };

  hubMeasureDocPut: { params: { text: string }; result: { doc: MeasureDocInfoRecord; created: boolean } };

  measureSimSet: { params: { on: boolean; seed?: number }; result: MeasureSimInfoRecord };
  measureSimInfo: { params: undefined; result: MeasureSimInfoRecord };
}
export type MeasureMethod = keyof MeasureMethods;

/** What a measurement screen needs of a core: a session's `CoreClient` is one. */
export interface MeasureCaller {
  call(method: never, params: never): Promise<unknown>;
}

/** A measurement call, typed (the console's `CoreMethods` does not list them: they are this file's). */
export function measureCall<M extends MeasureMethod>(core: MeasureCaller, method: M, params: MeasureMethods[M]["params"]): Promise<MeasureMethods[M]["result"]> {
  return core.call(method as never, params as never) as Promise<MeasureMethods[M]["result"]>;
}

// ---- the Worker's part ----

/** The Worker's own methods (they never reach the core). */
export const CAPTURE_OPEN = "measureCaptureOpen";
export const CAPTURE_CLOSE = "measureCaptureClose";

/** `measureCaptureOpen`'s answer: the port for the capture worklet, and where the worklet's script is. */
export interface CaptureOpenRecord {
  port: MessagePort;
  workletUrl: string;
}

/** The events the Worker pushes about a run. */
export type MeasureEvent = { event: "measure"; state: MeasureStateRecord } | { event: "measureLive"; state: MeasureLiveStateRecord };

export function isMeasureEvent(e: unknown): e is MeasureEvent {
  const x = e as { event?: unknown; state?: unknown } | null;
  return !!x && (x.event === "measure" || x.event === "measureLive") && typeof x.state === "object" && x.state !== null;
}

/** How often the Worker turns a running session, ms (the core asks for about 50 ms). */
export const TICK_MS = 50;

/** The calls after which the Worker turns the session, and with which tick. */
const TICKS: Record<string, "measureTick" | "measureLiveTick"> = {
  measureStart: "measureTick",
  measureNextPosition: "measureTick",
  measureLiveStart: "measureLiveTick",
};
/** The calls that end the session the Worker turns (one last state is pushed). */
const ENDS = new Set(["measureStop", "measureLiveStop", "measureClear", "close", "open", "documentOpen"]);

// A browser's timer functions refuse another `this` ("Illegal invocation"): called unbound.
function setTimer(env: { setTimeout: typeof setTimeout }, fn: () => void): ReturnType<typeof setTimeout> {
  const st = env.setTimeout;
  return st(fn, TICK_MS);
}
function clearTimer(env: { clearTimeout: typeof clearTimeout }, t: ReturnType<typeof setTimeout> | undefined): void {
  const ct = env.clearTimeout;
  ct(t);
}

/** The Worker's part of a measurement: the capture port and the ticks (worker.ts). */
export class MeasureWorkerRoute {
  private unbind: (() => void) | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private tick: "measureTick" | "measureLiveTick" | null = null;
  /** Blocks pushed and refused since the port opened (the core's refusals are its answers). */
  pushed = 0;
  refused = 0;

  constructor(
    private readonly call: (method: string, params: unknown) => Promise<unknown>,
    private readonly post: (e: MeasureEvent) => void,
    private readonly env: {
      MessageChannel: new () => { port1: MessagePort; port2: MessagePort };
      workletUrl: string;
      setTimeout: typeof setTimeout;
      clearTimeout: typeof clearTimeout;
    },
  ) {}

  /** The method is the Worker's own: `answer` it here, never in the core. */
  handles(method: string): boolean {
    return method === CAPTURE_OPEN || method === CAPTURE_CLOSE;
  }

  /** `measureCaptureOpen`: a new port (any earlier one is closed); `measureCaptureClose`: closed. */
  answer(method: string): { value: CaptureOpenRecord | null; transfer: MessagePort[] } {
    this.closeCapture();
    if (method !== CAPTURE_OPEN) return { value: null, transfer: [] };
    const ch = new this.env.MessageChannel();
    this.pushed = 0;
    this.refused = 0;
    const bound = bindCapturePort(ch.port1, {
      push: async (b) => {
        this.pushed++;
        await this.call("measureCapturePush", measurePushParams(b));
      },
      end: () => {
        this.unbind = null;
      },
      issue: (i) => {
        if (i.kind === "push_failed") this.refused++;
        // A gap or a repeat is the core's to judge (S5): it sees the frame indices itself.
      },
    });
    this.unbind = () => bound.unbind();
    return { value: { port: ch.port2, workletUrl: this.env.workletUrl }, transfer: [ch.port2] };
  }

  closeCapture(): void {
    this.unbind?.();
    this.unbind = null;
  }

  /** After a core call answered: start, keep or end the ticks. */
  after(method: string, ok: boolean, value: unknown): void {
    if (ok && method in TICKS) {
      const active = method === "measureLiveStart" ? (value as MeasureLiveStateRecord)?.active : (value as MeasureStateRecord)?.active;
      if (active !== false) this.start(TICKS[method]!);
    } else if (ENDS.has(method)) {
      this.stop();
      if (method === "close" || method === "open" || method === "documentOpen") this.closeCapture();
    }
  }

  get ticking(): boolean {
    return this.tick !== null;
  }

  private start(tick: "measureTick" | "measureLiveTick"): void {
    clearTimer(this.env, this.timer);
    this.tick = tick;
    this.timer = setTimer(this.env, () => void this.turn());
  }

  private stop(): void {
    clearTimer(this.env, this.timer);
    this.timer = undefined;
    this.tick = null;
  }

  private async turn(): Promise<void> {
    const tick = this.tick;
    if (!tick) return;
    let active = false;
    try {
      const st = await this.call(tick, undefined);
      if (tick === "measureLiveTick") {
        const s = st as MeasureLiveStateRecord;
        this.post({ event: "measureLive", state: s });
        active = !!s.active;
      } else {
        const s = st as MeasureStateRecord;
        this.post({ event: "measure", state: s });
        active = !!s.active;
      }
    } catch {
      // A failed tick (a lost session) ends the turns; the session's own state says why.
      active = false;
    }
    if (this.tick !== tick) return;
    if (active) this.timer = setTimer(this.env, () => void this.turn());
    else this.stop();
  }
}
