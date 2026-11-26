// -- WP WEB6 -- Analyse › Spectrum as Qt's Modern page (SpectrumPanel.qml): the chosen channels'
// spectrum as curves, bars or both over the page, with Hz marks under the plot and dB marks at its
// right; the tap and the channels in the head row with Curves / Bars / Both at its right; "On the
// Equaliser's Graph" in the page tools; the analyser's status line at the foot. The picture is the
// core's (analyser.ts reads `analyserView`); this only draws it.

import { useEffect, useRef, useState } from "preact/hooks";
import type { DspState, OutputLabel } from "../../core/protocol";
import type { AnalyserView } from "../../core/tools-protocol";
import { EmptyState, Segments } from "../../ui/design";
import { PageTools } from "../../ui/shell/Parts";
import type { Session } from "../../ui/session";
import { inputColour, outputColour } from "../eq/model";
import { inputRows, outputRows } from "../main/model";
import {
  FREQ_TICKS,
  TAP_INPUT,
  TAP_OUTPUT,
  barsOf,
  bassText,
  channelName,
  channelsOf,
  columnsFor,
  dbTicks,
  followingText,
  hzText,
  levelFrac,
  lowBandText,
  rangeText,
  statusParts,
  type Analyser,
  type SpectrumPrefs,
  type SpectrumView,
} from "./analyser";
import { tapNames, useAnalyser, useAnalyserView } from "./SpectrumStrip";

const MIN_HZ = 20;
const MAX_HZ = 20000;
/** Room under the plot for the Hz marks, px. */
const AXIS = 16;

/** One channel as drawn: its number on the tap, its name and colour. */
export interface Drawn {
  channel: number;
  name: string;
  colour: string;
}

function xOf(hz: number, width: number): number {
  const lo = Math.log10(MIN_HZ);
  return ((Math.log10(Math.max(hz, 1)) - lo) / (Math.log10(MAX_HZ) - lo)) * width;
}

function tone(el: Element, name: string, d: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || d;
}

/** The canvas sized to its box at the screen's density; its context and size in CSS px. */
function fit(c: HTMLCanvasElement): { ctx: CanvasRenderingContext2D; w: number; h: number } | null {
  const r = c.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(r.width));
  const h = Math.max(1, Math.round(r.height));
  if (c.width !== w * dpr || c.height !== h * dpr) {
    c.width = w * dpr;
    c.height = h * dpr;
  }
  const ctx = c.getContext("2d");
  if (!ctx) return null;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  return { ctx, w, h };
}

/** The dB marks' text ("−84", "+12"). */
function dbText(db: number): string {
  return db > 0 ? `+${db}` : String(db).replace("-", "−");
}

/** -- WP FIN015 -- (REVF D3) The room the dB marks take at the plot's right, px (their widest, and a margin). */
function dbGutter(ctx: CanvasRenderingContext2D, el: Element, p: SpectrumPrefs): number {
  ctx.font = `10px ${tone(el, "--mono", "monospace")}`;
  return Math.ceil(Math.max(0, ...dbTicks(p.floorDb, p.ceilingDb).map((db) => ctx.measureText(dbText(db)).width))) + 8;
}

/**
 * The grid and the marks of a plot `w` × `plotH` (the Hz marks under it, the dB marks at its right).
 * -- WP FIN015 -- (REVF D3) the frequencies span `pw` (≤ `w`): the bars stop short of the dB marks.
 */
function grid(ctx: CanvasRenderingContext2D, el: Element, w: number, plotH: number, p: SpectrumPrefs, marks: { hz: boolean; db: boolean }, pw = w) {
  const gridTone = tone(el, "--sp-grid", "#3a3a3a");
  const zeroTone = tone(el, "--sp-zero", "rgba(255,255,255,0.3)");
  const textTone = tone(el, "--sp-text", "#a7a7a7");
  ctx.lineWidth = 1;
  ctx.strokeStyle = gridTone;
  ctx.beginPath();
  for (const hz of FREQ_TICKS) {
    const x = Math.round(xOf(hz, pw)) + 0.5;
    ctx.moveTo(x, 0);
    ctx.lineTo(x, plotH);
  }
  const ticks = dbTicks(p.floorDb, p.ceilingDb);
  for (const db of ticks) {
    if (db === 0) continue;
    const y = Math.round(plotH - levelFrac(db, p.floorDb, p.ceilingDb) * plotH) + 0.5;
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
  }
  ctx.stroke();
  if (ticks.includes(0)) {
    const y = Math.round(plotH - levelFrac(0, p.floorDb, p.ceilingDb) * plotH) + 0.5;
    ctx.strokeStyle = zeroTone;
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(w, y);
    ctx.stroke();
  }
  ctx.fillStyle = textTone;
  ctx.font = `10px ${tone(el, "--mono", "monospace")}`;
  if (marks.hz) {
    ctx.textBaseline = "alphabetic";
    for (const hz of FREQ_TICKS) {
      const t = hzText(hz);
      const tw = ctx.measureText(t).width;
      // Kept whole inside the plot (§12): "20" from the left edge, "20k" to the right one
      const x = Math.min(Math.max(xOf(hz, pw) - tw / 2, 4), pw - tw - 4);
      ctx.fillText(t, x, plotH + AXIS - 4);
    }
  }
  if (marks.db) {
    ctx.textAlign = "right";
    ctx.textBaseline = "middle";
    // A small plot labels every other mark, so the labels never touch
    const every = plotH / Math.max(1, ticks.length) < 24 ? 2 : 1;
    for (const [i, db] of ticks.entries()) {
      if (i % every) continue;
      const y = plotH - levelFrac(db, p.floorDb, p.ceilingDb) * plotH;
      if (y < 6 || y > plotH - 6) continue;
      ctx.fillText(dbText(db), w - 4, y - 6);
    }
    ctx.textAlign = "left";
  }
}

