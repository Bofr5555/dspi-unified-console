// A stand-in core for the skeleton (WP WB5), until WB3's `dspi-wasm` exists.
//
// It answers the same calls with the same shapes, from a small in-memory demo
// device, so the page and its tests are written against the real contract. It is
// not a second implementation of the protocol: it has no wire, no registry and
// only the parameters the skeleton shows. When WB3 lands, worker.ts loads the
// wasm core and this file stays only as the tests' fake.

import {
  CoreCallError,
  type CoreEvent,
  type CoreImpl,
  type CoreMethods,
  type DeviceSnapshot,
  type DeviceTarget,
  type Method,
} from "./protocol";

/** `vol.user`'s range, from `dspi-proto`'s registry. */
const VOL_MIN = -60;
const VOL_MAX = 0;

/** `dspi_core`'s error for firmware it can read but not write (`connect.rs`). */
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
    this.device = {
      name: "DSPi demo",
      serial: "DEMO-0001",
      firmware: "1.1.6",
      platform: "RP2350",
      readOnly,
      readOnlyReason: readOnly ? READ_ONLY_REASON : "",
      volume: { db: -20, minDb: VOL_MIN, maxDb: VOL_MAX },
      presets: { active: 0, names: ["Living room", "Late night", "Flat"] },
      channels: [
        { name: "USB L" },
        { name: "USB R" },
        { name: "Out 1" },
        { name: "Out 2" },
        { name: "Out 3" },
        { name: "Out 4" },
      ],
    };
    return structuredClone(this.device);
  }

  private need(): DeviceSnapshot {
    if (!this.device) throw new CoreCallError("not_open", "No DSPi is open in this session");
    return this.device;
  }

  private write({ path, value }: { path: string; value: number }): number {
    const d = this.need();
    if (d.readOnly) throw new CoreCallError("read_only", d.readOnlyReason);
    if (path !== "vol.user") throw new CoreCallError("refused", `${path} is not available yet`);
    if (!Number.isFinite(value) || value < VOL_MIN || value > VOL_MAX) {
      throw new CoreCallError("out_of_range", `Volume must be between ${VOL_MIN} and ${VOL_MAX} dB`);
    }
    // The device keeps 0.5 dB steps; the read-back is what it kept.
    d.volume.db = Math.round(value * 2) / 2;
    this.push({ event: "state", snapshot: structuredClone(d) });
    return d.volume.db;
  }

  private meters(): number[] {
    const d = this.need();
    this.tick += 1;
    const level = d.volume.db - VOL_MIN; // louder volume, higher meters
    return d.channels.map((_, i) => {
      const wobble = 6 * Math.sin((this.tick + i * 7) / 5);
      return Math.min(0, Math.max(-90, level - 60 + wobble - i));
    });
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
