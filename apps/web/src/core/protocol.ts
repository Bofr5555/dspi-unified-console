// The contract between the page and a core Worker (WP WB5, widened by WB3).
//
// One Worker holds one device session. A fleet page opens as many Workers as it
// has devices open, so nothing here is global to the page and no device is "the"
// device. Inside a Worker every call runs alone, in arrival order: the core runs
// under JSPI there (docs/core-merge/WEB-STUDY.md section 2), and a second call
// entering it while one is suspended corrupts its stack. The queue in serial.ts
// keeps that rule; the binding (crates/dspi-wasm) refuses such a call as well.
//
// The shapes are the core's own records as `dspi-wasm` answers them
// (crates/dspi-wasm/src/calls.rs): `DspState` is `dspi_core::DspState` as its
// serde writes it (camelCase, the C ABI's JSON replies carry the same), less the
// raw image `bulkRaw`. Records the core has no serde for (`SystemStatus`, the
// analyser's) keep the core's field names, in camelCase. Error codes are the C
// ABI's (`read_only`).

/** Where a session's device is. A fleet server may add its own kinds later. */
export type DeviceTarget =
  /** The core's own demo device; `readOnly` opens it as older firmware, which the core only reads. */
  | { kind: "demo"; readOnly?: boolean; /** -- WP SW1 -- the demo as a unit receiving the audio link */ audioLink?: boolean }
  /**
   * A DSPi on USB (WebUSB, WB4a): the serial the page's picker or
   * `getDevices()` gave. The Worker claims that device and completes the target
   * for the core (`device`, `vendorId`); without a serial the core answers
   * `invalid`. The Worker's own calls `usbPush`, `usbGone` and `usbIds`
   * (crates/dspi-wasm/src/usb.rs) are not the page's.
   */
  | { kind: "usb"; serial?: string }
  /**
   * A DSPi behind a hub (WB4b): its serial as `deviceId`, from `linkDevices` (Link's
   * choose_device step) or from a fleet backend's own list (`hubDevices`). The
   * session must have connected first (`linkConnect`, and `linkPair` if the hub
   * wanted a PIN); the core then binds that device and reads it.
   */
  | { kind: "hub"; url: string; deviceId: string };

// -- WP WB4b --
/** The hub's hello (`dspi_core::LinkHello`), after `linkConnect`. */
export interface LinkHelloRecord {
  /** `hid`: stable per hub installation. Tokens belong to it. */
  hubId: string;
  hubName: string;
  /** `console` or `bridge`. */
  hubKind: string;
  /** `none` or `pin`. */
  auth: string;
  /** `window` (a PIN shown on the hub) or `fixed` (the hub's own PIN). */
  pinSource: string;
  /** Ask for the PIN and call `linkPair`. */
  needsPairing: boolean;
  /** The hub refused the token passed in: forget it. Only this says a token is bad. */
  tokenRefused: boolean;
  caps: string[];
  maxInflight: number;
}

/** A DSPi a hub shares (`dspi_core::LinkDevice`). */
export interface LinkDeviceRecord {
  handle: number;
  serial: string;
  name: string;
  firmware: string;
  platform: string;
  /** `online`, `offline`, `updating`. */
  state: string;
}

/** Where a Link session stands (`dspi_core::LinkStatus`). */
export interface LinkStatusRecord {
  phase: LinkPhase;
  pinSource: string;
  role: string;
  /** The hub's close code, -1 for none. */
  closeCode: number;
  /** Why it ended or was lost, in the core's words; "" while all is well. */
  closeReason: string;
  reconnectAttempt: number;
  reconnectInMs: number;
  tokenRefused: boolean;
}

/** Who the device is. Composed by the binding from the core's mirror. */
export interface DeviceRecord {
  name: string;
  /** Empty when the transport does not say. */
  serial: string;
  firmware: string;
  platform: string;
  /** The core's demo device: `demo` calls apply. */
  demo: boolean;
  /** The core reads this firmware but will not write it (another wire format). */
  readOnly: boolean;
  /** Why, in the core's wording; empty when it is not. */
  readOnlyReason: string;
}

