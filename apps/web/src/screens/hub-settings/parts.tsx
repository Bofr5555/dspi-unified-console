// WP P2WB2: what the hub settings page's sections share.

import type { ScreenProps } from "../../ui/screens";
import type { Session } from "../../ui/session";

/** What every section gets. */
export interface SectionProps {
  session: Session;
  /** An Admin of the hub, not in read-only mode. */
  edit: boolean;
  ask: ScreenProps["ask"];
  /** The open DSPi's serial ("" before it is known). */
  serial: string;
}

/** A failure line, as the other pages show one. */
export function Problem(props: { text: string }) {
  return props.text ? (
    <p class="error" role="alert">
      {props.text}
    </p>
  ) : null;
}

/** A line saying what an action did. */
export function Done(props: { text: string }) {
  return props.text ? (
    <p class="note hub-done" role="status">
      {props.text}
    </p>
  ) : null;
}
