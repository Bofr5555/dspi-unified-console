// WP SW3, WB6f: the status rail, one per session: the connection, the last
// scene recalled, the mode (with the switch), the system mute (`system.mute`)
// and the alarms, which stay until acknowledged and cleared. On a device the
// console holds itself there is no system mute and no hub alarm.

import { useState } from "preact/hooks";
import { Dialog } from "../main/Dialog";
import type { Session } from "../../ui/session";
import { useSessionState } from "../../ui/components";
import type { SystemFeed } from "./feed";
import { useFeed } from "./SystemScreen";
import { MODES, mayWrite, railAlarms, subjectName, systemMuted, writeNote, type Alarm, type Mode } from "./model";
// -- WP P1WB2 -- the mode switch onto the core's mode manifest and its resolution (P1LK)
import type { ModeChoice } from "../../core/protocol";

export function StatusRail(props: {
  feed: SystemFeed;
  session: Session;
  mode: Mode;
  onMode: (mode: Mode, every: boolean) => void;
  // -- WP P1WB2 --
  /** The core's resolution (`modeResolve`): a pinned or locked mode is not switchable. */
  choice?: ModeChoice | null;
  /** The manifest's modes and their labels. */
  modes?: { id: Mode; label: string }[];
}) {
  const st = useFeed(props.feed);
  const ss = useSessionState(props.session);
  const [choosing, setChoosing] = useState(false);
  const hub = st.kind === "hub" && st.model;
  const muted = hub ? systemMuted(st.model!, st.status) : null;
  const mayMute = mayWrite(st.role, "mute") && !props.feed.readOnly;
  const mayAck = mayWrite(st.role, "ack") && !props.feed.readOnly;
  const alarms = hub ? railAlarms(st.status?.alarms ?? []) : [];
  const on = alarms.filter((a) => a.state !== "cleared");
  const kind = props.session.source.kind;
  // -- WP P1WB1 -- a document opened with no device
  const where = kind === "hub" ? `hub ${ss.hubName || ""}`.trim() : kind === "usb" ? "USB" : kind === "document" ? "a file" : "the demo";
  const modes = props.modes?.length ? props.modes : MODES; // -- WP P1WB2 --
  const mode = modes.find((m) => m.id === props.mode) ?? MODES.find((m) => m.id === props.mode)!;
  // -- WP P1WB2 -- the locked-user state, or a mode the hub (or the fleet) pins
  const locked = props.choice?.mode === "locked_user";
  const pinned = props.choice?.reason === "pinned";
  const shown = locked ? "Locked" : pinned ? `${mode.label} (pinned)` : mode.label;
  return (
    <aside class="sys-rail" aria-label="Status">
      <div class="sys-rail-row">
        <span class="sys-rail-item" aria-label="Connection">
          <span class={`dot dot-${ss.status}`} aria-hidden="true" />
          {ss.status === "open" && (!ss.link || ss.link.phase === "bound") ? "Connected" : "Not connected"} · {where}
        </span>
        {hub ? (
          <span class="sys-rail-item" aria-label="Scene">
            Scene: {st.recalled ? st.recalled.name : "none recalled"}
            {/* -- fixup sw3x -- */}
            {st.scene?.refusal ? <span class="sys-rail-refused"> · {st.scene.name} refused</span> : null}
          </span>
        ) : null}
        <button type="button" class="sys-rail-item sys-mode" aria-label={`Mode: ${shown}`} onClick={() => setChoosing(true)}>
          Mode: {shown}
        </button>
        {hub ? (
          <button
            type="button"
            class={`sys-rail-item sys-sysmute${muted ? " on" : ""}`}
            aria-pressed={muted === true}
            aria-label="System mute"
            disabled={!mayMute}
            title={mayMute ? "Silence every unit on the audio link" : writeNote(st.role, "mute")}
            onClick={() => void props.feed.setMute(!muted)}
          >
            System mute: {muted === null ? "unknown" : muted ? "on" : "off"}
          </button>
        ) : null}
        <span class={`sys-rail-item sys-alarm-sum${on.length ? " raised" : ""}`} aria-label="Alarms">
          {hub ? (on.length ? `${on.length} ${on.length === 1 ? "alarm" : "alarms"}` : "No alarms") : "No hub alarms"}
        </span>
      </div>
      {st.mute?.error ? (
        <p class="error" role="alert">
          {st.mute.error}
        </p>
      ) : null}
      {alarms.length ? (
        <ul class="sys-alarms" aria-label="Alarm list">
          {alarms.map((a) => (
            <AlarmRow key={a.id} a={a} name={subjectName(st.model, a.subject)} mayAck={mayAck} onAck={() => void props.feed.acknowledge(a.id)} />
          ))}
        </ul>
      ) : null}
      {choosing ? (
        <ModeDialog mode={props.mode} modes={modes} choice={props.choice ?? null} onDone={(m, every) => (setChoosing(false), m && props.onMode(m, every))} />
      ) : null}
    </aside>
  );
}

