// The contract between the page and a core Worker (WP WB5).
//
// One Worker holds one device session. A fleet page opens as many Workers as it
// has devices open, so nothing here is global to the page and no device is "the"
// device. Inside a Worker every call runs alone, in arrival order: the core runs
// under JSPI there (docs/core-merge/WEB-STUDY.md section 2), and a second call
// entering it while one is suspended corrupts its stack. The queue in worker.ts
// is what keeps that rule; WB3's binding keeps its own guard as well.
//
// The shapes follow the core's own records (the C ABI's JSON) so that WB3 can
// answer them from `dspi-core` with no translation layer in the page.

/** Where a session's device is. A fleet server may add its own kinds later. */
export type DeviceTarget =
  | { kind: "demo"; readOnly?: boolean }
  | { kind: "usb"; serial?: string }
  | { kind: "hub"; url: string; deviceId: string };

/** What the page shows of a device. Read again after every write. */
export interface DeviceSnapshot {
  name: string;
  serial: string;
  firmware: string;
  platform: string;
  /** True when this session must not write: older firmware, or a viewer. */
  readOnly: boolean;
  /** Why it is read-only, in the core's wording; empty when it is not. */
  readOnlyReason: string;
  volume: { db: number; minDb: number; maxDb: number };
  presets: { active: number; names: string[] };
  channels: { name: string }[];
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
  | { event: "state"; snapshot: DeviceSnapshot }
  | { event: "link"; phase: LinkPhase; pinSource?: string }
  | { event: "lost"; reason: string };

/** Every call a core Worker answers. `params` and the result per method. */
export interface CoreMethods {
  open: { params: DeviceTarget; result: DeviceSnapshot };
  state: { params: undefined; result: DeviceSnapshot };
  /** A parameter by registry path (`vol.user`), as `Session::write`: the result is the value read back. */
  write: { params: { path: string; value: number }; result: number };
  meters: { params: undefined; result: number[] };
  close: { params: undefined; result: null };
  /** Tests only: make the stub core lose its link. WB3's core answers "unknown method". */
  simulate: { params: { what: "link-lost" | "link-closed" }; result: null };
}

export type Method = keyof CoreMethods;

export interface Request<M extends Method = Method> {
  id: number;
  method: M;
  params: CoreMethods[M]["params"];
}

export interface CoreError {
  /** Stable: `refused`, `read_only`, `out_of_range`, `not_open`, `lost`, `busy`, `unknown_method`, `internal`. */
  code: string;
  message: string;
}

export type Reply =
  | { id: number; ok: true; value: unknown }
  | { id: number; ok: false; error: CoreError };

/** A message from a Worker: a reply to a call, or an event. */
export type FromWorker = { reply: Reply } | { push: CoreEvent };

/** What runs inside the Worker: the stub now, `dspi-wasm` after WB3. */
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
