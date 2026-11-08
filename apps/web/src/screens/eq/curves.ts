// The graph's curves (WP WB6b), from the core (`curveGrid`, `curves`): asked
// for only when the bands drawn change, the newest request winning (one in
// flight at a time, the latest waiting), so a drag never queues curves up.

import { useEffect, useRef, useState } from "preact/hooks";
import type { BandCurve, FilterParams } from "../../core/protocol";
import type { Session } from "../../ui/session";

export interface Curves {
  /** Each chain's combined level, dB, on `freqs`. */
  chains: number[][];
  /** The edited channel's bands, each alone. */
  bands: BandCurve[];
}

export interface CurveRequest {
  chains: FilterParams[][];
  bands: FilterParams[];
}

/** The core's grid, once per session. */
export function useGrid(session: Session): number[] | null {
  const [freqs, setFreqs] = useState<number[] | null>(null);
  useEffect(() => {
    let alive = true;
    session.core.call("curveGrid", undefined).then(
      (g) => alive && setFreqs(g.freqs),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);
  return freqs;
}

/** The curves for `req`, which the caller memoises on its `key`; null until the first answer. */
export function useCurves(session: Session, key: string, req: CurveRequest): Curves | null {
  const [curves, setCurves] = useState<Curves | null>(null);
  const st = useRef({ busy: false, want: null as CurveRequest | null, alive: true });

  useEffect(() => {
    st.current.alive = true;
    return () => {
      st.current.alive = false;
    };
  }, []);

  useEffect(() => {
    const s = st.current;
    s.want = req;
    if (s.busy) return;
    s.busy = true;
    void (async () => {
      while (s.want) {
        const r = s.want;
        s.want = null;
        try {
          const c = await session.core.call("curves", r);
          if (s.alive && !s.want) setCurves(c);
        } catch {
          // Kept: the last curves stand (a closed session answers no more).
        }
      }
      s.busy = false;
    })();
    // `key` stands for `req`.
  }, [session, key]);

  return curves;
}
