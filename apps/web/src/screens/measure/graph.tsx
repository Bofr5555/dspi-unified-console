// WP P5WMS: the measurement graph (PLAN-P5 T1, T3, T9): curves on a log-frequency axis, the
// measured solid, the target dashed, the predicted dotted, the RTA's held copy and compare;
// untrusted points faint (never hidden), a spread band behind a curve. A click picks a frequency
// and a drag a span (the live view's "cut this peak" and "fit this range"); the same values have
// fields beside the graph, so nothing needs a pointer. P5WAT's tune draws on it too.

import { useLayoutEffect, useRef, useState } from "preact/hooks";
import type { MeasureCurveRecord } from "../../core/measure-protocol";
import { GRAPH_MAX_HZ, GRAPH_MIN_HZ, freqOf, hz, segments, xOf, yRange } from "./model";

export type SeriesKind = "measured" | "target" | "predicted" | "held" | "compare";

export interface Series {
  /** A name for the legend and the tests ("Output 1", "Target"). */
  label: string;
  kind: SeriesKind;
  curve: MeasureCurveRecord;
  /** A band behind the curve, dB per point (the spread across positions, centred on the curve). */
  spreadDb?: number[];
  /** The n-th measured curve: its colour. */
  colour?: number;
}

// -- WP WEB4 -- DL2's GraphFrame (§12): no box, the plot to the edges, the axis labels inside it 6 px
// from the edge (never cut, never stretched: the drawing is the element's own size), the grid in the
// graph grid's tone, "No curve yet" while there is nothing to draw. The plot's height is fixed.
const DEFAULT_W = 720;
const H = 300;
const INSET = 6;
const PAD_L = 0;
const PAD_B = 0;
const PAD_T = 0;
const PAD_R = 0;
const COLOURS = ["var(--accent)", "#d9822b", "#2f9e5c", "#a64fbf", "#c0362c", "#2a9fa8", "#8a6d3b", "#5f6068"];
const DASH: Record<SeriesKind, string | undefined> = { measured: undefined, target: "8 5", predicted: "2 4", held: "4 3", compare: undefined };
const TICKS_HZ = [20, 50, 100, 200, 500, 1000, 2000, 5000, 10_000, 20_000];

function colourOf(s: Series): string {
  if (s.kind === "target") return "var(--muted)";
  if (s.kind === "predicted") return "var(--ok)";
  if (s.kind === "held") return "var(--warn)";
  return COLOURS[(s.colour ?? 0) % COLOURS.length]!;
}

