// WP P5WMS: the measurement graph (PLAN-P5 T1, T3, T9): curves on a log-frequency axis, the
// measured solid, the target dashed, the predicted dotted, the RTA's held copy and compare;
// untrusted points faint (never hidden), a spread band behind a curve. A click picks a frequency
// and a drag a span (the live view's "cut this peak" and "fit this range"); the same values have
// fields beside the graph, so nothing needs a pointer. P5WAT's tune draws on it too.

import { useRef, useState } from "preact/hooks";
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

const W = 720;
const H = 300;
const PAD_L = 40;
const PAD_B = 22;
const PAD_T = 8;
const PAD_R = 8;
const PLOT_W = W - PAD_L - PAD_R;
const PLOT_H = H - PAD_T - PAD_B;
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
  const gridDb: number[] = [];
  for (let v = lo; v <= hi; v += (hi - lo) / 6) gridDb.push(Math.round(v));

  return (
    <figure class="measure-graph" style={{ margin: 0 }}>
      <svg
        ref={svg}
        role="img"
        aria-label={props.label}
        data-series={series.map((s) => `${s.kind}:${s.label}`).join("|")}
        viewBox={`0 0 ${W} ${H}`}
        style={{ width: "100%", height: "auto", display: "block", background: "var(--panel)", border: "1px solid var(--line)", borderRadius: "6px", touchAction: interactive ? "none" : "auto", cursor: interactive ? "crosshair" : "default" }}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerLeave={() => {
          drag.current = null;
          setBand(null);
        }}
      >
        {TICKS_HZ.map((f) => (
          <g key={f}>
            <line x1={x(f)} x2={x(f)} y1={PAD_T} y2={PAD_T + PLOT_H} stroke="var(--line)" stroke-width="1" />
            <text x={x(f)} y={H - 6} font-size="11" text-anchor="middle" fill="var(--muted)">
              {f >= 1000 ? `${f / 1000}k` : f}
            </text>
          </g>
        ))}
        {gridDb.map((v) => (
          <g key={v}>
            <line x1={PAD_L} x2={PAD_L + PLOT_W} y1={y(v)} y2={y(v)} stroke="var(--line)" stroke-width="1" />
            <text x={PAD_L - 4} y={y(v) + 4} font-size="11" text-anchor="end" fill="var(--muted)">
              {v}
            </text>
          </g>
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
      </svg>
      <figcaption class="muted" style={{ display: "flex", flexWrap: "wrap", gap: "4px 14px", fontSize: "12px", marginTop: "4px" }}>
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
    </figure>
  );
}
