// -- WP WEBM4 -- The command palette's entries, made as it opens: the other open devices, the rows
// of actions.json the console has a handler for now (the device's or the page's), the device's
// sections, pages, channels and Settings pages, and the page's own (`extra`). Nothing is listed here
// by hand but what the web has besides actions.json (the shortcut list).

import { ACTIONS, actionDef, type Handlers } from "../menus/actions";
import type { PaletteEntry, SessionActions } from "../menus/board";
import { actionEntry } from "../menus/session-actions";
import type { Session } from "../session";
import { SECTIONS } from "../shell/sections";

/** Where a row is: the section of the page it opens (actions.json's `page.<section>/…`), else its Classic menu. */
export function rowWhere(id: string): string {
  const d = actionDef(id);
  const page = d?.places.find((p) => p.startsWith("page."));
  const section = page ? SECTIONS.find((s) => s.id === page.slice(5).split("/")[0]) : undefined;
  return section?.title ?? d?.menu ?? "";
}

export function paletteEntries(args: {
  /** The device on show, and what it can do. */
  current: Session | null;
  session: SessionActions | null;
  /** The device's handlers over the page's. */
  handlers: Handlers;
  /** The other open devices, each with how to show it. */
  others: readonly { session: Session; name: string; where: string; show: () => void }[];
  extra: readonly PaletteEntry[];
}): PaletteEntry[] {
  const out: PaletteEntry[] = [];
  for (const o of args.others)
    out.push({ id: `device:${o.session.id}`, kind: "device", title: `Switch to ${o.name}`, where: o.where, keys: "", enabled: true, run: o.show });
  for (const e of args.session?.entries ?? []) out.push(e);
  for (const d of ACTIONS) {
    const e = actionEntry(d.id, args.handlers[d.id], args.current, rowWhere);
    if (e) out.push(e);
  }
  out.push(...args.extra);
  return out;
}
