// -- WP UB2 -- (UXP B2, D4) The Home page's rules: Simple mode's first page and the locked
// user's only one, a remote (UI-MODES section 2). What it lists and what it lets a user do,
// kept apart from the view so the unit tests read them.

import type { DspState } from "../../core/protocol";
import { PRESET_SLOTS, occupied, presetName } from "../main/model";

/** One preset of the Recall list: its slot (counted from 0), its name, whether it plays now. */
export interface RecallRow {
  slot: number;
  text: string;
  active: boolean;
}

/**
 * The presets the Recall list offers: the stored slots in their order, and the active one
 * even while it is empty (it is what plays). Empty slots are left out: nothing to recall.
 */
export function recallRows(s: Pick<DspState, "activePresetSlot" | "presetNames" | "presetOccupied">): RecallRow[] {
  const out: RecallRow[] = [];
  for (let slot = 0; slot < PRESET_SLOTS; slot++) {
    const active = slot === s.activePresetSlot;
    if (!active && !occupied(s, slot)) continue;
    out.push({ slot, text: presetName(s, slot), active });
  }
  return out;
}

/**
 * Whether the Home page lets the source be chosen: when there is more than one.
 * -- FIX016 -- (LOCKSRC, owner D4) the locked user too: the installer's lock lets the source
 * through as it does volume, mute and preset loads (the core's rule, crates/dspi-core/src/limits/lock.rs).
 */
export function sourceChoosable(choices: number): boolean {
  return choices > 1;
}

/** The one health line: the session's health summary as a sentence ("No problems", "2 problems"). */
export function healthLine(summary: { text: string; tone: "ok" | "warn" | "bad" }): string {
  return summary.tone === "ok" ? "No problems" : summary.text.charAt(0).toUpperCase() + summary.text.slice(1);
}

/** A room's or zone's level from its units' ("-20.0 dB", "-24.0 to -18.0 dB"; "–" with none read), as the dock's readout. */
export function roomLevel(levels: readonly (number | null | undefined)[]): string {
  const known = levels.filter((v): v is number => typeof v === "number" && Number.isFinite(v));
  if (!known.length) return "–";
  const lo = Math.min(...known);
  const hi = Math.max(...known);
  return lo === hi ? `${lo.toFixed(1)} dB` : `${lo.toFixed(1)} to ${hi.toFixed(1)} dB`;
}
