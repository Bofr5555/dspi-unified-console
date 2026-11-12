// A fake core for tests (WP WB5; since WB3 tests only, the page runs the real
// core from crates/dspi-wasm).
//
// It answers the same calls with the same shapes, from a small in-memory demo
// device, so unit tests and the stub-driven e2e page (the lost-link banner,
// which the core's demo cannot produce before Link, WB4b) run without the wasm
// module. It is not a second implementation of the protocol: it has no wire, a
// registry of one parameter, and only the record fields the console reads.

import {
  CoreCallError,
  type CoreEvent,
  type CoreImpl,
  type CoreMethods,
  type DeviceSnapshot,
  type DeviceTarget,
  type DspState,
  type Method,
  type ParamInfo,
  type ParamValue,
  type SystemStatus,
} from "./protocol";

/** `vol.user` as `dspi-proto`'s registry describes it. */
const VOL_USER: ParamInfo = {
  path: "vol.user",
  label: "Volume",
  plain: "The listening volume, shared with the OS volume slider",
  writable: true,
  readable: true,
  indices: 0,
  routed: true,
  kind: "float",
  unit: "dB",
  min: -60,
  max: 0,
};

/** `dspi_core`'s error for firmware it can read but not write (`error.rs`). */
const READ_ONLY_REASON = "the connected firmware is read-only in this version";

export interface StubOptions {
  /** Milliseconds each call takes, so tests can overlap calls. */
  latencyMs?: number;
}

export class StubCore implements CoreImpl {
  private push: (e: CoreEvent) => void = () => {};
  private device: DeviceSnapshot | null = null;
  private tick = 0;

  constructor(private readonly opts: StubOptions = {}) {}

  onEvent(push: (e: CoreEvent) => void): void {
    this.push = push;
  }

  async call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    if (this.opts.latencyMs) await new Promise((r) => setTimeout(r, this.opts.latencyMs));
    const result: unknown = this.dispatch(method, params);
    return result as CoreMethods[M]["result"];
  }

  private dispatch(method: Method, params: unknown): unknown {
    switch (method) {
      case "open":
        return this.open(params as DeviceTarget);
      case "state":
        return structuredClone(this.need());
      case "write":
        return this.write(params as CoreMethods["write"]["params"]);
      case "describe":
        if ((params as { path: string }).path !== VOL_USER.path) {
          throw new CoreCallError("unknown_param", `there is no parameter ${(params as { path: string }).path}`);
        }
        return { ...VOL_USER };
      case "meters":
        return this.meters();
      case "close":
        this.device = null;
        return null;
      case "simulate":
        return this.simulate(params as CoreMethods["simulate"]["params"]);
      default:
        throw new CoreCallError("unknown_method", `unknown method ${String(method)}`);
    }
  }

  private open(target: DeviceTarget): DeviceSnapshot {
    if (target.kind !== "demo") {
      throw new CoreCallError("refused", "This connection type is not available yet");
    }
    const readOnly = target.readOnly === true;
    const names = ["USB L", "USB R", "Out 1", "Out 2", "Out 3", "Out 4"];
    const state = {
      platformId: 1,
      numChannels: names.length,
      numInputChannels: 2,
      numOutputChannels: 4,
      formatVersion: readOnly ? 31 : 32,
      fwMajor: 1,
      fwMinor: 1,
      fwPatch: 6,
      fwBeta: 0,
      preampDb: 0,
      bypass: false,
      masterVolumeDb: 0,
      userVolumeDb: -20,
      userMute: false,
      loudnessEnabled: false,
      outputGainDb: [0, 0, 0, 0],
      outputMuted: [false, false, false, false],
      channelNames: names,
      presetNames: ["Living room", "Late night", "Flat"],
      activePresetSlot: 0,
      // -- WP WB6a -- the main window's fields
      masterVolumeMode: 0,
      presetOccupied: 0b111,
      presetStartupMode: 1,
      presetDefaultSlot: 0,
      inputPreampDb: [0, 0],
      outputEnabled: [true, true, true, true],
      inputSource: 0,
      i2sInputChannels: 2,
      // -- WP WB6b -- no bands: the stub has no curves to draw them with
      filters: [],
      xover: [],
      maxBands: 10,
      // -- WP WB6c -- no matrix, delays or limiter in the stub
      outputDelayMs: [0, 0, 0, 0],
      matrixRouting: [],
      matrixGain: [],
      matrixInvert: [],
      limiterEnabled: [],
      limiterLinkGroup: [],
      limiterThresholdDb: [],
      limiterReleaseMs: [],
    } satisfies DspState;
    this.device = {
      device: {
        name: "Stub DSPi",
        serial: "STUB-0001",
        firmware: "1.1.6",
        platform: "RP2350",
        demo: true,
        readOnly,
        readOnlyReason: readOnly ? READ_ONLY_REASON : "",
      },
      state,
    };
    return structuredClone(this.device);
  }

  private need(): DeviceSnapshot {
    if (!this.device) throw new CoreCallError("not_open", "No DSPi is open in this session");
    return this.device;
  }

  private write({ path, value }: CoreMethods["write"]["params"]): ParamValue {
    const d = this.need();
    if (path !== VOL_USER.path) throw new CoreCallError("refused", `${path} is not available in this version yet`);
    if (typeof value !== "object" || !("float" in value)) throw new CoreCallError("invalid", "Volume takes a number");
    const db = value.float;
    if (!Number.isFinite(db) || db < VOL_USER.min! || db > VOL_USER.max!) {
      throw new CoreCallError("out_of_range", `Volume must be between ${VOL_USER.min} and ${VOL_USER.max} dB`);
    }
    if (d.device.readOnly) throw new CoreCallError("read_only", d.device.readOnlyReason);
    // A device keeps its own steps; the read-back is what it kept.
    d.state.userVolumeDb = Math.round(db * 2) / 2;
    this.push({ event: "state", snapshot: structuredClone(d), flags: 1 });
    return { float: d.state.userVolumeDb };
  }

  private meters(): SystemStatus {
    const d = this.need();
    this.tick += 1;
    const level = d.state.userVolumeDb + 60; // louder volume, higher meters
    const peaks = d.state.channelNames.map((_, i) => {
      const db = Math.min(0, Math.max(-90, level - 60 + 6 * Math.sin((this.tick + i * 7) / 5) - i));
      return 10 ** (db / 20);
    });
    return {
      peaks,
      cpu0: 10,
      cpu1: 20,
      clipFlags: 0,
      activeInputChannels: 2,
      numChannels: peaks.length,
      stale: false,
      ageMs: 0,
      // -- WP P0M -- no ballistics in the stub: the hold is the level
      peakHold: peaks,
      peakMax: peaks,
      clipHold: 0,
    };
  }

  private simulate({ what }: { what: "link-lost" | "link-closed" }): null {
    this.need();
    if (what === "link-lost") this.push({ event: "link", phase: "reconnecting" });
    if (what === "link-closed") {
      this.push({ event: "link", phase: "closed" });
      this.device = null;
    }
    return null;
  }
}