function AlarmRow(props: { a: Alarm; name: string; mayAck: boolean; onAck: () => void }) {
  const { a } = props;
  const when = new Date(a.raisedAt * 1000).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return (
    <li class={`sys-alarm sys-alarm-${a.state} sys-sev-${a.severity}`} aria-label={`Alarm ${a.code} on ${props.name}`} data-state={a.state}>
      <span class="sys-sev" aria-hidden="true" />
      <span class="sys-alarm-text">
        <strong>{props.name}</strong>: {a.message}
        {a.count > 1 ? ` (${a.count}×)` : ""}
        <span class="sys-muted">
          {" "}
          · {when} · {a.state === "raised" ? "raised" : a.state === "acknowledged" ? "acknowledged" : "cleared"}
        </span>
      </span>
      {a.state === "raised" ? (
        <button type="button" disabled={!props.mayAck} onClick={props.onAck}>
          Acknowledge
        </button>
      ) : null}
    </li>
  );
}

/**
 * The mode switch (docs/UI-MODES.md section 5), its modes from the core's manifest. With no PIN
 * every mode is reachable. -- WP P1WB2 -- A mode the hub or the fleet pins, or the locked-user
 * state, is not switchable here: the dialog says why. The Installer mode with a PIN set asks
 * for it (the session's view, once chosen).
 */
function ModeDialog(props: { mode: Mode; modes: { id: Mode; label: string }[]; choice: ModeChoice | null; onDone: (m: Mode | null, every: boolean) => void }) {
  const [mode, setMode] = useState(props.mode);
  const [every, setEvery] = useState(false);
  const fixed = props.choice ? !props.choice.switchable : false;
  const why =
    props.choice?.mode === "locked_user"
      ? "Locked by the installer: unlock to switch modes."
      : props.choice?.reason === "pinned"
        ? `This console is pinned to ${props.modes.find((m) => m.id === props.choice!.mode)?.label ?? props.choice.mode} for this DSPi.`
        : "";
  return (
    <Dialog
      title="Mode"
      onCancel={() => props.onDone(null, false)}
      buttons={[
        { label: "Use", onClick: () => props.onDone(mode, every) },
        { label: "Cancel", onClick: () => props.onDone(null, false) },
      ]}
    >
      {why ? <p class="sys-note mode-fixed">{why}</p> : null}
      <fieldset class="sys-modes" disabled={fixed}>
        <legend>What this console shows</legend>
        {props.modes.map((m) => (
          <label key={m.id}>
            <input type="radio" name="mode" checked={mode === m.id} onChange={() => setMode(m.id)} /> {m.label}
          </label>
        ))}
      </fieldset>
      <label>
        <input type="checkbox" checked={every} onChange={(e) => setEvery((e.target as HTMLInputElement).checked)} /> Use for every device on this console
      </label>
      <p class="sys-note">A mode changes what is shown, never what the hub allows.</p>
    </Dialog>
  );
}
