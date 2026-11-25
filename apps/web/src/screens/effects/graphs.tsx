// -- WP WEB6 -- The effect pages' graphs, as Qt's Modern ones (EffectGraph.qml, SubharmonicGraph.qml,
// TubeTransferGraph.qml in a GraphFrame, DESIGN-LANGUAGE §12): the plot on the panel's tone, no
// outline, its labels inside it, "Disabled" while the effect is off. Every curve is the core's
// (`effectGraph`, dspi_core::effect_curves): the page only draws what it is given. The drawings are
// SVG in the element's own pixels, so a label keeps its size at every width.

import type { ComponentChildren } from "preact";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import type { EffectCurveGraph, EffectGraphParams, PsybassSketch, SubharmGraph, TubeGraph } from "../../core/protocol";
import type { Session } from "../../ui/session";

/** The drawing for `params`, the core's; the last one stays while a newer one is asked for. Null at first. */
export function useEffectGraph<T>(session: Session, params: EffectGraphParams | null): T | null {
  const [graph, setGraph] = useState<{ key: string; value: T } | null>(null);
  const key = params ? JSON.stringify(params) : "";
  const asked = useRef(0);
  useEffect(() => {
    if (!params) return;
    const n = ++asked.current;
    session.core.call("effectGraph", params).then(
      (value) => {
        if (n === asked.current) setGraph({ key, value: value as T });
      },
      () => undefined,
    );
  }, [session, key]);
  return graph?.value ?? null;
}

