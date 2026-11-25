// WP WB6j: the calls of the analyser's view, the signal generator, Stats for nerbs and the
// Interrupt Monitor (`crates/dspi-wasm/src/tools.rs`, the other side).

/** `analyserView`: the analyser's caps with its band centres, the picture, and who drives it. */
export interface AnalyserView {
  caps: {
    supported: boolean;
    inputChannels: number;
    outputChannels: number;
    orderMin: number;
    orderMax: number;
    orderDefault: number;
    bassBands: number;
    maxBands: number;
    dynamicRangeDb: number;
    /** Each band's centre, Hz. */
    centres: number[];
    /** -- WP WEB6 -- the bass bands' usable range, dB (0: not said). */
    bassDynamicRangeDb?: number;
  };
  snapshot: {
    version: number;
    tap: number;
    count: number;
    frames: { channel: number; nBands: number; silent: boolean; avgDb: number[]; peakDb: number[] }[];
    /** One bit per band with data, as a decimal string (64 bits). */
    populated: string;
    running: boolean;
    liveMask: number;
    framesPerS: number;
    sampleRate: number;
    fftOrder: number;
    firstBand: number;
    configRejected: boolean;
    // -- WP WEB6 -- the telemetry of Qt's status line (absent from an older core)
    liveCount?: number;
    refreshIntervalS?: number;
    lastFrameUs?: number;
    busyUsPerS?: number;
    bassBusyUsPerS?: number;
  };
  /** The device's one analyser: the last choice made wins; `following` while another console's runs it. */
  ownership: { following: boolean; tap: number; mask: number; fftOrder: number };
}

export interface SiggenParamInfo {
  label: string;
  /** 0 unused, 1 Hz, 2 ms, 3 cycles, 4 count, 5 ratio, 6 a pattern choice. */
  semantic: number;
  unit: string;
  min: number;
  max: number;
  def: number;
}

export interface SiggenTypeInfo {
  id: number;
  tile: string;
  name: string;
  blurb: string;
  /** 0 continuous, 1 sweep, 2 pattern. */
  timing: number;
  params: SiggenParamInfo[];
  /** Whether this device plays it. */
  offered: boolean;
}

export interface SiggenCapsInfo {
  supported: boolean;
  outputChannels: number;
  multitoneMax: number;
  validMask: number;
  types: SiggenTypeInfo[];
}

export interface SiggenConfigRecord {
  signalType: number;
  channelMask: number;
  invertMask: number;
  /** `SIGGEN_FLAG_*`: 1 raw (past the EQ), 2 decorrelated, 4 walk the outputs. */
  flags: number;
  levelDb: number;
  durationMs: number;
  repeat: number;
  gapMs: number;
  p: number[];
}

export interface SiggenStatusRecord {
  /** 0 idle, 1 fade in, 2 run, 3 gap, 4 fade out. */
  state: number;
  signalType: number;
  /** The walked output, 0xFF when not walking. */
  activeChannel: number;
  elapsedMs: number;
  cyclesDone: number;
  stopReason: number;
  currentFreq: number;
}

export interface RingHealthRecord {
  present: boolean;
  targetFrames?: number;
  targetUs?: number;
  batchFrames?: number;
  underrunEpisodes?: number;
  refusedBlocks?: number;
  lowestFill?: number | null;
  worstInputGapUs?: number;
  worstUsbGapUs?: number;
  overBudgetSteps?: number;
}

/** `statsRead`: a section is null when the device did not answer it. */
export interface StatsRead {
  counters: Record<string, number | null>;
  starvation: (number | null)[];
  ring: RingHealthRecord | null;
  spdifRx: {
    state: number;
    inputSource: number;
    lockCount: number;
    lossCount: number;
    sampleRate: number;
    parityErrors: number;
    fifoFillPct: number;
    libState: number;
    stableCallbacks: number;
    lostCallbacks: number;
    pin: number | null;
    channelStatus: number[] | null;
  } | null;
  lg: { enabled: boolean; present: boolean; volume: number; muted: boolean } | null;
  adat: { enabled: boolean; active: boolean; pin: number; rateOk: boolean; resyncCount: number; slipCount: number } | null;
  i2s: { state: number; clockMode: number; detectedRate: number; measuredHz: number; lockCount: number; lossCount: number } | null;
  clockDiag: { name: string; value: string }[] | null;
  usbErrors: { total: number; crc: number; bitstuff: number; rxOverflow: number; rxTimeout: number; dataSeq: number } | null;
}

export interface OutputFillRecord {
  fillPct: number;
  minPct: number;
  maxPct: number;
  band: "good" | "warn" | "bad";
}

/** `statsTick`: the fast poll. */
export interface StatsTickRecord {
  buffers: {
    numSpdif: number;
    pdmActive: boolean;
    streaming: boolean;
    sequence: number;
    spdif: { free: number; prepared: number; playing: number; fillPct: number; minPct: number; maxPct: number }[];
    pdm: { dmaFillPct: number; dmaMinPct: number; dmaMaxPct: number; ringFillPct: number; ringMinPct: number; ringMaxPct: number };
  };
  fills: OutputFillRecord[] | null;
}

export interface MonitorEntryRecord {
  id: number;
  timeMs: number;
  text: string;
}

export interface ToolsMethods {
  analyserView: { params: undefined; result: AnalyserView };
  rtaTakeControl: { params: undefined; result: null };
  siggenCaps: { params: undefined; result: SiggenCapsInfo };
  siggenConfig: { params: undefined; result: SiggenConfigRecord };
  siggenStatus: { params: undefined; result: SiggenStatusRecord };
  siggenSet: { params: { config: SiggenConfigRecord }; result: null };
  siggenControl: { params: { action: "start" | "stop" | "stopNow" }; result: null };
  /** Identify: channel-ID blips on `output` (0-based) alone, -12 dBFS, two passes. */
  siggenIdentify: { params: { output: number }; result: null };
  statsRead: { params: undefined; result: StatsRead };
  statsTick: { params: undefined; result: StatsTickRecord };
  statsResetWatermarks: { params: undefined; result: null };
  monitorRead: { params: { afterId: number }; result: { lastId: number; entries: MonitorEntryRecord[] } };
  monitorClear: { params: undefined; result: null };
}
