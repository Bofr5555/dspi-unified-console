// WP FEATUI (AIVIS's web part): who works on the hub's devices beside the people, per session.
// A console that runs an AI agent says so on the hub (dspi-hub's `presence`); every session of
// that hub hears `presence.changed` and reads the list here. The page shows it in a strip on
// every screen and marks the device's tab, as the desktop app's agent strip and marks do.

import { useEffect, useState } from "preact/hooks";
import type { Session } from "./session";
import type { HubPresenceRecord } from "./presence-protocol";

/** Text from the hub as the page shows it: no control or format characters (the hub drops them too). */
export function cleanPresence(s: string): string {
  return s.replace(/[\p{Cc}\p{Cf}]/gu, "");
}

/** One entry in words: "Claude, through Studio Mac, is working on Main left: Setting a gain". */
export function presenceText(p: HubPresenceRecord, deviceName: (serial: string) => string, mine: string): string {
  const who = cleanPresence(p.label) || "An AI agent";
  const via = p.by ? `, through ${cleanPresence(p.by)},` : "";
  const on = p.serial ? (p.serial === mine ? " on this device" : ` on ${deviceName(p.serial)}`) : "";
  const what = cleanPresence(p.what);
  return `${who}${via} is working${on}${what ? `: ${what}` : "."}`;
}

class PresenceWatch {
  rows: HubPresenceRecord[] = [];
  private readonly listeners = new Set<() => void>();
  private stopHub: (() => void) | null = null;
  private reading = false;
  private again = false;

  constructor(private readonly session: Session) {}

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    if (this.listeners.size === 1) this.start();
    return () => {
      this.listeners.delete(fn);
      if (!this.listeners.size) this.stop();
    };
  }

  private start(): void {
    this.stopHub = this.session.watchHub((events) => {
      if (events.some((e) => e.kind === "presence.changed")) void this.read();
    });
    void this.read(true);
  }

  private stop(): void {
    this.stopHub?.();
    this.stopHub = null;
  }

  /** The list again; `first`: only on a hub that has the family. One read at a time. */
  async read(first = false): Promise<void> {
    if (this.reading) {
      this.again = true;
      return;
    }
    this.reading = true;
    try {
      if (first) {
        const caps = await this.session.core.call("hubCaps", undefined).catch(() => null);
        if (!caps?.onHub || !(caps.caps ?? []).includes("presence")) return;
      }
      const rows = await this.session.core.call("hubPresenceList", undefined).catch(() => null);
      if (rows) this.set(rows);
    } finally {
      this.reading = false;
      if (this.again) {
        this.again = false;
        void this.read();
      }
    }
  }

  private set(rows: HubPresenceRecord[]): void {
    if (JSON.stringify(rows.map(({ ttlS: _, ...r }) => r)) === JSON.stringify(this.rows.map(({ ttlS: _, ...r }) => r))) return;
    this.rows = rows;
    for (const l of [...this.listeners]) l();
  }
}

const watches = new WeakMap<Session, PresenceWatch>();

function watchOf(s: Session): PresenceWatch {
  let w = watches.get(s);
  if (!w) {
    w = new PresenceWatch(s);
    watches.set(s, w);
  }
  return w;
}

/** Who works on this session's hub's devices now ([] off a hub, or a hub without the family). */
export function usePresence(s: Session): HubPresenceRecord[] {
  const w = watchOf(s);
  const [, bump] = useState(0);
  useEffect(() => w.watch(() => bump((n) => n + 1)), [w]);
  return w.rows;
}

// -- WP ODEC --
/** Who works on this session's hub's devices now, outside a component; `keep` holds the hub's list current until called again. */
export function presenceNow(s: Session): { rows: HubPresenceRecord[]; keep: () => () => void } {
  const w = watchOf(s);
  return { rows: w.rows, keep: () => w.watch(() => {}) };
}
