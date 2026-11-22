// -- WP WEBM2 -- The inspector's rules (MODLAY 2): it sits beside the page when the page has room,
// and stacks under it when not (the mockup's narrow layout). The viewer can hide it to give the
// page the full width; that choice is kept in this browser only, like the arrangement.

import type { KeyValueStore } from "../../fleet/local";

/** The row (page and inspector) this wide or wider has the inspector beside the page. */
export const BESIDE_MIN_PX = 960;

const KEY = "dspi-web.inspector";

/** Where the inspector shows: beside the page, under it, or hidden by the viewer. */
export type Placement = "beside" | "under" | "hidden";

export function placement(rowWidth: number, collapsed: boolean): Placement {
  if (collapsed) return "hidden";
  return rowWidth >= BESIDE_MIN_PX ? "beside" : "under";
}

/**
 * Whether the selected band's fields go to the inspector, or stay in the band bar under the
 * graph: only beside the page and with a fine pointer. On touch the band bar is used (owner,
 * 2026-11-21: the band bar stays as an option, and it is the touch choice).
 */
export function bandInInspector(p: Placement, coarse: boolean): boolean {
  return p === "beside" && !coarse;
}

/** Whether this viewer hid the inspector; shown when nothing (or nothing readable) is kept. */
export function loadCollapsed(store: KeyValueStore | null): boolean {
  try {
    const v: unknown = JSON.parse(store?.getItem(KEY) ?? "null");
    return !!v && typeof v === "object" && (v as { collapsed?: unknown }).collapsed === true;
  } catch {
    return false;
  }
}

export function saveCollapsed(store: KeyValueStore | null, collapsed: boolean): void {
  try {
    store?.setItem(KEY, JSON.stringify({ collapsed }));
  } catch {
    // A private window or blocked storage: the choice lasts as long as the page.
  }
}
