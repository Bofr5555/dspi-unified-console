// WP GAP2: the strip while an output is soloed: which one plays, that nothing is saved, and End Solo
// (as Qt's status strip says it). The solo is the core's (ui/solo.ts).

import type { Session } from "./session";
import { useSolo } from "./solo";
import { Glyph } from "./design";
import { outputLabel } from "../screens/main/model";

export function SoloStrip(props: { session: Session }) {
  const solo = useSolo(props.session);
  const snap = props.session.state.snapshot;
  const o = solo.output;
  if (o === null || !snap) return null;
  const name = outputLabel(snap.state, snap.outputLabels, o);
  return (
    <div class="banner banner-info solo-strip" role="status" aria-label="Solo">
      <Glyph name="headphones" class="banner-icon" />
      <span>Solo: only {name} plays. The other outputs are muted until you end it; nothing is saved.</span>
      <button type="button" onClick={() => void solo.end()}>
        End Solo
      </button>
    </div>
  );
}
