// WP WB6l: the app shortcuts beyond the graph keys (M24) and the quick strip
// of effect toggles (M60).
//
// The keys of table.ts open a screen of this console, as the Console's Tools
// menu opens a window; Shift+? lists every key this console has. As the other
// keys (ui/keys.ts, ui/undo.tsx): never while a text field has the focus or a
// dialog asks, and on a page with several consoles they act on the console the
// focus is in (the first one when focus is nowhere in particular).
//
// The quick strip is the Console's row of icons under the channel lists
// (`ContentView.swift:575-602`, Qt's QuickStrip.qml): the Matrix, the four
// effects, Stats, Settings and the master EQ bypass. A click switches an effect
// by `write` of its registry path (the core's `quickStrip` names the paths and
// which the device has: psychoacoustic bass is gone, not greyed, on firmware
// without it); a secondary click or a long press opens that effect's screen. A
// screen's button opens or leaves that screen. Read only, the switches are off;
// the screens still open. Every write goes through the core, which refuses it
// while read only or under the installer's lock.

import type { RefObject } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { QuickStripRow } from "../../core/protocol";
import { Dialog } from "../main/Dialog";
import type { Session } from "../../ui/session";
import { useSessionState } from "../../ui/components";
import { appleKeys, isTextField } from "../../ui/undo";
import { ariaKeys, keyText, listed, shortcutFor, toolRow, type OpenAction, type ShortcutRow } from "./table";

function platform(): string {
  const n = typeof navigator === "undefined" ? null : navigator;
  return (n as (Navigator & { userAgentData?: { platform?: string } }) | null)?.userAgentData?.platform || n?.platform || "";
}

export function isApple(): boolean {
  return appleKeys(platform());
}

/**
 * The table's keys for the console whose view is `root`. `act` runs a row; `blocked` is true while a question is asked.
 * -- WP WBCLIP -- `act` answering false leaves the key to the browser (a clipboard key with no channel to act on).
 */
export function useAppShortcuts(root: RefObject<HTMLElement | null>, act: (r: ShortcutRow) => boolean | void, blocked: () => boolean): void {
  useEffect(() => {
    const apple = isApple();
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat) return;
      const row = shortcutFor(e, apple);
      if (!row) return;
      const here = root.current;
      if (!here || blocked()) return;
      const focus = document.activeElement;
      if (isTextField(focus)) return;
      if (focus?.closest?.('dialog, [role="dialog"], [role="alertdialog"]') || document.querySelector('.dspi-web dialog[open], .dspi-web [role="dialog"]')) return;
      const nowhere = !focus || focus === document.body || focus === document.documentElement;
      const mine = nowhere ? document.querySelector(".dspi-web .session-screens") === here : here.contains(focus);
      if (!mine) return;
      // -- WP WBCLIP -- text the user selected on the page is the browser's to copy
      if (row.clip === "copy" && hasTextSelection()) return;
      if (act(row) === false) return;
      e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [root, act, blocked]);
}

// -- WP WBCLIP --
/** Whether the page has selected text, which Ctrl+C copies as text. */
function hasTextSelection(): boolean {
  const sel = typeof window === "undefined" ? null : window.getSelection?.();
  return !!sel && !sel.isCollapsed && sel.toString().length > 0;
}

/** The Keyboard Shortcuts list: every key this console has, by the Console's menus. */
export function ShortcutsDialog(props: { screens: Set<string>; onClose: () => void }) {
  const apple = isApple();
  return (
    <Dialog title="Keyboard Shortcuts" onCancel={props.onClose} buttons={[{ label: "Close", onClick: props.onClose }]}>
      <div class="shortcut-list">
        {listed(props.screens).map((g) => (
          <section key={g.menu} aria-label={g.menu}>
            <h3>{g.menu}</h3>
            <dl>
              {g.rows.map((r) => (
                <div key={r.id} class="shortcut-row">
                  <dt>{r.text}</dt>
                  <dd>
                    <kbd>{keyText(r.key, apple)}</kbd>
                  </dd>
                </div>
              ))}
            </dl>
          </section>
        ))}
      </div>
      <p class="empty">Keys the browser keeps for itself never reach the console; the screen tabs and the quick strip go to the same places.</p>
    </Dialog>
  );
}

/** The Console's words for the strip's switches. */
const STRIP_WORDS: Record<QuickStripRow["id"], string> = {
  crossfeed: "Headphone Crossfeed",
  loudness: "Loudness Compensation",
  leveller: "Volume Leveller",
  psybass: "Psychoacoustic Bass",
  bypass: "Bypass Master EQ",
};

