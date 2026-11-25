// -- WP WEB6 -- The Crossover tab's centre in the Equaliser's frame (Qt's EqualiserCrossover; the
// frame is EqScreen's with `eqTab` "crossover", CrossoverScreen.tsx): the graph, the table of every
// output's passes, the legend, and the chosen output's four crossover bands. Moved here from
// CrossoverScreen.tsx (WEBM2, WEB3), which now only puts the tab in the frame.

import { useMemo } from "preact/hooks";
import type { DspState, FilterParams, OutputLabel } from "../../core/protocol";
import type { Ask } from "../../ui/screens";
import type { Session } from "../../ui/session";
import { outputLabel } from "../main/model";
import { CrossoverList } from "../outputs/Crossover";
import { useCurves, useGrid, type CurveRequest } from "./curves";
import { bypassSupported, channelColour } from "./model";
import { PassFields } from "./Passes";
import { Pill } from "../../ui/Pill"; // -- WP WEB3 --

/** The plot's range: 20 Hz..20 kHz, ±25 dB (Qt's graph). */
const PLOT = { minFreq: 20, maxFreq: 20000, dbTop: 25, dbBottom: -25 };
const W = 600;
const H = 220;

/** One output's row: its wire, number, label and whether it is enabled. */
interface OutRow {
  wire: number;
  index: number;
  descriptor: string;
  name: string;
  enabled: boolean;
}

/** -- WP WEB6 -- Every output, enabled or not (Qt's table lists all of them). */
function everyOutput(s: DspState, labels?: OutputLabel[]): OutRow[] {
  return Array.from({ length: s.numOutputChannels }, (_, o) => ({
    wire: s.numInputChannels + o,
    index: o,
    descriptor: `OUT${o + 1}`,
    name: outputLabel(s, labels, o),
    enabled: s.outputEnabled[o] !== false,
  }));
}

