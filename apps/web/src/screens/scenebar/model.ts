// WP WB6e: the scene bar's arithmetic and its two verbs. The bar shows the
// device's ten preset slots as numbered recall buttons; Recall loads a slot and
// Store saves the live settings into one, both through the session (the core's
// `preset.load` and `preset.save` writes, with the save's confirmation token),
// so the lock, the limits, a safe apply and the undo history are the core's as
// for every other write. A recall with unsaved changes asks first: the session
// view's unsaved-changes prompt (M42, the core's lines) for another slot, a
// plain question for the slot already active, whose reload loses them.
//
// Words: a slot holds a preset (docs/GLOSSARY.md); a snapshot is the app's and
// a scene the hub's, and neither is what this bar recalls. Host-side scenes and
// A/B compare are P1WB1's (screens/snapshots), the hub's scenes SW3's
// (screens/system/SceneBar.tsx).

import type { DspState } from "../../core/protocol";
import type { Ask, Unsaved } from "../../ui/screens";
import { PRESET_SLOTS, presetName } from "../main/model";

/** One slot's button. */
export interface SlotView {
  slot: number;
  /** The number on the button: 1..10. */
  number: number;
  /** "Living room", "Preset 3", or "Empty". */
  name: string;
  occupied: boolean;
  active: boolean;
  /** The active slot's live settings differ from what it holds. */
  unsaved: boolean;
}

type Slots = Pick<DspState, "presetNames" | "presetOccupied" | "activePresetSlot">;

/** The ten buttons, in slot order. */
export function slotViews(s: Slots, dirty: boolean): SlotView[] {
  return Array.from({ length: PRESET_SLOTS }, (_, slot) => {
    const active = slot === s.activePresetSlot;
    return {
      slot,
      number: slot + 1,
      name: presetName(s, slot),
      occupied: (s.presetOccupied & (1 << slot)) !== 0,
      active,
      unsaved: active && dirty,
    };
  });
}

/** A button's accessible name: what pressing it does, and the slot's state. */
export function slotLabel(v: SlotView, storing: boolean): string {
  const state = [v.active ? "active" : "", v.unsaved ? "unsaved" : ""].filter(Boolean).join(", ");
  return `${storing ? "Store in" : "Recall"} ${v.number}: ${v.name}${state ? ` (${state})` : ""}`;
}

/** The line beside the bar's label: the active slot, and whether its settings are saved. */
export function activeLine(views: SlotView[]): string {
  const a = views.find((v) => v.active);
  if (!a) return "";
  return `${a.number}: ${a.name}${a.unsaved ? " · Unsaved" : ""}`;
}

/**
 * What a recall of `slot` does first. The active slot with nothing unsaved is
 * already what the device runs: nothing to do. The active slot with unsaved
 * changes reloads it, losing them: asked. Another slot with unsaved changes:
 * the unsaved-changes prompt (Save, Discard, Cancel). Otherwise it loads.
 */
export type RecallStep = "nothing" | "load" | "confirm-reload" | "ask-unsaved";

export function recallStep(slot: number, active: number, dirty: boolean): RecallStep {
  if (slot === active) return dirty ? "confirm-reload" : "nothing";
  return dirty ? "ask-unsaved" : "load";
}

/** What the bar's verbs need from the session: the core's writes. */
export interface SlotSession {
  readonly readOnlyReason: string;
  readonly dirty: boolean;
  readonly state: { snapshot?: { state: Slots } | null };
  write(path: string, value: "trigger", indices: number[]): Promise<boolean>;
  savePreset(slot: number): Promise<boolean>;
}

export interface Verbs {
  session: SlotSession;
  ask: Ask;
  unsaved: () => Promise<Unsaved>;
}

function slotsOf(s: SlotSession): Slots | null {
  return s.state.snapshot?.state ?? null;
}

/**
 * Recall `slot`: asks as `recallStep` says, then loads it. Answers whether the
 * device now runs that slot's preset from the load (false when cancelled,
 * refused or nothing to do).
 */
export async function recall(v: Verbs, slot: number): Promise<boolean> {
  const s = slotsOf(v.session);
  if (!s || v.session.readOnlyReason || slot < 0 || slot >= PRESET_SLOTS) return false;
  const active = s.activePresetSlot;
  const step = recallStep(slot, active, v.session.dirty);
  if (step === "nothing") return false;
  if (step === "confirm-reload") {
    const a = await v.ask({
      title: "Recall Preset",
      body: `Recall “${presetName(s, slot)}” from slot ${slot + 1} again? The unsaved changes are lost.`,
      buttons: [
        { label: "Recall", value: "recall", danger: true },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a !== "recall") return false;
  } else if (step === "ask-unsaved") {
    const a = await v.unsaved();
    if (a === "cancel") return false;
    if (a === "save" && !(await v.session.savePreset(active))) return false;
  }
  return v.session.write("preset.load", "trigger", [slot]);
}

/** The Store question's text for `slot`. */
export function storeText(s: Slots, slot: number): string {
  const n = slot + 1;
  if ((s.presetOccupied & (1 << slot)) === 0) return `Store the current settings in slot ${n}?`;
  const name = presetName(s, slot);
  if (slot === s.activePresetSlot) return `Store the current settings in slot ${n}, “${name}”? What it held is replaced.`;
  return `Store the current settings in slot ${n}? “${name}” is replaced; this cannot be undone.`;
}

/**
 * Store the live settings in `slot`, asked first (it writes the device's flash
 * and may replace a stored preset). An unnamed slot is named "Preset N" first,
 * as Save does. The slot becomes the active one. Answers whether it was stored.
 */
export async function store(v: Verbs, slot: number): Promise<boolean> {
  const s = slotsOf(v.session);
  if (!s || v.session.readOnlyReason || slot < 0 || slot >= PRESET_SLOTS) return false;
  const occupiedOther = (s.presetOccupied & (1 << slot)) !== 0 && slot !== s.activePresetSlot;
  const a = await v.ask({
    title: "Store Preset",
    body: storeText(s, slot),
    buttons: [
      { label: "Store", value: "store", danger: occupiedOther },
      { label: "Cancel", value: "cancel" },
    ],
  });
  if (a !== "store") return false;
  return v.session.savePreset(slot);
}
