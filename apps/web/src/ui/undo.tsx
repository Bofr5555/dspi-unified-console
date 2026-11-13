// Undo and Redo (WP WB8): the core's journal (WP P0W), as the other clients
// have it. Buttons beside the screen tabs, and the keys: Ctrl+Z (Cmd+Z on a
// Mac) undoes, Ctrl+Shift+Z or Ctrl+Y redoes. The keys are left to a text
// field that has focus (its own undo), and on a page with several consoles
// they act on the console the focus is in, or the first one when focus is
// nowhere in particular. Read only (a viewer, firmware the core only reads):
// both off.

import type { RefObject } from "preact";
import { useEffect } from "preact/hooks";
import type { Session, SessionState } from "./session";

/** Whether the platform's command key is Cmd (Apple) rather than Ctrl. */
export function appleKeys(platform: string): boolean {
  return /mac|iphone|ipad|ipod/i.test(platform);
}

/** What a key press asks for, or null: `undo`, `redo`. */
export function undoKey(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">, apple: boolean): "undo" | "redo" | null {
  const cmd = apple ? e.metaKey : e.ctrlKey;
  if (!cmd || e.altKey) return null;
  const k = e.key.toLowerCase();
  if (k === "z") return e.shiftKey ? "redo" : "undo";
  if (k === "y" && !e.shiftKey && !apple) return "redo";
  return null;
}

const NOT_TEXT = new Set(["range", "checkbox", "radio", "button", "submit", "reset", "color", "file", "image"]);

/** Whether `el` takes typing, so its own undo comes first. */
export function isTextField(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "TEXTAREA") return true;
  if (tag === "INPUT") return !NOT_TEXT.has(((el as HTMLInputElement).type || "text").toLowerCase());
  return (el as HTMLElement).isContentEditable === true;
}

function platform(): string {
  const n = typeof navigator === "undefined" ? null : navigator;
  return (n as (Navigator & { userAgentData?: { platform?: string } }) | null)?.userAgentData?.platform || n?.platform || "";
}

/** The keys, for the session whose view is `root`. */
export function useUndoKeys(session: Session, root: RefObject<HTMLElement | null>): void {
  useEffect(() => {
    const apple = appleKeys(platform());
    const onKey = (e: KeyboardEvent) => {
      const what = undoKey(e, apple);
      if (!what || e.defaultPrevented) return;
      const here = root.current;
      if (!here) return;
      const focus = document.activeElement;
      if (isTextField(focus)) return;
      const nowhere = !focus || focus === document.body || focus === document.documentElement;
      const mine = nowhere ? document.querySelector(".dspi-web .session-screens") === here : here.contains(focus);
      if (!mine) return;
      e.preventDefault();
      void (what === "undo" ? session.undo() : session.redo());
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [session, root]);
}

/** The two buttons. */
export function UndoButtons(props: { session: Session; state: SessionState }) {
  const { session, state } = props;
  const ro = !!session.readOnlyReason;
  const apple = appleKeys(platform());
  const mod = apple ? "⌘" : "Ctrl+";
  return (
    <div class="undo-redo" role="group" aria-label="Undo and Redo">
      <button
        type="button"
        disabled={ro || !state.history.canUndo}
        title={`Undo (${mod}Z)`}
        aria-keyshortcuts={apple ? "Meta+Z" : "Control+Z"}
        onClick={() => void session.undo()}
      >
        Undo
      </button>
      <button
        type="button"
        disabled={ro || !state.history.canRedo}
        title={apple ? "Redo (⇧⌘Z)" : "Redo (Ctrl+Shift+Z)"}
        aria-keyshortcuts={apple ? "Meta+Shift+Z" : "Control+Shift+Z Control+Y"}
        onClick={() => void session.redo()}
      >
        Redo
      </button>
    </div>
  );
}

/** What the last step left alone, until dismissed or the next step. */
export function StepNotice(props: { session: Session; notice: SessionState["notice"] }) {
  const n = props.notice;
  if (!n) return null;
  return (
    <p class={`step-notice ${n.tone}`} role="status">
      <span>{n.text}</span>
      <button type="button" class="dismiss" aria-label="Dismiss" onClick={() => props.session.dismissNotice()}>
        ×
      </button>
    </p>
  );
}