/** Every drawn channel's level as a filled curve, its peak hold as a thin line over it. Answers how many curves were drawn. */
function drawCurves(c: HTMLCanvasElement, v: AnalyserView, drawn: Drawn[], p: SpectrumPrefs): number {
  const f = fit(c);
  if (!f) return 0;
  const { ctx, w, h } = f;
  const plotH = Math.max(1, h - AXIS);
  grid(ctx, c, w, plotH, p, { hz: true, db: true });
  let n = 0;
  for (const d of drawn) {
    const bars = barsOf(v, d.channel);
    if (!bars || bars.length < 2) continue;
    const pts = bars.map((b) => [xOf(b.hz, w), plotH - levelFrac(b.db, p.floorDb, p.ceilingDb) * plotH] as const);
    const fill = ctx.createLinearGradient(0, 0, 0, plotH);
    ctx.globalAlpha = p.graphOpacity;
    ctx.fillStyle = fill;
    fill.addColorStop(0, `${d.colour}4d`);
    fill.addColorStop(1, `${d.colour}0a`);
    ctx.beginPath();
    ctx.moveTo(pts[0]![0], plotH);
    for (const [x, y] of pts) ctx.lineTo(x, y);
    ctx.lineTo(pts[pts.length - 1]![0], plotH);
    ctx.closePath();
    ctx.fill();
    ctx.strokeStyle = d.colour;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = "round";
    ctx.beginPath();
    pts.forEach(([x, y], i) => (i ? ctx.lineTo(x, y) : ctx.moveTo(x, y)));
    ctx.stroke();
    if (p.showPeakHold) {
      ctx.globalAlpha = 0.55 * p.graphOpacity;
      ctx.lineWidth = 1;
      ctx.beginPath();
      let started = false;
      for (const b of bars) {
        if (!Number.isFinite(b.peak) || b.peak <= p.floorDb) continue;
        const x = xOf(b.hz, w);
        const y = plotH - levelFrac(b.peak, p.floorDb, p.ceilingDb) * plotH;
        if (started) ctx.lineTo(x, y);
        else ctx.moveTo(x, y);
        started = true;
      }
      ctx.stroke();
    }
    ctx.globalAlpha = 1;
    n++;
  }
  return n;
}

/**
 * One channel's bands as bars in its colour, the peak holds as ticks over them. Answers how many bars were drawn.
 * -- WP FIN015 -- (REVF D3) the bars are opaque: they stop at the dB marks' gutter, never over them.
 */
export function drawBars(c: HTMLCanvasElement, v: AnalyserView, d: Drawn, p: SpectrumPrefs): number {
  const f = fit(c);
  if (!f) return 0;
  const { ctx, w, h } = f;
  const plotH = Math.max(1, h - AXIS);
  const pw = Math.max(1, w - dbGutter(ctx, c, p));
  grid(ctx, c, w, plotH, p, { hz: true, db: true }, pw);
  const bars = barsOf(v, d.channel);
  if (!bars?.length) return 0;
  const xs = bars.map((b) => xOf(b.hz, pw));
  ctx.globalAlpha = 0.85 * p.graphOpacity;
  ctx.fillStyle = d.colour;
  const edges = (i: number): [number, number] => {
    const left = i === 0 ? xs[0]! - (xs.length > 1 ? (xs[1]! - xs[0]!) / 2 : 2) : (xs[i - 1]! + xs[i]!) / 2;
    const right = i === xs.length - 1 ? xs[i]! + (i > 0 ? (xs[i]! - xs[i - 1]!) / 2 : 2) : (xs[i]! + xs[i + 1]!) / 2;
    return [Math.max(0, left), Math.min(pw, right)];
  };
  for (let i = 0; i < bars.length; i++) {
    const [left, right] = edges(i);
    const top = plotH - levelFrac(bars[i]!.db, p.floorDb, p.ceilingDb) * plotH;
    ctx.fillRect(left + 0.5, top, Math.max(1, right - left - 1), plotH - top);
  }
  if (p.showPeakHold) {
    ctx.globalAlpha = 1;
    for (let i = 0; i < bars.length; i++) {
      const pk = bars[i]!.peak;
      if (!Number.isFinite(pk) || pk <= p.floorDb) continue;
      const [left, right] = edges(i);
      ctx.fillRect(left + 0.5, plotH - levelFrac(pk, p.floorDb, p.ceilingDb) * plotH, Math.max(1, right - left - 1), 1.5);
    }
  }
  ctx.globalAlpha = 1;
  return bars.length;
}

