// A slider's writes (WP WB6a): while it moves, each new position is written as
// soon as the one before has been read back (the newest wins, nothing queues
// up), so the device follows the drag as on the Console; when it is let go the
// last position is written and the slider shows the device's value again.
//
// -- WP WB8 -- A pointer drag is marked for the core's journal (`setDragging`)
// so its writes are one undo step: `grab` on the pointer going down, and the
// mark ends once the last write of the drag has gone, whichever way the drag
// ends (the pointer let go anywhere, cancelled, its capture lost) and when the
// control goes away mid-drag.

import { useEffect, useRef, useState } from "preact/hooks";
import type { Session } from "../../ui/session";

export function useLiveWrite(write: (v: number) => Promise<boolean>, session?: Session) {
  const [draft, setDraft] = useState<number | null>(null);
  const st = useRef({ busy: false, pending: null as number | null, released: false, dragging: false, letGo: false, unlisten: null as (() => void) | null });
  const writeRef = useRef(write);
  writeRef.current = write;
  const sessionRef = useRef(session);
  sessionRef.current = session;

  const endDrag = () => {
    const s = st.current;
    s.unlisten?.();
    s.unlisten = null;
    if (!s.dragging) return;
    s.dragging = false;
    s.letGo = false;
    sessionRef.current?.setDragging(false);
  };

  const pump = async () => {
    const s = st.current;
    if (s.busy) return;
    s.busy = true;
    while (s.pending !== null) {
      const v = s.pending;
      s.pending = null;
      await writeRef.current(v);
    }
    s.busy = false;
    if (s.letGo) endDrag();
    if (s.released) {
      s.released = false;
      setDraft(null);
    }
  };

  /** The drag is over: the mark ends once its last write has gone. */
  const letGo = () => {
    const s = st.current;
    if (!s.dragging) return;
    s.unlisten?.();
    s.unlisten = null;
    // After the control's own change event, which writes the release.
    setTimeout(() => {
      if (!s.dragging) return;
      if (s.busy || s.pending !== null) s.letGo = true;
      else endDrag();
    }, 0);
  };

  // Gone mid-drag: the mark ends now.
  useEffect(() => () => endDrag(), []);

  return {
    /** The position shown while it moves; null: show the device's value. */
    draft,
    move(v: number) {
      setDraft(v);
      st.current.pending = v;
      void pump();
    },
    release(v: number) {
      setDraft(v);
      st.current.pending = v;
      st.current.released = true;
      void pump();
    },
    // -- WP WB8 --
    /** A pointer took the control: its writes are one undo step until it lets go. */
    grab() {
      const s = st.current;
      if (!sessionRef.current || s.dragging) return;
      s.dragging = true;
      s.letGo = false;
      sessionRef.current.setDragging(true);
      // Wherever the pointer is let go.
      const end = () => letGo();
      window.addEventListener("pointerup", end);
      window.addEventListener("pointercancel", end);
      s.unlisten = () => {
        window.removeEventListener("pointerup", end);
        window.removeEventListener("pointercancel", end);
      };
    },
    letGo,
    /** Whether a pointer holds the control. */
    get dragging() {
      return st.current.dragging;
    },
  };
}

/** The pointer handlers that mark a slider's drag (`useLiveWrite`'s `grab` and `letGo`). */
export function dragProps(live: { grab(): void; letGo(): void }) {
  return {
    onPointerDown: (e: PointerEvent) => {
      if (e.button === 0 || e.pointerType !== "mouse") live.grab();
    },
    onPointerCancel: () => live.letGo(),
    onLostPointerCapture: () => live.letGo(),
  };
}
