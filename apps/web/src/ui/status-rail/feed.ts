// WP WB6f: one session's health feed. It reads the meters (the clip hold and the
// two cores' load) and, while a limiter is on, the limiters' gain reduction,
// every `periodMs` through the session's own core, and follows the session's
// DSPi Link phase; `HealthTracker` keeps what it found until it is acknowledged.
// One read at a time, by the caller's timer: no thread, nothing queued. A stale
// reading (DSPi Link) raises nothing and ends nothing.

import { CoreCallError, type DspState } from "../../core/protocol";
import type { Session } from "../session";
import { meterNames, outputLabel } from "../../screens/main/model";
import { HealthTracker, conditions, readKinds, type Condition, type HubAlarmSource, type Problem, type Reading } from "./health";

export interface HealthFeedOptions {
  /** How often the meters are read (ms). */
  periodMs?: number;
  /** The clock (tests). */
  now?: () => number;
  /** Phase 2: a hub's alarms; none yet. */
  hubAlarms?: HubAlarmSource | null;
}

/** What the rail shows besides the list: the last load figures. */
export interface HealthView {
  problems: Problem[];
  load: [number, number] | null;
}

export class HealthFeed {
  private readonly tracker = new HealthTracker();
  private readonly listeners = new Set<() => void>();
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  private inflight: Promise<void> | null = null;
  private unwatch: (() => void) | null = null;
  private unhub: (() => void) | null = null;
  private noLimiter = false;
  private hubOn: Condition[] = [];
  private readonly periodMs: number;
  private readonly now: () => number;
  view: HealthView = { problems: [], load: null };
  private shownLoad: [number, number] | null = null;

  constructor(
    private readonly session: Session,
    private readonly opts: HealthFeedOptions = {},
  ) {
    this.periodMs = opts.periodMs ?? 500;
    this.now = opts.now ?? Date.now;
  }

  start(): void {
    if (this.running) return;
    this.running = true;
    let phase = this.session.state.link?.phase ?? null;
    // A change of the DSPi Link phase counts at once, not at the next read.
    this.unwatch = this.session.watch(() => {
      const p = this.session.state.link?.phase ?? null;
      if (p === phase) return;
      phase = p;
      this.apply({ clip: null, load: null, limiter: null, dlink: p });
    });
    if (this.opts.hubAlarms) {
      this.unhub = this.opts.hubAlarms.subscribe((on) => {
        this.hubOn = on;
        this.apply({ clip: null, load: null, limiter: null, dlink: this.session.state.link?.phase ?? null });
      });
    }
    void this.tick();
  }

  stop(): void {
    this.running = false;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.unwatch?.();
    this.unwatch = null;
    this.unhub?.();
    this.unhub = null;
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  acknowledge(key: string): void {
    if (key.startsWith("hub:") && this.opts.hubAlarms) void this.opts.hubAlarms.acknowledge(key).catch(() => undefined);
    if (this.tracker.acknowledge(key)) this.publish();
  }

  acknowledgeAll(): void {
    for (const p of this.view.problems) if (p.kind === "hub" && !p.acknowledged) this.acknowledge(p.key);
    if (this.tracker.acknowledgeAll()) this.publish();
  }

  /** One read now (tests call it directly; the timer calls it otherwise); during a read, that read. */
  tick(): Promise<void> {
    if (!this.running) return Promise.resolve();
    if (this.inflight) return this.inflight;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
    this.inflight = (async () => {
      try {
        await this.read();
      } finally {
        this.inflight = null;
      }
      if (this.running) this.timer = setTimeout(() => void this.tick(), this.periodMs);
    })();
    return this.inflight;
  }

  private async read(): Promise<void> {
    const s = this.session;
    const snap = s.state.snapshot;
    const dlink = s.state.link?.phase ?? null;
    const r: Reading = { clip: null, load: null, limiter: null, dlink };
    if (!snap || s.state.status !== "open" || s.document) {
      this.apply(r);
      return;
    }
    const st = snap.state;
    try {
      const m = await s.core.call("meters", undefined);
      if (!m.stale) {
        r.clip = { mask: m.clipHold >>> 0, names: meterNames(st, snap.outputLabels) };
        r.load = [m.cpu0, m.cpu1];
      }
    } catch {
      // Not read: the clip and load problems stay as they were.
    }
    if (!this.noLimiter && (st.limiterEnabled ?? []).some(Boolean) && !(dlink && dlink !== "bound")) {
      try {
        const gr = await s.core.call("limiterMeter", undefined);
        r.limiter = { gr, names: outputNames(st, snap.outputLabels), enabled: st.limiterEnabled };
      } catch (e) {
        if (e instanceof CoreCallError && e.code === "unsupported") this.noLimiter = true;
      }
    } else if ((st.limiterEnabled ?? []).every((on) => !on)) {
      // Every limiter off: nothing limits.
      r.limiter = { gr: [], names: [], enabled: [] };
    }
    if (!this.running) return;
    if (r.load && !sameLoad(r.load, this.view.load)) this.view = { ...this.view, load: r.load };
    this.apply(r);
  }

  private apply(r: Reading): void {
    const kinds = readKinds(r);
    const conds = conditions(r, this.tracker.on());
    if (this.opts.hubAlarms) {
      kinds.add("hub");
      conds.push(...this.hubOn);
    }
    this.tracker.update(this.now(), conds, kinds);
    this.publish();
  }

  private publish(): void {
    const problems = this.tracker.list();
    if (sameList(this.view.problems, problems) && this.view.load === this.shownLoad) return;
    this.shownLoad = this.view.load;
    this.view = { ...this.view, problems };
    for (const l of this.listeners) l();
  }
}

function outputNames(st: DspState, labels: Parameters<typeof outputLabel>[1]): string[] {
  return Array.from({ length: st.numOutputChannels }, (_, o) => outputLabel(st, labels, o));
}

function sameList(a: readonly Problem[], b: readonly Problem[]): boolean {
  return a.length === b.length && a.every((p, i) => JSON.stringify(p) === JSON.stringify(b[i]));
}

function sameLoad(a: [number, number], b: [number, number] | null): boolean {
  return !!b && a[0] === b[0] && a[1] === b[1];
}
