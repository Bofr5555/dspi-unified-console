// -- WP WEBM2 -- The Equaliser's inspector (MODLAY 2): the chosen band (a shape grid in place of
// the shape card, then the band bar's fields) and the chosen channel. For an input: what it feeds
// and its headphone profile; for an output: its crossover's high and low pass, its limiter and its
// Linkwitz Transform bands (each with its draft and Apply). Every control writes as the page's own
// do, so the inspector is a second place for them, never another model.
//
// -- WP WEB6 -- (REVE §3.1) As Qt's ChannelInspector and BandInspector: BAND first, with Qt's one
// sentence while no band is chosen (not a disabled form); a chosen band's shapes laid out flat (one
// picker, no second Shape select), its slope where it has two, Frequency, Gain and Q as rows with
// their units after the fields (Q a plain field: no − + on a decimal, DESIGN-LANGUAGE §10) and the
// bandwidth under it, then Band on, Invert and Reset. Then CROSSOVER (its summary "HP 90 Hz · LP
// 400 Hz"), LIMITER and LINKWITZ TRANSFORM closed with "Off" while off, and Edit Routing…; an
// input's FEEDS and HEADPHONE PROFILE.

import type { ComponentChildren } from "preact";
import { useState } from "preact/hooks";
import type { DspState, FilterParams, OutputLabel } from "../../core/protocol";
import { InspectorSection } from "../../ui/inspector/Inspector";
import type { Session } from "../../ui/session";
import { outputLabel } from "../main/model";
import { LimiterDialog } from "../outputs/Limiter";
import { compactGain, levelText, limiterSupported } from "../outputs/model";
import { passes } from "./align";
import { ValueField } from "./Field";
import { LinkwitzEditor } from "./Linkwitz";
import {
  SHAPES,
  T,
  bandwidthOct,
  bareFreq,
  bareGain,
  channelColour,
  clamp,
  orderTitle,
  parseFreq,
  parseNumber,
  qText,
  round2,
  round3,
  roundFreq,
  shapeOf,
  usesGain,
  usesQ,
  type Limits,
} from "./model";
import { PassFields } from "./Passes";
import { Switch } from "../../ui/design";

export interface InspectedChannel {
  wire: number;
  index: number;
  descriptor: string;
  name: string;
  output: boolean;
}

/** The shapes as buttons (the inspector's shape grid): the chosen bands to one, keeping their slope. */
export function ShapeGrid(props: { p: FilterParams | null; available: number[]; disabled: boolean; retype: (t: number) => void }) {
  const s = props.p ? shapeOf(props.p.filterType) : null;
  return (
    <div class="shape-grid" role="radiogroup" aria-label="Shapes">
      {SHAPES.filter((x) => props.available.includes(x.types[2])).map((x) => {
        const types = x.types as Partial<Record<1 | 2, number>>;
        const on = s?.shape.id === x.id;
        return (
          <button
            type="button"
            role="radio"
            key={x.id}
            aria-checked={on}
            disabled={props.disabled || !props.p}
            onClick={() => {
              const t = types[s?.order ?? 2] ?? types[2];
              if (!on && t !== undefined && props.available.includes(t)) props.retype(t);
            }}
          >
            {x.title}
          </button>
        );
      })}
    </div>
  );
}

/** -- WP WEB6 -- Qt's sentence while no band is chosen. */
export const NO_BAND = "No band selected. Click a dot or a row in the table, or double-click the graph to add one.";

/** A frequency in Qt's short words: "90 Hz", "2.4 kHz". */
function hzWords(hz: number): string {
  return hz >= 1000 ? `${Number((hz / 1000).toFixed(2))} kHz` : `${Math.round(hz)} Hz`;
}

/**
 * -- WP WEB6 -- The chosen band (Qt's BandInspector): its shapes, slope, values, Band on, Invert
 * and Reset; the sentence with none. Shape, slope, Band on and Reset act on every chosen band,
 * the values on the one shown, as the band bar does.
 */
