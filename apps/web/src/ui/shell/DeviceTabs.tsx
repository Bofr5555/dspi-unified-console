// -- WP WEBM1 -- The open devices as tabs (MPLAN 1.2 item 7): the state's dot, where the device is
// and what it is called, how it is reached, an unsaved mark, a close button; "+" opens another.

import { useEffect, useRef } from "preact/hooks";
import { TabLabel, useSessionState } from "../components";
import type { Session, SessionState } from "../session";
import { tabKeys } from "./tab-keys";
import { transportName } from "./open-rule";
// -- WP WEBM4 -- a tab's menu (right click, the menu key): actions.json's `device` menu
import { isMenuKey, useContextMenu } from "../menus/ActionMenu";
import type { MenuItem } from "../menus/actions";
import { usePresence } from "../presence"; // -- WP FEATUI --

/** A tab's dot: the session's state, with a link that is away apart from one that is open. */
export function tabState(st: Pick<SessionState, "status" | "link">): SessionState["status"] | "away" {
  return st.status === "open" && st.link && st.link.phase !== "bound" ? "away" : st.status;
}

const STATE_WORDS: Record<ReturnType<typeof tabState>, string> = {
  opening: "connecting",
  pairing: "PIN needed",
  choosing: "choose a DSPi",
  open: "connected",
  away: "reconnecting",
  failed: "not connected",
  closed: "closed",
};

export function DeviceTabs(props: {
  sessions: Session[];
  current: Session | null;
  /** Whether the devices' page is on show: no tab is chosen then, "+" is. */
  picking: boolean;
  onChoose: (s: Session) => void;
  onClose: (s: Session) => void;
  onAdd: () => void;
  /** -- WP WEBM4 -- a tab's menu */
  menu?: (s: Session) => MenuItem[];
}) {
  const { sessions, current } = props;
  const ctx = useContextMenu(); // -- WP WEBM4 --
  const at = current ? sessions.indexOf(current) : -1;
  const row = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const on = row.current?.querySelector<HTMLElement>('[aria-selected="true"]')?.parentElement;
    if (!row.current || !on) return;
    const l = on.offsetLeft;
    if (l < row.current.scrollLeft) row.current.scrollLeft = l;
    else if (l + on.offsetWidth > row.current.scrollLeft + row.current.clientWidth) row.current.scrollLeft = l + on.offsetWidth - row.current.clientWidth;
  }, [current, sessions.length]);
  return (
    <div class="device-tabs">
      {sessions.length ? (
        <div class="device-tab-row" role="tablist" aria-label="Open DSPi" ref={row} onKeyDown={tabKeys(sessions.length, at, (i) => props.onChoose(sessions[i]!))}>
          {sessions.map((s) => (
            <DeviceTab
              key={s.id}
              session={s}
              on={s === current && !props.picking}
              stop={s === (current ?? sessions[0])}
              onChoose={() => props.onChoose(s)}
              onClose={() => props.onClose(s)}
              onMenu={props.menu ? (e) => ctx.open(e, "Device", props.menu!(s)) : undefined}
            />
          ))}
        </div>
      ) : null}
      {ctx.menu /* -- WP WEBM4 -- */}
      <button type="button" class="device-add" aria-label="Open another DSPi" title="Open another DSPi" aria-pressed={props.picking} onClick={props.onAdd}>
        <span aria-hidden="true">+</span>
      </button>
    </div>
  );
}

function DeviceTab(props: { session: Session; on: boolean; stop: boolean; onChoose: () => void; onClose: () => void; onMenu?: (e: MouseEvent | KeyboardEvent) => void }) {
  const s = props.session;
  const st = useSessionState(s);
  const state = tabState(st);
  const name = st.snapshot?.device.name ?? "Connecting";
  const unsaved = st.unsaved.length > 0;
  const transport = transportName(s.source);
  // -- WP FEATUI -- an AI agent works on this device (its hub's `presence`): marked, as the desktop's tabs
  const agent = usePresence(s).some((p) => p.serial && p.serial === st.snapshot?.device.serial);
  const where = [transport, st.hubName, STATE_WORDS[state], unsaved ? "unsaved changes" : "", agent ? "an AI agent is working on it" : ""].filter(Boolean).join(" · ");
  return (
    <div class="device-tab" role="presentation">
      <button
        type="button"
        role="tab"
        aria-selected={props.on}
        tabIndex={props.stop ? 0 : -1}
        title={where}
        data-state={state}
        onClick={props.onChoose}
        // The middle button closes, as a browser's tabs; Delete too
        onAuxClick={(e) => e.button === 1 && (e.preventDefault(), props.onClose())}
        onKeyDown={(e) => (e.key === "Delete" ? (e.preventDefault(), props.onClose()) : isMenuKey(e) && props.onMenu?.(e))}
        onContextMenu={props.onMenu /* -- WP WEBM4 -- */}
      >
        <span class={`dot dot-${state}`} aria-hidden="true" />
        <span class="device-tab-name">
          <TabLabel session={s} />
        </span>
        <span class="device-tab-kind">{transport}</span>
        {agent ? (
          <span class="device-tab-agent" aria-label="AI agent working" title="An AI agent is working on this device">
            AI
          </span>
        ) : null}
        <span class={`device-tab-unsaved${unsaved ? " on" : ""}`} aria-hidden="true">
          •
        </span>
      </button>
      {/* -- WP WEBM2 -- (RB) reachable by Tab: after the tab on show, before the next control */}
      <button type="button" class="device-tab-close" aria-label={`Close ${name}`} title={`Close ${name}`} tabIndex={props.stop ? 0 : -1} onClick={props.onClose}>
        <span aria-hidden="true">×</span>
      </button>
    </div>
  );
}
