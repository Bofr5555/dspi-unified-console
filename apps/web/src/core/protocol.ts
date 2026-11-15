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
  | {
      kind: "demo";
      readOnly?: boolean;
      /** -- WP SW1 -- the demo as a unit receiving the audio link */
      alink?: boolean;
      /** -- WP AUSB1 -- the demo as an alink USB-direct unit */
      usbDirect?: boolean;
    }
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

// -- WP WB8 --
/** One change in the core's journal (`dspi_core::HistoryEntry`). */
export interface HistoryEntry {
  path: string;
  indices: number[];
  before: ParamValue | null;
  after: ParamValue;
  /** `user`, `undo`, `redo`, ... */
  source: string;
  atMs: number;
  undoable: boolean;
}

/** What `undo` or `redo` did (`dspi_core::HistoryMove`). */
export interface HistoryMove {
  entry: HistoryEntry | null;
  /** Stepped over, newest first: changes that cannot be undone. */
  skipped: HistoryEntry[];
  /** Why nothing moved: another writer changed the value since, named. */
  note: string | null;
}

/** `dspi_core::AlinkSceneRecall`. */
export interface SceneOutcome {
  scene: string;
  name: string;
  taken: boolean;
  frame: number;
  leadMs: number;
  onTime: boolean;
  heldFrame: number | null;
  /** One sentence to show: the outcome, or the refusal and what to do. */
  message: string;
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
  | { event: "lost"; reason: string }
  // -- WP HUBADMIN -- the hub's `limits.changed`, `limits.held`, `mode.changed` of a Link turn (`linkTick`'s `hubEvents`)
  | { event: "hubAdmin"; events: HubAdminEventRecord[] };

/** Every call a core Worker answers. `params` and the result per method. */
export interface CoreMethods extends AlinkMethods, DocumentMethods, InstallerMethods, HubAdminMethods /* -- WP HUBADMIN -- */ {
  open: { params: DeviceTarget; result: DeviceSnapshot };
  state: { params: undefined; result: DeviceSnapshot };
  /**
   * A parameter by registry path (`vol.user`, `out.gain` with `indices: [1]`),
   * as `Session::write(path, indices, value)`: the result is the value the
   * device kept, read back.
   */
  // -- WP P1WB2 -- `token`: a confirmation token for a gated path (`preset.save`, `preset.delete`, `dev.save`, `dev.save.io`)
  write: { params: { path: string; indices?: number[]; value: ParamValue; token?: string }; result: ParamValue };
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
  /** The core's demo simulations; refused for a real device. -- WP WB8 -- `clip`: the channels in `mask` clip. */
  demo: { params: { action: "renameElsewhere"; name: string } | { action: "clip"; mask: number }; result: null };
  // -- WP WB8 -- the change history (crates/dspi-wasm/src/history.rs, WP P0W)
  /** Step back; `entry` null when nothing moved, `note` names another writer whose change was left alone. */
  undo: { params: undefined; result: HistoryMove };
  redo: { params: undefined; result: HistoryMove };
  /** Both at once, whichever name is called. */
  canUndo: { params: undefined; result: { canUndo: boolean; canRedo: boolean } };
  /** The newest `n` changes, newest first. */
  history: { params: { n: number }; result: HistoryEntry[] };
  /** A continuous drag starts (`true`) or ends (`false`): its writes join one undo step. */
  setDragging: { params: { dragging: boolean }; result: null };
  /** A whole PEQ band (0..9) in one write (`set_filter`): a graph drag's ticks, one journal change each. The band kept. */
  filter: { params: { channel: number; band: number } & FilterParams; result: FilterParams };
  /** The diagnostics bundle (`uconsole diag`'s zip), for the viewer to save: crates/dspi-wasm/src/diag.rs. */
  diagnostics: { params: { app?: string; files?: { name: string; text: string }[] }; result: { name: string; size: number; base64: string } };
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
  // -- WP MERGEP1D -- the hub's installer lock of the bound DSPi (crates/dspi-wasm/src/modes.rs)
  /** The one lock state to show: `heldBy` `hub` (the hub's PIN), `console`, or "" for none. */
  installerLockRefresh: { params: undefined; result: LockStateRecord };
  // -- WP WB6a -- the preset flows `write` does not cover (crates/dspi-wasm/src/presets.rs)
  /** The live state against the active preset: the core's sentences (CG7), empty when it matches. */
  presetChanges: { params: undefined; result: { dirty: boolean; changes: string[] } };
  /** Copy to: the live state into `dest`, `source` stays active. Writes flash twice. */
  presetCopy: { params: { source: number; dest: number; token?: string }; result: null };
  /** Revert to Saved: reload the active preset. */
  presetRevert: { params: undefined; result: null };
  /** Factory Reset, once the user confirmed it: the defaults; stored presets are kept. */
  factoryReset: { params: undefined | { token?: string }; result: null };
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
   * message names what), `rate_limited`, `invalid`, `not_open` (no hub), `link`;
   * -- fixup sw3x -- `conflict`: `system.set` with a `revision` the hub has moved past.
   */
  hubSystem: { params: { t: string; [field: string]: unknown }; result: Record<string, unknown> };
  // -- WP WB8 --
  /**
   * What a recall sent with `hubSystem` (`system.scene.apply`) did, in the core's
   * words (the outcome every client shows): `report` the hub's answer, `model`
   * the installation, `heldFrame` the frame the sender was seen holding.
   */
  sceneOutcome: {
    params: { scene: string; report: unknown; model: unknown; heldFrame?: number | null; rateHz?: number };
    result: SceneOutcome;
  };
  // -- WP HWLK1F --
  /**
   * What a `system.align` sent with `hubSystem` did, in the core's words (the
   * outcome every client shows): what was written, what was skipped and why.
   */
  alignOutcome: { params: { report: unknown; model: unknown }; result: { applied: boolean; message: string } };
  /** The role the hub gave this session: `viewer`, `control`, `admin`; "" with no hub. */
  hubRole: { params: undefined; result: { role: string } };
  /** Tests only: make the stub core lose its link. The real core answers `unknown_method`. */
  simulate: { params: { what: "link-lost" | "link-closed" }; result: null };
  // -- WP WB6G -- the input sources' names (crates/dspi-wasm/src/inputs.rs), the core's cache, no traffic
  /** `names[s]`: source `s`'s name ("" unnamed); `supported` false on firmware without names. Renamed by `write` of `in.name`. */
  inputSourceNames: { params: undefined; result: { supported: boolean; names: string[] } };
}

