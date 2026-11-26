// WP GAP2: "Solo this output" on the web. The core keeps the solo (`soloOutput`, `soloEnd`, `soloState`):
// the other outputs muted on the device only, never saved, not an undo step; a save, a preset load, a recall
// or a disconnect ends it. One store per session follows it: asked again after every new snapshot, as the
// unsaved list is (no device I/O).

import { useEffect, useState } from "preact/hooks";
import type { DeviceSnapshot } from "../core/protocol";
import type { Session } from "./session";

export class SoloStore {
  /** The soloed output (from 0), null with none. */
  output: number | null = null;
  private asked: DeviceSnapshot | null | undefined = undefined;
  private running = false;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly session: Session) {
    session.watch(() => void this.follow());
    void this.follow();
  }

  /** Why a solo cannot be started here, "" when it can. */
  get reason(): string {
    const s = this.session;
    if (s.readOnlyReason) return s.readOnlyReason;
    if (s.state.status !== "open" || !s.state.snapshot || s.document) return "No DSPi is open";
    return s.away ? "The DSPi is away" : "";
  }

  /** Only `output` plays. */
  solo(output: number): Promise<void> {
    return this.ask(() => this.session.core.call("soloOutput", { output }));
  }

  /** Every output plays as the user left it. */
  end(): Promise<void> {
    return this.ask(() => this.session.core.call("soloEnd", undefined));
  }

  /** Solo `output`, or end the solo when it is the soloed one. */
  toggle(output: number): Promise<void> {
    return this.output === output ? this.end() : this.solo(output);
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private async ask(call: () => Promise<{ output: number | null }>): Promise<void> {
    try {
      this.set((await call()).output);
    } catch (e) {
      this.session.report(e instanceof Error ? e.message : String(e));
    }
    // The device's state as the solo left it (the mirror keeps the user's mutes)
    await this.session.refresh();
  }

  /** After a new snapshot: whether the solo still holds. One call at a time. */
  private async follow(): Promise<void> {
    if (this.running) return;
    this.running = true;
    try {
      while (this.asked !== this.session.state.snapshot) {
        const snap = this.session.state.snapshot;
        this.asked = snap;
        if (!snap || this.session.state.status !== "open") {
          this.set(null);
          continue;
        }
        try {
          this.set((await this.session.core.call("soloState", undefined)).output);
        } catch {
          this.set(null);
        }
      }
    } finally {
      this.running = false;
    }
  }

  private set(output: number | null): void {
    if (output === this.output) return;
    this.output = output;
    for (const l of this.listeners) l();
  }
}

const stores = new WeakMap<Session, SoloStore>();

/** The session's solo store, made on first use. */
export function soloOf(session: Session): SoloStore {
  let store = stores.get(session);
  if (!store) {
    store = new SoloStore(session);
    stores.set(session, store);
  }
  return store;
}

/** The session's solo store, the component drawn again when the solo changes. */
export function useSolo(session: Session): SoloStore {
  const store = soloOf(session);
  const [, bump] = useState(0);
  useEffect(() => {
    const seen = store.output;
    const stop = store.watch(() => bump((n) => n + 1));
    if (store.output !== seen) bump((n) => n + 1);
    return stop;
  }, [store]);
  return store;
}
