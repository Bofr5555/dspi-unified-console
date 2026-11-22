// -- WP WEBM4 -- Menus of actions.json's rows (ui/menus/actions.ts): a menu button (the app menu,
// Save ▾) and a context menu at a point (a device tab, a channel, a preset slot; the EQ graph has
// its own place for its menus). ARIA's menu pattern, as screens/main/Menu.tsx: the first row takes
// the focus, arrows move, Escape closes and gives the focus back, Tab leaves. A row shows its
// shortcut; a greyed row says why under it.

import type { ComponentChildren } from "preact";
import { createPortal } from "preact/compat";
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";
import { appleKeys } from "../undo";
import { ariaKeys, keyText } from "../../screens/shortcuts/table";
import type { MenuItem, MenuRow } from "./actions";

function apple(): boolean {
  return appleKeys(typeof navigator === "undefined" ? "" : (navigator.platform ?? ""));
}

/** The rows of a menu, as a `role="menu"` list. `onDone` closes it (after a row ran, or on Escape). */
export function MenuList(props: {
  label: string;
  items: MenuItem[];
  class?: string;
  style?: Record<string, string>;
  listRef?: { current: HTMLUListElement | null };
  onDone: (refocus: boolean) => void;
}) {
  const mac = apple();
  const own = useRef<HTMLUListElement>(null);
  const ref = props.listRef ?? own;
  const rows = () => Array.from(ref.current?.querySelectorAll<HTMLButtonElement>("[role=menuitem]:not([disabled]), [role=menuitemcheckbox]:not([disabled])") ?? []);
  const onKey = (e: KeyboardEvent) => {
    const all = rows();
    const at = all.indexOf(document.activeElement as HTMLButtonElement);
    const go = (i: number) => all[(i + all.length) % all.length]?.focus();
    if (e.key === "ArrowDown") go(at + 1);
    else if (e.key === "ArrowUp") go(at < 0 ? -1 : at - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(all.length - 1);
    else if (e.key === "Escape") props.onDone(true);
    else if (e.key === "Tab") props.onDone(false);
    else return;
    e.preventDefault();
    e.stopPropagation();
  };
  return (
    <ul ref={ref} role="menu" aria-label={props.label} class={props.class} style={props.style} onKeyDown={onKey}>
      {props.items.map((m, i) =>
        m === "separator" ? (
          <li key={`s${i}`} role="separator" />
        ) : "heading" in m ? (
          <li key={`h${i}`} role="presentation" class="menu-heading">
            {m.heading}
          </li>
        ) : (
          <Row key={m.id + m.label} row={m} mac={mac} onDone={props.onDone} />
        ),
      )}
    </ul>
  );
}

function Row(props: { row: MenuRow; mac: boolean; onDone: (refocus: boolean) => void }) {
  const r = props.row;
  return (
    <li role="none">
      <button
        type="button"
        role={r.checked === undefined ? "menuitem" : "menuitemcheckbox"}
        aria-checked={r.checked === undefined ? undefined : r.checked}
        aria-keyshortcuts={r.shortcut ? ariaKeys(r.shortcut, props.mac) : undefined}
        aria-description={r.reason || undefined}
        tabIndex={-1}
        class={`action-row${r.danger ? " danger" : ""}`}
        data-action={r.id}
        disabled={r.disabled}
        title={r.reason || undefined}
        onClick={() => {
          props.onDone(true);
          r.run();
        }}
      >
        {/* The check and the keys are drawn by the stylesheet (data-*), so a row's text is its words alone */}
        <span class="action-check" aria-hidden="true" />
        <span class="action-text">{r.label}</span>
        {r.shortcut ? <span class="action-keys" aria-hidden="true" data-keys={keyText(r.shortcut, props.mac)} /> : null}
      </button>
      {r.reason ? (
        <span class="action-reason" aria-hidden="true">
          {r.reason}
        </span>
      ) : null}
    </li>
  );
}

/** A labelled button that opens a menu of rows (made when it opens, so they are current). */
export function ActionMenuButton(props: { label: string; items: () => MenuItem[]; disabled?: boolean; children?: ComponentChildren; title?: string }) {
  const [open, setOpen] = useState(false);
  const button = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLUListElement | null>(null);
  useLayoutEffect(() => {
    if (!open) return;
    list.current?.querySelector<HTMLButtonElement>("[role^=menuitem]:not([disabled])")?.focus();
    const outside = (e: PointerEvent) => {
      const t = e.target as Node;
      if (!list.current?.contains(t) && !button.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  const done = (refocus: boolean) => {
    setOpen(false);
    if (refocus) button.current?.focus();
  };
  return (
    <div class="menu" onKeyDown={(e) => open && e.key === "Escape" && (e.preventDefault(), done(true))}>
      <button ref={button} type="button" aria-haspopup="menu" aria-expanded={open} disabled={props.disabled} title={props.title} onClick={() => setOpen(!open)}>
        {props.children ?? props.label} <span aria-hidden="true">▾</span>
      </button>
      {open ? <MenuList label={props.label} items={props.items()} listRef={list} onDone={done} /> : null}
    </div>
  );
}

/** Where a context menu opens: at the pointer, or (from the keyboard) under the element. */
export function menuPoint(e: MouseEvent | KeyboardEvent): { x: number; y: number } {
  if (e instanceof MouseEvent && (e.clientX || e.clientY)) return { x: e.clientX, y: e.clientY };
  const r = (e.currentTarget as Element | null)?.getBoundingClientRect();
  return r ? { x: r.left + 8, y: r.bottom } : { x: 16, y: 16 };
}

/** A key that asks for an element's context menu: the menu key, or Shift+F10. */
export function isMenuKey(e: KeyboardEvent): boolean {
  return e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey);
}

/**
 * A context menu: `open(event, label, items)` from a right click or the menu key on an element;
 * `menu` is the element to render anywhere: it goes to the console's root (fixed on the page, kept
 * inside the window), out of the containers the console's parts are (they would hold a fixed
 * element). The focus goes back to the element it was opened on.
 */
export function useContextMenu(): {
  open: (e: MouseEvent | KeyboardEvent, label: string, items: MenuItem[]) => void;
  menu: ComponentChildren;
} {
  const [at, setAt] = useState<{ x: number; y: number; label: string; items: MenuItem[]; from: HTMLElement | null } | null>(null);
  const list = useRef<HTMLUListElement | null>(null);
  const open = useCallback((e: MouseEvent | KeyboardEvent, label: string, items: MenuItem[]) => {
    if (!items.some((m) => m !== "separator" && !("heading" in m))) return;
    e.preventDefault();
    e.stopPropagation();
    const p = menuPoint(e);
    setAt({ ...p, label, items, from: (e.currentTarget as HTMLElement | null) ?? null });
  }, []);
  useLayoutEffect(() => {
    const el = list.current;
    if (!at || !el) return;
    // Inside the window: moved left or up where it would leave it
    const w = el.offsetWidth;
    const h = el.offsetHeight;
    el.style.left = `${Math.max(4, Math.min(at.x, window.innerWidth - w - 4))}px`;
    el.style.top = `${Math.max(4, Math.min(at.y, window.innerHeight - h - 4))}px`;
    el.querySelector<HTMLButtonElement>("[role^=menuitem]:not([disabled])")?.focus();
  }, [at]);
  useEffect(() => {
    if (!at) return;
    const outside = (e: PointerEvent) => {
      if (!list.current?.contains(e.target as Node)) setAt(null);
    };
    const away = () => setAt(null);
    document.addEventListener("pointerdown", outside, true);
    window.addEventListener("resize", away);
    window.addEventListener("blur", away);
    return () => {
      document.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("resize", away);
      window.removeEventListener("blur", away);
    };
  }, [at]);
  const host = at?.from?.closest<HTMLElement>(".dspi-web") ?? (typeof document === "undefined" ? null : document.body);
  const menu = at && host ? createPortal(
    <MenuList
      label={at.label}
      items={at.items}
      class="context-menu"
      style={{ left: `${at.x}px`, top: `${at.y}px` }}
      listRef={list}
      onDone={(refocus) => {
        const from = at.from;
        setAt(null);
        if (refocus && from?.isConnected) from.focus({ preventScroll: true });
      }}
    />,
    host,
  ) : null;
  return { open, menu };
}
