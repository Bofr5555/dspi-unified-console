// WP P3OV: the Overview of the session's device (PLAN-P3 U1, OD-P3j): every
// output in one table (routing sources, gain, delay, mute, limiter threshold
// and gain reduction, clip, meter) and the live inputs' meters, each with its
// peak hold. Gain and mute are edited inline through the session's writes, as
// on the output card (the core's guards, installer limits and undo; nothing is
// editable while read only). Labels and units are the registry's (`describe`)
// and the core's output labels. Dense at desktop width; a card per output at a
// phone's width (a container query: the session's own width, so a fleet page
// with two consoles side by side gets cards in each where they do not fit).
// Shown in every mode; it needs no hub.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { ParamInfo } from "../../core/protocol";
import { staleText, useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { liveInputs } from "../main/model";
import { InputMeters, useLive } from "./Meters";
import { anyLimiter, DEFAULT_UNITS, meterChannels, meterReading, overviewRows, type OverviewUnits } from "./model";
import { OutputRow } from "./OutputRow";

/** The registry's range and units of the Overview's columns, read once per session. */
function useUnits(session: ScreenProps["session"]): OverviewUnits {
  const [units, setUnits] = useState<OverviewUnits>(DEFAULT_UNITS);
  useEffect(() => {
    let alive = true;
    const d = (path: string) => session.core.call("describe", { path }).catch((): ParamInfo | null => null);
    void Promise.all([d("out.gain"), d("out.delay"), d("limit.threshold")]).then(([g, dl, th]) => {
      if (!alive) return;
      const D = DEFAULT_UNITS;
      setUnits({
        gain: {
          range: g?.min !== undefined && g.max !== undefined ? [g.min, g.max] : D.gain.range,
          unit: g?.unit || D.gain.unit,
        },
        delay: { unit: dl?.unit || D.delay.unit },
        threshold: { unit: th?.unit || D.threshold.unit },
      });
    });
    return () => {
      alive = false;
    };
  }, [session]);
  return units;
}

export function OverviewScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;
  const units = useUnits(session);
  const rows = useMemo(() => (snap ? overviewRows(snap.state, snap.outputLabels) : []), [snap]);
  const live = useLive(session, anyLimiter(rows));
  if (!snap) return null;
  const ro = session.readOnlyReason !== "";
  const stale = live.meters?.stale === true;
  const inputs = meterChannels(snap.state, snap.outputLabels, liveInputs(snap.state, live.meters?.activeInputChannels ?? null)).filter((c) => c.kind === "input");
  return (
    <section class="overview" aria-label="Overview">
      <h3>Overview</h3>
      {stale ? (
        <p class="note" role="status" aria-label="Overview meters stale">
          Meters stale since {staleText(Date.now() - (live.meters?.ageMs ?? 0))}: no fresh reading from the DSPi.
        </p>
      ) : null}
      <InputMeters channels={inputs} meters={live.meters} />
      <table class="ov-table" role="table" aria-label="Outputs">
        <thead>
          <tr role="row">
            <th scope="col" role="columnheader">Output</th>
            <th scope="col" role="columnheader">Routing</th>
            <th scope="col" role="columnheader">Gain ({units.gain.unit})</th>
            <th scope="col" role="columnheader">Delay ({units.delay.unit})</th>
            <th scope="col" role="columnheader">Mute</th>
            <th scope="col" role="columnheader">Limiter ({units.threshold.unit})</th>
            <th scope="col" role="columnheader">Reduction</th>
            <th scope="col" role="columnheader">Clip</th>
            <th scope="col" role="columnheader">Level</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <OutputRow key={r.index} session={session} row={r} units={units} meter={meterReading(live.meters, r.wire)} gr={live.gr} stale={stale} disabled={ro} />
          ))}
        </tbody>
      </table>
    </section>
  );
}
