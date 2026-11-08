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
  | { kind: "demo"; readOnly?: boolean }
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
export interface CoreMethods {
  open: { params: DeviceTarget; result: DeviceSnapshot };
  state: { params: undefined; result: DeviceSnapshot };
  /**
   * A parameter by registry path (`vol.user`, `out.gain` with `indices: [1]`),
   * as `Session::write(path, indices, value)`: the result is the value the
   * device kept, read back.
   */
  write: { params: { path: string; indices?: number[]; value: ParamValue }; result: ParamValue };
  describe: { params: { path: string }; result: ParamInfo };
  meters: { params: undefined; result: SystemStatus };
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
  /** Tests only: make the stub core lose its link. The real core answers `unknown_method`. */
  simulate: { params: { what: "link-lost" | "link-closed" }; result: null };
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