function Curves(props: { v: AnalyserView; drawn: Drawn[]; p: SpectrumPrefs }) {
  const c = useRef<HTMLCanvasElement>(null);
  const [n, setN] = useState(0);
  useEffect(() => {
    if (!c.current) return;
    const got = drawCurves(c.current, props.v, props.drawn, props.p);
    if (got !== n) setN(got);
  });
  // The canvas fills a box the layout sizes, so its own size never feeds back into the page's
  return (
    <div class="sp-box sp-curves-box">
      <canvas ref={c} class="sp-curves" role="img" aria-label={`Spectrum of ${props.drawn.map((d) => d.name).join(", ")}`} data-curves={n} />
    </div>
  );
}

function BarCell(props: { v: AnalyserView; d: Drawn; p: SpectrumPrefs }) {
  const c = useRef<HTMLCanvasElement>(null);
  const [n, setN] = useState(0);
  useEffect(() => {
    if (!c.current) return;
    const got = drawBars(c.current, props.v, props.d, props.p);
    if (got !== n) setN(got);
  });
  return (
    <figure class="sp-cell" data-bars={n}>
      <figcaption>
        <span class="sp-dot" style={{ background: props.d.colour }} aria-hidden="true" />
        {props.d.name}
      </figcaption>
      <div class="sp-box">
        <canvas ref={c} role="img" aria-label={`Bars of ${props.d.name}`} />
      </div>
    </figure>
  );
}

/** Lucide's eye and eye-off, for the page tool (Qt's icon). */
function Eye(props: { off: boolean }) {
  return (
    <svg class="shell-icon" viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      {props.off ? (
        <>
          <path d="M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49" />
          <path d="M14.084 14.158a3 3 0 0 1-4.242-4.242" />
          <path d="M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143" />
          <path d="m2 2 20 20" />
        </>
      ) : (
        <>
          <path d="M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0" />
          <circle cx="12" cy="12" r="3" />
        </>
      )}
    </svg>
  );
}

