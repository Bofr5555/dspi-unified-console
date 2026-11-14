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
  // -- WP WB8 --
  type HistoryMove,
  type LinkDeviceRecord,
  type LinkPhase,
  type ParamValue,
  // -- WP P1WB1 --
  type OpenedDocument,
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
  // -- WP WB4b --
  /** The hub's DSPis, while `choosing`. */
  devices: LinkDeviceRecord[];
  /** The hub's name once it has said it. */
  hubName: string;
  /** Why the last PIN was not taken. */
  pinError: string;
  // -- WP WB6b -- (c6p WB6a-4)
  /**
   * What is unsaved against the active preset, in the core's sentences (CG7,
   * `presetChanges`), kept current after every change the session sees. Empty
   * when nothing is, and always while read only.
   */
  unsaved: string[];
  // -- WP WB8 --
  /** What undo and redo can do now (the core's journal); both false while read only. */
  history: { canUndo: boolean; canRedo: boolean };
  /** What the last undo or redo has to say: another writer's change left alone (`warn`), changes stepped over (`info`). */
  notice: { text: string; tone: "warn" | "info" } | null;
  // -- WP P1WB1 --
  /** What an offline session was opened from (`documentOpen`): the strip says "File: <name>". Null on a device. */
  document?: OpenedDocument | null;
}

// -- WP WB6a --
/** The calls `Session.change` makes. */
export type ChangeMethod = "presetCopy" | "presetRevert" | "factoryReset"
  // -- WP WB6c --
  | "crossoverType"
  | "crosspoint"
  // -- WP WB8 -- a whole band during a graph drag
  | "filter";

// -- WP WB6b --
/** One write of `Session.writeAll`. */
export interface Write {
  path: string;
  value: ParamValue;
  indices?: number[];
}

