// WP WB6f: the health panel's model. What is wrong now is read from the core's
// state through the binding (the meters' clip hold and the two cores' load, the
// limiters' gain reduction, the session's DSPi Link phase); a problem, once
// raised, stays listed until it is acknowledged, even when it has ended by
// itself. Pure: the time comes in from the caller, nothing here polls.
//
// Alarms from a hub join the list later through `HubAlarmSource` (phase 2): the
// type is the seam, nothing here invents them.

import type { LinkPhase } from "../../core/protocol";
import { linkText } from "../texts";

/** What a problem is about. `hub`: an alarm a hub raised (phase 2, `HubAlarmSource`). */
export type ProblemKind = "clip" | "limiter" | "load" | "dlink" | "hub";

export type Severity = "warning" | "error";

/** A problem as the panel lists it. */
export interface Problem {
  /** One per subject: "clip:3", "limiter:0", "load:1", "dlink", "hub:<id>". */
  key: string;
  kind: ProblemKind;
  severity: Severity;
  /** The sentence shown, as of the last reading while it was on. */
  text: string;
  /** When it was first raised, ms since the epoch. */
  since: number;
  /** When it was last seen on. */
  last: number;
  /** When it ended by itself; null while it is still on. */
  ended: number | null;
  /** How often it came on while listed. */
  count: number;
  acknowledged: boolean;
  /** The worst figure while listed (dB of gain reduction, % load); absent where none applies. */
  worst?: number;
}

/** A condition that is on now. */
export interface Condition {
  key: string;
  kind: ProblemKind;
  severity: Severity;
  /** The sentence for it; `{worst}` is replaced by the worst figure so far. */
  text: string;
  value?: number;
}

// -- the thresholds --

/** A core's load above this (%) is a problem: the Console's red bar (M26, `CpuMeter`). */
export const LOAD_HIGH = 90;
/** ... and it has ended once the load is back below this. */
export const LOAD_OK = 85;
/** Gain reduction at or above this (dB) is a problem. */
export const LIMITER_ON_DB = 1;
/** ... and it has ended below this. */
export const LIMITER_OFF_DB = 0.5;
/** The core's display floor for gain reduction (`fetch_limiter_meter`): not a reading. */
const LIMITER_FLOOR_DB = 120;

/** One reading of what the panel watches. A part that could not be read is null and leaves its problems as they were. */
export interface Reading {
  /** The core's clip hold (`clipHold`, which the clip marker is drawn from: it ends once the hold is over), one bit per wire channel, and those channels' names. */
  clip: { mask: number; names: string[] } | null;
  /** The two cores' load, in %. */
  load: [number, number] | null;
  /** Gain reduction per output in dB, and the outputs' names; only outputs with their limiter on count. */
  limiter: { gr: number[]; names: string[]; enabled: boolean[] } | null;
  /** The session's DSPi Link phase; null off a hub. */
  dlink: LinkPhase | null;
}

/** Which kinds a reading speaks for: the others keep their state. */
export function readKinds(r: Reading): Set<ProblemKind> {
  const k = new Set<ProblemKind>(["dlink"]);
  if (r.clip) k.add("clip");
  if (r.load) k.add("load");
  if (r.limiter) k.add("limiter");
  return k;
}

/**
 * What is on in `r`. `on` is the set of keys on before it, for the thresholds that
 * end lower than they start (a load hovering at the limit is one problem, not many).
 */
export function conditions(r: Reading, on: ReadonlySet<string>): Condition[] {
  const out: Condition[] = [];
  if (r.clip) {
    for (let w = 0; w < 32; w++) {
      if ((r.clip.mask >>> w) & 1) {
        const name = r.clip.names[w] || `Channel ${w + 1}`;
        out.push({ key: `clip:${w}`, kind: "clip", severity: "error", text: `Clipping on ${name}` });
      }
    }
  }
  if (r.load) {
    r.load.forEach((pct, c) => {
      if (!Number.isFinite(pct)) return;
      const key = `load:${c}`;
      if (pct > LOAD_HIGH || (on.has(key) && pct >= LOAD_OK)) {
        out.push({ key, kind: "load", severity: "warning", text: `Core ${c} load high: up to {worst}%`, value: pct });
      }
    });
  }
  if (r.limiter) {
    r.limiter.gr.forEach((db, o) => {
      if (!r.limiter!.enabled[o] || !Number.isFinite(db) || db >= LIMITER_FLOOR_DB) return;
      const key = `limiter:${o}`;
      if (db >= LIMITER_ON_DB || (on.has(key) && db >= LIMITER_OFF_DB)) {
        const name = r.limiter!.names[o] || `Output ${o + 1}`;
        out.push({ key, kind: "limiter", severity: "warning", text: `Limiter reducing ${name}: up to {worst} dB`, value: round1(db) });
      }
    });
  }
  if (r.dlink === "reconnecting" || r.dlink === "closed" || r.dlink === "refused") {
    // CG10's words for the phase.
    out.push({ key: "dlink", kind: "dlink", severity: "error", text: `DSPi Link: ${dlinkWords(r.dlink)}` });
  }
  return out;
}

