// The channel clipboard on the web (WP WBCLIP, PARITY M45): Copy Parameters and
// Paste Parameters, over the core's clipboard (CLIP1: `copyChannel`,
// `pasteChannel` in crates/dspi-wasm/src/clipboard.rs), as Qt's
// SidebarController and the Android bridge are thin over it.
//
// - Copy reads the channel from the core's mirror (nothing is sent) and keeps
//   the clip, the core's document, in this page's memory: one clip per
//   session, never the system clipboard, never storage.
// - Paste is the core's: the clip's bands onto the channel and, for an input
//   whose pair is linked (WB6G's `inputLinks`), onto its partner too, so the
//   pair never drifts apart; crossover, gain, delay and mute only from one
//   output to another. One undo step ("Paste Parameters"); a refusal (read
//   only, the installer's lock, another clip version, a limit) writes nothing
//   and shows the core's words through the session's error line and notice.
// - The keys (Ctrl/Cmd+C and +V, screens/shortcuts/table.ts) act on the
//   channel chosen on the EQ screen, the Console's sidebar selection, while
//   that screen is shown; elsewhere they do nothing.

import { useEffect, useState } from "preact/hooks";
import type { ChannelClip } from "../../core/protocol";
import type { Session } from "../../ui/session";
import { inputLinks } from "../inputs/model";

export class ChannelClipboard {
  private readonly listeners = new Set<() => void>();
  private kept: ChannelClip | null = null;
  /** The wire channel the keys act on: the EQ screen's chosen one while it is shown, else null. */
  selected: number | null = null;

  constructor(private readonly session: Session) {}

  /** A copy under way: a paste right after it waits for it. */
  private copying: Promise<boolean> | null = null;

  /** Whether something was copied. */
  get hasClip(): boolean {
    return this.kept !== null;
  }

  /** Whether a copy is still under way. */
  get copyPending(): boolean {
    return this.copying !== null;
  }

  /** Whether Paste may be offered now: something copied, and the session may write. */
  get canPaste(): boolean {
    return this.kept !== null && !this.session.readOnlyReason && this.session.state.status === "open";
  }

  /**
   * Copy wire channel `ch` (inputs, then outputs). A refusal shows on the
   * session's error line and leaves the clip as it was. Answers whether it
   * was copied.
   */
  copy(ch: number): Promise<boolean> {
    if (this.session.state.status !== "open") return Promise.resolve(false);
    const run: Promise<boolean> = this.session.core.call("copyChannel", { ch }).then(
      ({ clip }) => {
        this.kept = clip;
        this.emit();
        return true;
      },
      (e: unknown) => {
        this.session.fail(e);
        return false;
      },
    );
    this.copying = run;
    void run.then(() => {
      if (this.copying === run) this.copying = null;
    });
    return run;
  }

  /**
   * Paste the clip onto wire channel `ch`, and onto its linked partner when
   * `ch` is an input whose pair is linked. Nothing before a copy. Answers
   * whether it went through (the session's error says why not).
   */
  async paste(ch: number): Promise<boolean> {
    if (this.copying) await this.copying;
    const kept = this.kept;
    if (!kept) return false;
    const s = this.session.state.snapshot?.state;
    const partner = s && ch < s.numInputChannels ? inputLinks(this.session).partner(ch) : -1;
    return this.session.change("pasteChannel", partner >= 0 ? { clip: kept, ch, mirror: partner } : { clip: kept, ch });
  }

  /** Forget the clip (tests; a session's clip otherwise lives as long as the page). */
  clear(): void {
    if (!this.kept) return;
    this.kept = null;
    this.emit();
  }

  watch(fn: () => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  private emit(): void {
    for (const l of [...this.listeners]) l();
  }
}

const boards = new WeakMap<Session, ChannelClipboard>();

/** The session's clipboard, one per session. */
export function channelClipboard(session: Session): ChannelClipboard {
  let b = boards.get(session);
  if (!b) {
    b = new ChannelClipboard(session);
    boards.set(session, b);
  }
  return b;
}

/** The menu entries for wire channel `ch`, in the Console's words. */
export function clipEntries(board: ChannelClipboard, ch: number, readOnly: boolean): { label: string; disabled?: boolean; onSelect: () => void }[] {
  return [
    { label: "Copy Parameters", onSelect: () => void board.copy(ch) },
    { label: "Paste Parameters", disabled: readOnly || !board.hasClip, onSelect: () => void board.paste(ch) },
  ];
}

/**
 * What Ctrl/Cmd+C or +V does: on the selected channel, if any. Answers false
 * when the key is not this console's to take (no channel selected; Paste with
 * nothing copied), so the browser keeps it.
 */
export function clipKey(board: ChannelClipboard, what: "copy" | "paste"): boolean {
  const ch = board.selected;
  if (ch === null) return false;
  if (what === "copy") {
    void board.copy(ch);
    return true;
  }
  if (!board.hasClip && !board.copyPending) return false;
  void board.paste(ch);
  return true;
}

/** The session's clipboard, re-rendering when something is copied. */
export function useClipboard(session: Session): ChannelClipboard {
  const board = channelClipboard(session);
  const [, bump] = useState(0);
  useEffect(() => board.watch(() => bump((n) => n + 1)), [board]);
  return board;
}
