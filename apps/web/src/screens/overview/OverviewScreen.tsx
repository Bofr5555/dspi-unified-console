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
//
// -- WP WEB3 -- In the Modern language: no second heading under the tab; the inputs' meters and the
// outputs' table as §9's (caps header over one rule, striped rows, each output's pill, a disabled
// one dimmed, the mute icon); the stale meters a notice row.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { ParamInfo } from "../../core/protocol";
import { staleText, useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { liveInputs } from "../main/model";
import { InputMeters, useLive } from "./Meters";
import { anyLimiter, DEFAULT_UNITS, meterChannels, meterReading, overviewRows, type OverviewUnits } from "./model";
import { OutputRow } from "./OutputRow";
import "./overview.css"; // -- WP WEB3 --
import "../eq/eq.css"; // -- WP WEB3 -- §9's table (.eq-table)

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
      {stale ? (
        <div class="notice-row notice-row-warning" role="status" aria-label="Overview meters stale">
          <span class="notice-text">Meters stale since {staleText(Date.now() - (live.meters?.ageMs ?? 0))}: no fresh reading from the DSPi.</span>
        </div>
      ) : null}
      <InputMeters channels={inputs} meters={live.meters} />
      <div class="eq-table ov-table-wrap">
      <table class="ov-table" role="table" aria-label="Outputs">
        {/* -- WP REVEFIX -- (REVE D3) a unit stays as written beside its caps label ("GAIN (dB)", not "(DB)") */}
        <thead>
          <tr role="row">
            <th scope="col" role="columnheader">Output</th>
            <th scope="col" role="columnheader">Routing</th>
            <th scope="col" role="columnheader">Gain <span class="th-unit">({units.gain.unit})</span></th>
            <th scope="col" role="columnheader">Delay <span class="th-unit">({units.delay.unit})</span></th>
            <th scope="col" role="columnheader">Mute</th>
            <th scope="col" role="columnheader">Limiter <span class="th-unit">({units.threshold.unit})</span></th>
            <th scope="col" role="columnheader">Reduction</th>
            <th scope="col" role="columnheader">Clip</th>
            <th scope="col" role="columnheader">Level</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <OutputRow key={r.index} session={session} row={r} units={units} meter={meterReading(live.meters, r.wire)} gr={live.gr} stale={stale} disabled={ro} s={snap.state} />
          ))}
        </tbody>
      </table>
      </div>
    </section>
  );
}
