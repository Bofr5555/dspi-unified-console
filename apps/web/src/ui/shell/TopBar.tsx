// -- WP WEBM1 -- The page's top row: the labelled app menu (the rare, page-wide items; every other
// action is on what it acts on), the open devices as tabs, and who is signed in.
// -- WP WEBM4 -- The app menu's rows are actions.json's (`appMenu`, made by the page as it opens);
// the search that opens the command palette (Ctrl+K); the bell of the notification drawer, which
// lies under this row. The device tabs are here unless the arrangement puts them in the rail.

import type { ComponentChildren } from "preact";
import { ActionMenuButton } from "../menus/ActionMenu";
import type { MenuItem } from "../menus/actions";
import type { Session } from "../session";
import { DeviceTabs } from "./DeviceTabs";

export function TopBar(props: {
  title: string;
  user: string;
  sessions: Session[];
  current: Session | null;
  picking: boolean;
  onChoose: (s: Session) => void;
  onClose: (s: Session) => void;
  onAdd: () => void;
  // -- WP WEBM4 --
  /** The app menu's rows, as they are now. */
  menu: () => MenuItem[];
  /** A device tab's menu. */
  tabMenu: (s: Session) => MenuItem[];
  /** The device tabs are in the rail. */
  tabsInRail: boolean;
  /** Opens the command palette; its key as this platform writes it. */
  onSearch: () => void;
  searchKey: string;
  /** The notification drawer's bell. */
  bell: ComponentChildren;
}) {
  return (
    <header class="top">
      <h1 class="visually-hidden">{props.title}</h1>
      <div class="app-menu">
        {/* The app's own name in short, as on the desktop; a host page's name as it gave it */}
        <ActionMenuButton label={props.title === "uConsole for DSPi" ? "uConsole" : props.title} items={props.menu} />
      </div>
      {props.tabsInRail ? (
        <span class="top-fill" />
      ) : (
        <DeviceTabs
          sessions={props.sessions}
          current={props.current}
          picking={props.picking}
          onChoose={props.onChoose}
          onClose={props.onClose}
          onAdd={props.onAdd}
          menu={props.tabMenu}
        />
      )}
      {/* -- WP WEBM4 -- */}
      <button type="button" class="top-search" aria-label="Search settings, channels, commands" aria-keyshortcuts={props.searchKey.replace("⌘", "Meta+").replace("Ctrl+", "Control+")} title={`Search settings, channels, commands (${props.searchKey})`} onClick={props.onSearch}>
        <svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
          <circle cx="11" cy="11" r="7" />
          <path d="m20 20-3.5-3.5" />
        </svg>
        <span class="top-search-text">Search…</span>
        <kbd class="top-search-key" aria-hidden="true">
          {props.searchKey}
        </kbd>
      </button>
      {props.bell}
      {props.user ? <span class="user">{props.user}</span> : null}
    </header>
  );
}
