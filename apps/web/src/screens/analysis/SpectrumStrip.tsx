// WP WB6j: the analyser's bar strip (M22, the Console's SpectrumStrip) and its overlay on the
// EQ graph (M23, RtaGraphOverlay). Both are views of the session's one analyser (analyser.ts):
// on screen they say what they want, and draw the core's picture.

import { useEffect, useId, useRef, useState } from "preact/hooks";
import type { DspState, OutputLabel } from "../../core/protocol";
import type { AnalyserView } from "../../core/tools-protocol";
import { inputRows, outputRows } from "../main/model";
import type { Session } from "../../ui/session";
import { analyserFor, barsOf, channelName, channelsOf, columnsFor, followingText, levelFrac, TAP_INPUT, TAP_OUTPUT, type Analyser, type ViewWant } from "./analyser";

/** The page's channel names per tap, by channel. */
export interface TapNames {
  inputs: string[];
  outputs: string[];
  /** -- WP INTE -- the outputs nothing is routed to (owner 2026-11-24), by channel */
  unused?: boolean[];
}

/** Re-render on every change of the session's analyser. */
export function useAnalyser(session: Session): Analyser {
  const a = analyserFor(session);
  const [, bump] = useState(0);
  useEffect(() => a.watch(() => bump((n) => n + 1)), [a]);
  return a;
}

/** Register a view of `want` (null: none) while mounted. */
export function useAnalyserView(a: Analyser, want: ViewWant | null): void {
  const id = useId();
  const key = want ? `${want.tap}:${want.mask}` : "";
  useEffect(() => {
    a.setView(id, want);
    return () => a.setView(id, null);
  }, [a, id, key]);
}

const MIN_HZ = 20;
const MAX_HZ = 20000;

function xOf(hz: number, width: number): number {
  const lo = Math.log10(MIN_HZ);
  return ((Math.log10(Math.max(hz, 1)) - lo) / (Math.log10(MAX_HZ) - lo)) * width;
}

function cssVar(el: Element, name: string, d: string): string {
  return getComputedStyle(el).getPropertyValue(name).trim() || d;
}

/** Draw one channel's bands as bars on `c`, sized to its box. Answers how many were drawn. */
function drawBars(c: HTMLCanvasElement, v: AnalyserView, channel: number, a: Analyser, overlay: boolean): number {
  const r = c.getBoundingClientRect();
  const dpr = window.devicePixelRatio || 1;
  const w = Math.max(1, Math.round(r.width));
  const h = Math.max(1, Math.round(r.height));
  if (c.width !== w * dpr || c.height !== h * dpr) {
    c.width = w * dpr;
    c.height = h * dpr;
  }
  const ctx = c.getContext("2d");
  if (!ctx) return 0;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  const bars = barsOf(v, channel);
  if (!bars?.length) return 0;
  const p = a.prefs;
  const colour = cssVar(c, "--rta-bar", "#4cc27f");
  const peakColour = cssVar(c, "--rta-peak", "#ffcc4d");
  ctx.globalAlpha = overlay ? 0.45 * p.graphOpacity : p.graphOpacity;
  ctx.fillStyle = colour;
  const xs = bars.map((b) => xOf(b.hz, w));
  for (let i = 0; i < bars.length; i++) {
    const left = i === 0 ? xs[0]! - (xs[1] !== undefined ? (xs[1] - xs[0]!) / 2 : 2) : (xs[i - 1]! + xs[i]!) / 2;
    const right = i === bars.length - 1 ? xs[i]! + (i > 0 ? (xs[i]! - xs[i - 1]!) / 2 : 2) : (xs[i]! + xs[i + 1]!) / 2;
    const top = h - levelFrac(bars[i]!.db, p.floorDb, p.ceilingDb) * h;
    ctx.fillRect(left + 0.5, top, Math.max(1, right - left - 1), h - top);
  }
  if (p.showPeakHold) {
    ctx.globalAlpha = overlay ? 0.7 : 1;
    ctx.fillStyle = peakColour;
    for (let i = 0; i < bars.length; i++) {
      const pk = bars[i]!.peak;
      if (!Number.isFinite(pk) || pk <= p.floorDb) continue;
      const y = h - levelFrac(pk, p.floorDb, p.ceilingDb) * h;
      const half = i > 0 ? (xs[i]! - xs[i - 1]!) / 2 : 2;
      ctx.fillRect(xs[i]! - half + 0.5, y, Math.max(1, half * 2 - 1), 1.5);
    }
  }
  ctx.globalAlpha = 1;
  return bars.length;
}

