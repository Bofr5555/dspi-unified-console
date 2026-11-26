// -- WP UB34 -- (UXP B3, owner D7) The top bar's A | B, as Qt's TopBar.ab: from the first edit, A is the
// preset as saved on the device and B is now (the core's `abStartSaved`); a click on A, or the B key, starts
// that compare, and B goes back. While any compare runs it shows which side is on; a compare of two
// snapshots (Library › Snapshots) names them. Hidden with nothing unsaved and nothing compared, in a
// read-only session and for a locked user.

import { useDesk } from "../../screens/snapshots/Bars";
import { mayStartSaved, mayToggle, type DeskState, type SnapshotDesk } from "../../screens/snapshots/model";
import { abKeyHint } from "../keys";

/** What A and B are now, as the segment's tooltip says it. `key` is the B key as this platform writes it. */
export function abTitle(st: DeskState, unsaved: number, key: string): string {
  if (!st.ab.active) {
    return unsaved > 0
      ? `A/B compare: A is the preset as saved on the device, B is now, with your unsaved changes. Click A, or press ${key}, to hear the saved settings.`
      : "";
  }
  if (st.ab.saved) {
    return st.ab.side === "a"
      ? `A/B compare: you hear A, the preset as saved on the device. B is now, with your changes. Click B, or press ${key}, to go back.`
      : `A/B compare: you hear B, now, with your changes. A is the preset as saved on the device. Click A, or press ${key}, to switch.`;
  }
  return `A/B compare: A is “${st.ab.aName}”, B is “${st.ab.bName}”. Click the other one, or press ${key}, to switch.`;
}

export function AbSegment(props: { desk: SnapshotDesk; unsaved: number; locked: boolean }) {
  const st = useDesk(props.desk);
  const ro = !!props.desk.readOnly;
  const offered = mayStartSaved(st, ro, props.unsaved);
  if (props.locked || ro || (!st.ab.active && !offered)) return null;
  const hint = abKeyHint();
  const title = abTitle(st, props.unsaved, hint.text);
  // B, now, is what plays until a compare starts
  const side = st.ab.active ? st.ab.side : "b";
  const choose = (to: "a" | "b") => {
    if (to === side) return;
    if (!st.ab.active) {
      if (to === "a") void props.desk.abStartSaved();
    } else if (mayToggle(st, ro)) void props.desk.abToggle();
  };
  const busy = !!st.busy || st.guard?.phase === "waiting";
  const named = st.ab.active && !st.ab.saved;
  return (
    <div class="segments top-ab" role="group" aria-label="A/B toggle" title={title} aria-keyshortcuts={hint.aria}>
      <button type="button" aria-pressed={side === "a"} aria-label={named ? `A, ${st.ab.aName}` : "A, as saved"} disabled={busy} onClick={() => choose("a")}>
        A
      </button>
      <button type="button" aria-pressed={side === "b"} aria-label={named ? `B, ${st.ab.bName}` : "B, now"} disabled={busy} onClick={() => choose("b")}>
        B
      </button>
    </div>
  );
}