/** The element's width in pixels, followed. */
function useWidth(fallback: number): [{ current: HTMLDivElement | null }, number] {
  const box = useRef<HTMLDivElement>(null);
  const [w, setW] = useState(fallback);
  useLayoutEffect(() => {
    const el = box.current;
    if (!el) return;
    const fit = () => {
      const r = Math.round(el.getBoundingClientRect().width);
      if (r > 0) setW(r);
    };
    fit();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(fit);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  return [box, w];
}

/** A label's room from the edges. */
const INSET = 6;

/**
 * §12's frame: the plot, `height` px tall, on the panel's tone; `draw` paints it at its pixel size.
 * Off (`active` false) it says "Disabled" and draws nothing. `tools` sit at its top right (a legend,
 * a badge).
 */
export function FxFrame(props: { label: string; height: number; active: boolean; tools?: ComponentChildren; draw: (w: number, h: number) => ComponentChildren; testId?: string }) {
  const [box, w] = useWidth(520);
  const h = props.height;
  return (
    <figure class="fx-graph" role="img" aria-label={props.label} data-active={props.active ? "true" : "false"} data-testid={props.testId}>
      <div class="fx-graph-plot" ref={box} style={{ height: `${h}px` }}>
        {props.active ? (
          <svg width={w} height={h} viewBox={`0 0 ${w} ${h}`} aria-hidden="true">
            {props.draw(w, h)}
          </svg>
        ) : (
          <span class="fx-graph-empty">Disabled</span>
        )}
        {props.active && props.tools ? <div class="fx-graph-tools">{props.tools}</div> : null}
      </div>
    </figure>
  );
}

/** A legend's key: a line, or a bar for a filled band. */
export function Legend(props: { items: { colour: string; label: string; bar?: boolean }[] }) {
  return (
    <span class="fx-legend">
      {props.items.map((i) => (
        <span key={i.label} class="fx-legend-item">
          <span class={`fx-legend-key${i.bar ? " bar" : ""}`} style={{ background: i.colour }} aria-hidden="true" />
          {i.label}
        </span>
      ))}
    </span>
  );
}

/** The effect graphs' frequency marks (Qt's: 100, 1k, 10k), as positions on the 20 Hz..20 kHz axis. */
const FREQ_MARKS: { x: number; text: string }[] = [100, 1000, 10000].map((hz) => ({
  x: (Math.log10(hz) - Math.log10(20)) / (Math.log10(20000) - Math.log10(20)),
  text: hz >= 1000 ? `${hz / 1000}k` : String(hz),
}));

const pathOf = (pts: [number, number][], x: (v: number) => number, y: (v: number) => number) =>
  pts.map((p, i) => `${i ? "L" : "M"}${x(p[0]).toFixed(1)},${y(p[1]).toFixed(1)}`).join("");

function FreqLabels(props: { w: number; h: number; marks: { x: number; text: string }[] }) {
  return (
    <>
      {props.marks.map((m) => (
        <text key={m.text} class="fx-label" x={Math.min(props.w - INSET, Math.max(INSET, m.x * props.w))} y={props.h - INSET} text-anchor="middle">
          {m.text}
        </text>
      ))}
    </>
  );
}

/**
 * Crossfeed's frequency response and loudness's compensation curve (Qt's EffectGraph): the grid at
 * 100 Hz, 1 kHz, 10 kHz and the dB steps, the curves 2 px in `colours[name]`, the dB labels at the
 * left. `bottom` keeps a strip under the plot (none here).
 */
export function CurveGraph(props: {
  label: string;
  height: number;
  active: boolean;
  graph: EffectCurveGraph | null;
  colours: Record<string, string>;
  tools?: ComponentChildren;
}) {
  const g = props.graph;
  return (
    <FxFrame
      label={props.label}
      height={props.height}
      active={props.active}
      tools={props.tools}
      testId="fx-curve-graph"
      draw={(w, h) => {
        if (!g) return null;
        const y = (t: number) => h - t * h;
        return (
          <>
            {FREQ_MARKS.map((m) => (
              <line key={m.text} class="fx-grid" x1={Math.round(m.x * w) + 0.5} x2={Math.round(m.x * w) + 0.5} y1={0} y2={h} />
            ))}
            {g.gridY.map((t, i) => {
              const yy = Math.round(y(t)) + 0.5;
              return yy <= 1 || yy >= h - 1 ? null : <line key={i} class="fx-grid" x1={0} x2={w} y1={yy} y2={yy} />;
            })}
            {g.curves.map((c) => (
              <path key={c.name} class="fx-curve" data-curve={c.name} d={pathOf(c.points, (v) => v * w, y)} stroke={props.colours[c.name] ?? "var(--accent)"} />
            ))}
            {g.gridDb.map((db, i) => (
              <text key={db} class="fx-label" x={INSET} y={Math.min(h - 20, Math.max(INSET + 8, y(g.gridY[i]!) + 4))}>
                {db}
              </text>
            ))}
            <FreqLabels w={w} h={h} marks={FREQ_MARKS} />
          </>
        );
      }}
    />
  );
}

/** Psy Bass's sketch (Qt's PsybassSpectrumView): the original below the cutoff, the harmonics up to 4x, the two marks. */
export function PsybassGraph(props: { height: number; active: boolean; sketch: PsybassSketch | null }) {
  const s = props.sketch;
  return (
    <FxFrame
      label="Spectrum"
      height={props.height}
      active={props.active}
      testId="fx-psybass-graph"
      tools={
        <Legend
          items={[
            { colour: "var(--accent)", label: "Original", bar: true },
            { colour: "var(--warn)", label: "Harmonics", bar: true },
          ]}
        />
      }
      draw={(w, full) => {
        if (!s) return null;
        // A strip under the plot for the frequency labels
        const h = full - 22;
        const fc = s.fc * w;
        const fc4 = s.fc4 * w;
        const marker = (x: number, label: string, strength: number) => (
          <g opacity={strength}>
            <line x1={x + 0.5} x2={x + 0.5} y1={14} y2={h} stroke="var(--text)" stroke-dasharray="3 3" />
            <text class="fx-mark" x={x} y={10} text-anchor="middle">
              {label}
            </text>
          </g>
        );
        return (
          <>
            {FREQ_MARKS.map((m) => (
              <line key={m.text} class="fx-grid" x1={Math.round(m.x * w) + 0.5} x2={Math.round(m.x * w) + 0.5} y1={0} y2={h} />
            ))}
            <rect class="fx-band" data-band="original" x={0} width={fc} y={h - s.original * h} height={s.original * h} fill="var(--accent)" opacity={0.35} />
            <rect class="fx-band" data-band="harmonics" x={fc} width={Math.max(0, fc4 - fc)} y={h - s.harmonics * h} height={s.harmonics * h} fill="var(--warn)" opacity={0.55} />
            {marker(fc, "fc", 0.4)}
            {marker(fc4, "4fc", 0.25)}
            <FreqLabels w={w} h={full} marks={FREQ_MARKS} />
          </>
        );
      }}
    />
  );
}

/** The subharmonic graph's strips: the annotations above the plot, the frequency labels under it (Qt's). */
const ANNOTATION = 38;
const AXIS = 22;

/** One label with the panel's tone behind it, so a line under it does not cut it (Qt's `backed`). */
function Backed(props: { x: number; y: number; text: string; cls: string; fill?: string }) {
  const w = props.text.length * 5.2 + 4;
  return (
    <g>
      <rect x={props.x - w / 2} y={props.y - 6} width={w} height={12} fill="var(--panel)" opacity={0.85} />
      <text class={props.cls} x={props.x} y={props.y + 3} text-anchor="middle" fill={props.fill}>
        {props.text}
      </text>
    </g>
  );
}

/**
 * The subharmonic synthesizer's bands (Qt's SubharmonicGraph, the Mac's SubharmBandView) on a dB
 * scale from 16 to 250 Hz: the program bands the dividers listen to, the subs they make an octave
 * below with the octave hop, the LF boost bell and the ceiling.
 */
export function SubharmBands(props: { height: number; active: boolean; graph: SubharmGraph | null }) {
  const g = props.graph;
  const COLOURS = ["var(--accent)", "var(--warn)", "var(--bad)"];
  return (
    <FxFrame
      label="Subharmonic bands"
      height={props.height}
      active={props.active}
      testId="fx-subharm-graph"
      draw={(w, h) => {
        if (!g) return null;
        const base = h - AXIS;
        const yAt = (t: number) => ANNOTATION + t * (h - ANNOTATION - AXIS);
        return (
          <>
            <g class="fx-grid-soft">
              {g.gridX.map((x, i) => (
                <line key={`x${i}`} x1={x * w} x2={x * w} y1={ANNOTATION} y2={base} />
              ))}
              {g.gridY.map((t, i) => (g.gridDb[i] === 0 ? null : <line key={`y${i}`} x1={0} x2={w} y1={yAt(t)} y2={yAt(t)} />))}
              <line x1={0} x2={w} y1={base} y2={base} />
            </g>
            <line class="fx-grid" x1={0} x2={w} y1={yAt(g.zeroY)} y2={yAt(g.zeroY)} />
            {/* The program bands, behind the subs, with a bracket and their range above */}
            {g.blocks.map((b, i) => {
              const x0 = b.srcX0 * w;
              const x1 = b.srcX1 * w;
              return (
                <g key={`p${i}`} data-band={b.source} data-off={b.off ? "true" : "false"}>
                  <rect x={x0} y={ANNOTATION} width={x1 - x0} height={base - ANNOTATION} fill="var(--text)" opacity={b.off ? 0.04 : 0.1} />
                  <path d={`M${x0 + 1},9L${x0 + 1},5L${x1 - 1},5L${x1 - 1},9`} fill="none" stroke="var(--muted)" opacity={b.off ? 0.25 : 0.5} />
                  <Backed x={(x0 + x1) / 2} y={5} text={b.source} cls={`fx-note${b.off ? " off" : ""}`} />
                </g>
              );
            })}
            {/* The LF boost over the dry path, only while it does something */}
            {g.bell.length ? (
              <g data-testid="fx-subharm-bell">
                <path d={pathOf(g.bell, (v) => v * w, yAt)} fill="none" stroke="var(--ok)" stroke-width={1.4} opacity={0.8} />
                <Backed x={g.bellX * w} y={Math.max(ANNOTATION + 5, yAt(g.bellLabelY) - 8)} text="LF boost" cls="fx-note" fill="var(--ok)" />
              </g>
            ) : null}
            {/* The subs, their tops following the bell, and the octave hop to each */}
            {g.blocks.map((b, i) => {
              if (b.off) return null;
              const colour = COLOURS[i]!;
              const x0 = b.x0 * w;
              const x1 = b.x1 * w;
              const top = b.top.map((p) => `L${(p[0] * w).toFixed(1)},${yAt(p[1]).toFixed(1)}`).join("");
              const mid = yAt(b.midY);
              const inside = base - mid > 18;
              const ax0 = b.fromX * w;
              const ax1 = b.toX * w;
              const ay = b.arrowRow;
              return (
                <g key={`s${i}`} data-sub={b.label}>
                  <path d={`M${x0},${base}${top}L${x1},${base}Z`} fill={colour} fill-opacity={0.4} stroke={colour} stroke-opacity={0.9} />
                  <text class="fx-note strong" x={(x0 + x1) / 2} y={(inside ? mid + 8 : mid - 6) + 3} text-anchor="middle" fill={inside ? "var(--on-accent)" : "var(--text)"}>
                    {b.label}
                  </text>
                  <line x1={ax0} x2={ax1 + 3} y1={ay} y2={ay} stroke="var(--text)" opacity={0.3} stroke-dasharray="2 2" />
                  <path d={`M${ax1 + 4},${ay - 3}L${ax1},${ay}L${ax1 + 4},${ay + 3}`} fill="none" stroke="var(--text)" opacity={0.45} />
                  <Backed x={(ax0 + ax1) / 2} y={ay} text="÷2" cls="fx-note" />
                </g>
              );
            })}
            {/* Where the ceiling starts holding the sub down, across the sub range only */}
            {g.ceilingOn ? (
              <g data-testid="fx-subharm-ceiling">
                <line x1={0} x2={g.ceilingX * w} y1={yAt(g.ceilingY)} y2={yAt(g.ceilingY)} stroke="var(--bad)" opacity={0.65} stroke-dasharray="3 2" />
                <Backed x={g.ceilingX * w + 18} y={yAt(g.ceilingY)} text="ceiling" cls="fx-note" fill="var(--bad)" />
              </g>
            ) : null}
            {g.allOff ? (
              <text class="fx-label strong" x={w / 2} y={(ANNOTATION + base) / 2 + 4} text-anchor="middle">
                All bands off
              </text>
            ) : null}
            {g.gridDb.map((db, i) => (
              <text key={`d${db}`} class="fx-label" x={INSET} y={yAt(g.gridY[i]!) + 4}>
                {db > 0 ? `+${db}` : String(db)}
              </text>
            ))}
            {g.freqs.map((f, i) => (
              <text key={`f${f}`} class="fx-label" x={Math.min(w - INSET, Math.max(INSET, g.gridX[i]! * w))} y={h - INSET} text-anchor="middle">
                {f}
              </text>
            ))}
          </>
        );
      }}
    />
  );
}

/**
 * The tube's transfer curve (Qt's TubeTransferGraph, the Mac's TubeTransferView): output against
 * input over one full-scale swing, the straight line a clean stage draws, full scale dashed, the
 * inputs the stage clips shaded. The static curve only.
 */
export function TubeTransfer(props: { height: number; active: boolean; graph: TubeGraph | null }) {
  const g = props.graph;
  return (
    <FxFrame
      label="Transfer curve"
      height={props.height}
      active={props.active}
      testId="fx-tube-graph"
      draw={(w, h) => {
        if (!g) return null;
        return (
          <>
            <g class="fx-grid-soft">
              {[0, 0.25, 0.75, 1].map((x) => (
                <line key={x} x1={x * w} x2={x * w} y1={0} y2={h} />
              ))}
              {g.half.map((y, i) => (
                <line key={`h${i}`} x1={0} x2={w} y1={y * h} y2={y * h} />
              ))}
            </g>
            <g stroke="var(--bad)" opacity={0.5} stroke-dasharray="3 2">
              <line x1={0} x2={w} y1={g.fullScaleTop * h} y2={g.fullScaleTop * h} />
              <line x1={0} x2={w} y1={g.fullScaleBottom * h} y2={g.fullScaleBottom * h} />
            </g>
            <line class="fx-grid" x1={0.5 * w} x2={0.5 * w} y1={0} y2={h} />
            <line class="fx-grid" x1={0} x2={w} y1={g.zeroY * h} y2={g.zeroY * h} />
            {g.clipPos !== null ? <rect data-clip="pos" x={g.clipPos * w} y={0} width={w - g.clipPos * w} height={h} fill="var(--warn)" opacity={0.12} /> : null}
            {g.clipNeg !== null ? <rect data-clip="neg" x={0} y={0} width={g.clipNeg * w} height={h} fill="var(--warn)" opacity={0.12} /> : null}
            <line x1={0} x2={w} y1={g.lineFrom * h} y2={g.lineTo * h} stroke="var(--text)" opacity={0.3} stroke-dasharray="3 3" />
            <path class="fx-curve" d={pathOf(g.curve, (v) => v * w, (v) => v * h)} stroke="var(--accent)" stroke-width={1.6} />
            <text class="fx-label" x={w - INSET} y={g.zeroY * h + 13} text-anchor="end">
              in
            </text>
            <text class="fx-label" x={w / 2 + 6} y={INSET + 8}>
              out
            </text>
            <text class="fx-label" x={INSET} y={Math.max(INSET + 8, g.fullScaleTop * h - 5)} fill="var(--bad)">
              0 dBFS
            </text>
          </>
        );
      }}
    />
  );
}
