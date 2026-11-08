// One open device session as the UI sees it (WP WB5): the core client, what the
// fleet backend allows, and the last snapshot and Link phase it reported.

import {
  CoreCallError,
  type CoreEvent,
  type DeviceSnapshot,
  type DeviceTarget,
  type LinkPhase,
  type ParamInfo,
  type ParamValue,
} from "../core/protocol";
import type { CoreClient } from "../core/client";
import { VIEW_ONLY_REASON, type Access, type Source } from "../fleet/types";

export interface SessionState {
  status: "opening" | "open" | "failed" | "closed";
  snapshot: DeviceSnapshot | null;
  link: { phase: LinkPhase; pinSource?: string } | null;
  error: string;
  /** What the registry says of the parameters the screens show, by path. */
  params: Record<string, ParamInfo>;
}

/** The parameters whose ranges the skeleton's controls need. */
const SHOWN_PARAMS = ["vol.user"];

export class Session {
  state: SessionState = { status: "opening", snapshot: null, link: null, error: "", params: {} };
  private readonly listeners = new Set<() => void>();
  private unsubscribe: () => void;

  constructor(
    readonly id: string,
    readonly installationId: string,
    readonly installationName: string,
    readonly source: Source,
    readonly access: Access,
    readonly core: CoreClient,
  ) {
    this.unsubscribe = core.subscribe((e) => this.onEvent(e));
  }

  /** Why writes are off, or "" when this session may write. */
  get readOnlyReason(): string {
    if (this.access === "view") return VIEW_ONLY_REASON;
    return this.state.snapshot?.device.readOnly ? this.state.snapshot.device.readOnlyReason : "";
  }

  async open(target: DeviceTarget): Promise<void> {
    try {
      const snapshot = await this.core.call("open", target);
      const params: Record<string, ParamInfo> = {};
      for (const path of SHOWN_PARAMS) params[path] = await this.core.call("describe", { path });
      this.set({ status: "open", snapshot, params, error: "" });
    } catch (e) {
      this.set({ status: "failed", error: message(e) });
    }
  }

  /** Writes and shows what the device read back; a refusal leaves the old value shown. */
  async write(path: string, value: ParamValue, indices?: number[]): Promise<void> {
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return;
    }
    try {
      await this.core.call("write", indices ? { path, indices, value } : { path, value });
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "" });
    } catch (e) {
      this.set({ error: message(e) });
    }
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
    else if (e.event === "link") this.set({ link: { phase: e.phase, pinSource: e.pinSource } });
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
