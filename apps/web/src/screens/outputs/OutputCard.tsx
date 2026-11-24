// An output's settings card (WP WB6c, B11), after the Console's
// `ChannelSettingsView`: the routing of the first two inputs into it (each a
// connect toggle, its level and its polarity, `InputRoutingPanel`), GAIN,
// DELAY (owner rule SL1, `Level.tsx`), mute and, with firmware that has it,
// the output limiter. It sits atop the output's page (the EQ screen with an
// output chosen), as on the Console. Every change is the core's, read back.
//
// -- WP WEB3 -- (DL8 on the web) Qt's channel strip: one row, no card: the output's pill and name,
// MIX (the inputs' dots, levels and ø), GAIN (field and slider), DELAY, the mute icon, the limiter.

import { memo } from "preact/compat";
import { Glyph } from "../../ui/design"; // -- WP WEB3 --
import { Pill } from "../../ui/Pill"; // -- WP WEB3 --
import "./outputs.css"; // -- WP WEB3 --
import { useState } from "preact/hooks";
import type { DspState, OutputLabel } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { ValueField } from "../eq/Field";
import { outputLabel, type ChannelRow } from "../main/model";
import { LevelControl } from "./Level";
import { LimiterButtons, LimiterDialog } from "./Limiter";
import { clamp, compactGain, delayText, levelText, limiterSupported, parseLevel } from "./model";

/** The card's level slider: the Console's -60..+10; typing reaches the registry's whole range. */
const GAIN_SLIDER: [number, number] = [-60, 10];
/** A crosspoint's level (`CompactGainField`). */
export const CROSSPOINT_RANGE: [number, number] = [-60, 12];
/** Inputs in the card's routing preview (`BASE_MATRIX_INPUTS`). */
const PREVIEW_INPUTS = 2;

export interface OutputLimits {
  /** `out.gain`'s range (the registry's). */
  gain: [number, number];
  /** 0..the device's longest delay. */
  delay: [number, number];
}

export const OutputCard = memo(function OutputCard(props: {
  session: Session;
  s: DspState;
  /** The output's index (`out.*`) and wire channel. */
  output: number;
  name: string;
  /** -- WP WEB3 -- the strip's head: the output's pill ("OUT1"), its name and its colour */
  tag?: string;
  title?: string;
  colour?: string;
  inputs: ChannelRow[];
  limits: OutputLimits;
  disabled: boolean;
  /** -- BUG1 -- the outputs' labels, the snapshot's */
  labels?: OutputLabel[];
}) {
  const { session, s, output, disabled, limits } = props;
  const [limiter, setLimiter] = useState(false);
  const muted = s.outputMuted[output] ?? false;
  const names = Array.from({ length: s.numOutputChannels }, (_, o) => outputLabel(s, props.labels, o));
  return (
    <section class="output-card ch-strip" aria-label={`Output settings, ${props.name}`}>
      {props.tag ? (
        <span class="strip-who">
          <Pill text={props.tag} colour={props.colour ?? "var(--accent)"} />
          <span class="strip-name">{props.title ?? props.name}</span>
        </span>
      ) : null}
      <span class="strip-label" aria-hidden="true">
        Mix
      </span>
      <div class="output-routes" role="group" aria-label="Routing">
        {props.inputs.slice(0, PREVIEW_INPUTS).map((i) => (
          <Route key={i.wire} session={session} s={s} input={i} output={output} outName={props.name} disabled={disabled} />
        ))}
      </div>
      <LevelControl
        session={session}
        label="Gain"
        name="Gain"
        unit="dB"
        value={s.outputGainDb[output] ?? 0}
        range={limits.gain}
        slider={[Math.max(GAIN_SLIDER[0], limits.gain[0]), Math.min(GAIN_SLIDER[1], limits.gain[1])]}
        step={0.1}
        page={10}
        text={levelText}
        write={(v) => session.write("out.gain", { float: v }, [output])}
        disabled={disabled}
        reset={0}
        class="output-gain"
      />
      <LevelControl
        session={session}
        label="Delay"
        name="Delay"
        unit="ms"
        value={s.outputDelayMs[output] ?? 0}
        range={limits.delay}
        slider={limits.delay}
        step={1}
        page={5}
        text={delayText}
        write={(v) => session.write("out.delay", { float: v }, [output])}
        disabled={disabled}
        reset={0}
        class="output-delay"
      />
      <div class="output-switches">
        <button
          type="button"
          class="eq-icon-toggle output-mute"
          aria-pressed={muted}
          aria-label="Mute"
          title={muted ? "Muted" : "Mute"}
          disabled={disabled}
          onClick={() => void session.write("out.mute", { bool: !muted }, [output])}
        >
          <Glyph name={muted ? "volume-x" : "volume"} />
        </button>
        {limiterSupported(s) ? <LimiterButtons session={session} s={s} output={output} name={props.name} disabled={disabled} open={() => setLimiter(true)} /> : null}
      </div>
      {limiter ? <LimiterDialog session={session} s={s} output={output} names={names} disabled={disabled} onClose={() => setLimiter(false)} /> : null}
    </section>
  );
});

/** One input's crosspoint into this output (`InputRoutingRow`): connect, level (settable before connecting), polarity. */
function Route(props: { session: Session; s: DspState; input: ChannelRow; output: number; outName: string; disabled: boolean }) {
  const { session, s, input, output, disabled } = props;
  const i = input.index;
  const on = s.matrixRouting[i]?.[output] ?? false;
  const gain = s.matrixGain[i]?.[output] ?? 0;
  const inv = s.matrixInvert[i]?.[output] ?? false;
  const set = (p: { enabled?: boolean; gain?: number; invert?: boolean }) =>
    void session.change("crosspoint", { input: i, output, enabled: p.enabled ?? on, gain: p.gain ?? gain, invert: p.invert ?? inv });
  return (
    <div class={on ? "route on" : "route"}>
      <button
        type="button"
        class="route-pick"
        aria-pressed={on}
        aria-label={`Route ${input.name}`}
        title={on ? `Disconnect from ${props.outName}` : `Connect to ${props.outName}`}
        disabled={disabled}
        onClick={() => set({ enabled: !on })}
      >
        <span class={on ? "route-dot on" : "route-dot"} aria-hidden="true" />
        <span class="route-name">{input.name}</span>
      </button>
      <span
        onContextMenu={(e) => {
          e.preventDefault();
          if (!disabled) set({ gain: 0 });
        }}
      >
        <ValueField
          label={`${input.name} level`}
          text={compactGain(gain)}
          parse={(t) => parseLevel(t, "db")}
          disabled={disabled}
          class="route-level"
          onCommit={(v) => set({ gain: clamp(v, CROSSPOINT_RANGE) })}
        />
      </span>
      <button
        type="button"
        class="eq-icon-toggle route-inv"
        aria-pressed={inv}
        aria-label={`${input.name} inverted`}
        title={inv ? "Phase inverted" : "Normal phase"}
        disabled={disabled || !on}
        onClick={() => set({ invert: !inv })}
      >
        ø
      </button>
    </div>
  );
}
