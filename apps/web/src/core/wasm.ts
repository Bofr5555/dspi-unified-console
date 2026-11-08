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
// A panic in the core traps. The module has marked itself dead first; the
// session reports `lost`, and every later call is refused as `dead`.

import { CoreCallError, type CoreEvent, type CoreImpl, type CoreMethods, type DeviceSnapshot, type Method } from "./protocol";

/** `dspi_call`'s answers when the call did not run (crates/dspi-wasm/src/lib.rs). */
const CALL_BUSY = 0xffff_ffff;
const CALL_DEAD = 0xffff_fffe;

/** `dspi_core::consts::NOTIFY_DISCONNECTED`. */
const NOTIFY_DISCONNECTED = 1 << 9;

/** How often the Worker drains notifications while a device is open, ms. */
const POLL_MS = 100;

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

/** Instantiate the module from its bytes or its URL. One module per Worker. */
export async function loadWasmModule(source: URL | string | BufferSource): Promise<WasmModule> {
  const missing = jspiMissing();
  if (missing) throw new Error(missing);
  const bytes =
    source instanceof URL || typeof source === "string" ? await (await fetch(source)).arrayBuffer() : source;
  const J = WebAssembly as unknown as {
    Suspending: new (f: (...a: number[]) => Promise<unknown>) => unknown;
    promising: <F extends (...a: never[]) => unknown>(f: F) => (...a: Parameters<F>) => Promise<ReturnType<F>>;
  };
  let memory: WebAssembly.Memory | null = null;
  const imports = {
    dspi: {
      dspi_now_ms: () => performance.now(),
      dspi_unix_ms: () => Date.now(),
      dspi_sleep_ms: new J.Suspending((ms: number) => new Promise((r) => setTimeout(r, ms))),
      dspi_random_fill: (ptr: number, len: number) => {
        for (let at = 0; at < len; at += 65536) {
          crypto.getRandomValues(new Uint8Array(memory!.buffer, ptr + at, Math.min(65536, len - at)));
        }
        return 0;
      },
    },
  };
  const { instance } = await WebAssembly.instantiate(bytes, imports as unknown as WebAssembly.Imports);
  const x = instance.exports as unknown as Exports;
  memory = x.memory;
  return new WasmModule(x, J.promising(x.dspi_call));
}

/** One instance of crates/dspi-wasm: may hold several sessions. */
export class WasmModule {
  private deadReason = "";

  constructor(
    private readonly x: Exports,
    private readonly promisedCall: (h: number, ptr: number, len: number) => Promise<number>,
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
    const reply = JSON.parse(text) as { ok: true; value: unknown } | { ok: false; code: string; error: string };
    if (!reply.ok) {
      if (reply.code === "dead") this.deadReason = reply.error;
      throw new CoreCallError(reply.code, reply.error);
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
  readonly handle: number;

  constructor(private readonly m: WasmModule) {
    this.handle = m.newSession();
  }

  onEvent(push: (e: CoreEvent) => void): void {
    this.push = push;
  }

  async call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    try {
      const value = await this.queued(method, params);
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
    this.open = false;
    this.m.freeSession(this.handle);
  }

  private queued(method: string, params: unknown): Promise<unknown> {
    const run = this.tail.then(() => this.m.call(this.handle, method, params));
    this.tail = run.catch(() => undefined);
    return run;
  }

  private after(method: Method): void {
    if (method === "open") {
      this.open = true;
      this.lostSent = false;
      this.schedulePoll(POLL_MS);
    } else if (method === "close") {
      this.open = false;
      this.stopTimers();
    } else if (method === "rtaStart") {
      this.rtaOn = true;
      this.scheduleRta(0);
    } else if (method === "rtaStop") {
      this.rtaOn = false;
      clearTimeout(this.rtaTimer);
    }
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
      const r = (await this.queued("poll", null)) as { flags: number; retryMs: number | null; connected: boolean };
      if (!r.connected || r.flags & NOTIFY_DISCONNECTED) {
        this.lose("The DSPi is no longer connected");
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

  private lose(reason: string): void {
    this.open = false;
    this.stopTimers();
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