// -- WP P1WB1 -- documents, snapshots, A/B, safe apply, offline files, limits status, notices
// (crates/dspi-wasm/src/{presets,snapshots,ab,guarded,offline,limits}.rs; the core's P1 calls)

/** An import block (`dspi_core::IMPORT_*`); with `IMPORT_BY_BLOCK` each bit says whether its block is applied. */
export const IMPORT_VOLUMES = 1;
export const IMPORT_HARDWARE = 1 << 1;
export const IMPORT_EQ = 1 << 2;
export const IMPORT_CROSSOVER = 1 << 3;
export const IMPORT_ROUTING = 1 << 4;
export const IMPORT_OUTPUTS = 1 << 5;
export const IMPORT_LIMITERS = 1 << 6;
export const IMPORT_EFFECTS = 1 << 7;
export const IMPORT_NAMES = 1 << 8;
export const IMPORT_ALL_BLOCKS = 0x1ff;
/** The options are per block (the bits above), not the two old switches. */
export const IMPORT_BY_BLOCK = 1 << 16;

/** A block's key in a diff's rows and counts. */
export type ImportBlock = "eq" | "crossover" | "routing" | "outputs" | "limiters" | "effects" | "volumes" | "hardware" | "names";

/** One changed value (`config_diff`'s rows): in the unsaved-changes list's words. */
export interface DiffRow {
  block: ImportBlock;
  section: string;
  item: string;
  before: string;
  after: string;
}

