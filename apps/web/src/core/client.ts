// The page's side of a core Worker (WP WB5). One CoreClient is one device
// session. The page may hold many at once (a fleet view); they share nothing.

import {
  CoreCallError,
  type CoreEvent,
  type CoreMethods,
  type FromWorker,
  type Method,
  type Request,
} from "./protocol";

/** What the client needs of a Worker; tests pass an in-process one. */
export interface WorkerLike {
  postMessage(m: Request): void;
  onmessage: ((ev: MessageEvent<FromWorker>) => void) | null;
  terminate(): void;
}

export class CoreClient {
  private next = 1;
  private readonly pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  private readonly listeners = new Set<(e: CoreEvent) => void>();
  private closed = false;

  constructor(private readonly worker: WorkerLike) {
    worker.onmessage = (ev) => this.receive(ev.data);
  }

  static spawn(workerUrl: string | URL): CoreClient {
    const w = new Worker(workerUrl, { type: "module", name: "dspi-core" }) as unknown as WorkerLike;
    return new CoreClient(w);
  }

  call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    if (this.closed) return Promise.reject(new CoreCallError("not_open", "This session is closed"));
    const id = this.next++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.worker.postMessage({ id, method, params });
    });
  }

  subscribe(fn: (e: CoreEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /**
   * Closes the session and stops its Worker. Pending calls fail with `not_open`.
   * A core call that never returns (a hung transfer) holds the close behind it,
   * so the Worker stops after `closeWaitMs` whatever happened (c6a F-4).
   */
  async dispose(closeWaitMs = 2000): Promise<void> {
    if (this.closed) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([this.call("close", undefined), new Promise((r) => (timer = setTimeout(r, closeWaitMs)))]);
    } catch {
      // The session may already be gone; the Worker stops either way.
    } finally {
      clearTimeout(timer);
    }
    this.closed = true;
    this.worker.terminate();
    for (const p of this.pending.values()) p.reject(new CoreCallError("not_open", "This session is closed"));
    this.pending.clear();
    this.listeners.clear();
  }

  private receive(m: FromWorker): void {
    if ("push" in m) {
      for (const l of this.listeners) l(m.push);
      return;
    }
    const p = this.pending.get(m.reply.id);
    if (!p) return;
    this.pending.delete(m.reply.id);
    if (m.reply.ok) p.resolve(m.reply.value);
    else p.reject(new CoreCallError(m.reply.error.code, m.reply.error.message, m.reply.error.noticeKey, m.reply.error.text)); // -- WP P1WB1 --, INTE
  }
}
