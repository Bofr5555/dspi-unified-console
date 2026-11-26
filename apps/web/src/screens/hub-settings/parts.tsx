// WP P2WB2: what the hub settings page's sections share.

import { createContext } from "preact";
import { useContext, useEffect, useRef } from "preact/hooks";
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

// -- WP UB6 -- (UXP B6, W10) One unsaved bar for the page: each section that keeps a draft (alarm rules, notification
// targets, MQTT) says how many changes it holds and how to save or discard them; the page's bar saves or discards them all.

/** A section's draft: its changes, and the ways to save (whether it went) or discard them. */
export interface Draft {
  count: number;
  save: () => Promise<boolean>;
  discard: () => void;
}

/** Where a section puts its draft (null: it has none now). */
export const DraftContext = createContext<((id: string, draft: Draft | null) => void) | null>(null);

/** Put this section's draft on the page's bar while it has changes; the latest `save` and `discard` are the ones run. */
export function useDraft(id: string, count: number, save: () => Promise<boolean>, discard: () => void): void {
  const put = useContext(DraftContext);
  const latest = useRef({ save, discard });
  latest.current = { save, discard };
  useEffect(() => {
    put?.(id, count ? { count, save: () => latest.current.save(), discard: () => latest.current.discard() } : null);
  }, [put, id, count]);
  useEffect(() => () => put?.(id, null), [put, id]);
}