/** `configDiff`, `configPreview` and `abDiff`: what differs, before an import or between two documents. */
export interface ConfigDiff {
  ok: true;
  error: string;
  rows: DiffRow[];
  count: number;
  /** Rows per block. */
  blocks: Partial<Record<ImportBlock, number>>;
  /** The unsaved-changes list's sentences for the same change. */
  summary: string[];
  /** What reading the document reported: channels missing, values left alone or clamped. */
  notes: string[];
  /** The (second) document's `meta.kind`: `preset`, `snapshot`, `template`, `limits`. */
  kind: string;
}

/** `configImport`'s report (`DspiCore::config_import_selected`). */
export interface ImportReport {
  ok: true;
  error: string;
  /** The report, line by line. */
  lines: string[];
  /** False when anything was missing, skipped, left alone, shortened or out of range. */
  clean: boolean;
  /** The input-pair link mask to restore, null when the document has none. */
  links: number | null;
  /** I/O changed while the device keeps its output configuration apart from presets (save that to keep it). */
  hardwareWritten: boolean;
  leftAlone: string[];
  outOfRange: { path: string; value: number; applied: number }[];
  /** The block bits applied. */
  blocks?: number;
}

/** What a snapshot recall did (`dspi_core::SnapshotRecall`). */
export interface SnapshotRecallRecord {
  name: string;
  /** The snapshot's own image was written (false: a document without one, written as an import). */
  image: boolean;
  changes: string[];
  /** What was not written, and why. */
  notes: string[];
}

/** An A/B compare (`dspi_core::AbCompare`). */
export interface AbCompareRecord {
  active: boolean;
  /** The side on the device; after `abKeep`, the one kept; null with no compare. */
  side: "a" | "b" | null;
  aName: string;
  bName: string;
  /** What this call recalled, if it recalled anything. */
  recall: SnapshotRecallRecord | null;
}

/** Where a guarded apply stands (`dspi_core::GuardPhase`). */
export type GuardPhase = "idle" | "waiting" | "kept" | "reverted" | "revert_failed" | "not_written" | "lost";

/** A guarded apply, safe apply with rollback (`dspi_core::GuardedApply`). */
export interface GuardedApplyRecord {
  id: number;
  phase: GuardPhase;
  label: string;
  /** The hub guards it too: a vanished page is reverted by the hub. */
  hub: boolean;
  timeoutMs: number;
  remainingMs: number;
  /** "" while waiting; `confirmed`, `timeout`, `reverted`, `failed`, `connection_lost`, `taken_over`, `hub_stopping`. */
  reason: string;
  /** One plain sentence, "" while waiting. */
  message: string;
  changes: string[];
  notes: string[];
}

/** `documentOpen`'s `document`: what the offline session was opened from (`DspiCore::open_document`). */
export interface OpenedDocument {
  ok: true;
  fileName: string;
  name: string;
  kind: string;
  platform: string;
  inputs: number;
  outputs: number;
  notes: string[];
}

/** `documentInfo` (`DspiCore::offline_document`). */
export type DocumentInfo = { offline: false } | { offline: true; fileName: string; name: string; kind: string; platform: string };

/** A template this build ships (`dspi_core::template_list`). */
export interface TemplateRecord {
  id: string;
  name: string;
  builtin: boolean;
  platform: string;
  inputs: number;
  outputs: number;
  savedUtc: string;
  /** Its own status line (a shipped template awaiting approval says so), else "". */
  status: string;
}

/** `limitsStatus` (`dspi_core::LimitsStatus`). */
export interface LimitsStatusRecord {
  active: boolean;
  serial: string;
  /** `none`, `ok`, `fell_back`, `unreadable`. */
  state: string;
  noticeKey: string;
  detail: string;
  lastHeld: string[];
}

// -- WP P1EFIX --
/** One limits field's range (`dspi_core::LimitFieldRange`): the one table every limits editor uses. */
export interface LimitFieldRangeRecord {
  /** The profile's field: `maxGainDb`, ..., `masterVolumeMaxDb`. */
  key: string;
  /** A field of each output's limits. */
  perOutput: boolean;
  unit: string;
  min: number;
  max: number;
  step: number;
}

