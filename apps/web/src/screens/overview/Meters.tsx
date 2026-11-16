// WP P3OV: the Overview's meters. One poll of the session's meter feed (the
// core's `meters`: peaks, the held peak its ballistics decay, the clip hold)
// while the page is seen and the session open, and, while a limiter is on, the
// limiters' gain reduction (`limiterMeter`) at a slower pace. A bar per
// channel with its peak-hold marker and its clip marker.

import { memo } from "preact/compat";
import { useEffect, useRef, useState } from "preact/hooks";
import { CoreCallError, type SystemStatus } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { meterPct, meterReading, type MeterChannel, type MeterReading } from "./model";

/** How often the meters are read while the Overview is shown (the System view's pace). */
const METER_MS = 150;
/** How often the gain reduction is read while a limiter is on. */
const GR_MS = 500;

export interface Live {
  meters: SystemStatus | null;
  /** Gain reduction per output, dB; null when not read (no limiter on, or none on the firmware). */
  gr: number[] | null;
}

/** The live readings, polled only while the page is visible and the session open. */
export function useLive(session: Session, wantGr: boolean): Live {
  const [meters, setMeters] = useState<SystemStatus | null>(null);
  const [gr, setGr] = useState<number[] | null>(null);
  const want = useRef(wantGr);
  want.current = wantGr;
  useEffect(() => {
    let alive = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let lastGr = 0;
    let noLimiter = false;
    const tick = async () => {
      if (!alive) return;
      if (document.visibilityState === "visible" && session.state.status === "open") {
        try {
          const m = await session.core.call("meters", undefined);
          if (alive) setMeters(m);
        } catch {
          // The last reading stands; the session's strip shows its state.
        }
        const now = Date.now();
        if (!want.current || noLimiter) {
          if (alive) setGr(null);
        } else if (now - lastGr >= GR_MS) {
          lastGr = now;
          try {
            const r = await session.core.call("limiterMeter", undefined);
            if (alive) setGr(Array.isArray(r) ? r : null);
          } catch (e) {
            if (e instanceof CoreCallError && e.code === "unsupported") noLimiter = true;
          }
        }
      }
      if (alive) timer = setTimeout(tick, METER_MS);
    };
    void tick();
    return () => {
      alive = false;
      if (timer) clearTimeout(timer);
    };
  }, [session]);
  return { meters, gr };
}

/** One channel's bar: the level, the held peak's marker, the clip marker. */
export const MeterBar = memo(function MeterBar(props: { name: string; r: MeterReading; stale?: boolean }) {
  const { r } = props;
  return (
    <span
      class={`ov-meter${r.clip ? " clip" : ""}${props.stale ? " stale" : ""}`}
      role="meter"
      aria-valuemin={-90}
      aria-valuemax={0}
      aria-valuenow={Math.round(r.db)}
      aria-valuetext={`${Math.round(r.db)} dB, peak ${Math.round(r.holdDb)} dB${r.clip ? ", clipped" : ""}`}
      aria-label={`Meter ${props.name}`}
      data-hold={Math.round(r.holdDb)}
    >
      <span class="ov-meter-fill" style={{ width: `${meterPct(r.db)}%` }} />
      <span class="ov-meter-hold" style={{ left: `calc(${meterPct(r.holdDb)}% - 1px)` }} />
    </span>
  );
});

/** The inputs' meters, one row each, the outputs' sit in their rows. */
export function InputMeters(props: { channels: MeterChannel[]; meters: SystemStatus | null }) {
  const stale = props.meters?.stale === true;
  return (
    <section class="ov-inputs" aria-label="Input meters">
      <h4>Inputs</h4>
      <ul class="ov-input-list">
        {props.channels.map((c) => (
          <li key={c.wire} class="ov-input">
            <span class="ov-input-name" title={c.name}>
              <span class="ov-kind">{c.descriptor}</span> {c.name}
            </span>
            <MeterBar name={`${c.descriptor} ${c.name}`} r={meterReading(props.meters, c.wire)} stale={stale} />
          </li>
        ))}
      </ul>
    </section>
  );
}
