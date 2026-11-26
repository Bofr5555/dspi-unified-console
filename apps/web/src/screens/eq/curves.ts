// The graph's curves (WP WB6b), from the core (`curveGrid`, `curves`): asked
// for only when the bands drawn change, the newest request winning (one in
// flight at a time, the latest waiting), so a drag never queues curves up.
// Each answer is shown as it comes (it is newer than the last shown); older
// waiting requests are dropped for the latest.

import { useEffect, useRef, useState } from "preact/hooks";
import type { BandCurve, FilterParams } from "../../core/protocol";
import type { Session } from "../../ui/session";

export interface Curves {
  /** Each chain's combined level, dB, on `freqs`. */
  chains: number[][];
  /** The edited channel's bands, each alone. */
  bands: BandCurve[];
  /** -- WP GAP1 -- The edited channel's phase in degrees, when asked. */
  phase?: number[];
}

export interface CurveRequest {
  chains: FilterParams[][];
  bands: FilterParams[];
  /** -- WP GAP1 -- Ask for the first chain's phase too (continuous with `unwrap`). */
  phase?: boolean;
  unwrap?: boolean;
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

/**
 * -- WP WB6c -- (WB6b-3) One request in flight at a time; a newer one waits,
 * replacing any that waited before it (dropped, never sent). Every answer is
 * shown as it comes: it is newer than the one shown, so a drag over a slow
 * queue keeps the curve moving instead of freezing until the pointer pauses,
 * and the last request is always sent (never starved).
 */
export class LatestOnly<Q, A> {
  private busy = false;
  private want: Q | null = null;
  alive = true;

  constructor(
    private readonly ask: (q: Q) => Promise<A>,
    private readonly show: (a: A) => void,
  ) {}

  request(q: Q): void {
    this.want = q;
    if (this.busy) return;
    this.busy = true;
    void (async () => {
      while (this.want !== null) {
        const r = this.want;
        this.want = null;
        try {
          const a = await this.ask(r);
          if (this.alive) this.show(a);
        } catch {
          // Kept: the last answer stands (a closed session answers no more).
        }
      }
      this.busy = false;
    })();
  }
}

/** The curves for `req`, which the caller memoises on its `key`; null until the first answer. */
export function useCurves(session: Session, key: string, req: CurveRequest): Curves | null {
  const [curves, setCurves] = useState<Curves | null>(null);
  const pump = useRef<LatestOnly<CurveRequest, Curves> | null>(null);
  if (!pump.current) pump.current = new LatestOnly((r) => session.core.call("curves", r), setCurves);

  useEffect(() => {
    const p = pump.current!;
    p.alive = true;
    return () => {
      p.alive = false;
    };
  }, []);

  useEffect(() => {
    pump.current!.request(req);
    // `key` stands for `req`.
  }, [session, key]);

  return curves;
}
