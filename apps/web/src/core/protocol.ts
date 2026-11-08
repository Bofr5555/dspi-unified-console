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
   * A DSPi behind a hub. `deviceId` "" lets the console ask the hub which ones
   * it has (Link's choose_device); a fleet backend may name one itself
   * (owner, 2026-11-08; WB4b).
   */
  | { kind: "hub"; url: string; deviceId: string };

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
  [field: string]: unknown;
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
  | { event: "link"; phase: LinkPhase; pinSource?: string }
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
   * stopped after an internal error), `unknown_method`, `internal`.
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
