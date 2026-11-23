// -- WP ODEC -- The wiring of ui/follow.ts: the shell watches every open session's changes from
// elsewhere and shows the device and page an agent is changing; the session's view, mounted for
// the device on show, takes its page from here.

import { useEffect, useRef } from "preact/hooks";
import type { Session } from "./session";
import { presenceNow } from "./presence";
import { placeOf } from "./shell/places";
import { agentOn, changedScreen, followGate, inputLeftMs, interacting, type FollowTarget } from "./follow";

const pageListeners = new Map<Session, Set<(screen: string) => void>>();

/** The session's view follows the page asked for here; returns the way to stop. */
export function watchFollow(s: Session, fn: (screen: string) => void): () => void {
  let set = pageListeners.get(s);
  if (!set) pageListeners.set(s, (set = new Set()));
  set.add(fn);
  return () => set!.delete(fn);
}

function serialOf(s: Session): string {
  return s.state.snapshot?.device.serial ?? "";
}

/** Follow the agents on `sessions`; `current` is on show, `show` brings another to the front. */
export function useFollowAgent(sessions: Session[], current: Session | null, show: (id: string) => void): void {
  const now = useRef({ current, show });
  now.current = { current, show };
  const pointer = useRef(false);
  // -- WP INTI -- (review RF7) the person's last key, pointer or wheel input
  const lastInput = useRef(Number.NEGATIVE_INFINITY);
  useEffect(() => {
    if (typeof document === "undefined") return;
    const stamp = () => (lastInput.current = Date.now());
    const down = () => {
      pointer.current = true;
      stamp();
    };
    const up = () => {
      pointer.current = false;
      stamp();
    };
    const inputs = ["keydown", "keyup", "wheel"] as const;
    document.addEventListener("pointerdown", down, true);
    document.addEventListener("pointerup", up, true);
    document.addEventListener("pointercancel", up, true);
    for (const k of inputs) document.addEventListener(k, stamp, true);
    return () => {
      document.removeEventListener("pointerdown", down, true);
      document.removeEventListener("pointerup", up, true);
      document.removeEventListener("pointercancel", up, true);
      for (const k of inputs) document.removeEventListener(k, stamp, true);
    };
  }, []);
  useEffect(() => {
    // -- WP INTI -- a follow held off by an input in flight is tried again once it rests
    let retry: ReturnType<typeof setTimeout> | undefined;
    const attempt = (s: Session, serial: string, screen: string) => {
      const { current: on, show: bring } = now.current;
      const hereScreen = on ? (placeOf(on).screen ?? "") : "";
      const left = inputLeftMs(lastInput.current, Date.now());
      if (left > 0 || pointer.current) {
        clearTimeout(retry);
        retry = setTimeout(() => attempt(s, serial, screen), Math.max(left, 200) + 50);
        return;
      }
      // Measure keeps its own rule: a page running a measurement is not left for a change
      const busy = interacting(typeof document === "undefined" ? null : document, pointer.current) || hereScreen === "measure";
      const here: FollowTarget | null = on ? { serial: serialOf(on), screen: hereScreen } : null;
      if (!followGate.take({ serial, screen }, Date.now(), busy, here)) return;
      if (s !== on) {
        placeOf(s).screen = screen;
        bring(s.id);
      } else for (const l of [...(pageListeners.get(s) ?? [])]) l(screen);
    };
    const stops = sessions.flatMap((s) => [
      presenceNow(s).keep(),
      s.watchElsewhere((before, after) => {
        const serial = after.device.serial ?? "";
        if (!agentOn(presenceNow(s).rows, serial)) return;
        const screen = changedScreen(before?.state, after.state);
        if (!screen) return;
        attempt(s, serial, screen);
      }),
    ]);
    return () => {
      clearTimeout(retry);
      stops.forEach((f) => f());
    };
  }, [sessions]);
}
