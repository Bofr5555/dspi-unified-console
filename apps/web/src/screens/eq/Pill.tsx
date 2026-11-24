// -- WP WEB3: move to ui/ -- §13's channel pill (Qt's `Pill`): the channel's tint with dark text, 18 px,
// pill radius ("OUT1", "IN2"); `ring` is the checkable form while it is not chosen (a ring in the
// tint). The tint is Qt's ChannelPalette (`channelColour`).

import "./pill.css";

export function Pill(props: { text: string; colour: string; ring?: boolean; title?: string; class?: string }) {
  return (
    <span class={`ch-pill${props.ring ? " ring" : ""}${props.class ? ` ${props.class}` : ""}`} style={{ "--ch": props.colour }} title={props.title}>
      {props.text}
    </span>
  );
}
