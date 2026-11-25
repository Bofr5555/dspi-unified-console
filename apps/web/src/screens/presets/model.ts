// -- WP WEB7 -- Library › Presets (Qt's LibraryPresets.qml and the slot menu of ContextMenus.qml):
// what each slot's row says and which of its actions are offered. Pure, so the rules a slip here
// would turn into data loss are unit-tested (REVE B1, C3):
// - the core's preset copy saves the LIVE settings into the destination and into the source, so
//   "Copy to…" is offered on the active row only and always copies from the active slot;
// - an active slot the device reports outside 0..9 is unknown: no row is active, nothing copies,
//   nothing is saved "to the active preset";
// - a read-only session (a viewer, read-only firmware) only looks: every action is off.

import type { DspState } from "../../core/protocol";
import { PRESET_SLOTS, occupied } from "../main/model";

export type Slots = Pick<DspState, "presetNames" | "presetOccupied" | "activePresetSlot" | "presetStartupMode" | "presetDefaultSlot">;

/** The active slot, or null when the device reports none this console knows (>= 10, < 0). */
export function knownActive(s: Pick<DspState, "activePresetSlot">): number | null {
  const a = s.activePresetSlot;
  return Number.isInteger(a) && a >= 0 && a < PRESET_SLOTS ? a : null;
}

/** The slot loaded at power-on, -1 when the device loads the last one it was on. */
export function startupSlot(s: Pick<DspState, "presetStartupMode" | "presetDefaultSlot">): number {
  return s.presetStartupMode === 0 ? s.presetDefaultSlot : -1;
}

/** The sentence over the list (Qt's header). */
export function startupWords(s: Pick<DspState, "presetStartupMode" | "presetDefaultSlot">): string {
  const n = startupSlot(s);
  return n >= 0 ? `At power-on the device loads preset ${n + 1}.` : "At power-on the device loads the preset it was last on.";
}

/** A slot's name as its row shows it: the stored name, "Preset N" when it has none, "Empty" when nothing is stored. */
export function slotTitle(s: Slots, slot: number): string {
  const name = (s.presetNames[slot] ?? "").trim();
  if (name) return name;
  return occupied(s, slot) || slot === knownActive(s) ? `Preset ${slot + 1}` : "Empty";
}

/** "3: Living Room", as menus and questions name a slot. */
export function slotLabel(s: Slots, slot: number): string {
  return `${slot + 1}: ${slotTitle(s, slot)}`;
}

export interface SlotRow {
  slot: number;
  title: string;
  /** "Empty", said quietly. */
  muted: boolean;
  /** "Active, with unsaved changes · Startup". */
  note: string;
  active: boolean;
  stored: boolean;
  startup: boolean;
}

export function slotRows(s: Slots, dirty: boolean): SlotRow[] {
  const active = knownActive(s);
  const start = startupSlot(s);
  return Array.from({ length: PRESET_SLOTS }, (_, slot) => {
    const stored = occupied(s, slot);
    const isActive = slot === active;
    const note = [isActive ? (dirty ? "Active, with unsaved changes" : "Active") : "", slot === start ? "Startup" : ""].filter(Boolean).join(" · ");
    return { slot, title: slotTitle(s, slot), muted: !stored && !isActive && !(s.presetNames[slot] ?? "").trim(), note, active: isActive, stored, startup: slot === start };
  });
}

/** One action as offered: listed (`shown`) and usable (`enabled`). */
export interface Offer {
  shown: boolean;
  enabled: boolean;
}

export interface SlotActions {
  /** The row's own button: Recall on a stored slot that is not the active one. */
  recall: Offer;
  /** The row's own Save, on the active row. */
  save: Offer;
  /** The menu's "Save Current Settings Here" (active) or `Copy "<active>" Here` (any other slot): copy(active, slot). */
  saveHere: Offer & { text: string; copy: boolean };
  rename: Offer;
  startup: Offer;
  /** Copy to…: on the active row only; its rows are the other slots. */
  copyTo: Offer;
  clear: Offer & { text: string };
  clearAll: Offer;
}

/**
 * What the row of `slot` offers. `free` is false while an action runs. `ro` is a read-only session:
 * nothing is enabled (a viewer only looks).
 */
export function slotActions(s: Slots, slot: number, opts: { ro: boolean; free: boolean; dirty: boolean }): SlotActions {
  const active = knownActive(s);
  const isActive = slot === active;
  const stored = occupied(s, slot);
  const can = !opts.ro && opts.free;
  const anyStored = s.presetOccupied !== 0;
  const start = startupSlot(s);
  return {
    recall: { shown: !isActive && stored, enabled: can },
    save: { shown: isActive, enabled: can },
    saveHere: {
      // Into another slot the device copies the active one: only when the active one is known.
      shown: isActive || active !== null,
      enabled: can && (isActive || active !== null),
      text: isActive ? "Save Current Settings Here" : active !== null ? `Copy “${slotLabel(s, active)}” Here` : "Save Current Settings Here",
      copy: !isActive,
    },
    rename: { shown: true, enabled: can && (isActive || stored) },
    startup: { shown: true, enabled: can && (isActive || stored) && start !== slot },
    copyTo: { shown: isActive, enabled: can },
    clear: { shown: stored, enabled: can, text: `Clear “${slotLabel(s, slot)}”…` },
    clearAll: { shown: anyStored, enabled: can },
  };
}

/** Where Copy to… may go from the active slot: every other slot. */
export function copyTargets(s: Slots): number[] {
  const active = knownActive(s);
  if (active === null) return [];
  return Array.from({ length: PRESET_SLOTS }, (_, i) => i).filter((i) => i !== active);
}
