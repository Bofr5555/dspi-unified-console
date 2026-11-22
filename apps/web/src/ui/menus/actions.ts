// -- WP WEBM4 -- The desktop app's actions (assets/actions.json, written from Qt's MenuTable by
// LAY3b): every menu row and every action on one thing, with its words, its shortcut and the menus
// it is in, by group. The web's menus, context menus and command palette are made from it, so a
// row has the same words and the same place on both. A row the web has no handler for is left
// out, never shown greyed: a handler is what makes a row the web's (`Handlers`).

import table from "../../../../../assets/actions.json";

export interface ActionDef {
  id: string;
  text: string;
  /** The Classic menu it is in (for the palette's "where"); none for an item's action. */
  menu?: string;
  shortcut?: string;
  places: string[];
  /** The Classic submenu it is a row of ("Recall Preset"). */
  submenu?: string;
  /** The row this item's action is the same as (`channel.copy` of `edit.copy`). */
  of?: string;
  /** Its words while it is on ("Unmute"). */
  textOn?: string;
  /** Rows the client makes itself (`submenu`, `dynamic`). */
  rows?: "submenu" | "dynamic";
  optional?: boolean;
  hiddenOn?: string;
}

interface Table {
  version: number;
  actions: ActionDef[];
  itemActions: ActionDef[];
  menus: Record<string, string[][]>;
}

const t = table as Table;

/** The menus' rows (the app's actions); then the actions on one thing (a channel, a band, a slot). */
export const ACTIONS: readonly ActionDef[] = t.actions;
export const ITEM_ACTIONS: readonly ActionDef[] = t.itemActions;
/** Each menu's rows, by group. */
export const MENUS: Readonly<Record<string, readonly (readonly string[])[]>> = t.menus;

const byId = new Map<string, ActionDef>([...t.actions, ...t.itemActions].map((a) => [a.id, a]));

export function actionDef(id: string): ActionDef | undefined {
  return byId.get(id);
}

/** The app's name in a row's words ("About {app}"). */
export const APP_NAME = "uConsole";

/** A row's words, as on or off where it has words for on; the app's name filled in. */
export function actionText(id: string, on = false): string {
  const d = byId.get(id);
  if (!d) return id;
  return (on && d.textOn ? d.textOn : d.text).split("{app}").join(APP_NAME);
}

/** The shortcut a row shows: its own, or the one of the row it is the same as. */
export function actionShortcut(id: string): string {
  const d = byId.get(id);
  return d?.shortcut ?? (d?.of ? (byId.get(d.of)?.shortcut ?? "") : "");
}

/** What the web does for a row, now. */
export interface Handler {
  run: () => void;
  /** Greyed (listed, not chosen) when false; `reason` says why. */
  enabled?: boolean;
  reason?: string;
  /** On (its `textOn` words; a check where it has none). */
  on?: boolean;
  /** Words of the web's own in place of the table's, where the row has to say more (a paste's source). */
  text?: string;
  danger?: boolean;
}

export type Handlers = Readonly<Partial<Record<string, Handler>>>;

/** A row of a menu as shown. */
export interface MenuRow {
  id: string;
  label: string;
  shortcut: string;
  disabled: boolean;
  reason: string;
  /** A check mark: for a row that is on and has no words for it. */
  checked?: boolean;
  danger: boolean;
  run: () => void;
}

export type MenuItem = MenuRow | { heading: string } | "separator";

export function menuRow(id: string, h: Handler): MenuRow {
  const d = byId.get(id);
  const words = h.text ?? actionText(id, h.on === true);
  return {
    id,
    label: words,
    shortcut: actionShortcut(id),
    disabled: h.enabled === false,
    reason: h.enabled === false ? (h.reason ?? "") : "",
    checked: h.on !== undefined && !d?.textOn ? h.on : undefined,
    danger: h.danger === true,
    run: h.run,
  };
}

/** A row of the web's own, which actions.json does not have (Open a DSPi…, the shortcut list). */
export function ownRow(id: string, label: string, run: () => void, more: Partial<Omit<MenuRow, "id" | "label" | "run">> = {}): MenuRow {
  return { id, label, shortcut: "", disabled: false, reason: "", danger: false, run, ...more };
}

/**
 * The rows of `menu` the web has handlers for, its groups apart: a group with no row is left out,
 * and so is the menu when it has none. `extra` adds the web's own rows: at the end of the group that
 * has the row `with` (whether the web handles that row or not), else as a group of their own at the end.
 */
export function menuItems(menu: string, handlers: Handlers, extra?: { with?: string; rows: MenuRow[] }[]): MenuItem[] {
  const table = MENUS[menu] ?? [];
  const groups = table.map((g) => g.flatMap((id) => (handlers[id] ? [menuRow(id, handlers[id]!)] : [])));
  for (const e of extra ?? []) {
    const at = e.with ? table.findIndex((g) => g.includes(e.with!)) : -1;
    if (at >= 0) groups[at] = [...groups[at]!, ...e.rows];
    else groups.push(e.rows);
  }
  const out: MenuItem[] = [];
  for (const g of groups) {
    if (!g.length) continue;
    if (out.length) out.push("separator");
    out.push(...g);
  }
  return out;
}

/** The rows as the EQ graph's popup menu takes them (screens/eq/Menu.tsx). */
export function popupItems(items: MenuItem[]): ({ label: string; disabled?: boolean; checked?: boolean; danger?: boolean; onSelect: () => void } | { heading: string } | "separator")[] {
  return items.map((m) => (m === "separator" || "heading" in m ? m : { label: m.label, disabled: m.disabled, checked: m.checked, danger: m.danger, onSelect: m.run }));
}

/** Whether `id` is a row of actions.json (a handler for anything else is a mistake). */
export function isAction(id: string): boolean {
  return byId.has(id);
}
