// -- WP WEBM2 -- The Equaliser's Delay & gain tab (MODLAY 2): every output's gain, delay and the
// distance it stands for, an arrival bar for lining the outputs up in time, and mute and the
// limiter. Typing a distance writes the delay it takes sound to travel it. Writes are the output
// card's (`out.gain`, `out.delay`, `out.mute`, `limit.on`: the core's guards, limits and undo);
// read only, nothing is editable. Disabled outputs are listed, greyed, as on the Overview.

import { useEffect, useState } from "preact/hooks";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { overviewRows } from "../overview/model";
import { clamp, delayText, levelText, limiterSupported, parseLevel } from "../outputs/model";
import { arrivals, delayToMetres, distanceText, metresToDelay, parseDistance, SOUND_MPS } from "./align";
import { ValueField } from "./Field";
import { channelColour } from "./model";

const DEFAULT_GAIN: [number, number] = [-60, 20];
const DEFAULT_DELAY: [number, number] = [0, 21];

export function DelaysScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const snap = st.snapshot;
  const [gain, setGain] = useState(DEFAULT_GAIN);
  const [delay, setDelay] = useState(DEFAULT_DELAY);
  useEffect(() => {
    let alive = true;
    void Promise.all([
      session.core.call("describe", { path: "out.gain" }).catch(() => null),
      session.core.call("outputLimits", undefined).catch(() => null),
    ]).then(([g, d]) => {
      if (!alive) return;
      if (g?.min !== undefined && g.max !== undefined) setGain([g.min, g.max]);
      if (d) setDelay([0, d.maxDelayMs]);
    });
    return () => {
      alive = false;
    };
  }, [session]);
  if (!snap) return <p class="empty">This DSPi's outputs are not known yet.</p>;
  const s = snap.state;
  const ro = session.readOnlyReason !== "";
  const rows = overviewRows(s, snap.outputLabels);
  const bars = arrivals(rows.map((r) => (r.enabled ? r.delayMs : 0)));
  const limiter = limiterSupported(s);
  return (
    <section class="card band-list delays" aria-label="Delay and gain of every output">
      <div class="tbl-scroll">
        <table>
          <thead>
            <tr>
              <th scope="col">Output</th>
              <th scope="col">Gain</th>
              <th scope="col">Delay</th>
              <th scope="col">Distance</th>
              <th scope="col" class="c-arrival">
                Arrival
              </th>
              <th scope="col">Mute</th>
              {limiter ? <th scope="col">Limiter</th> : null}
            </tr>
          </thead>
          <tbody>
            {rows.map((r, i) => {
              const who = `${r.descriptor} ${r.name}`;
              const o = r.index;
              const writeDelay = (ms: number) => void session.write("out.delay", { float: clamp(ms, delay) }, [o]);
              return (
                <tr key={o} class={r.enabled ? undefined : "xover-off"} aria-label={who} data-output={o}>
                  <th scope="row">
                    <span class="swatch" style={{ background: channelColour(r.wire) }} aria-hidden="true" />
                    {who}
                    {r.enabled ? null : <span class="ov-state"> Disabled</span>}
                  </th>
                  <td>
                    <ValueField
                      label={`${who} gain`}
                      text={levelText(r.gainDb)}
                      parse={(t) => parseLevel(t, "db")}
                      disabled={ro}
                      onCommit={(v) => void session.write("out.gain", { float: clamp(v, gain) }, [o])}
                    />
                  </td>
                  <td>
                    <ValueField label={`${who} delay`} text={delayText(r.delayMs)} parse={(t) => parseLevel(t, "ms")} disabled={ro} onCommit={writeDelay} />
                  </td>
                  <td>
                    <ValueField
                      label={`${who} distance`}
                      text={distanceText(delayToMetres(r.delayMs))}
                      parse={parseDistance}
                      disabled={ro}
                      onCommit={(m) => writeDelay(metresToDelay(m))}
                    />
                  </td>
                  <td class="c-arrival">
                    <span
                      class="arrival"
                      role="meter"
                      aria-label={`${who} arrival`}
                      aria-valuemin={0}
                      aria-valuemax={100}
                      aria-valuenow={Math.round(bars[i]! * 100)}
                    >
                      <span style={{ width: `${(bars[i]! * 100).toFixed(1)}%`, background: channelColour(r.wire) }} />
                    </span>
                  </td>
                  <td>
                    <button
                      type="button"
                      class="toggle"
                      aria-pressed={r.muted}
                      aria-label={`Mute ${who}`}
                      disabled={ro}
                      onClick={() => void session.write("out.mute", { bool: !r.muted }, [o])}
                    >
                      {r.muted ? "Muted" : "Mute"}
                    </button>
                  </td>
                  {limiter ? (
                    <td>
                      <button
                        type="button"
                        class="toggle"
                        aria-pressed={r.limiter?.on ?? false}
                        aria-label={`Limiter on ${who}`}
                        title={r.limiter?.on ? `Ceiling ${levelText(r.limiter.thresholdDb, "dBFS")}` : "Off"}
                        disabled={ro}
                        onClick={() => void session.write("limit.on", { bool: !(r.limiter?.on ?? false) }, [o])}
                      >
                        {r.limiter?.on ? levelText(r.limiter.thresholdDb, "dBFS") : "Off"}
                      </button>
                    </td>
                  ) : null}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      <p class="note">Time alignment across the outputs: the longest delay fills its bar. Distances at {SOUND_MPS} m/s; Measure can work the delays out.</p>
    </section>
  );
}
