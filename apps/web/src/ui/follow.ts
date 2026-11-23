// -- WP ODEC -- The screens follow an AI agent (owner 2026-11-23, answer 9; the desktop app's
// AiFollow): while the hub's presence says an agent works on a device of this page, a change
// that reaches that device's session from elsewhere shows its device tab and the page it is on.
// Never while the person here is interacting (a pointer held down, typing in a field, a dialog
// open): then only the strip says it. At most one switch per FOLLOW_MIN_MS, never twice in a row
// to the same place. Measure keeps its own rule (an agent's measurement opens Measure there).

import type { DspState } from "../core/protocol";
import type { HubPresenceRecord } from "./presence-protocol";

/** The least time between two switches, ms. */
export const FOLLOW_MIN_MS = 3_000;

// -- WP INTI -- (review RF7) an input in flight: a key or a click acts on the screen it lands on
/** No switch within this long after the person's last key, pointer or wheel input, ms. */
export const INPUT_IDLE_MS = 1_000;

/** How long until the input last seen at `lastInputAt` (ms; -Infinity for none) is at rest. */
export function inputLeftMs(lastInputAt: number, now: number): number {
  return Math.max(0, INPUT_IDLE_MS - (now - lastInputAt));
}

/** What the agent changed: the device and the screen it is on. */
export interface FollowTarget {
  serial: string;
  screen: string;
}

const same = (a: unknown, b: unknown): boolean => JSON.stringify(a) === JSON.stringify(b);

/**
 * The fields of a snapshot's state and the screen each is on, in the order the first change is
 * looked for (the channel's filters before its level, the routes before the volumes).
 */
const FIELDS: readonly (readonly [string, string])[] = [
  ["xover", "xover"],
  ["matrixRouting", "matrix"],
  ["matrixGain", "matrix"],
  ["matrixInvert", "matrix"],
  ["outputDelayMs", "delays"],
  ["outputGainDb", "delays"],
  ["outputMuted", "delays"],
  ["outputEnabled", "delays"],
  ["limiterEnabled", "delays"],
  ["limiterThresholdDb", "delays"],
  ["limiterReleaseMs", "delays"],
  ["limiterLinkGroup", "delays"],
  ["inputPreampDb", "inputs"],
  ["inputSource", "inputs"],
  ["preampDb", "inputs"],
  ["masterVolumeDb", "main"],
  ["userVolumeDb", "main"],
  ["userMute", "main"],
];

/** The effects' fields: their names start so (loudness, the leveller, bass, the upmixer...). */
const EFFECTS = /^(loudness|leveller|leveler|psybass|subharmonic|sub|tube|upmix|crossfeed|bass)/i;

/** The screen a change from `before` to `after` is on; null for none this page follows. */
export function changedScreen(before: DspState | null | undefined, after: DspState | null | undefined): string | null {
  if (!before || !after) return null;
  if (!same(before.filters, after.filters)) return "eq";
  for (const [field, screen] of FIELDS) if (!same(before[field], after[field])) return screen;
  for (const field of Object.keys(after)) if (EFFECTS.test(field) && !same(before[field], after[field])) return "effects";
  return null;
}

/** Whether a presence row says an agent works on the device `serial`. */
export function agentOn(rows: readonly HubPresenceRecord[], serial: string): boolean {
  return !!serial && rows.some((p) => p.kind === "agent" && p.serial === serial);
}

/** Whether the person here is interacting: a pointer held down, focus in a field, a dialog open. */
export function interacting(doc: Pick<Document, "activeElement" | "querySelector"> | null, pointerDown: boolean): boolean {
  if (pointerDown) return true;
  if (!doc) return false;
  const el = doc.activeElement as (Element & { isContentEditable?: boolean }) | null;
  if (el && (/^(INPUT|TEXTAREA|SELECT)$/.test(el.tagName) || el.isContentEditable)) return true;
  return !!doc.querySelector('dialog[open], [role="dialog"], [role="alertdialog"]');
}

/** The rate limit and the person's gate, one per page. */
export class FollowGate {
  private last: FollowTarget | null = null;
  private at = Number.NEGATIVE_INFINITY;

  /** Whether to switch to `t` now (`now` ms); true marks it done. `here`: the place on show. */
  take(t: FollowTarget, now: number, busy: boolean, here: FollowTarget | null): boolean {
    if (busy) return false;
    if (here && here.serial === t.serial && here.screen === t.screen) return false;
    if (now - this.at < FOLLOW_MIN_MS) return false;
    if (this.last && this.last.serial === t.serial && this.last.screen === t.screen) return false;
    this.last = t;
    this.at = now;
    return true;
  }
}

/** The page's one gate (the shell and every session's view share it). */
export const followGate = new FollowGate();

// -- WP ODEC -- item 4: the grant the agent's console says (" · test signals allowed, 9 min left")
/** An entry's `what` split into the work and the test-signal grant, "" for none. */
export function splitGrant(what: string): { work: string; grant: string } {
  const m = /^(.*?)\s*·\s*(test signals allowed.*)$/i.exec(what);
  return m ? { work: m[1]!, grant: m[2]! } : { work: what, grant: "" };
}
