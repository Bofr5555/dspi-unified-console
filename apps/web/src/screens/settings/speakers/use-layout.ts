// WP ROLEW: the speaker layout for a screen (Settings › Speakers, the Measure setup): read when the
// screen shows, kept on the person's word.

import { useEffect, useRef, useState } from "preact/hooks";
import { safeLocalStorage } from "../../../fleet/local";
import type { Session } from "../../../ui/session";
import { NO_VIEW, keepLayout, loadLayout, type LayoutCaller, type LayoutView, type SpeakerLayoutRecord } from "./model";

export interface LayoutHandle {
  view: LayoutView;
  /** Read once. */
  loaded: boolean;
  /** Keep `layout` (confirmed or not); the failure in words, or "". */
  keep: (layout: SpeakerLayoutRecord, confirmed: boolean) => Promise<string>;
  // -- WP WEBM3 -- (RB C3) an edit built on the view as last kept when its turn comes, not as
  // rendered: two quick edits are kept one after the other, the second on the first's result.
  /** Keep what `edit` makes of the view as last kept; the failure in words, or "". */
  update: (edit: (v: LayoutView) => { layout: SpeakerLayoutRecord; confirmed: boolean }) => Promise<string>;
}

function words(e: unknown): string {
  const m = e instanceof Error ? e.message : String(e);
  return m ? m[0]!.toUpperCase() + m.slice(1) : m;
}

export function serialOf(session: Session): string {
  return session.state.snapshot?.device.serial || session.deviceId;
}

/** Sets the copy kept in this page again once a device is open (the core keeps it for its lifetime only). */
export async function restoreSpeakerLayout(session: Session): Promise<void> {
  await loadLayout(session.core as unknown as LayoutCaller, serialOf(session), safeLocalStorage());
}

export function useSpeakerLayout(session: Session): LayoutHandle {
  const [view, setView] = useState<LayoutView>(NO_VIEW);
  const [loaded, setLoaded] = useState(false);
  // The view as last kept, so two quick edits each build on the one before.
  const now = useRef(view);
  now.current = view;
  const serial = serialOf(session);
  useEffect(() => {
    let alive = true;
    setLoaded(false);
    void loadLayout(session.core as unknown as LayoutCaller, serial, safeLocalStorage()).then(
      (v) => alive && ((now.current = v), setView(v), setLoaded(true)),
      (e) => alive && (setView({ ...NO_VIEW, problem: `The speaker layout could not be read: ${words(e)}` }), setLoaded(true)),
    );
    return () => {
      alive = false;
    };
  }, [session, serial]);
  const keep = async (layout: SpeakerLayoutRecord, confirmed: boolean) => {
    try {
      const v = await keepLayout(session.core as unknown as LayoutCaller, serial, safeLocalStorage(), layout, confirmed, now.current);
      now.current = v;
      setView(v);
      return "";
    } catch (e) {
      return words(e);
    }
  };
  // -- WP WEBM3 -- one edit at a time, each reading `now` when it runs
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const update = (edit: (v: LayoutView) => { layout: SpeakerLayoutRecord; confirmed: boolean }) => {
    const run = queue.current.then(() => {
      const e = edit(now.current);
      return keep(e.layout, e.confirmed);
    });
    queue.current = run.catch(() => undefined);
    return run;
  };
  return { view, loaded, keep, update };
}
