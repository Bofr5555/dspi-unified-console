// -- WP WEBM1 -- Where each session was left: its page, and the last page of each section. Kept
// here, not in the view, so that switching device tabs comes back to the same page (the view of a
// session not on show is unmounted, and so costs nothing).

import type { Session } from "../session";
import type { SectionId } from "./sections";

export interface Place {
  /** The screen on show, null before one was chosen. */
  screen: string | null;
  /** The last page of each section. */
  last: Map<SectionId, string>;
}

const places = new WeakMap<Session, Place>();

export function placeOf(session: Session): Place {
  let p = places.get(session);
  if (!p) places.set(session, (p = { screen: null, last: new Map() }));
  return p;
}
