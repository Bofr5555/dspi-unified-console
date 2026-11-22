// -- WP WEBM4 -- What the notification drawer shows (MODLAY 3: everything that limits nothing goes to
// the drawer; review RB): what waits now on each open device (its error line, its step notice), and
// under it what was said earlier while this page was open, newest first, each with its device and
// time. Kept for the page's life only, at most 50.

import type { Session } from "../session";

export type NoticeKind = "error" | "warning" | "info";

export interface Notice {
  id: number;
  text: string;
  kind: NoticeKind;
  /** When it was said. */
  at: Date;
  /** The device it is about, "" for the page's own. */
  device: string;
}

export interface Waiting {
  key: string;
  session: Session;
  device: string;
  text: string;
  kind: NoticeKind;
  /** Takes it away (the device's error line, its notice). */
  dismiss: () => void;
}

export const KEPT = 50;

export function deviceName(s: Session): string {
  return s.state.snapshot?.device.name || s.installationName;
}

/** The error line and the step notice of each open device that has one. */
export function waiting(sessions: readonly Session[]): Waiting[] {
  const out: Waiting[] = [];
  for (const s of sessions) {
    const st = s.state;
    if (st.error) {
      const n = st.errorNotice;
      out.push({ key: `${s.id}:error`, session: s, device: deviceName(s), text: n ? `${n.title}. ${n.reason} ${st.error}` : st.error, kind: "error", dismiss: () => s.report("") });
    }
    if (st.notice)
      out.push({ key: `${s.id}:notice`, session: s, device: deviceName(s), text: st.notice.text, kind: st.notice.tone === "warn" ? "warning" : "info", dismiss: () => s.dismissNotice() });
  }
  return out;
}

export class NoticeLog {
  earlier: Notice[] = [];
  private next = 1;
  private said = new Map<Session, { error: string; notice: string }>();
  private stops = new Map<Session, () => void>();
  private fns = new Set<() => void>();

  /** Follows these sessions (and stops following the others). */
  track(sessions: readonly Session[]): void {
    for (const [s, stop] of this.stops)
      if (!sessions.includes(s)) {
        stop();
        this.stops.delete(s);
        this.said.delete(s);
      }
    for (const s of sessions) {
      if (this.stops.has(s)) continue;
      this.said.set(s, { error: s.state.error, notice: s.state.notice?.text ?? "" });
      this.stops.set(s, s.watch(() => this.follow(s)));
    }
    this.changed();
  }

  private follow(s: Session): void {
    const was = this.said.get(s) ?? { error: "", notice: "" };
    const now = { error: s.state.error, notice: s.state.notice?.text ?? "" };
    this.said.set(s, now);
    if (now.error && now.error !== was.error) this.say(now.error, "error", deviceName(s));
    if (now.notice && now.notice !== was.notice) this.say(now.notice, s.state.notice?.tone === "warn" ? "warning" : "info", deviceName(s));
    // What waits may have changed
    this.changed();
  }

  say(text: string, kind: NoticeKind, device = ""): void {
    if (!text) return;
    this.earlier = [{ id: this.next++, text, kind, at: new Date(), device }, ...this.earlier].slice(0, KEPT);
    this.changed();
  }

  clear(): void {
    this.earlier = [];
    this.changed();
  }

  watch(fn: () => void): () => void {
    this.fns.add(fn);
    return () => this.fns.delete(fn);
  }

  stop(): void {
    this.track([]);
  }

  private changed(): void {
    for (const f of [...this.fns]) f();
  }
}
