// The Worker's WebSocket shim (WP WB4b): the browser's WebSocket as the core's
// socket imports (crates/dspi-wasm/src/link.rs), so DSPi Link runs in our Rust
// client (dspi-link) under the core's own session. A thin pipe: whole messages
// in and out, the open and the close. Nothing of the protocol lives here.
//
// The module reads a socket only while a call into it is running: a call that
// waits for the hub suspends in `dspi_ws_wait` (JSPI) until a message comes; in
// between, messages wait in the socket's queue and `onActivity` asks the Worker
// to turn the session (`linkTick`).
//
// What a page cannot do, and what stands in for it: a page cannot send a
// WebSocket ping (the core sends a request the hub answers instead), may close
// only with 1000 or 3000-4999 (the core maps the rest), and cannot see why an
// upgrade failed: a refused `Origin` (a hub accepts only its own pages, Link spec
// 4), a full hub and an unreachable one all look alike. The error says so.

/** What `dspi_ws_poll` / `dspi_ws_wait` answer (link.rs `next`). */
const IDLE = 0;
const TEXT = 1;
const BINARY = 2;
const CLOSED = 3;

/** The open (name lookup, TCP, upgrade) gives up after this, as the core's own connector does. */
const OPEN_TIMEOUT_MS = 5000;
/** Largest message kept (dspi-link's limit). */
const MAX_MESSAGE = 1 << 20;

/** The constructor the shim uses; tests pass their own. */
export type WebSocketLike = {
  new (url: string, protocols?: string | string[]): WebSocket;
};

interface Sock {
  ws: WebSocket;
  queue: { kind: number; bytes: Uint8Array }[];
  /** The close code; -1 when it ended without one (a drop); null while open. */
  closed: number | null;
  waiter: (() => void) | null;
}

export class WsHost {
  private readonly socks = new Map<number, Sock>();
  private next = 1;
  private lastError = "";
  private memory: () => WebAssembly.Memory = () => {
    throw new Error("no module memory");
  };
  private readonly activity = new Set<() => void>();

  constructor(private readonly Socket: WebSocketLike | undefined = globalThis.WebSocket) {}

  bindMemory(m: () => WebAssembly.Memory): void {
    this.memory = m;
  }

  /** `fn` is called when a socket has something for the core and nobody is reading. Returns the way to stop. */
  onActivity(fn: () => void): () => void {
    this.activity.add(fn);
    return () => this.activity.delete(fn);
  }

  /** Sockets open now, for tests. */
  get openCount(): number {
    return this.socks.size;
  }

  imports(Suspending: new (f: (...a: number[]) => Promise<number>) => unknown): Record<string, unknown> {
    return {
      dspi_ws_open: new Suspending((u, ul, p, pl) => this.open(this.text(u, ul), this.text(p, pl))),
      dspi_ws_error: (out: number, cap: number) => this.put(out, cap, new TextEncoder().encode(this.lastError)),
      dspi_ws_send: (sock: number, binary: number, ptr: number, len: number) => this.send(sock, binary !== 0, ptr, len),
      dspi_ws_close: (sock: number, code: number) => this.close(sock, code),
      dspi_ws_poll: (sock: number) => this.peek(sock),
      dspi_ws_wait: new Suspending((sock, ms) => this.wait(sock, ms)),
      dspi_ws_len: (sock: number) => this.socks.get(sock)?.queue[0]?.bytes.length ?? 0,
      dspi_ws_take: (sock: number, out: number, cap: number) => this.take(sock, out, cap),
      dspi_ws_closed: (sock: number) => this.closedCode(sock),
    };
  }