/** A notice from the core's table (`dspi_core::Notice`). */
export interface NoticeRecord {
  key: string;
  kind: "info" | "warning" | "error";
  title: string;
  reason: string;
  action: string;
  detail: string;
}

/** The P1 calls the web uses. */
export interface DocumentMethods {
  /** Two documents' difference, with no device. */
  configDiff: { params: { before: string; after: string }; result: ConfigDiff };
  /** What importing would change on the open device, nothing written. `eqChannels`: bit k wire channel k (every one when absent). */
  configPreview: { params: { text: string; options?: number; eqChannels?: number }; result: ConfigDiff };
  /** The chosen blocks of a document onto the open device. Fails `bad_document` or `refused`. */
  configImport: { params: { text: string; options?: number; eqChannels?: number; token?: string }; result: ImportReport };
  /** The open device as a `.dspipreset` document, to download. */
  configExport: { params: { name?: string; appVersion?: string; links?: number }; result: { text: string } };
  /** The open device as a snapshot document (`meta.kind = "snapshot"`), to download. */
  snapshotCapture: { params: { name?: string; note?: string }; result: { text: string } };
  /** Recall an uploaded snapshot document: one bulk image and the rest. Ends the undo history. */
  snapshotRecall: { params: { text: string }; result: SnapshotRecallRecord };
  /** Start a compare: B's document, A's (absent or "": the device now). Refused while a guard waits. */
  abStart: { params: { a?: string; b: string }; result: AbCompareRecord };
  abToggle: { params: undefined; result: AbCompareRecord };
  abDiff: { params: undefined; result: ConfigDiff };
  abKeep: { params: { side: "a" | "b" }; result: AbCompareRecord };
  abState: { params: undefined; result: AbCompareRecord };
  /** Import with a way back: waits for `applyConfirm`; a timeout (0: 30 s), Revert or a lost connection reverts. */
  applyGuardedImport: { params: { text: string; options?: number; eqChannels?: number; timeoutS?: number }; result: GuardedApplyRecord };
  /** Recall a snapshot document with a way back. */
  applyGuardedSnapshot: { params: { text: string; timeoutS?: number }; result: GuardedApplyRecord };
  applyGuardBegin: { params: { label?: string; timeoutS?: number }; result: GuardedApplyRecord };
  /** Keep. */
  applyConfirm: { params: undefined; result: GuardedApplyRecord };
  /** Revert. */
  applyRevert: { params: undefined; result: GuardedApplyRecord };
  /** Drives the guard: the timeout's revert runs here (no timer in the core). */
  applyPoll: { params: undefined; result: GuardedApplyRecord };
  applyStatus: { params: undefined; result: GuardedApplyRecord };
  /** Imports and recalls are guarded by default here (DSPi Link to a hub with the guard). */
  applyGuardDefault: { params: undefined; result: boolean };
  /** Open a document with no device (offline editing): the snapshot and what was opened. */
  documentOpen: { params: { text: string; fileName?: string }; result: DeviceSnapshot & { document: OpenedDocument } };
  documentInfo: { params: undefined; result: DocumentInfo };
  /** The open document as it now stands; nothing is unsaved afterwards. */
  documentSave: { params: { appVersion?: string; links?: number }; result: { text: string } };
  /** The open device or document as a template (`meta.kind = "template"`). */
  templateSave: { params: { name: string; appVersion?: string; links?: number }; result: { text: string } };
  templateList: { params: undefined; result: TemplateRecord[] };
  templateText: { params: { id: string }; result: { text: string } };
  /** The limits profile's state for the open device (P1LM). */
  limitsStatus: { params: undefined; result: LimitsStatusRecord };
  /** The notice for a failure's `noticeKey` (P1NT). */
  noticeFor: { params: { key: string; detail?: string }; result: NoticeRecord };
  noticeTexts: { params: undefined; result: NoticeRecord[] };
  // -- WP P1EFIX --
  /** Every limits field's range, in the editor's order (`dspi_core::limits_field_ranges`). */
  limitsFieldRanges: { params: undefined; result: LimitFieldRangeRecord[] };
}