/**
 * `dspi_core::DspState` (crates/dspi-core/src/records.rs): the core's mirror of
 * the device. The fields the console uses so far are typed; the record has many
 * more, all present, read them by their camelCase names.
 */
export interface DspState {
  platformId: number;
  numChannels: number;
  numInputChannels: number;
  numOutputChannels: number;
  formatVersion: number;
  fwMajor: number;
  fwMinor: number;
  fwPatch: number;
  fwBeta: number;
  preampDb: number;
  bypass: boolean;
  masterVolumeDb: number;
  userVolumeDb: number;
  userMute: boolean;
  loudnessEnabled: boolean;
  outputGainDb: number[];
  outputMuted: boolean[];
  /** Per wire channel; the first `numChannels` are live. */
  channelNames: string[];
  presetNames: string[];
  activePresetSlot: number;
  // -- WP WB6a -- the main window's
  /** 0 independent, 1 saved with the preset (the registry's `vol.master.mode`). */
  masterVolumeMode: number;
  /** Bit per slot: the slot holds a preset. */
  presetOccupied: number;
  /** 0 the default slot at power on, 1 the last active one. */
  presetStartupMode: number;
  presetDefaultSlot: number;
  /** Per input (wire channels 0..numInputChannels). */
  inputPreampDb: number[];
  /** Per output (wire channel numInputChannels + output). */
  outputEnabled: boolean[];
  /** 0 USB, 1 S/PDIF, 2 I2S, 3 ADAT, 4..6 S/PDIF 2..4 (the registry's `in.source`). */
  inputSource: number;
  /** The configured I2S input channel count (2, 4, 6, 8). */
  i2sInputChannels: number;
  // -- WP WB6b -- the EQ's
  /** PEQ bands, `[wire channel][band]`; the first `maxBands` of each are the device's. */
  filters: FilterParams[][];
  /** Crossover bands of each wire channel (outputs'), drawn with its curve. */
  xover: FilterParams[][];
  /** PEQ bands per channel the firmware takes. */
  maxBands: number;
  // -- WP WB6c -- outputs, matrix, limiter
  /** Per output, ms (fractions kept to 0.0001 ms). */
  outputDelayMs: number[];
  /** `[input][output]`: the crosspoint routes. */
  matrixRouting: boolean[][];
  /** `[input][output]`, dB. */
  matrixGain: number[][];
  /** `[input][output]`: polarity inverted. */
  matrixInvert: boolean[][];
  /** Per output (wire format 32 on). */
  limiterEnabled: boolean[];
  /** Per output: 0 unlinked, 1..4 a link group. */
  limiterLinkGroup: number[];
  limiterThresholdDb: number[];
  limiterReleaseMs: number[];
  [field: string]: unknown;
}

// -- WP WB6b --
/** One filter band (`dspi_core::FilterParams`). */
export interface FilterParams {
  /** The firmware's type: 0 off, 1..13 PEQ, 32..63 crossover. */
  filterType: number;
  bypass: boolean;
  /** Centre or corner, Hz. */
  freq: number;
  q: number;
  /** dB; for the Linkwitz Transform its target frequency. */
  gain: number;
  /** The Linkwitz Transform's target Q. */
  qp: number;
}

// -- WP WB6c --
/** One matrix crosspoint. */
export interface Crosspoint {
  input: number;
  output: number;
  enabled: boolean;
  /** dB, kept to 0.1 dB. */
  gain: number;
  invert: boolean;
}

/** One band of the edited channel, as `curves` answers it (crates/dspi-wasm/src/curves.rs). */
export interface BandCurve {
  /** The band alone, its bypass ignored, on the grid; empty for an off band. */
  curve: number[];
  /** Its level at its own frequency, dB. */
  at: number;
  /** For a type with gain: its level there per dB of gain (1 for a bell, about ½ for a shelf). */
  unit: number;
  usesGain: boolean;
  usesQ: boolean;
}

