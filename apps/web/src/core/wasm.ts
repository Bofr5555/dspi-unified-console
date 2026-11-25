// The real core in the Worker (WP WB3): crates/dspi-wasm under JSPI.
//
// The module exports one call, `dspi_call(session, json) -> reply length`; this
// file is its glue: the four imports the core's seams use, JSON in and out, and
// the Worker's own timers, because the core is caller-driven on the web (WB2):
// `poll` drains notifications, `rtaTick` turns the analyser. Every call into the
// module, the page's and the timers', goes through one queue here, so the module
// is entered by one call at a time. The module refuses a second one anyway
// (`busy`), which `unqueued` lets a test show.
//
// USB (WB4a, usb.ts): an `open` of `{kind: "usb", serial}` first claims the
// device here, then opens it in the core over the module's USB imports;
// notification packets wait in `packets` and go in (`usbPush`) ahead of the next
// `poll`; an unplug ends the session (`usbGone`) in the core's words.
//
// Link (WB4b, ws.ts): the page drives the core's choose_device flow with
// `linkConnect`, `linkPair`, `linkDevices` and an `open` of `{kind: "hub"}`. Once
// connected, the Worker turns the session (`linkTick`) at the wait the core asks
// for and whenever the socket has something, and pushes a `link` event when the
// phase changes: the banner's phases are the core's own.
//
// A panic in the core traps. The module has marked itself dead first; the
// session reports `lost`, and every later call is refused as `dead`.

import {
  CoreCallError,
  type CoreEvent,
  type CoreImpl,
  type CoreMethods,
  type DeviceSnapshot,
  type DeviceTarget,
  type HubAdminEventRecord, // -- WP HUBADMIN --
  type HubServerEventRecord, // -- WP P2WB1 --
  type LinkPhase,
  type LinkStatusRecord,
  type Method,
} from "./protocol";
// -- WP WB4a --
import { UsbHost } from "./usb";
// -- WP WB4b --
import { WsHost } from "./ws";

/** `dspi_call`'s answers when the call did not run (crates/dspi-wasm/src/lib.rs). */
const CALL_BUSY = 0xffff_ffff;
const CALL_DEAD = 0xffff_fffe;

/** `dspi_core::consts::NOTIFY_DISCONNECTED`. */
const NOTIFY_DISCONNECTED = 1 << 9;

/** How often the Worker drains notifications while a device is open, ms. */
const POLL_MS = 100;
/** The longest a Link session goes without a turn, ms (the core asks for less). */
const LINK_MAX_MS = 1000;

interface Exports {
  memory: WebAssembly.Memory;
  dspi_alloc(len: number): number;
  dspi_free(ptr: number, len: number): void;
  dspi_session_new(): number;
  dspi_session_free(h: number): number;
  dspi_call(h: number, ptr: number, len: number): number;
  dspi_reply_ptr(): number;
  dspi_panic_ptr(): number;
  dspi_panic_len(): number;
}

/** Why the page cannot run the core, or "" when it can. */
export function jspiMissing(): string {
  const w = WebAssembly as unknown as { Suspending?: unknown; promising?: unknown };
  return typeof w.Suspending === "function" && typeof w.promising === "function"
    ? ""
    : "This browser cannot run the DSPi core: it needs WebAssembly JavaScript Promise Integration";
}

/** Instantiate the module from its bytes or its URL. One module per Worker. `usb`: the Worker's WebUSB (WB4a). */
export async function loadWasmModule(
  source: URL | string | BufferSource,
  usb: UsbHost = new UsbHost(),
  ws: WsHost = new WsHost(),
): Promise<WasmModule> {
  const missing = jspiMissing();
  if (missing) throw new Error(missing);
  const bytes =
    source instanceof URL || typeof source === "string" ? await (await fetch(source)).arrayBuffer() : source;
  const J = WebAssembly as unknown as {
    Suspending: new (f: (...a: number[]) => Promise<number>) => unknown;
    promising: <F extends (...a: never[]) => unknown>(f: F) => (...a: Parameters<F>) => Promise<ReturnType<F>>;
  };
  let memory: WebAssembly.Memory | null = null;
  const imports = {
    dspi: {
      dspi_now_ms: () => performance.now(),
      dspi_unix_ms: () => Date.now(),
      dspi_sleep_ms: new J.Suspending((ms: number) => new Promise((r) => setTimeout(() => r(0), ms))),
      dspi_random_fill: (ptr: number, len: number) => {
        for (let at = 0; at < len; at += 65536) {
          crypto.getRandomValues(new Uint8Array(memory!.buffer, ptr + at, Math.min(65536, len - at)));
        }
        return 0;
      },
      // -- WP WB4a --
      ...usb.imports(J.Suspending),
      // -- WP WB4b --
      ...ws.imports(J.Suspending),
    },
  };
  usb.bindMemory(() => memory!);
  ws.bindMemory(() => memory!);
  const { instance } = await WebAssembly.instantiate(bytes, imports as unknown as WebAssembly.Imports);
  const x = instance.exports as unknown as Exports;
  memory = x.memory;
  return new WasmModule(x, J.promising(x.dspi_call), usb, ws);
}

