// -- WP WEBM1 -- The dock along the bottom, on every page of a device: the source, the User or
// Master volume with Mute and Bypass (the Main window's card, which lived on its page until now)
// and a meter per channel. -- WP REVEFIX -- (REVE §3.1, Qt's Dock) User | Master segments, icons,
// "Bypass EQ", the source a combo in the control tone, the two cores' load at the right end.

import type { ComponentChildren } from "preact";
import { VolumeCard } from "../../screens/main/Volume";
import { useSourceNames } from "../../screens/inputs/InputsScreen";
import { sourceOptions, sourceTitle } from "../../screens/inputs/model";
import type { DspState } from "../../core/protocol";
import type { Session, SessionState } from "../session";

export function Dock(props: {
  session: Session;
  state: SessionState;
  /** The locked-user state: volume, mute and presets only. */
  locked: boolean;
  meters: ComponentChildren;
  /** -- WP REVEFIX -- the two cores' load, at the right end */
  load?: ComponentChildren;
}) {
  const { session, state } = props;
  const s = state.snapshot?.state;
  if (!s) return null;
  const off = session.readOnlyReason !== "";
  return (
    <footer class="dock" aria-label="Dock">
      <SourcePicker session={session} s={s} disabled={off || props.locked} />
      <VolumeCard session={session} userDb={s.userVolumeDb} masterDb={s.masterVolumeDb} mute={s.userMute} bypass={s.bypass} disabled={off} />
      <div class="dock-meters">{props.meters}</div>
      {props.load}
    </footer>
  );
}

/** The sources the device offers now, and the one it plays if it is none of them (an audio link's). */
export function dockSources(s: DspState): number[] {
  const all = sourceOptions(s);
  return all.includes(s.inputSource) ? all : [...all, s.inputSource];
}

function SourcePicker(props: { session: Session; s: DspState; disabled: boolean }) {
  const { session, s } = props;
  const names = useSourceNames(session).names ?? [];
  const all = dockSources(s);
  return (
    <select
      class="dock-source"
      aria-label="Source"
      value={String(s.inputSource)}
      disabled={props.disabled || all.length < 2}
      onChange={(e) => {
        const el = e.target as HTMLSelectElement;
        const to = Number(el.value);
        // The picker shows what the device plays; the write moves it.
        el.value = String(s.inputSource);
        void session.write("in.source", { choice: to });
      }}
    >
      {all.map((x) => (
        <option key={x} value={x}>
          {sourceTitle(s, x, names)}
        </option>
      ))}
    </select>
  );
}