/** What the page shows of a device. Read again after every write. */
export interface DeviceSnapshot {
  device: DeviceRecord;
  state: DspState;
  // -- BUG1 -- the core's, worked out once per snapshot; absent from a host or stub without them
  /** Each output's label, in output order (`DspiCore::output_labels`). */
  outputLabels?: OutputLabel[];
  /** The dashboard's output cards (`DspiCore::output_cards`). */
  outputCards?: OutputCard[];
}

/**
 * -- BUG1 -- What an output is called: the name the user set, else "Out 1 · R · Low" from the
 * routing and the filters while the device still has the slot's default name.
 */
export interface OutputLabel {
  output: number;
  label: string;
  /** A phone row's form beside its OUTn button: the user's name, else "R · Low". */
  shortLabel: string;
  /** The firmware's name for the slot, "I2S 1 L" or "PDM": for a tooltip. */
  slotName: string;
  userNamed: boolean;
  /** "L", "R", "L+R", an input's name, "Mix" or "None". */
  channel: string;
  /** "Low", "High", "Band" or "Full". */
  band: string;
}

/** -- BUG1 -- A dashboard card: two outputs side by side ("Woofers L" | "Woofers R"), or one. */
export interface OutputCard {
  left: number;
  right: number | null;
  title: string;
  leftName: string;
  rightName: string;
  /** A phone's narrow columns: "Woofers L" | "R", else the short labels. */
  leftShortName: string;
  rightShortName: string;
}

/**
 * A parameter value, as `dspi_proto::value::Value` (what `Session::write`
 * takes), tagged by kind.
 */
export type ParamValue =
  | { bool: boolean }
  | { float: number }
  | { int: number }
  | { choice: number }
  | { mask: number }
  | { text: string }
  | { bytes: number[] }
  | "trigger";

/** What the registry (`dspi_proto::registry`) says of a parameter path. */
export interface ParamInfo {
  path: string;
  label: string;
  plain: string;
  writable: boolean;
  readable: boolean;
  /** How many `indices` a write takes (an output, a channel and band…). */
  indices: number;
  /** The binding writes it in this version. */
  routed: boolean;
  kind: "bool" | "trigger" | "float" | "int" | "choice" | "mask" | "text" | "bytes" | "status";
  unit?: string;
  min?: number;
  max?: number;
  choices?: { value: number; label: string }[];
}

/** `dspi_core::SystemStatus`: the meters. */
export interface SystemStatus {
  /** Peak per wire channel, 0..1. */
  peaks: number[];
  cpu0: number;
  cpu1: number;
  clipFlags: number;
  activeInputChannels: number;
  numChannels: number;
  /** Not a fresh reading (Link): grey the meters. */
  stale: boolean;
  ageMs: number;
  // -- WP P0M -- the core's ballistics
  /** Held peak per wire channel, 0..1: the hold marker. */
  peakHold: number[];
  /** Highest peak per wire channel since the connection opened or `resetMeterMax`. */
  peakMax: number[];
  /** The clip hold, one bit per wire channel: draw the clip marker from this. */
  clipHold: number;
}

// -- WP P0M --
/** `dspi_core::MeterBallistics`. */
export interface MeterBallistics {
  holdMs: number;
  decayPerS: number;
  clipHoldMs: number;
}

/** One line of the event log (`dspi_session::events::EventRecord`, the file format). */
export interface EventRecord {
  v: number;
  id: number;
  /** ms since the epoch. */
  t: number;
  kind: "notification" | "connected" | "disconnected" | "clip" | "temperature" | "counter" | "reset" | "firmware" | "import" | "recall" | "audio_link" | "unknown";
  serial?: string;
  text: string;
  source?: string;
  session?: number;
  own?: boolean;
  channel?: number;
  value?: number;
  fw?: string;
  uptime_s?: number;
  lines?: string[];
  count?: number;
}

/** `dspi_core::RtaRequest`, the fields the page sets. */
export interface RtaRequest {
  /** 0 input tap, 1 output tap. */
  tap: number;
  /** Channels within the tap. */
  mask: number;
  wantsBins?: boolean;
  fftOrder?: number;
  avgMs?: number;
  peakDecayDbS?: number;
}

