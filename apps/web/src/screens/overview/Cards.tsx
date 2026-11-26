// -- WP WEB3 -- (DL6 on the web) Qt's Modern Overview's channel cards, on the Overview section's
// Dashboard under its tiles (WEB2's screens/main/Tiles.tsx): INPUTS and OUTPUTS as sections of cards.
// A card is one button (it opens the channel on Equaliser › PEQ): the channel's pill and name, what
// it is, a 48 px mini response (the core's `curves`, +12..−24 dB, a 0 dB guide, in the channel's
// tint), a 3 px meter, and read-only chips for what is set (channel-cards.ts).

import { useMemo } from "preact/hooks";
import type { DeviceSnapshot } from "../../core/protocol";
import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";
import type { ComponentChildren } from "preact";
import { useCurves, useGrid, type CurveRequest } from "../eq/curves";
import { Pill } from "../../ui/Pill";
import { liveInputs } from "../main/model";
import { MINI_ZERO_Y, miniPath, overviewCards, type ChannelCard } from "./channel-cards";
import { meterPct, meterReading } from "./model";
import { useLive } from "./Meters";
import "./overview.css";

/** INPUTS and OUTPUTS: the channel cards, the curves asked for once for all of them. */
export function ChannelCards(props: { session: Session; snap: DeviceSnapshot; usbActive: number | null; open?: ScreenProps["open"] }) {
  const { session, snap } = props;
  const s = snap.state;
  const cards = overviewCards(s, snap.outputLabels, liveInputs(s, props.usbActive));
  const all = [...cards.inputs, ...cards.outputs];
  const key = JSON.stringify(all.map((c) => c.chain));
  const req = useMemo<CurveRequest>(() => ({ chains: all.map((c) => c.chain), bands: [] }), [key]);
  const curves = useCurves(session, key, req);
  const freqs = useGrid(session);
  const live = useLive(session, false);
  const card = (c: ChannelCard, i: number) => (
    <Card key={c.wire} c={c} curve={curves?.chains[i] ?? null} freqs={freqs} level={meterPct(meterReading(live.meters, c.wire).db)} open={props.open} />
  );
  return (
    <>
      {/* Named apart from the Dashboard's level lists ("Inputs", "Outputs" regions) */}
      <CardSection label="Inputs" name="Input channels">
        {cards.idle.length ? <p class="ov-idle">Not in use: {cards.idle.join(", ")}</p> : null}
        {cards.inputs.length ? <div class="ov-cards">{cards.inputs.map((c, i) => card(c, i))}</div> : null}
      </CardSection>
      <CardSection label="Outputs" name="Output channels">
        <div class="ov-cards">{cards.outputs.map((c, i) => card(c, cards.inputs.length + i))}</div>
      </CardSection>
    </>
  );
}

/** §8's section (as ui/design's `Section`) with its own accessible name. */
function CardSection(props: { label: string; name: string; children: ComponentChildren }) {
  return (
    <section class="ds-section ov-cards-section" aria-label={props.name}>
      <div class="ds-section-head">
        <h4 class="section-label">{props.label}</h4>
      </div>
      {props.children}
    </section>
  );
}

function Card(props: { c: ChannelCard; curve: number[] | null; freqs: number[] | null; level: number; open?: ScreenProps["open"] }) {
  const { c } = props;
  const path = props.curve && props.freqs ? miniPath(props.freqs, props.curve, c.offsetDb) : { line: "", fill: "" };
  return (
    <button
      type="button"
      class="ov-card"
      style={{ "--ch": c.colour }}
      aria-label={`${c.descriptor} ${c.name}, ${c.chips.map((x) => x.text).join(", ")}`}
      title={`Opens ${c.name} in the Equaliser`}
      disabled={!props.open}
      onClick={() => props.open?.({ screen: "eq", wire: c.wire })}
    >
      <span class="ov-card-head">
        <Pill text={c.descriptor} colour={c.colour} />
        <span class="ov-card-name">{c.name}</span>
      </span>
      <span class="ov-card-caption">{c.caption}</span>
      <svg class="ov-mini" viewBox="0 0 100 48" preserveAspectRatio="none" aria-hidden="true">
        <line class="ov-mini-zero" x1={0} x2={100} y1={MINI_ZERO_Y} y2={MINI_ZERO_Y} vector-effect="non-scaling-stroke" />
        {path.fill ? <path class="ov-mini-fill" d={path.fill} /> : null}
        {path.line ? <path class="ov-mini-line" d={path.line} vector-effect="non-scaling-stroke" /> : null}
      </svg>
      <span class="ov-card-meter" aria-hidden="true">
        <span style={{ width: `${props.level.toFixed(1)}%` }} />
      </span>
      <span class="ov-chips">
        {c.chips.map((x, i) => (
          <span key={i} class={`ov-chip${x.kind ? ` ${x.kind}` : ""}`}>
            {x.text}
          </span>
        ))}
      </span>
    </button>
  );
}
