// -- WP GAP1 -- An output's polarity, as Qt's Delay & Gain row and routing inspector show and set it
// (ChannelController::outputPolarity / setOutputPolarity): from the routes that feed the output (the
// core's, in the snapshot), and set by writing every feeding route's invert flag, one undo step.

import type { DeviceSnapshot, OutputPolarity } from "../../core/protocol";
import type { Session } from "../../ui/session";

/** Output `o`'s polarity in `snap` ("none" when the snapshot has none for it). */
export function polarityOf(snap: Pick<DeviceSnapshot, "outputPolarity"> | null | undefined, o: number): OutputPolarity {
  return snap?.outputPolarity?.[o] ?? "none";
}

/** Delay & Gain's words for it (Qt's): Normal, Inverted, Mixed, or a dash when no input feeds it. */
export function polarityText(p: OutputPolarity): string {
  return p === "normal" ? "Normal" : p === "inverted" ? "Inverted" : p === "mixed" ? "Mixed" : "–";
}

/** The routing inspector's line for it (Qt's): "Polarity inverted", "Polarity mixed", else "Polarity normal". */
export function polarityLine(p: OutputPolarity): string {
  return p === "inverted" ? "Polarity inverted" : p === "mixed" ? "Polarity mixed" : "Polarity normal";
}

/** What a click on Delay & Gain's polarity asks for: a normal output inverts, an inverted or mixed one goes normal. */
export function flipTo(p: OutputPolarity): boolean {
  return p === "normal";
}

/** The inputs whose routes to output `o` change for `inverted`: the feeding ones that differ. */
export function polarityRoutes(s: { numInputChannels: number; matrixRouting: boolean[][]; matrixInvert: boolean[][] }, o: number, inverted: boolean): number[] {
  return Array.from({ length: s.numInputChannels }, (_, i) => i).filter((i) => s.matrixRouting[i]?.[o] === true && (s.matrixInvert[i]?.[o] ?? false) !== inverted);
}

/** Set output `o` inverted (or not) on every route that feeds it, one undo step; true when nothing was refused. */
export async function setOutputPolarity(session: Session, o: number, inverted: boolean): Promise<boolean> {
  const s = session.state.snapshot?.state;
  if (!s) return false;
  const change = polarityRoutes(s, o, inverted);
  if (!change.length) return true;
  return session.group("Output polarity", async () => {
    for (const i of change) {
      const now = session.state.snapshot?.state ?? s;
      if (!(await session.change("crosspoint", { input: i, output: o, enabled: true, gain: now.matrixGain[i]?.[o] ?? 0, invert: inverted }))) return false;
    }
    return true;
  });
}