/** The Crossover tab's centre: the graph, the table, the legend, the chosen output's bands. */
export function CrossoverCentre(props: {
  session: Session;
  s: DspState;
  freq: [number, number];
  labels?: OutputLabel[];
  /** The page's chosen output's wire; -1 when an input is chosen. */
  chosen: number;
  choose: (wire: number) => void;
  ask: Ask;
}) {
  const { session, s } = props;
  const ro = session.readOnlyReason !== "";
  const outs = everyOutput(s, props.labels);
  const drawn = outs.filter((o) => o.enabled);
  const bandsOf = (wire: number): FilterParams[] => s.xover?.[wire] ?? [];
  const peqOf = (wire: number): FilterParams[] => (s.filters?.[wire] ?? []).slice(0, s.maxBands || 10);
  // Each enabled output's response: its PEQ and crossover (flat while the EQ is bypassed: the crossover only)
  const req = useMemo<CurveRequest>(
    () => ({ chains: drawn.map((o) => (s.bypass ? bandsOf(o.wire) : [...peqOf(o.wire), ...bandsOf(o.wire)])), bands: [] }),
    [JSON.stringify([s.bypass, drawn.map((o) => [o.wire, peqOf(o.wire), bandsOf(o.wire)])])],
  );
  const curves = useCurves(session, JSON.stringify(req), req);
  const freqs = useGrid(session);
  // The chosen output last, so it is drawn over the others
  const order = drawn.map((o, i) => ({ o, i })).sort((a, b) => Number(a.o.wire === props.chosen) - Number(b.o.wire === props.chosen));
  const chosenRow = outs.find((o) => o.wire === props.chosen) ?? null;
  return (
    <div class="eq-xover">
      <section class="xo-plot-card" aria-label="Crossover curves">
        <XoverPlot
          freqs={freqs}
          curves={order.map(({ i }) => curves?.chains[i] ?? [])}
          offsets={order.map(({ o }) => s.outputGainDb[o.index] ?? 0)}
          colours={order.map(({ o }) => channelColour(s, o.wire))}
          active={order.map(({ o }) => o.wire === props.chosen)}
          names={order.map(({ o }) => `${o.descriptor} ${o.name}`)}
        />
      </section>
      <section class="band-list xo-table eq-table" aria-label="Crossovers of every output">
        <div class="tbl-scroll">
          <table>
            <thead>
              <tr>
                <th scope="col">Output</th>
                <th scope="colgroup" colSpan={2}>
                  High pass
                </th>
                <th scope="colgroup" colSpan={2}>
                  Low pass
                </th>
              </tr>
            </thead>
            <tbody>
              {outs.map((o) => {
                const who = `${o.descriptor} ${o.name}`;
                const on = o.wire === props.chosen;
                return (
                  <tr key={o.wire} class={`${on ? "chosen" : ""}${o.enabled ? "" : " xo-disabled"}`.trim() || undefined} onClick={() => props.choose(o.wire)}>
                    <th scope="row">
                      <button type="button" class="link xo-out" aria-pressed={on} title={`Choose ${who}`} onClick={() => props.choose(o.wire)}>
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
        <p class="eq-help">LR is Linkwitz-Riley, BW Butterworth, BS Bessel.</p>
      </section>
      {chosenRow ? (
        // The chosen output's four bands: a family the table cannot show, a second pass, Bypass
        <section class="xo-bands" aria-label={`Crossover bands of ${chosenRow.descriptor} ${chosenRow.name}`}>
          <h4 class="section-label">Crossover bands of {chosenRow.name}</h4>
          <CrossoverList
            session={session}
            wire={chosenRow.wire}
            channelName={`${chosenRow.descriptor} ${chosenRow.name}`}
            bands={bandsOf(chosenRow.wire)}
            freq={props.freq}
            bypassOk={bypassSupported(s)}
            disabled={ro}
            ask={props.ask}
          />
        </section>
      ) : null}
    </div>
  );
}

/** The x of `f` on the plot. */
function px(f: number): number {
  return (Math.log10(f / PLOT.minFreq) / Math.log10(PLOT.maxFreq / PLOT.minFreq)) * W;
}

function py(db: number): number {
  const d = Math.max(PLOT.dbBottom - 1, Math.min(PLOT.dbTop + 1, db));
  return ((PLOT.dbTop - d) / (PLOT.dbTop - PLOT.dbBottom)) * H;
}

/** One curve as an SVG path, shifted by `offset` dB. */
export function curvePath(freqs: readonly number[], curve: readonly number[], offset = 0): string {
  let d = "";
  for (let i = 0; i < freqs.length && i < curve.length; i++) {
    const f = freqs[i]!;
    if (f < PLOT.minFreq || f > PLOT.maxFreq || !Number.isFinite(curve[i]!)) continue;
    d += `${d ? "L" : "M"}${px(f).toFixed(1)} ${py(curve[i]! + offset).toFixed(1)}`;
  }
  return d;
}

function XoverPlot(props: { freqs: number[] | null; curves: number[][]; offsets: number[]; colours: string[]; active: boolean[]; names: string[] }) {
  return (
    <div class="xo-frame">
      <svg class="xo-plot" viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none" role="img" aria-label={`Responses of ${props.names.join(", ")}`}>
        {X_TICKS.map((f) => (
          <line key={f} class={`xo-grid${[100, 1000, 10000].includes(f) ? "" : " xo-minor"}`} x1={px(f)} x2={px(f)} y1={0} y2={H} />
        ))}
        {Y_LINES.map((db) => (
          <line key={db} class={db === 0 ? "xo-zero" : "xo-grid xo-minor"} x1={0} x2={W} y1={py(db)} y2={py(db)} />
        ))}
        {props.freqs
          ? props.curves.map((c, i) => (
              <path
                key={i}
                class={`xo-curve${props.active[i] ? " active" : ""}`}
                d={curvePath(props.freqs!, c, props.offsets[i] ?? 0)}
                stroke={props.colours[i]}
                fill="none"
                vector-effect="non-scaling-stroke"
              />
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
          <span key={db} class="xo-y" style={{ top: `${((py(db) / H) * 100).toFixed(2)}%` }} data-edge={db === Y_TICKS[0] ? "start" : undefined}>
            {db > 0 ? `+${db}` : db === 0 ? "0" : `−${-db}`}
          </span>
        ))}
      </div>
    </div>
  );
}

/** -- WP WEB3 -- The axis labels: Qt's frequencies; -- WP WEB6 -- Qt's dB steps of 5 (the foot's −25 unlabelled, under the Hz labels). */
const X_TICKS = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000];
const Y_LINES = [20, 15, 10, 5, 0, -5, -10, -15, -20];
const Y_TICKS = [25, 20, 15, 10, 5, 0, -5, -10, -15, -20];