/** The analyser: `rta_get_caps` and `rta_get_snapshot`. */
export interface RtaView {
  caps: {
    supported: boolean;
    inputChannels: number;
    outputChannels: number;
    orderMin: number;
    orderMax: number;
    orderDefault: number;
    bassBands: number;
  };
  snapshot: {
    version: number;
    tap: number;
    count: number;
    /** The analysed channels, `count` of them. */
    frames: { channel: number; nBands: number; silent: boolean; avgDb: number[]; peakDb: number[] }[];
    populated: number;
    running: boolean;
    framesPerS: number;
    sampleRate: number;
    fftOrder: number;
    configRejected: boolean;
    refreshIntervalS: number;
  };
}

/** The core's Link phases (`dspi_core::LinkPhase`, keys from `link_phase_key`). */
export type LinkPhase =
  | "idle"
  | "needs_pairing"
  | "no_device"
  | "choose_device"
  | "bound"
  | "reconnecting"
  | "closed"
  | "refused";

/** Something the core reports without being asked. */
export type CoreEvent =
  /** The device changed under us (a notification): the new snapshot. */
  | { event: "state"; snapshot: DeviceSnapshot; flags: number }
  | { event: "link"; phase: LinkPhase; pinSource?: string; reason?: string }
  /** The connection is gone (unplugged, the core stopped). */
  | { event: "lost"; reason: string };