/** One instance of crates/dspi-wasm: may hold several sessions. */
export class WasmModule {
  private deadReason = "";

  constructor(
    private readonly x: Exports,
    private readonly promisedCall: (h: number, ptr: number, len: number) => Promise<number>,
    // -- WP WB4a --
    readonly usb: UsbHost = new UsbHost(),
    // -- WP WB4b --
    readonly ws: WsHost = new WsHost(),
  ) {}

  /** Empty while alive; why the core stopped otherwise. */
  get dead(): string {
    return this.deadReason;
  }

  newSession(): number {
    const h = this.x.dspi_session_new() >>> 0;
    if (h === 0xffff_ffff) throw new CoreCallError(this.deadReason ? "dead" : "busy", this.deadReason || "the core is already running a call");
    return h;
  }

  freeSession(h: number): void {
    if (!this.deadReason) this.x.dspi_session_free(h);
  }

  /** One call, straight into the module: no queue. Callers queue (WasmCore). */
  async call(h: number, method: string, params: unknown): Promise<unknown> {
    if (this.deadReason) throw new CoreCallError("dead", this.deadReason);
    const req = new TextEncoder().encode(JSON.stringify({ method, params: params ?? null }));
    const ptr = this.x.dspi_alloc(req.length);
    new Uint8Array(this.x.memory.buffer, ptr, req.length).set(req);
    let n: number;
    try {
      n = await this.promisedCall(h, ptr, req.length);
    } catch (e) {
      // A trap: the core panicked (its hook marked the module dead) or worse.
      const len = this.x.dspi_panic_len();
      const why = len
        ? new TextDecoder().decode(new Uint8Array(this.x.memory.buffer, this.x.dspi_panic_ptr(), len))
        : e instanceof Error
          ? e.message
          : String(e);
      this.deadReason = `the core stopped after an internal error: ${why}`;
      throw new CoreCallError("dead", this.deadReason);
    } finally {
      if (!this.deadReason) this.x.dspi_free(ptr, req.length);
    }
    n >>>= 0;
    if (n === CALL_BUSY) throw new CoreCallError("busy", "the core is already running a call");
    if (n === CALL_DEAD) throw new CoreCallError("dead", this.deadReason || "the core stopped after an internal error");
    const text = new TextDecoder().decode(new Uint8Array(this.x.memory.buffer, this.x.dspi_reply_ptr(), n));
    const reply = JSON.parse(text) as { ok: true; value: unknown } | { ok: false; code: string; error: string; noticeKey?: string; text?: string };
    if (!reply.ok) {
      if (reply.code === "dead") this.deadReason = reply.error;
      throw new CoreCallError(reply.code, reply.error, reply.noticeKey, reply.text); // -- WP P1WB1 -- with its notice key
    }
    return reply.value;
  }
}

/** A device session on the real core: the Worker's CoreImpl. */
export class WasmCore implements CoreImpl {
  private push: (e: CoreEvent) => void = () => {};
  private tail: Promise<unknown> = Promise.resolve();
  private pollTimer: ReturnType<typeof setTimeout> | undefined;
  private rtaTimer: ReturnType<typeof setTimeout> | undefined;
  private open = false;
  private rtaOn = false;
  private lostSent = false;
  // -- WP WB4a -- the claimed USB device (usb.ts), and its packets not yet pushed in
  private usbDevice: number | null = null;
  private packets: number[][] = [];
  // -- WP WB4b -- a Link session's turns, and the phase last told to the page
  private linkTimer: ReturnType<typeof setTimeout> | undefined;
  private linkOn = false;
  private linkDue = Infinity;
  private phase = "";
  private linkReason = "";
  private readonly stopActivity: () => void;
  readonly handle: number;

  constructor(private readonly m: WasmModule) {
    this.handle = m.newSession();
    this.stopActivity = m.ws.onActivity(() => this.scheduleLink(0));
  }

