// -- WP WEBM2 -- The Equaliser's Crossover tab (MODLAY 2): every output's crossover curve on one
// graph, and one table of their high and low passes (`PassFields`). An output's name opens its
// PEQ page at its XO bands, where all four bands are listed. The curves are the core's (`curves`),
// asked for when the crossovers change. Read only, nothing is editable.
//
// -- WP WEB3 -- (DL8 on the web, REVD W7) The graph unboxed on the panel tone, its axis labels in
// HTML inside the plot (6 px in, never stretched or cut); the table as §9's (each output's pill),
// with Qt's legend under it.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { DspState, FilterParams } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { outputRows } from "../main/model";
import { useCurves, useGrid, type CurveRequest } from "./curves";
import { channelColour, DEFAULT_LIMITS } from "./model";
import { PassFields } from "./Passes";
import { EmptyState } from "../../ui/design"; // -- WP WEB --
import { Pill } from "../../ui/Pill"; // -- WP WEB3 --
import "./eq.css"; // -- WP WEB3 --

/** The plot's range: 20 Hz..20 kHz, +6..-48 dB. */
const PLOT = { minFreq: 20, maxFreq: 20000, dbTop: 6, dbBottom: -48 };
const W = 600;
const H = 220;

export function CrossoverScreen(props: ScreenProps) {
  const { session } = props;
  const st = useSessionState(session);
  const s = st.snapshot?.state;
  const [freq, setFreq] = useState<[number, number]>(DEFAULT_LIMITS.freq);
  useEffect(() => {
    let alive = true;
    session.core.call("describe", { path: "eq.freq" }).then(
      (i) => alive && i.min !== undefined && i.max !== undefined && setFreq([i.min, i.max]),
      () => undefined,
    );
    return () => {
      alive = false;
    };
  }, [session]);
  if (!s) return <EmptyState icon="chart-line" title="Reading the outputs…" text="This DSPi's outputs are not known yet." />; // -- WP WEB --
  if (!Array.isArray(s.xover) || !s.xover.length) return <EmptyState icon="chart-line" title="No crossovers" text="This DSPi's firmware has no crossovers." />;
  return <CrossoverView session={session} s={s} freq={freq} labels={st.snapshot?.outputLabels} open={props.open} />;
}