/** Every call a core Worker answers. `params` and the result per method. */
export interface CoreMethods extends AudioLinkMethods {
  open: { params: DeviceTarget; result: DeviceSnapshot };
  state: { params: undefined; result: DeviceSnapshot };
  /**
   * A parameter by registry path (`vol.user`, `out.gain` with `indices: [1]`),
   * as `Session::write(path, indices, value)`: the result is the value the
   * device kept, read back.
   */
  write: { params: { path: string; indices?: number[]; value: ParamValue }; result: ParamValue };
  describe: { params: { path: string }; result: ParamInfo };
  // -- WP CG36 --
  /**
   * Whether enabling `output` (0-based) would collide on Core 1, as the device
   * answers it, with the core's reason: the words `write` of `out.enable`
   * refuses such an enable with.
   */
  core1Conflict: { params: { output: number }; result: { conflict: boolean; reason: string | null } };
  meters: { params: undefined; result: SystemStatus };
  // -- WP P0M -- meter ballistics and the event log (in memory on the web)
  meterBallistics: { params: undefined; result: MeterBallistics };
  setMeterBallistics: { params: MeterBallistics; result: MeterBallistics };
  resetMeterMax: { params: undefined; result: null };
  /** `serial` absent for the open device; `afterId` 0 (default) for the newest `limit` (default 500). */
  eventLogRead: { params: { serial?: string; afterId?: number; limit?: number }; result: { records: EventRecord[]; lastId: number; skipped: number } };
  /** The whole event log as JSON lines. */
  eventLogExport: { params: { serial?: string }; result: string };
  eventLogDevices: { params: undefined; result: string[] };
  rtaStart: { params: RtaRequest; result: null };
  rtaStop: { params: undefined; result: null };
  rta: { params: undefined; result: RtaView };
  /** The core's demo simulations; refused for a real device. */
  demo: { params: { action: "renameElsewhere"; name: string }; result: null };
  close: { params: undefined; result: null };
  // -- WP WB4b -- Link: the choose_device flow, then `open` of `{kind: "hub"}`
  /**
   * A session with the hub at `url` (`ws://host:port`). `token` and `hubId`: the
   * token kept for that hub, which goes out only once the hub has said it is
   * that one. Fails in the core's words (`link`).
   */
  linkConnect: { params: { url: string; token?: string; hubId?: string; clientName?: string }; result: LinkHelloRecord };
  /** The PIN the user typed: the token to keep for this hub. A wrong one is `wrong_pin`. */
  linkPair: { params: { pin: string }; result: { token: string } };
  linkDevices: { params: undefined; result: LinkDeviceRecord[] };
  linkStatus: { params: undefined; result: LinkStatusRecord };
  // -- WP WB6a -- the preset flows `write` does not cover (crates/dspi-wasm/src/presets.rs)
  /** The live state against the active preset: the core's sentences (CG7), empty when it matches. */
  presetChanges: { params: undefined; result: { dirty: boolean; changes: string[] } };
  /** Copy to: the live state into `dest`, `source` stays active. Writes flash twice. */
  presetCopy: { params: { source: number; dest: number }; result: null };
  /** Revert to Saved: reload the active preset. */
  presetRevert: { params: undefined; result: null };
  /** Factory Reset, once the user confirmed it: the defaults; stored presets are kept. */
  factoryReset: { params: undefined; result: null };
  // -- WP WB6b -- the EQ graph's curves: the core's maths, no device I/O
  /** The frequency (Hz) of each curve point: 201, log-spaced 10 Hz..20 kHz. */
  curveGrid: { params: undefined; result: { freqs: number[] } };
  /**
   * Each chain's combined magnitude (dB) on the grid, and each band's own
   * curve and dot level. Bands need not be the device's (a drag in progress).
   */
  curves: { params: { chains?: FilterParams[][]; bands?: FilterParams[] }; result: { chains: number[][]; bands: BandCurve[] } };
  // -- WP CG34 -- the filter types' names, the core's one table (the Console's band-list names)
  /** Every type the firmware defines, PEQ 0..13 then crossovers 32..63: `{code: 4, name: "High Cut (12dB)"}`. */
  filterTypeNames: { params: undefined; result: { code: number; name: string }[] };
  // -- WP WB6c -- what `write` does not cover for the outputs (crates/dspi-wasm/src/outputs.rs)
  /**
   * A crossover band's type (0 off, 32..63), and with it its corner: the
   * core's `set_crossover` (the registry's `eq.type` takes PEQ types only).
   * `channel` is the output's wire channel, `band` 0..3. Frequency and bypass
   * alone are `write`s of `eq.freq` / `eq.bypass` on band 20 + `band`.
   */
  crossoverType: { params: { channel: number; band: number; filterType: number; freq?: number }; result: FilterParams };
  /** One matrix crosspoint: `write` of `mix` with the packet built in the binding; the fields kept. */
  crosspoint: { params: Crosspoint; result: Crosspoint };
  /** The longest delay this device takes (21 ms on RP2040, 42 on RP2350). */
  outputLimits: { params: undefined; result: { maxDelayMs: number } };
  // -- WP SW3 -- the hub's audio-link system model (crates/dspi-wasm/src/system.rs)
  /**
   * One of the hub's `system.*` requests on this session's connection
   * (docs/integrator/audio-link.md), judged by the hub against this session's
   * role: the hub's answer without `t` and `id`. Refusals: `denied` (the
   * message names what), `rate_limited`, `invalid`, `not_open` (no hub), `link`.
   */
  hubSystem: { params: { t: string; [field: string]: unknown }; result: Record<string, unknown> };
  /** The role the hub gave this session: `viewer`, `control`, `admin`; "" with no hub. */
  hubRole: { params: undefined; result: { role: string } };
  /** Tests only: make the stub core lose its link. The real core answers `unknown_method`. */
  simulate: { params: { what: "link-lost" | "link-closed" }; result: null };
}

// -- WP SW1 -- the audio link (crates/dspi-wasm/src/audio_link.rs; not DSPi Link)
/** `EXT_LINK_INFO` as the core keeps it (extension_channel_spec.md 2.8.1). */
export interface AudioLinkInfo {
  role: string;
  roleCode: number;
  protocolVersion: number;
  classMask: number;
  maxLinks: number;
  maxChannels: number;
  slotsPerUnit: number;
  rxPin: number;
  txPin: number;
  featureBits: number;
  dLocalMin: number;
  dLocalMax: number;
  linkInput: boolean;
  stateEvents: boolean;
  applyEvents: boolean;
}