export class Session {
  state: SessionState = {
    status: "opening",
    snapshot: null,
    link: null,
    error: "",
    devices: [],
    hubName: "",
    pinError: "",
    unsaved: [],
    history: { canUndo: false, canRedo: false },
    notice: null,
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

  // -- WP P1WB1 -- offline editing (P1OF): a document opened with no device
  /** Open the `.dspipreset` document `text` as this session's device; nothing touches USB. */
  async openDocument(text: string, fileName: string): Promise<void> {
    try {
      const r = await this.core.call("documentOpen", { text, fileName });
      const { document, ...snapshot } = r;
      this.set({ status: "open", snapshot, document, error: "" });
    } catch (e) {
      this.set({ status: "failed", error: message(e) });
    }
  }

  /** The document this session edits, or null on a device. */
  get document(): OpenedDocument | null {
    return this.state.document ?? null;
  }

  /** Read the device again and show it (after a recall, an import, a revert). */
  async refresh(): Promise<void> {
    try {
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot });
    } catch (e) {
      this.set({ error: message(e) });
    }
  }

  /** Show `text` as the session's error line, or clear it with "". */
  report(text: string): void {
    this.set({ error: text });
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
    this.set({ status: "open", snapshot, error: "" });
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
   * Asked before the user closes this session (Disconnect): the session's
   * view sets it (the unsaved-changes guard, M42; -- WP WB6b -- for every
   * screen, not one). Answers whether to go on.
   */
  leaveGuard: (() => Promise<boolean>) | null = null;

  async mayLeave(): Promise<boolean> {
    return this.leaveGuard ? this.leaveGuard() : true;
  }

  // -- WP WB6b --
  /**
   * Several writes in a row (the fields of one filter band), then one
   * read-back of the state, as `write`. Stops at the first refusal, which
   * the error shows; answers whether all went through.
   */
  async writeAll(writes: Write[]): Promise<boolean> {
    if (this.readOnlyReason) {
      this.set({ error: this.readOnlyReason });
      return false;
    }
    let ok = true;
    try {
      for (const w of writes) await this.core.call("write", w.indices ? { path: w.path, indices: w.indices, value: w.value } : w);
    } catch (e) {
      ok = false;
      this.set({ error: message(e) });
    }
    try {
      const snapshot = await this.core.call("state", undefined);
      this.set(ok ? { snapshot, error: "" } : { snapshot });
    } catch (e) {
      if (ok) this.set({ error: message(e) });
      ok = false;
    }
    return ok;
  }

  /**
   * Save the live state to `slot`. (c6p WB6a-3) A slot without a name is
   * named "Preset N" first, as the Console and the Qt app do.
   */
  async savePreset(slot: number): Promise<boolean> {
    const name = this.state.snapshot?.state.presetNames[slot] ?? "";
    if (!name.trim() && !(await this.write("preset.name", { text: `Preset ${slot + 1}` }, [slot]))) return false;
    return this.write("preset.save", "trigger", [slot]);
  }

  /** Whether anything is unsaved (`unsaved`); the page's leave prompt and a host page ask this. */
  get dirty(): boolean {
    return this.state.unsaved.length > 0;
  }

  private unsavedAsked: DeviceSnapshot | null = null;
  private unsavedRunning = false;

  /**
   * After a new snapshot: what is unsaved now (`presetChanges`, the core's
   * comparison, no device I/O). One call at a time; a snapshot that comes
   * meanwhile is asked about once that one is answered.
   */
  private async refreshUnsaved(): Promise<void> {
    if (this.unsavedRunning) return;
    this.unsavedRunning = true;
    try {
      while (this.unsavedAsked !== this.state.snapshot) {
        const snap = this.state.snapshot;
        this.unsavedAsked = snap;
        if (!snap || this.readOnlyReason || this.state.status !== "open") {
          if (this.state.unsaved.length) this.set({ unsaved: [] });
          continue;
        }
        try {
          const r = await this.core.call("presetChanges", undefined);
          if (!sameLines(r.changes, this.state.unsaved)) this.set({ unsaved: r.changes });
        } catch {
          // Kept: the last answer stands.
        }
      }
    } finally {
      this.unsavedRunning = false;
    }
  }

  // -- WP WB8 -- undo and redo through the core's journal (P0W)

  /** Step back (`forward` false) or forward through the core's journal, then the read-back. */
  async step(forward: boolean): Promise<void> {
    if (this.readOnlyReason || this.state.status !== "open") return;
    try {
      const m = await this.core.call(forward ? "redo" : "undo", undefined);
      const snapshot = await this.core.call("state", undefined);
      this.set({ snapshot, error: "", notice: await this.stepNotice(m) });
    } catch (e) {
      this.set({ error: message(e) });
      void this.refreshHistory();
    }
  }

  undo(): Promise<void> {
    return this.step(false);
  }

  redo(): Promise<void> {
    return this.step(true);
  }

  /** The words for what a step left alone: the core's note naming the other writer, else what it stepped over. */
  private async stepNotice(m: HistoryMove): Promise<SessionState["notice"]> {
    if (m.note) return { text: capitalise(m.note), tone: "warn" };
    if (!m.skipped.length) return null;
    const names = await Promise.all(m.skipped.map((e) => this.changeLabel(e.path)));
    return {
      text: names.length === 1 ? `Stepped over a change that cannot be undone: ${names[0]}` : `Stepped over changes that cannot be undone: ${names.join("; ")}`,
      tone: "info",
    };
  }

  /** A journal path in the registry's words (its label), as the core's notes name it. */
  private async changeLabel(path: string): Promise<string> {
    if (path === "eq.band") return "Filter band";
    try {
      return (await this.core.call("describe", { path })).label || path;
    } catch {
      return path;
    }
  }

  dismissNotice(): void {
    if (this.state.notice) this.set({ notice: null });
  }

  private drags = 0;
  private dragSent = false;

  /**
   * A continuous drag (a slider, a graph handle) starts or ends: the core joins
   * its writes into one undo step. Counted, so two pointers at once end the
   * mark only when both have let go. Nothing is sent while read only.
   */
  setDragging(on: boolean): void {
    if (on) {
      if (this.drags++ === 0) {
        this.watchDragEnd(true);
        if (!this.readOnlyReason) {
          this.dragSent = true;
          void this.core.call("setDragging", { dragging: true }).catch(() => undefined);
        }
      }
    } else if (this.drags > 0 && --this.drags === 0) this.endDragMark();
  }

  // -- fixup wb8 -- a safety net: the page hidden or the window left mid-drag ends the
  // mark, whatever the controls hear (a browser may deliver no pointer event then), so a
  // typed value afterwards is never joined into the drag's step. The controls' own later
  // end is then a no-op.
  private readonly dragEndNet = () => {
    if (typeof document !== "undefined" && document.visibilityState === "visible" && document.hasFocus?.()) return;
    if (this.drags > 0) {
      this.drags = 0;
      this.endDragMark();
    }
  };

  private watchDragEnd(on: boolean): void {
    if (typeof window === "undefined" || typeof document === "undefined") return;
    if (on) {
      window.addEventListener("blur", this.dragEndNet);
      document.addEventListener("visibilitychange", this.dragEndNet);
    } else {
      window.removeEventListener("blur", this.dragEndNet);
      document.removeEventListener("visibilitychange", this.dragEndNet);
    }
  }

  private endDragMark(): void {
    this.watchDragEnd(false);
    if (!this.dragSent) return;
    this.dragSent = false;
    void this.core.call("setDragging", { dragging: false }).catch(() => undefined);
  }

  private historyAsked: DeviceSnapshot | null = null;
  private historyRunning = false;

  /** After a new snapshot: what undo and redo can do now (no device I/O). One call at a time, as `refreshUnsaved`. */
  private async refreshHistory(): Promise<void> {
    if (this.historyRunning) return;
    this.historyRunning = true;
    try {
      do {
        const snap = this.state.snapshot;
        this.historyAsked = snap;
        const none = { canUndo: false, canRedo: false };
        let next = none;
        if (snap && !this.readOnlyReason && this.state.status === "open") {
          try {
            next = (await this.core.call("canUndo", undefined)) ?? this.state.history;
          } catch {
            next = this.state.history;
          }
        }
        const h = this.state.history;
        if (next.canUndo !== h.canUndo || next.canRedo !== h.canRedo) this.set({ history: { canUndo: next.canUndo, canRedo: next.canRedo } });
      } while (this.historyAsked !== this.state.snapshot);
    } finally {
      this.historyRunning = false;
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
    // -- WP WB6b --
    if (this.state.snapshot !== this.unsavedAsked) void this.refreshUnsaved();
    // -- WP WB8 --
    if (this.state.snapshot !== this.historyAsked) void this.refreshHistory();
  }
}

function message(e: unknown): string {
  if (e instanceof CoreCallError) return capitalise(e.message);
  return e instanceof Error ? e.message : String(e);
}

function sameLines(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((x, i) => x === b[i]);
}

function capitalise(s: string): string {
  return s ? s[0]!.toUpperCase() + s.slice(1) : s;
}
