// A channel's band edits (WP WB6b), as the Console's graph editor runs them
// (`setLive`, `sendToDevice`, `commitLive`): a change in progress is shown at
// once (the graph, the chip and the list follow it) and written to the device
// as fast as the device reads it back, the newest of each band winning and
// nothing queuing up. When the gesture ends the bands show what the device
// kept. A write that fails drops what was waiting (it is never sent later,
// the owner's rule) and the bands snap back to the device's.

import { useMemo, useRef, useState } from "preact/hooks";
import type { FilterParams } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { bandWrites, sameBand } from "./model";

export interface BandEdits {
  /** The bands shown: the device's, with the live changes over them. */
  shown: FilterParams[];
  /** A band as it is now (live or the device's). */
  current(b: number): FilterParams;
  /** Show and write these, the gesture going on (a drag). */
  live(changes: Map<number, FilterParams>): void;
  /** The gesture ended: once written, show the device's. */
  release(): void;
  /** A single edit (a key, a field, a menu): live, then released. */
  commit(changes: Map<number, FilterParams>): void;
}

export function useBandEdits(session: Session, wire: number, device: FilterParams[], disabled: boolean): BandEdits {
  const [overrides, setOverrides] = useState<ReadonlyMap<number, FilterParams>>(new Map());
  const st = useRef({
    busy: false,
    pending: new Map<number, FilterParams>(),
    released: false,
    live: new Map<number, FilterParams>(),
  });
  const ctx = useRef({ session, wire, device });
  ctx.current = { session, wire, device };

  // A new channel starts clean.
  const shownFor = useRef(wire);
  if (shownFor.current !== wire) {
    shownFor.current = wire;
    st.current = { busy: false, pending: new Map(), released: false, live: new Map() };
    if (overrides.size) setOverrides(new Map());
  }

  const show = (m: Map<number, FilterParams>) => {
    st.current.live = m;
    setOverrides(new Map(m));
  };

  const pump = async () => {
    const s = st.current;
    if (s.busy) return;
    s.busy = true;
    const mine = s;
    let failed = false;
    while (mine.pending.size && st.current === mine) {
      const { session, wire } = ctx.current;
      const take = mine.pending;
      mine.pending = new Map();
      // Against what the device has now (the last read-back).
      const now = session.state.snapshot?.state.filters[wire] ?? [];
      const writes = [...take].flatMap(([b, p]) => (now[b] ? bandWrites(wire, b, now[b]!, p) : []));
      if (!writes.length) continue;
      if (!(await session.writeAll(writes))) {
        failed = true;
        mine.pending.clear();
        break;
      }
    }
    mine.busy = false;
    if (st.current !== mine) return;
    if (failed || mine.released) {
      mine.released = false;
      show(new Map());
    }
  };

  return useMemo<BandEdits>(() => {
    const shown = device.map((p, b) => overrides.get(b) ?? p);
    const current = (b: number) => st.current.live.get(b) ?? ctx.current.device[b]!;
    const live = (changes: Map<number, FilterParams>) => {
      if (disabled || !changes.size) return;
      const s = st.current;
      const next = new Map(s.live);
      for (const [b, p] of changes) {
        if (b < 0 || b >= ctx.current.device.length) continue;
        next.set(b, p);
        s.pending.set(b, p);
      }
      s.released = false;
      show(next);
      void pump();
    };
    const release = () => {
      const s = st.current;
      s.released = true;
      if (!s.busy) {
        s.released = false;
        // Only what differs from the device was kept live; drop it now.
        if (s.live.size) show(new Map());
      }
    };
    return {
      shown,
      current,
      live,
      release,
      commit(changes) {
        const real = new Map([...changes].filter(([b, p]) => !sameBand(current(b), p)));
        if (!real.size) return;
        live(real);
        release();
      },
    };
    // `device` and `overrides` are what is shown; the rest reads refs.
  }, [device, overrides, disabled]);
}