/** The Spectrum page. */
export function SpectrumPage(props: { session: Session; s: DspState; labels?: OutputLabel[] }) {
  const { session, s } = props;
  const a: Analyser = useAnalyser(session);
  const choice = a.choice;
  useAnalyserView(a, choice.mask ? { tap: choice.tap, mask: choice.mask } : null);
  const v = a.view;
  const p = a.prefs;
  const ro = session.readOnlyReason !== "";
  const names = tapNames(s, props.labels);
  // The channels' own names and colours per tap ("Out 1 · L"), as Qt's legend
  const own = (tap: number) =>
    tap === TAP_OUTPUT
      ? outputRows(s, props.labels).map((r) => ({ channel: r.index, name: r.name, colour: outputColour(s, r.index) }))
      : inputRows(s, s.numInputChannels || 2).map((r) => ({ channel: r.index, name: r.name, colour: inputColour(r.index) }));
  const count = choice.tap === TAP_OUTPUT ? (v?.caps.outputChannels ?? s.numOutputChannels) : (v?.caps.inputChannels ?? (s.numInputChannels || 2));
  // An unused output (no input routed, owner 2026-11-24) is left out of this view, unless it is being analysed
  // -- WP FIN015 -- (REVF R1) or the generator plays on it (`hidden`)
  const pickable = own(choice.tap)
    .slice(0, Math.min(16, count))
    .filter((r) => choice.tap !== TAP_OUTPUT || !props.labels?.[r.channel]?.hidden || (choice.mask & (1 << r.channel)) !== 0);
  // What is drawn: the device's tap and channels while following, this page's otherwise
  const following = v?.ownership.following === true;
  const shownTap = following ? v!.ownership.tap : choice.tap;
  const shownMask = following ? v!.ownership.mask : choice.mask;
  const rows = own(shownTap);
  const drawn: Drawn[] = channelsOf(shownMask)
    .filter((ch) => v?.snapshot.frames.some((f) => f.channel === ch) ?? false)
    .map((ch) => rows.find((r) => r.channel === ch) ?? { channel: ch, name: channelName(shownTap, ch, []), colour: "#2563eb" });
  const supported = v?.caps.supported ?? true;
  const view = choice.view;
  const graphOn = choice.graph;
  const low = v && view !== "curves" ? lowBandText(v, p.fftOrder) : "";

  const tools = (
    <PageTools label="Spectrum tools">
      <button
        type="button"
        class="sp-tool"
        role="checkbox"
        aria-checked={graphOn}
        title={graphOn ? "The Equaliser's graph shows the spectrum. Click to hide it there." : "Show the spectrum on the Equaliser's graph"}
        onClick={() => a.setChoice({ graph: !graphOn })}
      >
        <Eye off={!graphOn} />
        On the Equaliser's Graph
      </button>
    </PageTools>
  );

  if (!supported)
    return (
      <section class="sp-page" aria-label="Spectrum">
        <EmptyState icon="warning" title="Spectrum analyser unavailable" text="The connected firmware does not provide a compatible analyser. Update the firmware to use it." />
      </section>
    );

  return (
    <section class="sp-page" aria-label="Spectrum">
      {tools}
      <div class="sp-head">
        <Segments
          label="Analyse"
          options={[
            { value: TAP_INPUT, text: "Inputs" },
            { value: TAP_OUTPUT, text: "Outputs" },
          ]}
          value={choice.tap}
          choose={(tap) => tap !== choice.tap && a.setChoice({ tap, mask: 0b1 })}
        />
        <span class="sp-rule" aria-hidden="true" />
        {/* Qt's legend: a dot in the channel's colour and its name; here it also chooses what is analysed */}
        <div class="sp-legend" role="group" aria-label="Channels analysed">
          {pickable.map((r) => {
            const on = (choice.mask & (1 << r.channel)) !== 0;
            return (
              <button key={r.channel} type="button" class="sp-entry" aria-pressed={on} title={on ? "Stop analysing this channel" : "Analyse this channel"} onClick={() => a.setChoice({ mask: choice.mask ^ (1 << r.channel) })}>
                <span class="sp-dot" style={{ background: r.colour }} aria-hidden="true" />
                {r.name}
              </button>
            );
          })}
        </div>
        <Segments
          label="View"
          options={[
            { value: "curves" as SpectrumView, text: "Curves", title: "The chosen channels' curves over one plot" },
            { value: "bars" as SpectrumView, text: "Bars", title: "Each channel's bands as bars" },
            { value: "both" as SpectrumView, text: "Both", title: "The curves above, the bars under them" },
          ]}
          value={view}
          choose={(view) => a.setChoice({ view })}
        />
      </div>
      <div class={`sp-plot sp-mode-${view}`} data-version={v?.snapshot.version ?? 0}>
        {!choice.mask && !following ? (
          <EmptyState icon="chart-line" title="No channels selected" text="Choose a channel above to analyse it." />
        ) : v && drawn.length ? (
          <>
            {view !== "bars" ? <Curves v={v} drawn={drawn} p={p} /> : null}
            {view !== "curves" ? (
              <div class="sp-bars" style={{ gridTemplateColumns: `repeat(${columnsFor(drawn.length)}, minmax(0, 1fr))` }}>
                {drawn.map((d) => (
                  <BarCell key={`${shownTap}:${d.channel}`} v={v} d={d} p={p} />
                ))}
              </div>
            ) : null}
          </>
        ) : (
          <EmptyState icon="chart-line" title={a.error ? "The analyser does not answer" : "Starting the analyser…"} text={a.error || undefined} />
        )}
      </div>
      <div class="sp-status" role="status" aria-label="Analyser status">
        <span class={`sp-run${v?.snapshot.running ? " on" : ""}`}>
          <span class="sp-run-dot" aria-hidden="true" />
          {v?.snapshot.running ? "Running" : "Idle"}
        </span>
        <span class="sp-telemetry" title="Bass processing time summed across both cores. Already included in the audio CPU meters; ≥ means the counter is saturated.">
          {v ? [...statusParts(v.snapshot), ...(bassText(v.snapshot) ? [bassText(v.snapshot)] : [])].join("   ") : ""}
        </span>
        {v?.snapshot.configRejected ? (
          <span class="sp-warn">Device refused this configuration</span>
        ) : following ? (
          <span class="sp-following" title={followingText(v!.ownership, names)}>
            Following another console
            {ro ? null : (
              <button type="button" onClick={() => void a.takeControl()}>
                Take Control
              </button>
            )}
          </span>
        ) : low ? (
          <span class="sp-warn">{low}</span>
        ) : v && rangeText(v) ? (
          <span class="sp-range" title="FFT / bass usable dynamic range. Bass bands use overlapping filters calibrated for tones.">
            {rangeText(v)}
          </span>
        ) : null}
      </div>
    </section>
  );
}
