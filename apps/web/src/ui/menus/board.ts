// -- WP WEBM4 -- What the console can do now, for the app menu and the command palette: the page's
// own handlers (open a DSPi, About, the arrangement) and those of the device on show, which its view
// sets as it renders (`setSession`) and takes back as it goes. A device tab not on show has none: its
// view is not mounted (WEBM1), so its rows are those of the page alone.

import type { Searchable } from "../palette/search";
import type { Handlers } from "./actions";

/** One entry of the command palette. */
export interface PaletteEntry extends Searchable {
  /** Unique in the palette: an action's id, "page:eq", "channel:3"… */
  id: string;
  kind: "action" | "section" | "page" | "device" | "channel" | "settings";
  /** Where it is, or what it is: "Equaliser", "Settings › Display", "Device · USB". */
  where: string;
  /** Its shortcut, as actions.json writes it. */
  keys: string;
  /** Why it is greyed. */
  reason?: string;
  run: () => void;
}

export interface SessionActions {
  handlers: Handlers;
  /** Its pages, channels and settings pages (the palette's; the rows come from `handlers`). */
  entries: PaletteEntry[];
}

export class ActionBoard {
  /** The page's own. */
  app: Handlers = {};
  private shown: { id: string; actions: SessionActions } | null = null;

  /** The device on show says what it can do (each render). */
  setSession(id: string, actions: SessionActions): void {
    this.shown = { id, actions };
  }

  /** Its view goes: what it said goes with it (only if nothing else said more since). */
  clearSession(id: string): void {
    if (this.shown?.id === id) this.shown = null;
  }

  /** The device on show's, when it is `id` (the page's view of it may lag a render). */
  session(id: string | null): SessionActions | null {
    return this.shown && (id === null || this.shown.id === id) ? this.shown.actions : null;
  }

  /** The device's handler where it has one, else the page's. */
  handlers(id: string | null): Handlers {
    return { ...this.app, ...(this.session(id)?.handlers ?? {}) };
  }
}
