// WP P2WB3: the hub's history of this session's DSPi, kept current by the hub
// itself: the newest page read (`hubEventsRead`), then the hub sends each new
// record (`hubEventsWatch`, the hub's `event.watch`), which reaches the page as
// `event.added` through the session's `watchHub` (each DSPi Link turn). No poll.
// A watch ends with the hub session: after a reconnect (the DSPi Link bound
// again) it is asked again and what came meanwhile is read. When the hub's
// numbering started again (a forget), the history is read afresh.

import type { HubHistoryRecord, HubServerEventRecord } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { noticeOf } from "../../ui/texts";
import { mergeHistory } from "../hub/model";

/** Records asked for in one read. */
export const HUB_HISTORY_PAGE = 200;

export interface HubHistoryView {
  records: HubHistoryRecord[];
  error: string;
}

export class HubHistoryFeed {
  view: HubHistoryView = { records: [], error: "" };
  private readonly listeners = new Set<() => void>();
  private after = 0;
  private serial = "";
  private running = false;
  private syncing: Promise<void> | null = null;
  private again = false;
  private unhub: (() => void) | null = null;
  private unwatch: (() => void) | null = null;
  private phase: string | null = null;

  constructor(private readonly session: Session) {}

  start(): void {
    if (this.running) return;
    this.running = true;
    this.serial = this.openSerial();
    this.unhub = this.session.watchHub((events) => this.onEvents(events));
    this.phase = this.session.state.link?.phase ?? null;
    this.unwatch = this.session.watch(() => {
      const p = this.session.state.link?.phase ?? null;
      if (p === this.phase) return;
      this.phase = p;
      // Bound again (a reconnect, another DSPi): the watch is asked again, the gap read.
      if (p === "bound") void this.sync();
    });
    void this.sync();
  }

  stop(): void {
    if (!this.running) return;
    this.running = false;
    this.unhub?.();
    this.unhub = null;
    this.unwatch?.();
    this.unwatch = null;
    const serial = this.serial;
    void this.session.core.call("hubEventsWatch", { on: false, ...(serial ? { serial } : {}) }).catch(() => undefined);
  }

  subscribe(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  /** The watch asked for, then what the hub has since the last record kept; during a sync, one more after it. */
  sync(): Promise<void> {
    if (!this.running) return Promise.resolve();
    if (this.syncing) {
      this.again = true;
      return this.syncing;
    }
    this.syncing = (async () => {
      try {
        do {
          this.again = false;
          await this.syncOnce();
        } while (this.again && this.running);
      } finally {
        this.syncing = null;
      }
    })();
    return this.syncing;
  }

  private async syncOnce(): Promise<void> {
    const core = this.session.core;
    const serial = this.openSerial();
    if (serial !== this.serial) {
      // Another DSPi: its history, not the last one's.
      this.serial = serial;
      this.after = 0;
      this.set({ records: [] });
    }
    try {
      // First the watch, so a record written during the read is not missed (repeats merge away).
      await core.call("hubEventsWatch", { on: true, ...(serial ? { serial } : {}) });
      for (;;) {
        let page = await core.call("hubEventsRead", { afterId: this.after, limit: HUB_HISTORY_PAGE });
        let restarted = false;
        if (this.after > 0 && page.newestId < this.after) {
          // The hub's numbering started again: the newest page afresh.
          restarted = true;
          this.after = 0;
          page = await core.call("hubEventsRead", { afterId: 0, limit: HUB_HISTORY_PAGE });
        }
        if (!this.running) return;
        this.take(page.records, restarted);
        if (!page.more) break;
      }
      this.set({ error: "" });
    } catch (e) {
      if (this.running) this.set({ error: noticeOf(e)?.title ?? (e instanceof Error ? e.message : String(e)) });
    }
  }

  private onEvents(events: HubServerEventRecord[]): void {
    const fresh: HubHistoryRecord[] = [];
    let behind = false;
    for (const e of events) {
      if (e.kind !== "event.added" || !e.record) continue;
      const serial = e.record.serial || e.serial;
      if (this.serial && serial && serial !== this.serial) continue;
      const r = e.record;
      if (r.id > this.after) fresh.push(r);
      // At or below the last one kept: a repeat of one read meanwhile, or the numbering started again.
      else if (!this.view.records.some((k) => k.id === r.id && k.timeMs === r.timeMs && k.text === r.text)) behind = true;
    }
    if (fresh.length) this.take(fresh, false);
    // The numbering started again (a forget): read afresh.
    if (behind) void this.sync();
  }

  private take(records: readonly HubHistoryRecord[], restarted: boolean): void {
    if (!records.length && !restarted) return;
    const kept = mergeHistory(this.view.records, records, restarted);
    const last = records.length ? Math.max(...records.map((r) => r.id)) : 0;
    this.after = Math.max(restarted ? 0 : this.after, last);
    this.set({ records: kept });
  }

  private openSerial(): string {
    return this.session.state.snapshot?.device.serial || this.session.deviceId || "";
  }

  private set(patch: Partial<HubHistoryView>): void {
    this.view = { ...this.view, ...patch };
    for (const l of [...this.listeners]) l();
  }
}