function CrossoverView(props: {
  session: ScreenProps["session"];
  s: DspState;
  freq: [number, number];
  labels: Parameters<typeof outputRows>[1];
  open: ScreenProps["open"];
}) {
  const { session, s } = props;
  const ro = session.readOnlyReason !== "";
  const outs = outputRows(s, props.labels);
  const bandsOf = (wire: number): FilterParams[] => s.xover?.[wire] ?? [];
  const req = useMemo<CurveRequest>(
    () => ({ chains: outs.map((o) => bandsOf(o.wire)), bands: [] }),
    [JSON.stringify(outs.map((o) => [o.wire, bandsOf(o.wire)]))],
  );
  const curves = useCurves(session, JSON.stringify(req), req);
  const freqs = useGrid(session);
  return (
    <div class="eq-xover">
      <section class="xo-plot-card" aria-label="Crossover curves">
        <XoverPlot
          freqs={freqs}
          curves={curves?.chains ?? []}
          colours={outs.map((o) => channelColour(s, o.wire))}
          names={outs.map((o) => `${o.descriptor} ${o.name}`)}
        />
      </section>
      <section class="band-list xo-table eq-table" aria-label="Crossovers of every output">
        <div class="tbl-scroll">
          <table>
            <thead>
              <tr>
                <th scope="col">Output</th>
                <th scope="col">High pass</th>
                <th scope="col" class="c-freq">
                  Freq
                </th>
                <th scope="col">Low pass</th>
                <th scope="col" class="c-freq">
                  Freq
                </th>
              </tr>
            </thead>
            <tbody>
              {outs.map((o) => {
                const who = `${o.descriptor} ${o.name}`;
                return (
                  <tr key={o.wire}>
                    <th scope="row">
                      <button
                        type="button"
                        class="link xo-out"
                        title={`Open ${who}'s XO bands`}
                        onClick={() => props.open?.({ screen: "eq", wire: o.wire, tab: "xo" })}
                      >
                        <Pill text={o.descriptor} colour={channelColour(s, o.wire)} />
                        <span class="eq-who-name">{o.name}</span>
                      </button>
                    </th>
                    <PassFields session={session} wire={o.wire} bands={bandsOf(o.wire)} freq={props.freq} disabled={ro} who={who} as="cells" />
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <p class="eq-help">LR is Linkwitz-Riley, BW Butterworth, BS Bessel. The first high and low pass of each output; its four bands and Bypass are on its PEQ page, XO.</p>
      </section>
    </div>
  );
}

/** The x of `f` on the plot. */
function px(f: number): number {
  return (Math.log10(f / PLOT.minFreq) / Math.log10(PLOT.maxFreq / PLOT.minFreq)) * W;
}

function py(db: number): number {
  const d = Math.max(PLOT.dbBottom, Math.min(PLOT.dbTop, db));
  return ((PLOT.dbTop - d) / (PLOT.dbTop - PLOT.dbBottom)) * H;
}

/** One curve as an SVG path. */
export function curvePath(freqs: readonly number[], curve: readonly number[]): string {
  let d = "";
  for (let i = 0; i < freqs.length && i < curve.length; i++) {
    const f = freqs[i]!;
    if (f < PLOT.minFreq || f > PLOT.maxFreq || !Number.isFinite(curve[i]!)) continue;
    d += `${d ? "L" : "M"}${px(f).toFixed(1)} ${py(curve[i]!).toFixed(1)}`;
  }
  return d;
}

function XoverPlot(props: { freqs: number[] | null; curves: number[][]; colours: string[]; names: string[] }) {
  const grid = [100, 1000, 10000];
  return (
    <div class="xo-frame">
      <svg class="xo-plot" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={`Crossover curves of ${props.names.join(", ")}`}>
        {[50, 200, 500, 2000, 5000].map((f) => (
          <line key={f} class="xo-grid xo-minor" x1={px(f)} x2={px(f)} y1={0} y2={H} />
        ))}
        {grid.map((f) => (
          <line key={f} class="xo-grid" x1={px(f)} x2={px(f)} y1={0} y2={H} />
        ))}
        {Y_TICKS.map((db) => (
          <line key={db} class={db === 0 ? "xo-zero" : "xo-grid"} x1={0} x2={W} y1={py(db)} y2={py(db)} />
        ))}
        {props.freqs
          ? props.curves.map((c, i) => (
              <path key={i} class="xo-curve" d={curvePath(props.freqs!, c)} stroke={props.colours[i]} fill="none" vector-effect="non-scaling-stroke" />
            ))
          : null}
      </svg>
      {/* -- WP WEB3 -- the labels in HTML over the plot, at its proportions, 6 px in (never stretched) */}
      <div class="xo-labels" aria-hidden="true">
        {X_TICKS.map((f) => (
          <span key={f} class="xo-x" style={{ left: `${((px(f) / W) * 100).toFixed(2)}%` }} data-edge={f === X_TICKS[0] ? "start" : f === X_TICKS[X_TICKS.length - 1] ? "end" : undefined}>
            {f >= 1000 ? `${f / 1000}k` : f}
          </span>
        ))}
        {Y_TICKS.map((db) => (
          <span key={db} class="xo-y" style={{ top: `${((py(db) / H) * 100).toFixed(2)}%` }} data-edge={db === Y_TICKS[0] ? "start" : db === Y_TICKS[Y_TICKS.length - 1] ? "end" : undefined}>
            {db > 0 ? `+${db}` : db === 0 ? "0" : `\u2212${-db}`}
          </span>
        ))}
      </div>
    </div>
  );
}

/** -- WP WEB3 -- The axis labels: Qt's frequencies, and every 12 dB. */
const X_TICKS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
const Y_TICKS = [6, 0, -12, -24, -36, -48];