function Cell(props: { a: Analyser; view: AnalyserView; channel: number; name: string }) {
  const c = useRef<HTMLCanvasElement>(null);
  const [drawn, setDrawn] = useState(0);
  useEffect(() => {
    if (!c.current) return;
    const n = drawBars(c.current, props.view, props.channel, props.a, false);
    if (n !== drawn) setDrawn(n);
  });
  const silent = props.view.snapshot.frames.find((f) => f.channel === props.channel)?.silent ?? true;
  return (
    <figure class="rta-cell" data-bars={drawn}>
      <figcaption>
        {props.name}
        {silent ? <span class="rta-silent"> · silent</span> : null}
      </figcaption>
      <canvas ref={c} role="img" aria-label={`Spectrum of ${props.name}`} />
    </figure>
  );
}

/** The line about who runs the analyser, and the way back to this console's choice. */
function Following(props: { a: Analyser; names: TapNames; readOnly: boolean }) {
  const o = props.a.view?.ownership;
  if (!o?.following) return null;
  return (
    <div class="rta-following" role="status">
      <span>{followingText(o, props.names)}</span>
      {props.readOnly ? null : (
        <button type="button" onClick={() => void props.a.takeControl()}>
          Take Control
        </button>
      )}
    </div>
  );
}

/**
 * The bar strip. On the main screen (`want` absent) it shows the strip's own choice of tap and
 * channels, with the chooser; on a channel page, `want` (that channel).
 */
export function SpectrumStrip(props: { session: Session; names: TapNames; want?: ViewWant; title?: string; always?: boolean }) {
  const a = useAnalyser(props.session);
  // -- WP WEB2 -- (REVD W4) `always`: the bars shown, no Show/Hide (the Analyser page is the bars)
  const on = props.always === true || a.choice.bars;
  const own: ViewWant = props.want ?? { tap: a.choice.tap, mask: a.choice.mask };
  useAnalyserView(a, on && own.mask ? own : null);
  const ro = props.session.readOnlyReason !== "";
  const v = a.view;
  const supported = v?.caps.supported ?? true;
  // What is drawn: the device's tap and channels while following, this view's otherwise.
  const shownTap = v?.ownership.following ? v.ownership.tap : own.tap;
  const shownMask = v?.ownership.following ? v.ownership.mask : own.mask;
  const channels = channelsOf(shownMask).filter((ch) => v?.snapshot.frames.some((f) => f.channel === ch) ?? false);
  const list = shownTap === TAP_OUTPUT ? props.names.outputs : props.names.inputs;
  const count = props.want ? 1 : shownTap === TAP_OUTPUT ? (v?.caps.outputChannels ?? props.names.outputs.length) : (v?.caps.inputChannels ?? props.names.inputs.length);
  return (
    // -- WP WEB4 -- on the Analyser page (`always`) the bars are the page: no card around them
    <section class={props.always ? "rta-strip rta-page" : "card rta-strip"} aria-label="Spectrum">
      {props.always ? null : (
        <div class="card-head">
          <h3>{props.title ?? "Spectrum"}</h3>
          <button type="button" class="toggle" aria-pressed={on} onClick={() => a.setChoice({ bars: !on })}>
            {on ? "Hide Bars" : "Show Bars"}
          </button>
        </div>
      )}
      {on && !props.want ? (
        <div class="rta-chooser">
          <div class="seg" role="group" aria-label="Analyse">
            <button type="button" aria-pressed={a.choice.tap === TAP_INPUT} onClick={() => a.setChoice({ tap: TAP_INPUT, mask: 0b11 })}>
              Inputs
            </button>
            <button type="button" aria-pressed={a.choice.tap === TAP_OUTPUT} onClick={() => a.setChoice({ tap: TAP_OUTPUT, mask: 0b11 })}>
              Outputs
            </button>
          </div>
          <div class="rta-channels" role="group" aria-label="Channels analysed">
            {Array.from({ length: Math.min(16, count) }, (_, ch) => {
              const name = channelName(a.choice.tap, ch, a.choice.tap === TAP_OUTPUT ? props.names.outputs : props.names.inputs);
              const pressed = (a.choice.mask & (1 << ch)) !== 0;
              // -- WP INTE -- an unused output is left out of this view, unless it is being analysed (as SpectrumPage)
              if (a.choice.tap === TAP_OUTPUT && props.names.unused?.[ch] === true && !pressed) return null;
              return (
                <button key={ch} type="button" aria-pressed={pressed} onClick={() => a.setChoice({ mask: a.choice.mask ^ (1 << ch) })}>
                  {name}
                </button>
              );
            })}
          </div>
        </div>
      ) : null}
      {on ? (
        <>
          <Following a={a} names={props.names} readOnly={ro} />
          {!supported ? <p class="empty">This DSPi has no spectrum analyser.</p> : null}
          {supported && !own.mask && !v?.ownership.following ? <p class="empty">Choose a channel to analyse.</p> : null}
          {v && channels.length ? (
            <div class="rta-grid" style={{ gridTemplateColumns: `repeat(${columnsFor(channels.length)}, minmax(0, 1fr))` }} data-version={v.snapshot.version}>
              {channels.map((ch) => (
                <Cell key={`${shownTap}:${ch}`} a={a} view={v} channel={ch} name={channelName(shownTap, ch, list)} />
              ))}
            </div>
          ) : null}
          {a.error ? <p class="error">{a.error}</p> : null}
        </>
      ) : null}
    </section>
  );
}

