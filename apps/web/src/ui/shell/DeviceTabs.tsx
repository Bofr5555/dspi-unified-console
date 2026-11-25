// -- WP WEBM1 -- The open devices as tabs (MPLAN 1.2 item 7): the state's dot, where the device is
// and what it is called, how it is reached, a close button; "+" opens another.
// -- WP REVEFIX -- (REVE §3.1, Qt's DeviceTab) two lines: the name, then how it is reached and its
// firmware ("Demo · firmware 1.1.6"); no unsaved mark (the preset button has the one dirty marker).

import { useEffect, useRef, useState } from "preact/hooks";
import { sessionName, useSessionState } from "../components";
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

// -- WP UWN -- (UXP A15) devices you can tell apart: the device's name first (the installation only when the open
// tabs span several), the second line where it is (its location on the hub, else the hub or how it is reached) and
// its firmware, as Qt's "Centre / Demo · firmware 1.1.6"; two tabs of one name take the unit's name on the audio
// link, else their location. The tab's accessible name has a separator between its two lines.
/** What a tab knows to name its device. */
export interface TabFacts {
  /** The device's name (`sessionName`). */
  name: string;
  /** Where it is reached: "Demo", "USB", "Hub", "File" (`transportName`). */
  transport: string;
  hubName: string;
  /** The hub's location for it and its unit's name on the audio link ("" when not known). */
  location: string;
  unit: string;
  firmware: string;
  /** The document's file, when the tab edits one. */
  file: string;
  /** The installation's name, said only when the open tabs span several. */
  installation: string;
}

/** A tab's two lines; `collides`: another open tab has the same name. */
export function tabWords(f: TabFacts, collides: boolean): { title: string; meta: string } {
  const title = collides && f.unit ? f.unit : collides && f.location ? `${f.name} · ${f.location}` : f.name;
  const usedLocation = title.includes(f.location) && !!f.location;
  const where = f.location && !usedLocation ? f.location : f.transport === "Hub" && f.hubName ? f.hubName : f.transport;
  const what = f.file ? f.file : f.firmware ? `firmware ${f.firmware}` : "";
  return { title, meta: [f.installation, where, what].filter(Boolean).join(" · ") };
}

/** The facts of `s` as its state has them now. */
function factsOf(s: Session, st: SessionState, several: boolean): TabFacts {
  const snap = st.snapshot;
  return {
    name: sessionName(st),
    transport: transportName(s.source),
    hubName: st.hubName,
    location: st.place?.location ?? "",
    unit: st.place?.unit ?? "",
    firmware: snap && !st.document ? snap.device.firmware : "",
    file: st.document ? st.document.fileName || st.document.name : "",
    installation: several ? s.installationName : "",
  };
}

// -- WP UWN -- the state in the tab's accessible name (never "PIN": that is the PIN field's own name on the page)
const STATE_SPOKEN: Record<ReturnType<typeof tabState>, string> = { ...STATE_WORDS, pairing: "pairing with the hub" };

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
  const several = new Set(sessions.map((s) => s.installationId)).size > 1; // -- WP UWN --
  // -- WP UWN -- the other tabs' names and places, for their collisions: re-render when any of them changes
  const [, bump] = useState(0);
  useEffect(() => {
    const stops = sessions.map((s) => s.watch(() => bump((n) => n + 1)));
    return () => stops.forEach((f) => f());
  }, [sessions]);
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
              several={several}
              collides={sessions.some((o) => o !== s && sessionName(o.state) === sessionName(s.state) && !!o.state.snapshot)}
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

function DeviceTab(props: { session: Session; several: boolean; collides: boolean; on: boolean; stop: boolean; onChoose: () => void; onClose: () => void; onMenu?: (e: MouseEvent | KeyboardEvent) => void }) {
  const s = props.session;
  const st = useSessionState(s);
  const state = tabState(st);
  const name = sessionName(st); // -- WP WEB2 --
  const unsaved = st.unsaved.length > 0;
  // -- WP UWN -- (UXP A15) the two lines from the tab's facts (tabWords); the serial stays in the tooltip
  const { title, meta } = tabWords(factsOf(s, st, props.several), props.collides);
  const serial = st.snapshot && !st.document ? st.snapshot.device.serial : "";
  // -- WP FEATUI -- an AI agent works on this device (its hub's `presence`): marked, as the desktop's tabs
  const agent = usePresence(s).some((p) => p.serial && p.serial === st.snapshot?.device.serial);
  const where = [meta, serial, STATE_WORDS[state], unsaved ? "unsaved changes" : "", agent ? "an AI agent is working on it" : ""].filter(Boolean).join(" · ");
  // -- WP UWN -- (UXP A15) the name, a separator, the second line; then a state that is not "connected"
  const spoken = [title, meta, state === "open" ? "" : STATE_SPOKEN[state], agent ? "AI agent working" : ""].filter(Boolean).join(", ");
  return (
    <div class="device-tab" role="presentation">
      <button
        type="button"
        role="tab"
        aria-selected={props.on}
        aria-label={spoken}
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
        <span class="device-tab-text">
          <span class="device-tab-name">{title}</span>
          <span class="device-tab-kind device-meta">{meta}</span>
        </span>
        {agent ? (
          <span class="device-tab-agent" aria-label="AI agent working" title="An AI agent is working on this device">
            AI
          </span>
        ) : null}
      </button>
      {/* -- WP WEBM2 -- (RB) reachable by Tab: after the tab on show, before the next control */}
      <button type="button" class="device-tab-close" aria-label={`Close ${name}`} title={`Close ${name}`} tabIndex={props.stop ? 0 : -1} onClick={props.onClose}>
        <span aria-hidden="true">×</span>
      </button>
    </div>
  );
}