/** `EXT_LINK_STATUS` (2.8.2). */
export interface AudioLinkStatus {
  state: "off" | "searching" | "locked" | "holdover" | "unknown";
  stateCode: number;
  linkClass: number;
  rateCode: number;
  channels: number;
  hops: number;
  losPin: number;
  senderId: number;
  flags: number;
  frame: number;
  lockCount: number;
  lossCount: number;
  crc8Errors: number;
  crc16Errors: number;
  bitSlips: number;
  superframeErrors: number;
  vcxoPpm: number;
  phaseErrorFrames: number;
  fillFrames: number;
}

/** `EXT_LINK_PHASE` (2.8.5). */
export interface AudioLinkPhase {
  mode: number;
  dLocal: number;
  hopsOffset: number;
  flags: number;
  state: number;
}

/** `EXT_APPLY_AT_FRAME` IN (2.8.7). */
export interface ApplyAtFrameStatus {
  pending: number;
  lastOp: string;
  lastOpCode: number;
  lastResult: string;
  lastResultCode: number;
  lastFrame: number;
}

/** The audio link's methods: `unsupported` on a unit without the link. */
export interface AudioLinkMethods {
  /** No traffic; null without the link. */
  audioLinkInfo: { params: undefined; result: AudioLinkInfo | null };
  /** Re-read: the delay limits follow the output latency option. */
  fetchAudioLinkInfo: { params: undefined; result: AudioLinkInfo };
  audioLinkStatus: { params: undefined; result: AudioLinkStatus };
  /** 8 values, one per matrix input: a link channel 0..63, or 255 for silence. */
  audioLinkSlots: { params: undefined; result: number[] };
  setAudioLinkSlots: { params: { slots: number[] }; result: null };
  audioLinkPhase: { params: undefined; result: AudioLinkPhase };
  /** `mode` 0 fill servo, 1 phase lock; `dLocal` in link frames; `hopsOffset` -64..64. */
  setAudioLinkPhase: { params: { mode: number; dLocal: number; hopsOffset?: number }; result: null };
  audioLinkFrameCounter: { params: undefined; result: { frame: number; timeUs: number; outputFrames: number } };
  /** `op` 0 cancel all, 1 mute, 2 unmute, 3 preset load, 4 master volume, 5 output delay, 6 rate, 7 click, 8 sync pulse. */
  applyAtFrame: { params: { frame: number; op: number; arg?: number; value?: number }; result: null };
  applyAtFrameStatus: { params: undefined; result: ApplyAtFrameStatus };
  resetAudioLinkStats: { params: undefined; result: null };
}

export type Method = keyof CoreMethods;

export interface Request<M extends Method = Method> {
  id: number;
  method: M;
  params: CoreMethods[M]["params"];
}

export interface CoreError {
  /**
   * Stable: `refused`, `read_only`, `out_of_range`, `invalid`, `unknown_param`,
   * `unsupported`, `not_open`, `transfer`, `locked`, `denied`, `link`, `lost`,
   * `busy` (another call is running; this one did not run), `dead` (the core
   * stopped after an internal error), `unknown_method`, `internal`; Link (WB4b):
   * `wrong_pin`, `not_found` (the hub does not offer that DSPi now).
   */
  code: string;
  message: string;
}

export type Reply =
  | { id: number; ok: true; value: unknown }
  | { id: number; ok: false; error: CoreError };

/** A message from a Worker: a reply to a call, or an event. */
export type FromWorker = { reply: Reply } | { push: CoreEvent };

/** What runs inside the Worker: `dspi-wasm` (wasm.ts), or the tests' stub. */
export interface CoreImpl {
  call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]>;
  /** The core reports events through this; set once by the Worker. */
  onEvent(push: (e: CoreEvent) => void): void;
}

export class CoreCallError extends Error {
  constructor(readonly code: string, message: string) {
    super(message);
    this.name = "CoreCallError";
  }
}

/** A peak (0..1) in dBFS, floored at -90 for a meter. */
export function peakDb(peak: number): number {
  return peak > 0 ? Math.max(-90, 20 * Math.log10(peak)) : -90;
}