// -- WP SW1 -- the audio link (crates/dspi-wasm/src/alink.rs; not DSPi Link)
/** `EXT_ALINK_INFO` as the core keeps it (extension_channel_spec.md 2.8.1). */
export interface AlinkInfo {
  role: string;
  roleCode: number;
  protocolVersion: number;
  classMask: number;
  maxAlinks: number;
  maxChannels: number;
  slotsPerUnit: number;
  rxPin: number;
  txPin: number;
  featureBits: number;
  dLocalMin: number;
  dLocalMax: number;
  alinkInput: boolean;
  stateEvents: boolean;
  applyEvents: boolean;
}

/** `EXT_ALINK_STATUS` (2.8.2). */
export interface AlinkStatus {
  state: "off" | "searching" | "locked" | "holdover" | "unknown";
  stateCode: number;
  alinkClass: number;
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

/** `EXT_ALINK_PHASE` (2.8.5). */
export interface AlinkPhase {
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
export interface AlinkMethods {
  /** No traffic; null without the link. */
  alinkInfo: { params: undefined; result: AlinkInfo | null };
  /** Re-read: the delay limits follow the output latency option. */
  fetchAlinkInfo: { params: undefined; result: AlinkInfo };
  alinkStatus: { params: undefined; result: AlinkStatus };
  /** 8 values, one per matrix input: an audio link channel 0..63, or 255 for silence. */
  alinkSlots: { params: undefined; result: number[] };
  setAlinkSlots: { params: { slots: number[] }; result: null };
  alinkPhase: { params: undefined; result: AlinkPhase };
  /** `mode` 0 fill servo, 1 phase lock; `dLocal` in audio link frames; `hopsOffset` -64..64. */
  setAlinkPhase: { params: { mode: number; dLocal: number; hopsOffset?: number }; result: null };
  alinkFrameCounter: { params: undefined; result: { frame: number; timeUs: number; outputFrames: number } };
  /** `op` 0 cancel all, 1 mute, 2 unmute, 3 preset load, 4 master volume, 5 output delay, 6 rate, 7 click, 8 sync pulse. */
  applyAtFrame: { params: { frame: number; op: number; arg?: number; value?: number }; result: null };
  applyAtFrameStatus: { params: undefined; result: ApplyAtFrameStatus };
  resetAlinkStats: { params: undefined; result: null };
  // -- WP AUSB1 -- the USB-direct block (extension_channel_spec.md 2.9): `unsupported` without it
  /** No traffic. */
  usbDirectSupported: { params: undefined; result: boolean };
  alinkUsb: { params: undefined; result: AlinkUsbConfig };
  /** `mode` 0 off, 1 rate lock, 2 phase lock; `d` samples (170..399 at 48 kHz in mode 2). */
  setAlinkUsb: { params: { mode: number; d: number; autoReanchor?: boolean }; result: null };
  alinkUsbStatus: { params: undefined; result: AlinkUsbStatus };
  alinkUsbDiag: { params: undefined; result: Record<string, unknown> };
  clearAlinkUsbDiag: { params: undefined; result: null };
  frameCounterTimeBase: {
    params: undefined;
    result: { frame: number; timeUs: number; outputFrames: number; timeBase: string; timeBaseCode: number; busFrame: number };
  };
  /** `timeBase` 0 alink frame, 1 USB sample index, 2 bus frame (bits 0-10) and sample (bits 16-23). */
  applyAtTimeBase: { params: { timeBase: number; frame: number; op: number; arg?: number; value?: number }; result: null };
}

// -- WP AUSB1 --
/** `EXT_ALINK_USB` (2.9.2). */
export interface AlinkUsbConfig {
  mode: number;
  modeName: "off" | "rate_lock" | "phase_lock" | "unknown";
  autoReanchor: boolean;
  d: number;
  dBelowMin: boolean;
  dAboveMax: boolean;
  rateUnsupported: boolean;
  state: string;
  stateCode: number;
}

/** `EXT_ALINK_USB_STATUS` (2.9.3). */
export interface AlinkUsbStatus {
  state: string;
  stateCode: number;
  phaseState: number;
  rateCode: number;
  rateHz: number;
  hwTimestamp: boolean;
  stream: boolean;
  nominalFeedback: boolean;
  timeBase: boolean;
  holdover: boolean;
  hostPpm: number;
  vcxoPpm: number;
  phaseErrorSamples: number;
  sample: number;
  busFrame: number;
  dMin: number;
  dMax: number;
  fill: number;
  streamStartBusFrame: number | null;
  streamAgeFrames: number;
  streamShift: number;
  gapCount: number;
  anchors: number;
  reanchors: number;
  missedSamples: number;
  lateSamples: number;
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
  /** -- WP P1WB1 -- the core's notice key (P1NT, `noticeFor`), when it has one. */
  noticeKey?: string;
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
  constructor(
    readonly code: string,
    message: string,
    // -- WP P1WB1 -- the core's notice key (P1NT), when the failure has one
    readonly noticeKey?: string,
  ) {
    super(message);
    this.name = "CoreCallError";
  }
}

/** A peak (0..1) in dBFS, floored at -90 for a meter. */
export function peakDb(peak: number): number {
  return peak > 0 ? Math.max(-90, 20 * Math.log10(peak)) : -90;
}

// -- WP P1WB2 -- the Installer page, the lock, the mode manifest, tokens, groups, protect all,
// AutoEQ preview, the power-on fields (crates/dspi-wasm/src/{modes,limits,outputs,history,autoeq,power_on}.rs)

/** The open device's installer lock (`dspi_core::LockState`). */
export interface LockStateRecord {
  serial: string;
  pinSet: boolean;
  unlocked: boolean;
  /** The locked-user state: only `allowedWrites` pass. */
  locked: boolean;
  /** The Installer pages may change without a PIN prompt. */
  installerOpen: boolean;
  covers: string[];
  allowedWrites: string[];
  wrongTries: number;
  retryAfterMs: number;
  recovered: boolean;
  note: string;
  contact: string;
  /** `installer_lock.scope` (this console) or `installer_lock.hub`. */
  scopeNoticeKey: string;
  detail: string;
  /** `console`, `hub`, or "" with no PIN. */
  heldBy: string;
  /** The mode the hub pins for this console's role on this device, "" for none. */
  hubPinnedMode: string;
}

/** The mode manifest (`assets/modes/modes.json`, `dspi_core::ModeManifest`). */
export interface ModeManifest {
  version: number;
  platforms: string[];
  modes: { id: string; label: string; defaultOn: string[]; needsPinWhenSet: boolean }[];
  lockedUser: { label: string; screens: string[]; settings: string[]; writes: string[] };
  screens: { id: string; title: string; modes: string[]; firstIn?: string[]; layer?: string }[];
  operations: { id: string; title: string; modes: string[]; platforms?: string[]; confirm?: boolean }[];
}

/** The mode to show and why (`dspi_core::ModeChoice`). */
export interface ModeChoice {
  /** `simple`, `advanced`, `installer` or `locked_user`. */
  mode: string;
  /** `locked`, `pinned`, `chosen` or `default`. */
  reason: string;
  switchable: boolean;
  needsPin: boolean;
  openModes: string[];
  screens: string[];
  firstScreen: string;
  operations: string[];
}

/** What applying a limits profile did (`dspi_core::LimitsApply`). */
export interface LimitsApplyRecord {
  lines: string[];
  /** The values held to the profile, one row each. */
  held: string[];
  notes: string[];
}

/** A PEQ or crossover band as AutoEQ leaves it. */
export interface AutoEqChannelRecord {
  channel: number;
  isOutput: boolean;
  levelDb: number;
  bands: FilterParams[];
  crossover: FilterParams[];
  /** dB at each `curveGrid` point. */
  curve: number[];
}

/** What an AutoEQ profile would do to a channel pair (`dspi_core::AutoEqPreview`). */
export interface AutoEqPreviewRecord {
  profileId: string;
  channels: AutoEqChannelRecord[];
  /** `input_preamp`, `output_gain` or `none`. */
  levelTarget: string;
  suggestedTrimDb: number;
  held: string[];
  unsupported: string[];
  dropped: number;
  notes: string[];
}

/** The DAC's hardware mute (`dacMuteFetch`). */
export interface DacMuteRecord {
  supported: boolean;
  enabled: boolean;
  activeLow: boolean;
  pin: number;
  holdMs: number;
  releaseMs: number;
}

export interface InstallerMethods {
  modeManifest: { params: undefined; result: ModeManifest };
  /** The mode to show on the web for the user's `chosen` ("" for none): lock, hub pin, choice, default. */
  modeResolve: { params: { platform: string; chosen?: string }; result: ModeChoice };
  /** Pin the open device's mode (a fleet's or a hub's); "" clears it. */
  modeSetPinned: { params: { mode: string }; result: Record<string, never> };
  modePinned: { params: undefined; result: { mode: string } };
  // -- WP ROMODE -- the client read-only mode (crates/dspi-wasm/src/read_only.rs)
  /**
   * Turn the core's client read-only mode on or off for this session: while on, every write
   * fails with `read_only` (notice key `read_only.client`) before anything is sent; reads,
   * notifications and the meters go on. A diagnostic (the page's `?readonly`), not a security boundary.
   */
  setClientReadOnly: { params: { on: boolean }; result: { on: boolean } };
  clientReadOnly: { params: undefined; result: { on: boolean; refusals: number } };
  installerLockState: { params: undefined; result: LockStateRecord };
  /** With the hub's PIN when the hub holds the lock. A wrong one: `installer_lock` (`installer_lock.wrong_pin`, `.wait`). */
  installerUnlock: { params: { pin: string }; result: LockStateRecord };
  installerLock: { params: undefined; result: LockStateRecord };
  /** The web keeps no PIN: `unsupported`. */
  installerPinSet: { params: { pin: string }; result: { recoveryCode: string; state: LockStateRecord } };
  installerPinClear: { params: undefined; result: LockStateRecord };
  /** What the lock covers, by the manifest's screen ids. */
  installerLockCover: { params: { pages: string[] }; result: LockStateRecord };
  /** Set the open device's limits profile (a limits document) and hold the device to it. */
  limitsApply: { params: { text: string }; result: LimitsApplyRecord };
  /** The open device's profile as a document, "" with none. */
  limitsProfile: { params: undefined; result: { text: string } };
  limitsClear: { params: undefined; result: Record<string, never> };
  /** Every limiter on at the ceiling (default -1 dBFS), stereo pairs linked: one undo step. */
  protectAllOutputs: { params: undefined | { ceilingDbfs: number }; result: null };
  /** The outputs (0-based) that are on, above the threshold (default -10 dB) and without a limiter. */
  outputsWithoutLimiter: { params: undefined | { thresholdDb: number }; result: number[] };
  // -- WP WB6F --
  /** The limiters' gain reduction per output slot, in dB (0: none); `unsupported` without a limiter. */
  limiterMeter: { params: undefined; result: number[] };
  /** Both inputs of a linked pair, one undo step. */
  inputPairPreamp: { params: { input: number; db: number }; result: null };
  // -- WP PAIRBAND --
  /** A linked input pair's PEQ band (0..9), both sides (`input`, `input ^ 1`), one undo step; a drag's ticks too. */
  inputPairFilter: { params: { input: number; band: number } & FilterParams; result: null };
  /** A linked input pair's band bypass, both sides, one undo step. */
  inputPairBandBypass: { params: { input: number; band: number; bypass: boolean }; result: null };
  /** Everything written until `endGroup` is one undo step named `label`. */
  beginGroup: { params: { label: string }; result: null };
  endGroup: { params: undefined; result: null };
  /** A single-use token for one gated operation (`save_preset`, `delete_preset`, `copy_preset`, `factory_reset`, `config_import`, ...), 10 s. */
  confirmToken: { params: { op: string }; result: string };
  // -- WP P1TZ -- tokens are always required: requireConfirmation and confirmationRequired are gone
  /** What an AutoEQ database entry would do to the pair `left`, `right` (wire channels); nothing written. */
  autoeqPreview: { params: { profile: unknown; left: number; right: number; applyLevel?: boolean }; result: AutoEqPreviewRecord };
  /** Apply it, in one undo group, through the limits; the state before as a snapshot document. */
  autoeqApply: {
    params: { profile: unknown; left: number; right: number; applyLevel?: boolean };
    result: { preview: AutoEqPreviewRecord; snapshotDocument: string; snapshotId: string; notes: string[] };
  };
  /** The DAC's hardware mute config (read back about 150 ms later with `dacMuteFetch`). */
  dacMuteSet: { params: Omit<DacMuteRecord, "supported">; result: null };
  dacMuteFetch: { params: undefined; result: DacMuteRecord };
}

// -- WP HUBADMIN -- which copy the Installer page edits, and the hub's copy over DSPi Link
// (crates/dspi-wasm/src/hub_admin.rs over dspi_core::hub_admin)

/** Which copy the Installer page edits (`dspi_core::hub_admin::InstallerTarget`). */
export interface InstallerTargetRecord {
  /** `hub`, `console` (a USB DSPi, the demo) or `none`. */
  copy: string;
  /** The page may change that copy (the hub's: an Admin of the hub only). */
  editable: boolean;
  serial: string;
  /** This console's role on the hub (`viewer`, `control`, `admin`), "" off a hub. */
  role: string;
  /** The notice row saying which copy is edited. */
  noticeKey: string;
  /** Why the page is read-only, as a notice row; "" when editable. */
  readOnlyKey: string;
}

/** A role's pinned mode on the hub; `mode` "" for none. */
export interface HubModePinRecord {
  role: string;
  mode: string;
}

/** The hub's limits, PIN and pinned modes for the open device (`dspi_core::hub_admin::HubAdminState`). */
export interface HubAdminStateRecord {
  serial: string;
  role: string;
  canEdit: boolean;
  /** The hub's profile as a limits document, "" for none. */
  profileDocument: string;
  pinSet: boolean;
  locked: boolean;
  unlocked: boolean;
  held: string[];
  wrongTries: number;
  retryAfterMs: number;
  pinnedModes: HubModePinRecord[];
}

/** A hub admin event, after the core read the lock again (`dspi_core::hub_admin::HubAdminEvent`). */
export interface HubAdminEventRecord {
  /** `limits.changed`, `limits.held` or `mode.changed`. */
  kind: string;
  serial: string;
  rows: string[];
  role: string;
  mode: string;
  lock: LockStateRecord;
}

export interface HubAdminMethods {
  /** Which copy the Installer page edits; no I/O. */
  installerTarget: { params: undefined; result: InstallerTargetRecord };
  /** The hub's copy (every role may read). Off a hub: `dlink.no_limits`. */
  hubLimitsGet: { params: undefined; result: HubAdminStateRecord };
  /** Set the hub's limits profile (a limits document): what it held at once. Admin only (`denied`). */
  hubLimitsSet: { params: { text: string }; result: { held: string[]; state: HubAdminStateRecord } };
  hubLimitsClear: { params: undefined; result: HubAdminStateRecord };
  /** Set the hub's installer PIN (an Admin needs no current one): the recovery code, shown once. */
  hubPinSet: { params: { pin: string }; result: { recoveryCode: string; state: HubAdminStateRecord } };
  hubPinClear: { params: undefined; result: HubAdminStateRecord };
  /** Pin `role`'s mode on the hub; "" unpins. */
  hubModePin: { params: { role: string; mode: string }; result: HubAdminStateRecord };
  /** The hub's admin events since the last call (also handed out by `linkTick`). */
  hubAdminEvents: { params: undefined; result: HubAdminEventRecord[] };
}
