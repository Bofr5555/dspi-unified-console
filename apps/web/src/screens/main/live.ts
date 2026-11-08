// A slider's writes (WP WB6a): while it moves, each new position is written as
// soon as the one before has been read back (the newest wins, nothing queues
// up), so the device follows the drag as on the Console; when it is let go the
// last position is written and the slider shows the device's value again.

import { useRef, useState } from "preact/hooks";

export function useLiveWrite(write: (v: number) => Promise<boolean>) {
  const [draft, setDraft] = useState<number | null>(null);
  const st = useRef({ busy: false, pending: null as number | null, released: false });
  const writeRef = useRef(write);
  writeRef.current = write;

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
    if (s.released) {
      s.released = false;
      setDraft(null);
    }
  };

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
  };
}
