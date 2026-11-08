// What is unsaved in a mounted console's sessions (WP WB6b, c6p WB6a-4): the
// one place the page's leave prompt and a host page learn of it. Each session
// keeps its own `unsaved` (the core's sentences); this follows the sessions a
// console has open and says whether any of them has something unsaved.
//
// With `leavePrompt` (the default) it holds the browser's own leave-page
// question while something is unsaved: one `beforeunload` listener per mounted
// console, not one per session or screen. A fleet host page that asks in its
// own way mounts with `leavePrompt: false` and uses `unsaved()` and
// `onUnsavedChange` instead.

import type { Session } from "./session";

export class UnsavedWatch {
  private sessions: Session[] = [];
  private stops: (() => void)[] = [];
  private listeners = new Set<(unsaved: boolean) => void>();
  private last = false;
  private readonly hold = (e: BeforeUnloadEvent) => e.preventDefault();

  constructor(private readonly leavePrompt: boolean) {}

  /** Whether any open session has something unsaved. */
  get unsaved(): boolean {
    return this.sessions.some((s) => s.dirty);
  }

  /** The console's open sessions, whenever they change. */
  track(sessions: Session[]): void {
    for (const stop of this.stops) stop();
    this.sessions = sessions;
    this.stops = sessions.map((s) => s.watch(() => this.update()));
    this.update();
  }

  /** `fn` is called with the new answer each time `unsaved` changes; the result stops it. */
  onChange(fn: (unsaved: boolean) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private update(): void {
    const now = this.unsaved;
    if (now === this.last) return;
    this.last = now;
    if (this.leavePrompt && typeof window !== "undefined") {
      if (now) window.addEventListener("beforeunload", this.hold);
      else window.removeEventListener("beforeunload", this.hold);
    }
    for (const l of this.listeners) l(now);
  }
}
