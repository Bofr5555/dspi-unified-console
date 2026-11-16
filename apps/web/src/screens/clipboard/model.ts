// The channel clipboard on the web (WP WBCLIP, PARITY M45): Copy Parameters and
// Paste Parameters, over the core's clipboard (CLIP1: `copyChannel`,
// `pasteChannel` in crates/dspi-wasm/src/clipboard.rs), as Qt's
// SidebarController and the Android bridge are thin over it.
//
// - Copy reads the channel from the core's mirror (nothing is sent) and keeps
//   the clip, the core's document, in this page's memory, never the system
//   clipboard, never storage.
//   -- WP WBCLIP2 -- one clip per PAGE (review-p3c N9), not per session: a
//   channel copied on one DSPi pastes onto another on the fleet page. The
//   paste is still the core's, on the TARGET session's core, which judges
//   the clip against the target before any write (its read-only state, the
//   installer's lock, its limits; the delay clamped to its line). When the
//   clip came from another session's device, the Paste entry names its
//   source: "Paste Parameters from <device> OUT1".
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

// -- WP WBCLIP2 -- the page's one clip, and where it came from
/** The page's clip: the core's document, and the session, device and channel it was copied from. */
export interface PageClip {
  clip: ChannelClip;
  /** The session it was copied on, by id (a closed session is not kept alive by it). */
  from: number;
  /** The source device as the user knows it: its name, with its serial when it has one. */
  device: string;
  /** The source channel's descriptor: "IN1", "OUT1". */
  channel: string;
}

const page = {
  kept: null as PageClip | null,
  /** A copy under way on any session: a paste right after it waits for it. */
  copying: null as Promise<boolean> | null,
  listeners: new Set<() => void>(),
};

const sessionIds = new WeakMap<Session, number>();
let nextSessionId = 1;
function sessionId(s: Session): number {
  let id = sessionIds.get(s);
  if (id === undefined) {
    id = nextSessionId++;
    sessionIds.set(s, id);
  }
  return id;
}

function emitPage(): void {
  for (const l of [...page.listeners]) l();
}

/** Forget the page's clip (tests; it otherwise lives as long as the page). */
export function clearPageClip(): void {
  if (!page.kept) return;
  page.kept = null;
  emitPage();
}

/** Wire channel `ch`'s descriptor on a device with `numInputs` inputs: "IN1", "OUT1". */
function descriptorOf(ch: number, numInputs: number): string {
  return ch < numInputs ? `IN${ch + 1}` : `OUT${ch - numInputs + 1}`;
}

/** The session's device for a Paste entry: "<name> (<serial>)", the name alone without a serial. */
function deviceOf(session: Session): string {
  const d = session.state.snapshot?.device;
  const name = d?.name || "DSPi";
  const serial = d?.serial || session.deviceId;
  return serial && serial !== name ? `${name} (${serial})` : name;
}

export class ChannelClipboard {
  /** The wire channel the keys act on: the EQ screen's chosen one while it is shown, else null. */
  selected: number | null = null;

  constructor(private readonly session: Session) {}

  /** Whether something was copied (on this page, on any session). */
  get hasClip(): boolean {
    return page.kept !== null;
  }

  /** Whether a copy is still under way (on this page). */
  get copyPending(): boolean {
    return page.copying !== null;
  }

  /** Whether Paste may be offered now: something copied, and this session may write. */
  get canPaste(): boolean {
    return page.kept !== null && !this.session.readOnlyReason && this.session.state.status === "open";
  }

  // -- WP WBCLIP2 --
  /** The page's clip and its source, null before a copy. */
  get clip(): PageClip | null {
    return page.kept;
  }

  /** Whether the page's clip was copied on another session (another device). */
  get fromElsewhere(): boolean {
    return page.kept !== null && page.kept.from !== sessionId(this.session);
  }

  /** The Paste entry's words: "Paste Parameters", naming the source when it is another device's. */
  get pasteLabel(): string {
    const k = page.kept;
    return k && this.fromElsewhere ? `Paste Parameters from ${k.device} ${k.channel}` : "Paste Parameters";
  }

  /**
   * Copy wire channel `ch` (inputs, then outputs) as the page's clip. A
   * refusal shows on the session's error line and leaves the clip as it was.
   * Answers whether it was copied.
   */
  copy(ch: number): Promise<boolean> {
    if (this.session.state.status !== "open") return Promise.resolve(false);
    const run: Promise<boolean> = this.session.core.call("copyChannel", { ch }).then(
      ({ clip }) => {
        const n = this.session.state.snapshot?.state.numInputChannels ?? 0;
        page.kept = { clip, from: sessionId(this.session), device: deviceOf(this.session), channel: descriptorOf(ch, n) };
        emitPage();
        return true;
      },
      (e: unknown) => {
        this.session.fail(e);
        return false;
      },
    );
    page.copying = run;
    void run.then(() => {
      if (page.copying === run) page.copying = null;
    });
    return run;
  }

  /**
   * Paste the page's clip onto this session's wire channel `ch`, and onto its
   * linked partner when `ch` is an input whose pair is linked. Nothing before
   * a copy. Every check is the core's, on this session's core. Answers
   * whether it went through (the session's error says why not).
   */
  async paste(ch: number): Promise<boolean> {
    if (page.copying) await page.copying;
    const kept = page.kept?.clip;
    if (!kept) return false;
    const s = this.session.state.snapshot?.state;
    const partner = s && ch < s.numInputChannels ? inputLinks(this.session).partner(ch) : -1;
    return this.session.change("pasteChannel", partner >= 0 ? { clip: kept, ch, mirror: partner } : { clip: kept, ch });
  }

  /** Forget the clip (tests; the page's clip otherwise lives as long as the page). */
  clear(): void {
    clearPageClip();
  }

  /** Called when something is copied, on any session of this page. */
  watch(fn: () => void): () => void {
    page.listeners.add(fn);
    return () => page.listeners.delete(fn);
  }
}

const boards = new WeakMap<Session, ChannelClipboard>();

/** The session's clipboard, one per session (its chosen channel); the clip itself is the page's. */
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
    // -- WP WBCLIP2 -- naming the source when it is another device's
    { label: board.pasteLabel, disabled: readOnly || !board.hasClip, onSelect: () => void board.paste(ch) },
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

/** The session's clipboard, re-rendering when something is copied on any session of the page. */
export function useClipboard(session: Session): ChannelClipboard {
  const board = channelClipboard(session);
  const [, bump] = useState(0);
  useEffect(() => board.watch(() => bump((n) => n + 1)), [board]);
  return board;
}