  private open(url: string, protocol: string): Promise<number> {
    const Socket = this.Socket;
    if (!Socket) return Promise.resolve(this.fail("This browser has no WebSocket"));
    let ws: WebSocket;
    try {
      ws = new Socket(url, protocol);
    } catch (e) {
      // A malformed address, or a page that may not open ws:// at all.
      return Promise.resolve(this.fail(`Cannot open a connection to ${hostOf(url)}: ${e instanceof Error ? e.message : String(e)}`));
    }
    ws.binaryType = "arraybuffer";
    const id = this.next++;
    const s: Sock = { ws, queue: [], closed: null, waiter: null };
    return new Promise((resolve) => {
      let settled = false;
      const settle = (v: number) => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        resolve(v);
      };
      // -- WP WB7 -- (c6m N3) the text of a failed open, set only while the open is
      // still unsettled: a later close of an open socket keeps the first failure's words.
      const failOpen = (why: string) => {
        if (!settled) settle(this.fail(why));
      };
      const timer = setTimeout(() => {
        ws.close();
        failOpen(`The hub at ${hostOf(url)} did not answer`);
      }, OPEN_TIMEOUT_MS);
      ws.onopen = () => {
        if (ws.protocol !== protocol) {
          ws.close(1000);
          failOpen(`The server at ${hostOf(url)} is not a DSPi hub`);
          return;
        }
        this.socks.set(id, s);
        settle(id);
      };
      ws.onmessage = (ev: MessageEvent) => {
        const bytes =
          typeof ev.data === "string" ? new TextEncoder().encode(ev.data) : ev.data instanceof ArrayBuffer ? new Uint8Array(ev.data) : null;
        if (!bytes || bytes.length > MAX_MESSAGE) return;
        s.queue.push({ kind: typeof ev.data === "string" ? TEXT : BINARY, bytes });
        this.wake(s);
      };
      ws.onclose = (ev: CloseEvent) => {
        // 1005: no code sent; 1006: no close frame at all (a drop). A hub's own
        // code counts even when the TCP close was not clean (Firefox says so when
        // the hub hangs up right after its close frame): 4000 must end the session.
        s.closed = ev.code === 1005 || ev.code === 1006 ? -1 : ev.code;
        failOpen(
          `Cannot connect to the hub at ${hostOf(url)}. It may be off or busy, or it refuses this page: ` +
            "a hub accepts only pages it serves itself.",
        );
        this.wake(s);
      };
      ws.onerror = () => {
        // A close event follows and says the rest.
      };
    });
  }

  private fail(why: string): number {
    this.lastError = why;
    return -1;
  }

  private wake(s: Sock): void {
    const w = s.waiter;
    s.waiter = null;
    if (w) w();
    else for (const fn of this.activity) fn();
  }

  private peek(sock: number): number {
    const s = this.socks.get(sock);
    if (!s) return CLOSED;
    if (s.queue.length) return s.queue[0]!.kind;
    return s.closed == null ? IDLE : CLOSED;
  }

  private wait(sock: number, ms: number): Promise<number> {
    const now = this.peek(sock);
    const s = this.socks.get(sock);
    if (now !== IDLE || !s) return Promise.resolve(now);
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        s.waiter = null;
        resolve(this.peek(sock));
      }, ms);
      s.waiter = () => {
        clearTimeout(timer);
        resolve(this.peek(sock));
      };
    });
  }

  private take(sock: number, out: number, cap: number): number {
    const m = this.socks.get(sock)?.queue.shift();
    if (!m || m.bytes.length > cap) return -1;
    new Uint8Array(this.memory().buffer, out, m.bytes.length).set(m.bytes);
    return m.bytes.length;
  }

  private send(sock: number, binary: boolean, ptr: number, len: number): number {
    const s = this.socks.get(sock);
    if (!s || s.closed != null || s.ws.readyState !== 1) return -1;
    const bytes = new Uint8Array(this.memory().buffer, ptr, len).slice();
    try {
      s.ws.send(binary ? bytes : new TextDecoder().decode(bytes));
      return 0;
    } catch {
      return -1;
    }
  }

  private close(sock: number, code: number): void {
    const s = this.socks.get(sock);
    if (!s) return;
    this.socks.delete(sock);
    s.closed ??= code;
    try {
      s.ws.close(code === 1000 || (code >= 3000 && code <= 4999) ? code : 1000);
    } catch {
      // Already closing.
    }
  }

  private closedCode(sock: number): number {
    return this.socks.get(sock)?.closed ?? 1000;
  }

  private text(ptr: number, len: number): string {
    return new TextDecoder().decode(new Uint8Array(this.memory().buffer, ptr, len));
  }

  private put(out: number, cap: number, bytes: Uint8Array): number {
    const n = Math.min(cap, bytes.length);
    new Uint8Array(this.memory().buffer, out, n).set(bytes.subarray(0, n));
    return n;
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
