// One open device session as the UI sees it (WP WB5): the core client, what the
// fleet backend allows, and the last snapshot and Link phase it reported.
//
// A hub (WP WB4b) is reached in Link's own steps, each the core's: connect with
// the token kept for that hub; if it wants a PIN, `pairing` until the user gives
// it (the token is then kept); then either the DSPi the fleet named, or
// `choosing` from the hub's own list; then open. From then on the banner follows
// the core's phases (reconnecting, closed), which the Worker pushes.

import {
  CoreCallError,
  type CoreEvent,
  type CoreMethods,
  type DeviceSnapshot,
  type DeviceTarget,
  type LinkDeviceRecord,
  type LinkPhase,
  type ParamInfo,
  type ParamValue,
} from "../core/protocol";
import type { CoreClient } from "../core/client";
import { VIEW_ONLY_REASON, type Access, type Source } from "../fleet/types";
// -- WP WB4b --
import type { HubToken, HubTokens } from "../fleet/hub-tokens";

export interface SessionState {
  /** -- WP WB4b -- `pairing`: a hub wants its PIN; `choosing`: pick one of a hub's DSPis. */
  status: "opening" | "pairing" | "choosing" | "open" | "failed" | "closed";
  snapshot: DeviceSnapshot | null;
  link: { phase: LinkPhase; pinSource?: string } | null;
  error: string;
  /** What the registry says of the parameters the screens show, by path. */
  params: Record<string, ParamInfo>;
  // -- WP WB4b --
  /** The hub's DSPis, while `choosing`. */
  devices: LinkDeviceRecord[];
  /** The hub's name once it has said it. */
  hubName: string;
  /** Why the last PIN was not taken. */
  pinError: string;
}

// -- WP WB6a --
/** The calls `Session.change` makes. */
export type ChangeMethod = "presetCopy" | "presetRevert" | "factoryReset";

/** The parameters whose ranges the skeleton's controls need. */
const SHOWN_PARAMS = ["vol.user"];

export class Session {
  state: SessionState = {
    status: "opening",
    snapshot: null,
    link: null,
    error: "",
    params: {},
    devices: [],
    hubName: "",
    pinError: "",
  };
  private readonly listeners = new Set<() => void>();
  private unsubscribe: () => void;
  // -- WP WB4b -- the hub this session is on, and the id its token belongs to
  private hub: { url: string; hubId: string } | null = null;
  /** The serial of the hub DSPi this session opens or has open, if any. */
  deviceId = "";

  constructor(
    readonly id: string,
    readonly installationId: string,
    readonly installationName: string,
    readonly source: Source,
    readonly access: Access,
    readonly core: CoreClient,
    // -- WP WB4b --
    private readonly tokens: HubTokens | null = null,
  ) {
    this.unsubscribe = core.subscribe((e) => this.onEvent(e));
  }

  /** Why writes are off, or "" when this session may write. */
  get readOnlyReason(): string {
    if (this.access === "view") return VIEW_ONLY_REASON;
    return this.state.snapshot?.device.readOnly ? this.state.snapshot.device.readOnlyReason : "";
  }

  async open(target: DeviceTarget): Promise<void> {
    if (target.kind === "hub") return this.openHub(target.url, target.deviceId);
    try {
      await this.load(target);
    } catch (e) {
      this.set({ status: "failed", error: message(e) });
    }
  }

  private async load(target: DeviceTarget): Promise<void> {
    const snapshot = await this.core.call("open", target);
    const params: Record<string, ParamInfo> = {};
    for (const path of SHOWN_PARAMS) params[path] = await this.core.call("describe", { path });
    this.set({ status: "open", snapshot, params, error: "" });
  }

  // -- WP WB4b --
  private async openHub(url: string, deviceId: string): Promise<void> {
    this.deviceId = deviceId;
    try {
      const saved = (await this.tokens?.get(url)) ?? null;
      const connect = (t: HubToken | null) => this.core.call("linkConnect", { url, token: t?.token ?? "", hubId: t?.hubId ?? "" });
      let hello: Awaited<ReturnType<typeof connect>>;
      try {
        hello = await connect(saved);
      } catch (e) {
        // -- WP WB7 -- (c6m N2) another hub answers where the token's hub was (it was
        // replaced, or the address moved): that token is for no hub here. Forget it and
        // connect without one; this hub then asks for its PIN.
        if (!(saved && e instanceof CoreCallError && e.code === "wrong_hub")) throw e;
        this.tokens?.forget(url);
        hello = await connect(null);
      }
      if (hello.tokenRefused) this.tokens?.forget(url);
      this.hub = { url, hubId: hello.hubId };
      this.set({ hubName: hello.hubName });
      if (hello.needsPairing) {
        this.set({ status: "pairing", link: { phase: "needs_pairing", pinSource: hello.pinSource } });
        return;
      }
      await this.afterAuth();
    } catch (e) {
      this.set({ status: "failed", error: message(e) });
    }
  }