/** The toggle of the overlay, for the EQ screen's tools. */
export function OverlayToggle(props: { session: Session }) {
  const a = useAnalyser(props.session);
  return (
    <button type="button" aria-pressed={a.choice.graph} onClick={() => a.setChoice({ graph: !a.choice.graph })}>
      Spectrum on Graph
    </button>
  );
}

/**
 * The spectrum over the EQ graph (M23): `want`'s channel drawn behind the curves' colours,
 * pointer events passing through to the graph. Placed in a box with the graph.
 */
export function RtaOverlay(props: { session: Session; want: ViewWant; names: TapNames }) {
  const a = useAnalyser(props.session);
  const on = a.choice.graph;
  useAnalyserView(a, on ? props.want : null);
  const c = useRef<HTMLCanvasElement>(null);
  const v = a.view;
  const channel = v?.ownership.following ? (channelsOf(v.ownership.mask)[0] ?? -1) : (channelsOf(props.want.mask)[0] ?? -1);
  const [drawn, setDrawn] = useState(0);
  useEffect(() => {
    if (!on || !c.current || !v) return;
    const n = drawBars(c.current, v, channel, a, true);
    if (n !== drawn) setDrawn(n);
  });
  if (!on) return null;
  return (
    <>
      <canvas ref={c} class="rta-overlay" aria-hidden="true" data-bars={drawn} data-version={v?.snapshot.version ?? 0} />
      <div class="rta-overlay-note">
        <Following a={a} names={props.names} readOnly={props.session.readOnlyReason !== ""} />
      </div>
    </>
  );
}

/** The page's names per tap: "IN1 Left", "OUT3 Sub", by channel. */
export function tapNames(s: DspState, labels?: OutputLabel[]): TapNames {
  const inputs: string[] = [];
  for (const r of inputRows(s, s.numInputChannels || 2)) inputs[r.index] = `${r.descriptor} ${r.name}`;
  const outputs: string[] = [];
  for (const r of outputRows(s, labels)) outputs[r.index] = `${r.descriptor} ${r.name}`;
  const unused = (labels ?? []).map((l) => l.unused === true); // -- WP INTE --
  return { inputs, outputs, unused };
}
