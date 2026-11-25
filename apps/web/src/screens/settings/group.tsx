// -- WP WEB7 -- Qt's SettingsGroup on the web (the owner's default for card titles, 2026-11-24):
// the group's icon and DemiBold title OUTSIDE the card, anything beside the title (a switch, a
// refresh button) at its right, the rows on the panel, an optional footnote under it. `flat`
// drops the panel (DESIGN-LANGUAGE §8: a page whose one group would frame everything). Used by
// the Settings, Control Surfaces, System, Devices, Library and Measure pages.

import type { ComponentChildren } from "preact";
import { Glyph } from "../../ui/design";
import "./group.css";

/** A Lucide icon from its strokes (24 px box), for icons `ui/design`'s Glyph has not. */
export function Lucide(props: { paths: ComponentChildren; class?: string }) {
  return (
    <svg class={`shell-icon${props.class ? ` ${props.class}` : ""}`} viewBox="0 0 24 24" aria-hidden="true" focusable="false">
      {props.paths}
    </svg>
  );
}

export function Group(props: {
  title?: string;
  /** A `Glyph` name, or an icon element (`<Lucide …/>`). */
  icon?: string | ComponentChildren;
  /** Beside the title, at its right (a switch, a refresh button). */
  trailing?: ComponentChildren;
  footer?: ComponentChildren;
  flat?: boolean;
  class?: string;
  id?: string;
  children?: ComponentChildren;
}) {
  const icon = typeof props.icon === "string" ? <Glyph name={props.icon} class="group-icon" /> : props.icon ? <span class="group-icon">{props.icon}</span> : null;
  return (
    <section class={`group${props.flat ? " group-flat" : ""}${props.class ? ` ${props.class}` : ""}`} aria-label={props.title} id={props.id}>
      {props.title ? (
        <div class="group-head">
          {icon}
          <h3 class="group-title">{props.title}</h3>
          {props.trailing ? <div class="group-trailing">{props.trailing}</div> : null}
        </div>
      ) : null}
      {props.children ? <div class="group-panel">{props.children}</div> : null}
      {props.footer ? <p class="group-footer">{props.footer}</p> : null}
    </section>
  );
}
