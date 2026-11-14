// WP P1WB1: the keys the web lacked from Qt's table (MenuTable.qml, U3): Preset 1-9
// (Ctrl+1..9), Commit (Ctrl+S), Revert to Saved (Ctrl+Shift+R), Mute (Ctrl+M), Bypass Master
// EQ (Ctrl+B), and the A/B toggle (Ctrl+K). Cmd on a Mac. Undo and redo stay undo.tsx's.
// As there: never while a text field has focus, and on a page with several consoles they act
// on the console the focus is in (the first one when focus is nowhere in particular). A key
// the browser keeps for itself (some take Ctrl+1..9 or Ctrl+Shift+R) simply never arrives.

import type { RefObject } from "preact";
import { useEffect } from "preact/hooks";
import { appleKeys, isTextField } from "./undo";

export type ConsoleKey =
  | { action: "preset"; slot: number }
  | { action: "commit" }
  | { action: "revert" }
  | { action: "mute" }
  | { action: "bypass" }
  | { action: "ab" };

/** What a key press asks for, or null. */
export function consoleKey(e: Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">, apple: boolean): ConsoleKey | null {
  const cmd = apple ? e.metaKey : e.ctrlKey;
  const other = apple ? e.ctrlKey : e.metaKey;
  if (!cmd || other || e.altKey) return null;
  // A digit by its key, or by its place on the keyboard when Shift or a layout makes it another character.
  const digit = /^[1-9]$/.test(e.key) ? Number(e.key) : /^Digit[1-9]$/.test(e.code ?? "") && !e.shiftKey ? Number(e.code.slice(5)) : 0;
  if (digit && !e.shiftKey) return { action: "preset", slot: digit - 1 };
  const k = e.key.toLowerCase();
  if (e.shiftKey) return k === "r" ? { action: "revert" } : null;
  if (k === "s") return { action: "commit" };
  if (k === "m") return { action: "mute" };
  if (k === "b") return { action: "bypass" };
  if (k === "k") return { action: "ab" };
  return null;
}

function platform(): string {
  const n = typeof navigator === "undefined" ? null : navigator;
  return (n as (Navigator & { userAgentData?: { platform?: string } }) | null)?.userAgentData?.platform || n?.platform || "";
}

/** The A/B key as a button's title and its `aria-keyshortcuts`. */
export function abKeyHint(): { text: string; aria: string } {
  return appleKeys(platform()) ? { text: "⌘K", aria: "Meta+K" } : { text: "Ctrl+K", aria: "Control+K" };
}

/** The keys for the console whose view is `root`; `act` runs one. */
export function useConsoleKeys(root: RefObject<HTMLElement | null>, act: (k: ConsoleKey) => void): void {
  useEffect(() => {
    const apple = appleKeys(platform());
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.repeat) return;
      const what = consoleKey(e, apple);
      if (!what) return;
      const here = root.current;
      if (!here) return;
      const focus = document.activeElement;
      if (isTextField(focus)) return;
      const nowhere = !focus || focus === document.body || focus === document.documentElement;
      const mine = nowhere ? document.querySelector(".dspi-web .session-screens") === here : here.contains(focus);
      if (!mine) return;
      e.preventDefault();
      act(what);
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [root, act]);
}
