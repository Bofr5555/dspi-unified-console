// -- WP WEBM3 -- A page's parts (MODLAY 2: "Pages use tabs for their parts"): the tabs a page
// puts beside its title, in the frame's page head, through a portal (as the inspector's `Inspect`
// does), so the frame knows nothing about pages. A section of one page shows its page's parts
// there in place of the section's tabs (Effects, Measure); a section of several shows both. Which
// part is on show is the session's (`ui/shell/places.ts`): switching device tabs or sections and
// coming back returns to it.

import type { ComponentChildren } from "preact";
import { createContext } from "preact";
import { createPortal } from "preact/compat";
import { useCallback, useContext, useState } from "preact/hooks";
import type { Session } from "../session";
import { placeOf } from "./places";
import { tabKeys } from "./tab-keys";

const SlotContext = createContext<{ body: HTMLElement | null; set: (el: HTMLElement | null) => void } | null>(null);

/** The frame's part: the head's slot and the page share it. */
export function PartsArea(props: { children: ComponentChildren }) {
  const [body, set] = useState<HTMLElement | null>(null);
  return <SlotContext.Provider value={{ body, set }}>{props.children}</SlotContext.Provider>;
}

/** Where a page's parts go, in the page head. */
export function PartsSlot() {
  const slot = useContext(SlotContext);
  return <div class="page-parts-slot" ref={slot?.set} />;
}

export interface Part {
  id: string;
  label: string;
  /** A dot before the label: on (true) or off (false); none when undefined. */
  on?: boolean;
}

/**
 * The page's parts as tabs (ARIA's tabs pattern), each `tabpanel` named `<id>-part`. In the head
 * when the page is in a frame, else where the page draws it.
 */
export function PageParts(props: { label: string; parts: readonly Part[]; current: string; onPart: (id: string) => void }) {
  const slot = useContext(SlotContext);
  const { parts } = props;
  if (parts.length < 2) return null;
  const at = parts.findIndex((p) => p.id === props.current);
  const row = (
    <div class="page-tabs page-parts" role="tablist" aria-label={props.label} onKeyDown={tabKeys(parts.length, at, (i) => props.onPart(parts[i]!.id))}>
      {parts.map((p, i) => (
        <button
          type="button"
          role="tab"
          key={p.id}
          id={`${p.id}-part-tab`}
          data-part={p.id}
          aria-selected={i === at}
          tabIndex={i === Math.max(0, at) ? 0 : -1}
          onClick={() => props.onPart(p.id)}
        >
          {/* -- WP REVEFIX -- (REVE D3, §6) the state's dot before the label, as Qt's tabs */}
          {p.on === undefined ? null : <span class={`part-dot${p.on ? " on" : ""}`} aria-hidden="true" title={p.on ? "On" : "Off"} />}
          {p.label}
          {p.on === undefined ? null : <span class="visually-hidden"> {p.on ? "On" : "Off"}</span>}
        </button>
      ))}
    </div>
  );
  if (!slot) return row;
  return slot.body ? createPortal(row, slot.body) : null;
}

// -- WP MXW --
/**
 * A page's tools at the right of its head (DESIGN-LANGUAGE §7: secondary buttons, at most four),
 * through the same slot as its parts; where the page draws without a frame, where it is.
 */
export function PageTools(props: { label: string; children: ComponentChildren }) {
  const slot = useContext(SlotContext);
  const row = (
    <div class="page-tools" role="toolbar" aria-label={props.label}>
      {props.children}
    </div>
  );
  if (!slot) return row;
  return slot.body ? createPortal(row, slot.body) : null;
}

/**
 * The part of `screen` on show for this session: `first` until one is chosen, and the one chosen
 * since (also when the screen was left and opened again). `ids` are the parts shown now; a kept
 * part that is not one of them shows the first.
 */
export function usePart(session: Session, screen: string, ids: readonly string[], first?: string): [string, (id: string) => void] {
  const place = placeOf(session);
  const [, bump] = useState(0);
  const kept = place.parts.get(screen);
  const current = kept !== undefined && ids.includes(kept) ? kept : first !== undefined && ids.includes(first) ? first : (ids[0] ?? "");
  const set = useCallback(
    (id: string) => {
      place.parts.set(screen, id);
      bump((n) => n + 1);
    },
    [place, screen],
  );
  return [current, set];
}