  /** The PIN the user typed; a wrong one keeps the prompt, with why. */
  async pair(pin: string): Promise<void> {
    if (!this.hub || this.state.status !== "pairing") return;
    try {
      const { token } = await this.core.call("linkPair", { pin });
      this.tokens?.save(this.hub.url, { token, hubId: this.hub.hubId });
      this.set({ pinError: "" });
      await this.afterAuth();
    } catch (e) {
      if (e instanceof CoreCallError && e.code !== "wrong_pin" && e.code !== "link") this.set({ status: "failed", error: message(e) });
      else this.set({ pinError: message(e) });
    }
  }

  /** Open one of the hub's DSPis (`choosing`). */
  async choose(serial: string): Promise<void> {
    if (!this.hub || this.state.status !== "choosing") return;
    this.deviceId = serial;
    try {
      await this.load({ kind: "hub", url: this.hub.url, deviceId: serial });
    } catch (e) {
      this.deviceId = "";
      this.set({ error: message(e) });
    }
  }

  private async afterAuth(): Promise<void> {
    if (!this.hub) return;
    if (this.deviceId) {
      await this.load({ kind: "hub", url: this.hub.url, deviceId: this.deviceId });
      return;
    }
    this.set({ status: "choosing", devices: await this.core.call("linkDevices", undefined), error: "" });
  }

  private refreshDevices(): void {
    this.core.call("linkDevices", undefined).then(
      (devices) => {
        if (this.state.status === "choosing") this.set({ devices });
      },
      () => undefined,
    );
  }

  /**
   * Writes and shows what the device read back; a refusal leaves the old value
   * shown. -- WP WB6a -- Answers whether the write went through.
   */
  async write(path: string, value: ParamValue, indices?: number[]): Promise<boolean> {
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return false;
    }
    try {
      await this.core.call("write", indices ? { path, indices, value } : { path, value });
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "" });
      return true;
    } catch (e) {
      this.set({ error: message(e) });
      return false;
    }
  }

  // -- WP WB6a --
  /**
   * A core call that changes the device without a path (the preset flows:
   * `presetCopy`, `presetRevert`, `factoryReset`), then the read-back, as
   * `write`. Refused while read only. Answers whether it went through.
   */
  async change<M extends ChangeMethod>(method: M, params: CoreMethods[M]["params"]): Promise<boolean> {
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return false;
    }
    try {
      await this.core.call(method, params);
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "" });
      return true;
    } catch (e) {
      this.set({ error: message(e) });
      return false;
    }
  }

  /**
   * Asked before the user closes this session (Disconnect): a screen with
   * something unsaved sets it (the main window's unsaved-changes guard, M42).
   * Answers whether to go on.
   */
  leaveGuard: (() => Promise<boolean>) | null = null;

  async mayLeave(): Promise<boolean> {
    return this.leaveGuard ? this.leaveGuard() : true;
  }

  async close(): Promise<void> {
    this.unsubscribe();
    await this.core.dispose();
    this.set({ status: "closed" });
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private onEvent(e: CoreEvent): void {
    if (e.event === "state") this.set({ snapshot: e.snapshot });
    else if (e.event === "link") {
      this.set({ link: { phase: e.phase, pinSource: e.pinSource } });
      // -- WP WB4b -- the hub's list may have changed (a DSPi came or went)
      if (this.state.status === "choosing") this.refreshDevices();
    }
    else if (e.event === "lost") this.set({ status: "failed", error: e.reason });
  }

  private set(patch: Partial<SessionState>): void {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }
}

function message(e: unknown): string {
  if (e instanceof CoreCallError) return capitalise(e.message);
  return e instanceof Error ? e.message : String(e);
}

function capitalise(s: string): string {
  return s ? s[0]!.toUpperCase() + s.slice(1) : s;
}