export function BandPart(props: {
  band: { b: number; p: FilterParams } | null;
  count: number;
  available: number[];
  bypassOk: boolean;
  limits: Limits;
  disabled: boolean;
  set: (p: FilterParams) => void;
  retype: (t: number) => void;
  bypass: (on: boolean) => void;
  reset: () => void;
  freqRef?: (el: HTMLInputElement | null) => void;
}) {
  const { band, limits, disabled } = props;
  const p = band?.p ?? null;
  const s = p ? shapeOf(p.filterType) : null;
  const types = s?.shape.types as Partial<Record<1 | 2, number>> | undefined;
  const slopes = !!s && types?.[1] !== undefined && types[2] !== undefined && props.available.includes(types[1]) && props.available.includes(types[2]);
  const title = band ? `Band ${band.b + 1}` : "Band";
  return (
    <InspectorSection title={title} summary={props.count > 1 ? `${props.count} chosen` : undefined}>
      {!band || !p ? (
        <p class="note insp-none">{NO_BAND}</p>
      ) : (
        <div class="insp-band">
          <ShapeGrid p={p} available={props.available} disabled={disabled} retype={props.retype} />
          {slopes && s ? (
            <div class="insp-row">
              <span class="insp-label">Slope</span>
              <select
                class="combo"
                aria-label="Slope"
                disabled={disabled}
                value={String(s.order)}
                onChange={(e) => {
                  const t = types?.[Number((e.target as HTMLSelectElement).value) as 1 | 2];
                  if (t !== undefined) props.retype(t);
                }}
              >
                {([2, 1] as const).map((o) => (
                  <option key={o} value={o}>
                    {orderTitle(s.shape, o)}
                  </option>
                ))}
              </select>
            </div>
          ) : null}
          <div class="insp-row">
            <span class="insp-label">Frequency</span>
            <ValueField
              label="Frequency"
              text={bareFreq(p.freq)}
              unit="Hz"
              parse={parseFreq}
              disabled={disabled || p.filterType === T.linkwitz}
              inputRef={props.freqRef}
              onCommit={(v) => props.set({ ...p, freq: roundFreq(clamp(v, limits.freq)) })}
            />
          </div>
          {usesGain(p.filterType) ? (
            <div class="insp-row">
              <span class="insp-label">Gain</span>
              <ValueField
                label="Gain"
                text={bareGain(p.gain)}
                unit="dB"
                parse={(t) => parseNumber(t, "db")}
                disabled={disabled}
                onCommit={(v) => props.set({ ...p, gain: round2(clamp(v, limits.gain)) })}
              />
            </div>
          ) : null}
          {usesQ(p.filterType) && p.filterType !== T.linkwitz ? (
            <div class="insp-row insp-q">
              <span class="insp-label">
                Q<span class="insp-caption">Bandwidth {bandwidthOct(p.q).toFixed(2)} oct</span>
              </span>
              <ValueField
                label="Q"
                text={qText(p.q)}
                parse={(t) => parseNumber(t, "")}
                disabled={disabled}
                onCommit={(v) => props.set({ ...p, q: round3(clamp(v, limits.q)) })}
              />
            </div>
          ) : null}
          <div class="insp-actions">
            {props.bypassOk ? (
              <label class="insp-switch">
                <Switch label="Band on" on={!p.bypass} disabled={disabled} toggle={() => props.bypass(!p.bypass)} />
                <span aria-hidden="true">Band on</span>
              </label>
            ) : null}
            <span class="insp-gap" />
            {usesGain(p.filterType) && p.filterType !== T.linkwitz ? (
              <button type="button" title="Invert Gain" disabled={disabled} onClick={() => props.set({ ...p, gain: round2(-p.gain) })}>
                Invert
              </button>
            ) : null}
            <button type="button" title="Switch this band off and clear its values" disabled={disabled} onClick={props.reset}>
              Reset
            </button>
          </div>
        </div>
      )}
    </InspectorSection>
  );
}

export function EqInspector(props: {
  session: Session;
  s: DspState;
  channel: InspectedChannel;
  labels?: OutputLabel[];
  /** The band part (`BandPart`) on the PEQ page, beside the page; none on Crossover. */
  band?: ComponentChildren;
  /** The channel's PEQ bands as shown. */
  bands: FilterParams[];
  freq: [number, number];
  disabled: boolean;
  /** Choose another channel of the page. */
  choose: (wire: number) => void;
  /** Open Routing or the headphone profiles. */
  openRouting: () => void;
  headphones: () => void;
  /** Write one PEQ band (a Linkwitz Transform's Apply). */
  setBand: (b: number, p: FilterParams) => void;
}) {
  const c = props.channel;
  return (
    <div class="eq-inspector">
      <div class="insp-head">
        <span class="swatch" style={{ background: channelColour(props.s, c.wire) }} aria-hidden="true" />
        <span class="insp-tag">{c.descriptor}</span> <span class="insp-name">{c.name}</span>
      </div>
      {props.band}
      {c.output ? <OutputParts {...props} /> : <InputParts {...props} />}
    </div>
  );
}