/** How long a press holds before it opens the tool instead (the Android and iOS consoles' long press). */
const LONG_PRESS_MS = 550;

/** The quick strip of one console. `screens` are the screens it shows now, `current` the one on show. */
export function QuickStrip(props: { session: Session; screens: Set<string>; current: string; open: (a: OpenAction) => void; leave: () => void; onHelp: () => void }) {
  const { session } = props;
  const st = useSessionState(session);
  const [rows, setRows] = useState<QuickStripRow[]>([]);
  const snap = st.snapshot;
  useEffect(() => {
    let alive = true;
    if (!snap || st.status !== "open") {
      setRows([]);
      return;
    }
    void session.core.call("quickStrip", undefined).then(
      (r) => alive && setRows(r),
      () => alive && setRows([]),
    );
    return () => {
      alive = false;
    };
  }, [session, snap, st.status]);
  const held = useRef<{ timer: ReturnType<typeof setTimeout> | null; fired: boolean }>({ timer: null, fired: false });
  const apple = isApple();
  const ro = !!session.readOnlyReason;
  const row = (id: QuickStripRow["id"]) => rows.find((r) => r.id === id && r.present);

  const windowButton = (screen: string, label: string, glyph: string, key?: string) => {
    if (!props.screens.has(screen)) return null;
    const on = props.current === screen;
    return (
      <button
        type="button"
        class="toggle strip-button"
        aria-pressed={on}
        aria-label={label}
        title={key ? `${label} (${keyText(key, apple)})` : label}
        aria-keyshortcuts={key ? ariaKeys(key, apple) : undefined}
        onClick={() => (on ? props.leave() : props.open({ screen }))}
      >
        <span aria-hidden="true">{glyph}</span>
      </button>
    );
  };

  const effectButton = (id: QuickStripRow["id"], glyph: string) => {
    const r = row(id);
    if (!r) return null;
    const tool = id === "bypass" ? undefined : toolRow(id);
    const canOpen = !!tool?.open && props.screens.has(tool.open.screen);
    const openTool = () => {
      if (canOpen && tool?.open) props.open(tool.open);
    };
    const words = STRIP_WORDS[id];
    const clear = () => {
      if (held.current.timer) clearTimeout(held.current.timer);
      held.current.timer = null;
    };
    return (
      <button
        type="button"
        class="toggle strip-button"
        aria-pressed={r.on}
        aria-label={words}
        title={`${words}: ${r.on ? "on" : "off"}.${ro ? "" : " Click to switch."}${canOpen ? " Right-click or hold to open." : ""}`}
        disabled={ro}
        onClick={() => {
          if (held.current.fired) {
            held.current.fired = false;
            return;
          }
          void session.write(r.path, { bool: !r.on });
        }}
        onContextMenu={(e) => {
          if (!canOpen) return;
          e.preventDefault();
          clear();
          openTool();
        }}
        onPointerDown={(e) => {
          if (!canOpen || e.pointerType === "mouse") return;
          held.current.fired = false;
          clear();
          held.current.timer = setTimeout(() => {
            held.current.timer = null;
            held.current.fired = true;
            openTool();
          }, LONG_PRESS_MS);
        }}
        onPointerUp={clear}
        onPointerLeave={clear}
        onPointerCancel={clear}
      >
        <span aria-hidden="true">{glyph}</span>
      </button>
    );
  };

  if (!snap || st.status !== "open") return null;
  return (
    <div class="quick-strip" role="toolbar" aria-label="Quick access">
      {windowButton("matrix", "Matrix Mixer", "▦", "Ctrl+Shift+M")}
      {effectButton("crossfeed", "🎧")}
      {effectButton("loudness", "🔉")}
      {effectButton("leveller", "∿")}
      {effectButton("psybass", "𝄢")}
      {windowButton("stats", "Stats for nerbs", "ⓘ", "Ctrl+Shift+T")}
      {windowButton("settings", "Settings", "⚙", "Ctrl+,")}
      {effectButton("bypass", "✕")}
      <button type="button" class="strip-button strip-help" aria-label="Keyboard Shortcuts" title={`Keyboard Shortcuts (${keyText("Shift+?", apple)})`} aria-keyshortcuts="Shift+?" onClick={props.onHelp}>
        <span aria-hidden="true">⌨</span>
      </button>
    </div>
  );
}