  onEvent(push: (e: CoreEvent) => void): void {
    this.push = push;
  }

  async call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    try {
      // -- WP WB4b -- (c6g N1) a session that opens something else gives its USB device back
      if (method === "open" && (params as DeviceTarget).kind !== "usb") this.releaseUsb();
      // -- WP P1WB1 -- a document replaces the device as `open` does
      if (method === "documentOpen") this.releaseUsb();
      const value =
        method === "open" && (params as DeviceTarget).kind === "usb"
          ? await this.openUsb(params as DeviceTarget & { kind: "usb" })
          : await this.queued(method, params);
      this.after(method);
      return value as CoreMethods[M]["result"];
    } catch (e) {
      if (e instanceof CoreCallError && e.code === "dead") this.lose(e.message);
      throw e;
    }
  }

  /** Tests only: a call that skips the queue, so the module's own guard is what stands in the way. */
  unqueued(method: string, params: unknown): Promise<unknown> {
    return this.m.call(this.handle, method, params);
  }

  /** Stops the timers and frees the session. */
  dispose(): void {
    this.stopTimers();
    this.stopLink();
    this.stopActivity();
    this.releaseUsb();
    this.open = false;
    this.m.freeSession(this.handle);
  }

  private queued(method: string, params: unknown): Promise<unknown> {
    const run = this.tail.then(() => this.m.call(this.handle, method, params));
    this.tail = run.catch(() => undefined);
    return run;
  }

  private after(method: Method): void {
    // -- WP P1WB1 -- an offline document is polled as a device is
    if (method === "open" || method === "documentOpen") {
      this.open = true;
      this.lostSent = false;
      this.schedulePoll(POLL_MS);
      // -- WP FLAKE1 -- a DSPi opened over Link is bound now: say so at once, not at the next
      // Link turn (up to LINK_MAX_MS later), so "choose a DSPi" does not stay over the open device
      this.scheduleLink(0);
    } else if (method === "close") {
      this.open = false;
      this.stopTimers();
      this.releaseUsb();
      // -- WP WB4b --
      this.stopLink();
    } else if (method === "linkConnect") {
      // -- WP WB4b --
      this.linkOn = true;
      this.phase = "";
      this.scheduleLink(0);
    } else if (method === "rtaStart") {
      this.rtaOn = true;
      this.scheduleRta(0);
    } else if (method === "rtaStop") {
      this.rtaOn = false;
      clearTimeout(this.rtaTimer);
    }
  }

  // -- WP WB4a --
  /**
   * Claim the chosen DSPi, then open it in the core, in the queue as one step so
   * no other call runs between. A failed open gives the device back.
   */
  private openUsb(t: DeviceTarget & { kind: "usb" }): Promise<unknown> {
    const run = this.tail.then(async () => {
      this.releaseUsb();
      if (!t.serial) return this.m.call(this.handle, "open", t);
      const claimed = await this.m.usb.claim(t.serial, () => void this.usbGone(claimed.device));
      try {
        const snap = await this.m.call(this.handle, "open", { kind: "usb", ...claimed });
        this.usbDevice = claimed.device;
        this.packets = [];
        this.m.usb.readNotifications(claimed.device, (p) => {
          this.packets.push(p);
          this.schedulePoll(0);
        });
        return snap;
      } catch (e) {
        await this.m.usb.release(claimed.device);
        throw e;
      }
    });
    this.tail = run.catch(() => undefined);
    return run;
  }

  /** The device left: the core ends the connection and says so in its words. */
  private async usbGone(device: number): Promise<void> {
    if (this.usbDevice !== device) return;
    this.releaseUsb();
    // No more turns: a poll after this would report the end in other words.
    this.open = false;
    this.stopTimers();
    try {
      const r = (await this.queued("usbGone", null)) as { reason: string };
      this.lose(r.reason);
    } catch (e) {
      this.lose(e instanceof Error ? e.message : String(e));
    }
  }

  private releaseUsb(): void {
    if (this.usbDevice == null) return;
    void this.m.usb.release(this.usbDevice);
    this.usbDevice = null;
    this.packets = [];
  }

  private schedulePoll(ms: number): void {
    clearTimeout(this.pollTimer);
    if (this.open) this.pollTimer = setTimeout(() => void this.pollOnce(), ms);
  }

  private scheduleRta(ms: number): void {
    clearTimeout(this.rtaTimer);
    if (this.open && this.rtaOn) this.rtaTimer = setTimeout(() => void this.rtaOnce(), ms);
  }

  /** The notification turn: what changed, then the new snapshot for the page. */
  private async pollOnce(): Promise<void> {
    if (!this.open) return;
    try {
      // -- WP WB4a -- the packets read since the last turn, then the turn.
      if (this.packets.length) {
        const packets = this.packets;
        this.packets = [];
        await this.queued("usbPush", { packets });
      }
      const r = (await this.queued("poll", null)) as { flags: number; retryMs: number | null; connected: boolean };
      // -- WP WB4b -- (c6g N2) ended meanwhile (an unplug): that path has the core's words
      if (!this.open) return;
      if (!r.connected || r.flags & NOTIFY_DISCONNECTED) {
        // -- WP WB4b -- a hub session's end is its phase, in the core's words
        if (this.linkOn) await this.linkOnce();
        this.lose(this.linkReason || "The DSPi is no longer connected");
        return;
      }
      if (r.flags) {
        const snapshot = (await this.queued("state", null)) as DeviceSnapshot;
        this.push({ event: "state", snapshot, flags: r.flags });
      }
      this.schedulePoll(r.retryMs == null ? POLL_MS : Math.min(POLL_MS, r.retryMs));
    } catch (e) {
      if (e instanceof CoreCallError && e.code === "dead") this.lose(e.message);
      else this.schedulePoll(POLL_MS);
    }
  }

  private async rtaOnce(): Promise<void> {
    if (!this.open || !this.rtaOn) return;
    try {
      const r = (await this.queued("rtaTick", null)) as { waitMs: number | null };
      if (r.waitMs == null) this.rtaOn = false;
      else this.scheduleRta(r.waitMs);
    } catch (e) {
      if (e instanceof CoreCallError && e.code === "dead") this.lose(e.message);
      else this.scheduleRta(POLL_MS);
    }
  }

  // -- WP WB4b --
  /** The next Link turn in `ms`, unless one is due sooner. */
  private scheduleLink(ms: number): void {
    if (!this.linkOn) return;
    const due = performance.now() + ms;
    if (this.linkTimer !== undefined && this.linkDue <= due) return;
    clearTimeout(this.linkTimer);
    this.linkDue = due;
    this.linkTimer = setTimeout(() => {
      this.linkTimer = undefined;
      this.linkDue = Infinity;
      void this.linkOnce();
    }, ms);
  }

  private stopLink(): void {
    this.linkOn = false;
    clearTimeout(this.linkTimer);
    this.linkTimer = undefined;
    this.linkDue = Infinity;
  }

  /** One turn of the Link session; a `link` event when its phase changed. */
  private async linkOnce(): Promise<void> {
    if (!this.linkOn) return;
    try {
      const r = (await this.queued("linkTick", null)) as { waitMs: number | null; status: LinkStatusRecord; hubEvents?: HubAdminEventRecord[]; hubServerEvents?: HubServerEventRecord[] /* -- WP P2WB1 -- */ };
      const st = r.status;
      // -- WP HUBADMIN -- the hub's admin events of this turn, for the Installer page
      if (Array.isArray(r.hubEvents) && r.hubEvents.length) this.push({ event: "hubAdmin", events: r.hubEvents });
      // -- WP P2WB1 -- the hub's own families' events of this turn (inventory, history, alarms, ...)
      if (Array.isArray(r.hubServerEvents) && r.hubServerEvents.length) this.push({ event: "hubServer", events: r.hubServerEvents });
      this.linkReason = st.closeReason;
      if (st.phase !== this.phase) {
        this.phase = st.phase;
        this.push({ event: "link", phase: st.phase as LinkPhase, pinSource: st.pinSource, reason: st.closeReason });
      }
      if (r.waitMs == null || st.phase === "idle") this.stopLink();
      else this.scheduleLink(Math.min(LINK_MAX_MS, r.waitMs));
    } catch (e) {
      if (e instanceof CoreCallError && e.code === "dead") this.lose(e.message);
      else this.scheduleLink(LINK_MAX_MS);
    }
  }

  private lose(reason: string): void {
    this.open = false;
    this.stopTimers();
    this.releaseUsb();
    this.stopLink();
    if (this.lostSent) return;
    this.lostSent = true;
    this.push({ event: "lost", reason: capitalise(reason) });
  }

  private stopTimers(): void {
    clearTimeout(this.pollTimer);
    clearTimeout(this.rtaTimer);
    this.rtaOn = false;
  }
}

function capitalise(s: string): string {
  return s ? s[0]!.toUpperCase() + s.slice(1) : s;
}
