// -- WP WEBM2 -- The Equaliser's Crossover tab (MODLAY 2): every output's crossover curve on one
// graph, and one table of their high and low passes (`PassFields`). An output's name opens its
// PEQ page at its XO bands, where all four bands are listed. The curves are the core's (`curves`),
// asked for when the crossovers change. Read only, nothing is editable.

import { useEffect, useMemo, useState } from "preact/hooks";
import type { DspState, FilterParams } from "../../core/protocol";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { outputRows } from "../main/model";
import { useCurves, useGrid, type CurveRequest } from "./curves";
import { channelColour, DEFAULT_LIMITS } from "./model";
import { PassFields } from "./Passes";
import { EmptyState } from "../../ui/design"; // -- WP WEB --

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
      <section class="card xo-plot-card" aria-label="Crossover curves">
        <XoverPlot
          freqs={freqs}
          curves={curves?.chains ?? []}
          colours={outs.map((o) => channelColour(o.wire))}
          names={outs.map((o) => `${o.descriptor} ${o.name}`)}
        />
      </section>
      <section class="card band-list xo-table" aria-label="Crossovers of every output">
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
                        <span class="swatch" style={{ background: channelColour(o.wire) }} aria-hidden="true" />
                        {who}
                      </button>
                    </th>
                    <PassFields session={session} wire={o.wire} bands={bandsOf(o.wire)} freq={props.freq} disabled={ro} who={who} as="cells" />
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
        <p class="note">The first high and low pass of each output. Its four bands and Bypass are on its PEQ page, XO.</p>
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
    <svg class="xo-plot" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={`Crossover curves of ${props.names.join(", ")}`}>
      {grid.map((f) => (
        <line key={f} class="xo-grid" x1={px(f)} x2={px(f)} y1={0} y2={H} />
      ))}
      {[0, -12, -24, -36].map((db) => (
        <line key={db} class={db === 0 ? "xo-zero" : "xo-grid"} x1={0} x2={W} y1={py(db)} y2={py(db)} />
      ))}
      {props.freqs
        ? props.curves.map((c, i) => (
            <path key={i} class="xo-curve" d={curvePath(props.freqs!, c)} stroke={props.colours[i]} fill="none" vector-effect="non-scaling-stroke" />
          ))
        : null}
      {grid.map((f) => (
        <text key={f} class="xo-label" x={px(f) + 3} y={H - 4}>
          {f >= 1000 ? `${f / 1000}k` : f}
        </text>
      ))}
    </svg>
  );
}