function InputParts(props: Parameters<typeof EqInspector>[0]) {
  const { s, channel: c } = props;
  const feeds = Array.from({ length: s.numOutputChannels }, (_, o) => o).filter((o) => s.matrixRouting[c.index]?.[o] && s.outputEnabled[o] !== false);
  return (
    <>
      <InspectorSection title="Feeds" summary={`${feeds.length} output${feeds.length === 1 ? "" : "s"}`}>
        {feeds.length ? (
          <ul class="insp-feeds">
            {feeds.map((o) => (
              <li key={o}>
                <button type="button" class="link" title="Show this output" onClick={() => props.choose(s.numInputChannels + o)}>
                  <span class="swatch" style={{ background: channelColour(s, s.numInputChannels + o) }} aria-hidden="true" />
                  {`OUT${o + 1} ${outputLabel(s, props.labels, o)}`}
                </button>{" "}
                <span class="insp-value">
                  {compactGain(s.matrixGain[c.index]?.[o] ?? 0)}
                  {s.matrixInvert[c.index]?.[o] ? " inv" : ""}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p class="note">This input is connected to no output.</p>
        )}
        <button type="button" onClick={props.openRouting}>
          Edit Routing…
        </button>
      </InspectorSection>
      {/* -- WP UWN -- (UXP A17) AutoEQ, as the docs, the Library tab and the page tool name it */}
      <InspectorSection title="AutoEQ" open={false}>
        <p class="note">Load a headphone's correction from the AutoEQ profiles into this input's bands.</p>
        <button type="button" onClick={props.headphones}>
          AutoEQ…
        </button>
      </InspectorSection>
    </>
  );
}

function OutputParts(props: Parameters<typeof EqInspector>[0]) {
  const { session, s, channel: c, disabled } = props;
  const [limiter, setLimiter] = useState(false);
  const xo = s.xover?.[c.wire] ?? [];
  const both = passes(xo);
  const xoSummary = [both.hp ? `HP ${hzWords(both.hp.freq)}` : "", both.lp ? `LP ${hzWords(both.lp.freq)}` : ""].filter(Boolean).join(" · ") || "Off";
  const lim = limiterSupported(s);
  const limOn = s.limiterEnabled[c.index] ?? false;
  const lts = props.bands.map((p, b) => (p.filterType === T.linkwitz ? b : -1)).filter((b) => b >= 0);
  const names = Array.from({ length: s.numOutputChannels }, (_, o) => outputLabel(s, props.labels, o));
  return (
    <>
      {xo.length ? (
        <InspectorSection title="Crossover" summary={xoSummary}>
          <PassFields session={session} wire={c.wire} bands={xo} freq={props.freq} disabled={disabled} as="rows" />
        </InspectorSection>
      ) : null}
      {lim ? (
        <InspectorSection title="Limiter" summary={limOn ? levelText(s.limiterThresholdDb[c.index] ?? -1, "dBFS") : "Off"} open={false}>
          <label class="insp-row">
            <span class="insp-label">Output limiter</span>
            <input
              type="checkbox"
              role="switch"
              checked={limOn}
              disabled={disabled}
              onChange={(e) => void session.write("limit.on", { bool: (e.target as HTMLInputElement).checked }, [c.index])}
            />
          </label>
          {limOn ? (
            <p class="note">
              Ceiling {levelText(s.limiterThresholdDb[c.index] ?? -1, "dBFS")}, release {Math.round(s.limiterReleaseMs[c.index] ?? 100)} ms
              {s.limiterLinkGroup[c.index] ? `, link group ${s.limiterLinkGroup[c.index]}` : ""}.
            </p>
          ) : null}
          <button type="button" aria-haspopup="dialog" onClick={() => setLimiter(true)}>
            Limiter settings…
          </button>
          {limiter ? <LimiterDialog session={session} s={s} output={c.index} names={names} disabled={disabled} onClose={() => setLimiter(false)} /> : null}
        </InspectorSection>
      ) : null}
      <InspectorSection title="Linkwitz transform" summary={lts.length ? `Band ${lts.map((b) => b + 1).join(", ")}` : "Off"} open={lts.length > 0}>
        {lts.length ? (
          lts.map((b) => <LinkwitzEditor key={`${c.wire}-${b}`} band={b} p={props.bands[b]!} disabled={disabled} apply={(p) => props.setBand(b, p)} />)
        ) : (
          <p class="note">No band of this output is a Linkwitz Transform. Choose it as a band's type in the band list.</p>
        )}
      </InspectorSection>
      {/* -- WP WEB6 -- Qt's Edit Routing… under the sections */}
      <button type="button" class="insp-routing" onClick={props.openRouting}>
        Edit Routing…
      </button>
    </>
  );
}
