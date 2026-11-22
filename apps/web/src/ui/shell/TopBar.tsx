// -- WP WEBM1 -- The page's top row: the labelled app menu (the rare, page-wide items; every other
// action is on what it acts on), the open devices as tabs, and who is signed in.

import { Menu, type MenuEntry } from "../../screens/main/Menu";
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
  /** The shortcut list, while a device's screens are on show; null otherwise. */
  onShortcuts: (() => void) | null;
  onAbout: () => void;
}) {
  const entries: MenuEntry[] = [
    { label: "Open a DSPi…", onSelect: props.onAdd },
    ...(props.onShortcuts ? [{ label: "Keyboard Shortcuts", onSelect: props.onShortcuts }] : []),
    "separator",
    { label: "About", onSelect: props.onAbout },
  ];
  return (
    <header class="top">
      <h1 class="visually-hidden">{props.title}</h1>
      <div class="app-menu">
        {/* The app's own name in short, as on the desktop; a host page's name as it gave it */}
        <Menu label={props.title === "uConsole for DSPi" ? "uConsole" : props.title} entries={entries} />
      </div>
      <DeviceTabs
        sessions={props.sessions}
        current={props.current}
        picking={props.picking}
        onChoose={props.onChoose}
        onClose={props.onClose}
        onAdd={props.onAdd}
      />
      {props.user ? <span class="user">{props.user}</span> : null}
    </header>
  );
}
