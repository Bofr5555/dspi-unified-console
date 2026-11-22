// -- WP WEBM2 -- The Equaliser's inspector (MODLAY 2): the chosen band (a shape grid in place of
// the shape card, then the band bar's fields) and the chosen channel. For an input: what it feeds
// and its headphone profile; for an output: its crossover's high and low pass, its limiter and its
// Linkwitz Transform bands (each with its draft and Apply). Every control writes as the page's own
// do, so the inspector is a second place for them, never another model.

import type { ComponentChildren } from "preact";
import { useState } from "preact/hooks";
import type { DspState, FilterParams, OutputLabel } from "../../core/protocol";
import { InspectorSection } from "../../ui/inspector/Inspector";
import type { Session } from "../../ui/session";
import { outputLabel } from "../main/model";
import { LimiterDialog } from "../outputs/Limiter";
import { compactGain, levelText, limiterSupported } from "../outputs/model";
import { passes, passName } from "./align";
import { LinkwitzEditor } from "./Linkwitz";
import { SHAPES, T, channelColour, freqText, shapeOf } from "./model";
import { PassFields } from "./Passes";

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

export function EqInspector(props: {
  session: Session;
  s: DspState;
  channel: InspectedChannel;
  labels?: OutputLabel[];
  /** The band part (the band bar and its shape grid), when it is here and not under the graph. */
  band?: ComponentChildren;
  /** The channel's PEQ bands as shown. */
  bands: FilterParams[];
  freq: [number, number];
  disabled: boolean;
  /** Choose another channel of the page. */
  choose: (wire: number) => void;
  /** Open Routing, the Crossover tab or the headphone profiles. */
  openRouting: () => void;
  openCrossover: () => void;
  headphones: () => void;
  /** Write one PEQ band (a Linkwitz Transform's Apply). */
  setBand: (b: number, p: FilterParams) => void;
}) {
  const c = props.channel;
  return (
    <div class="eq-inspector">
      <div class="insp-head">
        <span class="swatch" style={{ background: channelColour(c.wire) }} aria-hidden="true" />
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
      <InspectorSection title="Feeds" summary={feeds.length ? `${feeds.length} output${feeds.length > 1 ? "s" : ""}` : "None"}>
        {feeds.length ? (
          <ul class="insp-feeds">
            {feeds.map((o) => (
              <li key={o}>
                <button type="button" class="link" title="Show this output" onClick={() => props.choose(s.numInputChannels + o)}>
                  <span class="swatch" style={{ background: channelColour(s.numInputChannels + o) }} aria-hidden="true" />
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
          <p class="note">This input feeds no output.</p>
        )}
        <button type="button" onClick={props.openRouting}>
          Edit routing
        </button>
      </InspectorSection>
      <InspectorSection title="Headphone profile" open={false}>
        <p class="note">Load a headphone's correction from the AutoEQ profiles into this input's bands.</p>
        <button type="button" onClick={props.headphones}>
          Headphone profile…
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
  const xoSummary =
    [both.hp ? `HP ${passName(both.hp)} ${freqText(both.hp.freq)}` : "", both.lp ? `LP ${passName(both.lp)} ${freqText(both.lp.freq)}` : ""]
      .filter(Boolean)
      .join(", ") || "Off";
  const lim = limiterSupported(s);
  const limOn = s.limiterEnabled[c.index] ?? false;
  const lts = props.bands.map((p, b) => (p.filterType === T.linkwitz ? b : -1)).filter((b) => b >= 0);
  const names = Array.from({ length: s.numOutputChannels }, (_, o) => outputLabel(s, props.labels, o));
  return (
    <>
      {xo.length ? (
        <InspectorSection title="Crossover" summary={xoSummary}>
          <PassFields session={session} wire={c.wire} bands={xo} freq={props.freq} disabled={disabled} as="rows" />
          <button type="button" class="link" onClick={props.openCrossover}>
            Every output's crossover
          </button>
        </InspectorSection>
      ) : null}
      {lim ? (
        <InspectorSection title="Limiter" summary={limOn ? levelText(s.limiterThresholdDb[c.index] ?? -1, "dBFS") : "Off"}>
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
      <InspectorSection title="Linkwitz transform" summary={lts.length ? `Band ${lts.map((b) => b + 1).join(", ")}` : "None"} open={lts.length > 0}>
        {lts.length ? (
          lts.map((b) => <LinkwitzEditor key={`${c.wire}-${b}`} band={b} p={props.bands[b]!} disabled={disabled} apply={(p) => props.setBand(b, p)} />)
        ) : (
          <p class="note">No band of this output is a Linkwitz Transform. Choose it as a band's type in the band list.</p>
        )}
      </InspectorSection>
    </>
  );
}