function dlinkWords(phase: LinkPhase): string {
  try {
    return linkText(phase).text;
  } catch {
    return "Connection lost";
  }
}

/**
 * The listed problems. `update` takes what is on now for the kinds a reading
 * speaks for; a problem that ends stays until acknowledged, and one acknowledged
 * while on goes once it has ended.
 */
export class HealthTracker {
  private items = new Map<string, Problem>();

  /** Feed one reading's conditions; `kinds`: what the reading covered. Returns whether the list changed. */
  update(now: number, conds: Condition[], kinds: ReadonlySet<ProblemKind>): boolean {
    let changed = false;
    const seen = new Set<string>();
    for (const c of conds) {
      seen.add(c.key);
      const p = this.items.get(c.key);
      if (!p) {
        this.items.set(c.key, {
          key: c.key,
          kind: c.kind,
          severity: c.severity,
          text: fill(c.text, c.value),
          since: now,
          last: now,
          ended: null,
          count: 1,
          acknowledged: false,
          ...(c.value !== undefined ? { worst: c.value } : {}),
        });
        changed = true;
        continue;
      }
      const worst = c.value === undefined ? p.worst : Math.max(p.worst ?? c.value, c.value);
      const back = p.ended !== null;
      const text = fill(c.text, worst);
      if (back || text !== p.text || worst !== p.worst) changed = true;
      // Back on after it ended: raised again, and asks to be acknowledged again.
      Object.assign(p, { last: now, ended: null, text, ...(worst !== undefined ? { worst } : {}) });
      if (back) {
        p.count += 1;
        p.acknowledged = false;
      }
    }
    for (const p of [...this.items.values()]) {
      if (seen.has(p.key) || p.ended !== null || !kinds.has(p.kind)) continue;
      if (p.acknowledged) this.items.delete(p.key);
      else p.ended = now;
      changed = true;
    }
    return changed;
  }

  /** The keys on now (not ended): for `conditions`' thresholds. */
  on(): Set<string> {
    return new Set([...this.items.values()].filter((p) => p.ended === null).map((p) => p.key));
  }

  /** Acknowledge one: an ended problem goes, one still on stays, marked. */
  acknowledge(key: string): boolean {
    const p = this.items.get(key);
    if (!p || p.acknowledged) return false;
    if (p.ended !== null) this.items.delete(key);
    else p.acknowledged = true;
    return true;
  }

  acknowledgeAll(): boolean {
    let any = false;
    for (const k of [...this.items.keys()]) any = this.acknowledge(k) || any;
    return any;
  }

  /** The list: unacknowledged first, errors before warnings, newest first. */
  list(): Problem[] {
    const rank = (p: Problem) => (p.acknowledged ? 2 : 0) + (p.severity === "error" ? 0 : 1);
    return [...this.items.values()].map((p) => ({ ...p })).sort((a, b) => rank(a) - rank(b) || b.since - a.since);
  }
}

function fill(text: string, worst: number | undefined): string {
  return text.replace("{worst}", worst === undefined ? "" : String(worst));
}

function round1(v: number): number {
  return Math.round(v * 10) / 10;
}

/** What the rail's summary says about the list. */
export function healthSummary(list: readonly Problem[]): { text: string; tone: "ok" | "warn" | "bad" } {
  const open = list.filter((p) => !p.acknowledged);
  if (open.length) {
    const bad = open.some((p) => p.severity === "error");
    return { text: `${open.length} ${open.length === 1 ? "problem" : "problems"}`, tone: bad ? "bad" : "warn" };
  }
  if (list.length) return { text: `${list.length} acknowledged`, tone: "warn" };
  return { text: "OK", tone: "ok" };
}

// -- phase 2 --

/**
 * Alarms a hub raises (phase 2: when the hub has device alarms for the health panel). A
 * source lists its alarms as conditions of kind `hub` and takes their acknowledgement back
 * to the hub. Not implemented by anything yet; `HealthFeed` takes one when there is one.
 */
export interface HubAlarmSource {
  /** The hub's alarms that are on, as conditions (`kind: "hub"`, keys "hub:<id>"); called again on every change. Returns the unsubscribe. */
  subscribe(fn: (on: Condition[]) => void): () => void;
  /** Acknowledge one of them at the hub. */
  acknowledge(key: string): Promise<void>;
}