export function MeasureGraph(props: {
  label: string;
  series: Series[];
  /** The y unit's words ("dB", "dB SPL"). */
  unit?: string;
  /** A click on the plot: the frequency there. */
  onPick?: (f: number) => void;
  /** A drag across the plot: the span. */
  onSpan?: (f1: number, f2: number) => void;
  /** A marked frequency or span (the pending adjustment). */
  mark?: { f1: number; f2?: number } | null;
}) {
  const { series } = props;
  // -- WP WEB4 -- the drawing in the element's own pixels, so its text keeps its size at every width
  const box = useRef<HTMLDivElement>(null);
  const [W, setW] = useState(DEFAULT_W);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const fit = () => {
      const w = Math.round(el.getBoundingClientRect().width);
      if (w > 0) setW(w);
    };
    fit();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  const PLOT_W = W - PAD_L - PAD_R;
  const PLOT_H = H - PAD_T - PAD_B;
  const [lo, hi] = yRange(series.map((s) => s.curve));
  const y = (v: number) => PAD_T + PLOT_H - ((Math.min(hi, Math.max(lo, v)) - lo) / (hi - lo)) * PLOT_H;
  const x = (f: number) => PAD_L + xOf(f, PLOT_W);
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<number | null>(null);
  const [band, setBand] = useState<[number, number] | null>(null);

  const plotX = (ev: PointerEvent): number | null => {
    const r = svg.current?.getBoundingClientRect();
    if (!r || r.width <= 0) return null;
    return ((ev.clientX - r.left) / r.width) * W - PAD_L;
  };
  const interactive = !!(props.onPick || props.onSpan);
  const down = (ev: PointerEvent) => {
    const px = plotX(ev);
    if (px === null || !interactive) return;
    drag.current = px;
    setBand(null);
  };
  const move = (ev: PointerEvent) => {
    const px = plotX(ev);
    if (px === null || drag.current === null || !props.onSpan) return;
    if (Math.abs(px - drag.current) > 6) setBand([Math.min(px, drag.current), Math.max(px, drag.current)]);
  };
  const up = (ev: PointerEvent) => {
    const px = plotX(ev);
    const start = drag.current;
    drag.current = null;
    setBand(null);
    if (px === null || start === null) return;
    if (Math.abs(px - start) > 6 && props.onSpan) {
      const a = freqOf(Math.min(px, start), PLOT_W);
      const b = freqOf(Math.max(px, start), PLOT_W);
      props.onSpan(a, b);
    } else props.onPick?.(freqOf(px, PLOT_W));
  };

  const path = (pts: [number, number][]) => pts.map(([f, m], i) => `${i ? "L" : "M"}${x(f).toFixed(1)},${y(m).toFixed(1)}`).join("");
  const spreadPath = (c: MeasureCurveRecord, sp: number[]) => {
    const top: string[] = [];
    const bottom: string[] = [];
    for (let i = 0; i < c.freqsHz.length; i++) {
      const f = c.freqsHz[i]!;
      const m = c.magDb[i]!;
      const s = sp[i] ?? 0;
      if (f < GRAPH_MIN_HZ || f > GRAPH_MAX_HZ || !Number.isFinite(m) || !Number.isFinite(s)) continue;
      top.push(`${x(f).toFixed(1)},${y(m + s / 2).toFixed(1)}`);
      bottom.push(`${x(f).toFixed(1)},${y(m - s / 2).toFixed(1)}`);
    }
    return top.length ? `M${top.join("L")}L${bottom.reverse().join("L")}Z` : "";
  };
  // -- WP WEB7 -- (REVE D4) 5 or 10 dB steps, never a label over the frequencies' row; an empty frame
  // draws no grid and no labels, only "No curve yet"
  const gridDb = series.length ? dbTicks(lo, hi) : [];
  const ticksHz = series.length ? TICKS_HZ : [];

  return (
    <figure class="measure-graph">
      <div class="mg-plot" ref={box}>
      <svg
        ref={svg}
        role="img"
        aria-label={props.label}
        data-series={series.map((s) => `${s.kind}:${s.label}`).join("|")}
        viewBox={`0 0 ${W} ${H}`}
        width={W}
        height={H}
        style={{ touchAction: interactive ? "none" : "auto", cursor: interactive ? "crosshair" : "default" }}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerLeave={() => {
          drag.current = null;
          setBand(null);
        }}
      >
        {ticksHz.map((f) => (
          <line key={f} class="mg-grid" x1={x(f)} x2={x(f)} y1={PAD_T} y2={PAD_T + PLOT_H - 20} />
        ))}
        {gridDb.map((v) => (
          <line key={v} class="mg-grid" x1={PAD_L + 34} x2={PAD_L + PLOT_W} y1={y(v)} y2={y(v)} />
        ))}
        {props.mark ? (
          props.mark.f2 ? (
            <rect x={x(props.mark.f1)} y={PAD_T} width={Math.max(1, x(props.mark.f2) - x(props.mark.f1))} height={PLOT_H} fill="var(--accent)" opacity="0.1" />
          ) : (
            <line x1={x(props.mark.f1)} x2={x(props.mark.f1)} y1={PAD_T} y2={PAD_T + PLOT_H} stroke="var(--accent)" stroke-dasharray="3 3" />
          )
        ) : null}
        {band ? <rect x={PAD_L + band[0]} y={PAD_T} width={band[1] - band[0]} height={PLOT_H} fill="var(--accent)" opacity="0.15" /> : null}
        {series.map((s, k) =>
          s.spreadDb ? <path key={`s${k}`} d={spreadPath(s.curve, s.spreadDb)} fill={colourOf(s)} opacity="0.15" stroke="none" /> : null,
        )}
        {series.map((s, k) =>
          segments(s.curve).map((seg, j) => (
            <path
              key={`${k}-${j}`}
              class={seg.trusted ? "trusted" : "untrusted"}
              data-kind={s.kind}
              d={path(seg.points)}
              fill="none"
              stroke={colourOf(s)}
              stroke-width={s.kind === "measured" ? 2 : 1.6}
              stroke-dasharray={DASH[s.kind]}
              opacity={seg.trusted ? 1 : 0.3}
            />
          )),
        )}
        {/* -- WP WEB4 -- the axis labels inside the plot, over the curves; the unit on the top label */}
        {ticksHz.map((f, i) => (
          <text key={`f${f}`} class="mg-label" x={Math.min(W - INSET, Math.max(INSET, x(f)))} y={H - INSET} text-anchor={i === 0 ? "start" : i === TICKS_HZ.length - 1 ? "end" : "middle"}>
            {f >= 1000 ? `${f / 1000}k` : f}
          </text>
        ))}
        {dbLabels(gridDb, y, INSET + 9, H - 24).map(([v, ly], i, all) => (
          <text key={`d${v}`} class="mg-label" x={INSET} y={ly}>
            {i === all.length - 1 ? `${v} dB` : v}
          </text>
        ))}
        {series.length ? null : (
          <text class="mg-empty" x={W / 2} y={H / 2} text-anchor="middle">
            No curve yet
          </text>
        )}
      </svg>
      </div>
      {series.length ? (
      <figcaption class="mg-legend">
        {series.map((s, k) => (
          <span key={k}>
            <svg width="22" height="8" aria-hidden="true">
              <line x1="0" x2="22" y1="4" y2="4" stroke={colourOf(s)} stroke-width="2" stroke-dasharray={DASH[s.kind]} />
            </svg>{" "}
            {s.label}
          </span>
        ))}
        <span>{props.unit ?? "dB"}; faint: not trusted (too little signal, or the repeats disagree)</span>
        {props.mark ? <span>{props.mark.f2 ? `${hz(props.mark.f1)} to ${hz(props.mark.f2)}` : hz(props.mark.f1)}</span> : null}
      </figcaption>
      ) : null}
    </figure>
  );
}

// -- WP WEB7 -- (REVE D4) the dB grid: 5 dB steps up to a 40 dB span, 10 up to 100, 20 beyond
export function dbTicks(lo: number, hi: number): number[] {
  if (!Number.isFinite(lo) || !Number.isFinite(hi) || hi <= lo) return [];
  const span = hi - lo;
  const step = span <= 40 ? 5 : span <= 100 ? 10 : 20;
  const out: number[] = [];
  for (let v = Math.ceil(lo / step) * step; v <= hi + 1e-9; v += step) out.push(v);
  return out;
}

/** The labels of `ticks` that fit between `top` and `bottom`, at least 12 px apart (from the top label down). */
export function dbLabels(ticks: readonly number[], y: (v: number) => number, top: number, bottom: number): [number, number][] {
  const out: [number, number][] = [];
  for (const v of [...ticks].sort((a, b) => b - a)) {
    const ly = y(v) + 4;
    if (ly < top || ly > bottom) continue;
    if (out.length && ly - out[out.length - 1]![1] < 12) continue;
    out.push([v, ly]);
  }
  return out.reverse();
}
