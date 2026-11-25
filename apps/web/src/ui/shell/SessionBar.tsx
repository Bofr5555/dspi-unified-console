// -- WP WEBM1 -- The actions of one device, above its pages: the presets (INTD: DL5's popover), Undo and Redo, Save
// to Device with what belongs to it, and the installer's lock. -- WP REVEFIX -- in the top bar, as Qt's. Every action is the core's and the
// one other places already run (the scene bar's recall, the keys' Commit and Revert), with the
// same questions.

import type { ComponentChildren } from "preact";
import { useState } from "preact/hooks";
import type { SnapshotDesk } from "../../screens/snapshots/model";
// -- WP WEBM4 -- Save ▾'s rows are actions.json's `save` menu
import { ActionMenuButton } from "../menus/ActionMenu";
import { menuItems } from "../menus/actions";
import { factoryReset as askFactoryReset } from "../menus/session-actions";
import { PresetPopover } from "./PresetPopover"; // -- WP INTD --
import { UndoButtons, appleKeys } from "../undo";
import { runKey, type Ask, type Unsaved } from "../screens";
import type { Session, SessionState } from "../session";
import { LockIcon, SaveIcon } from "./icons";

/**
 * -- WP REVEFIX -- (REVE §3.1, Qt's TopBar) The device's parts of the one top bar: `left` beside its tab (the presets),
 * `right` before the bell (Undo, Redo, Save to Device ▾ and the lock). Save to Device is the primary only while
 * something is unsaved (Qt's `primary: unsaved`, REVE D1).
 */
export function useSessionBar(props: {
  session: Session;
  state: SessionState;
  desk: SnapshotDesk;
  ask: Ask;
  unsaved: () => Promise<Unsaved>;
  /** The locked-user state: what the front page allows (volume, mute, presets), and its own Unlock. */
  locked: boolean;
  onUnlock: () => void;
}): { left: ComponentChildren; right: ComponentChildren } | undefined {
  const { session, state, desk, ask, unsaved } = props;
  const [busy, setBusy] = useState(false);
  const s = state.snapshot?.state;
  if (!s) return undefined;
  const off = !!session.readOnlyReason || busy;
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } finally {
      setBusy(false);
    }
  };
  const key = (action: "commit" | "revert") => void run(() => runKey({ action }, session, desk, ask, unsaved));
  const factoryReset = () => void run(() => askFactoryReset(session, ask)); // -- WP WEBM4 -- the question is session-actions' now
  const n = state.unsaved.length;
  const mod = appleKeys(navigator.platform ?? "") ? "⌘" : "Ctrl+";
  return {
    // -- WP INTD -- the presets as one popover (DL5), in place of the slot picker; -- WP REVEFIX -- (REVE C3) a locked user's Recall only
    left: <PresetPopover session={session} state={state} ask={ask} unsaved={unsaved} disabled={busy} locked={props.locked} />,
    right: (
      <div class="session-actions" role="group" aria-label="Device actions">
        <UndoButtons session={session} state={state} />
        {props.locked ? null : (
          <span class={`save-split${n ? " primary" : ""}`}>
            <button type="button" class={`bar-save${n ? " primary" : ""}`} disabled={off} title={`Save to the active preset (${mod}S)`} onClick={() => key("commit")}>
              <SaveIcon />
              <span class="bar-save-text">Save to Device</span>
              <span class={`bar-count${n ? " on" : ""}`} aria-label={n ? `${n} unsaved` : undefined} aria-hidden={!n}>
                {n || 0}
              </span>
            </button>
            <ActionMenuButton
              label="Save options"
              disabled={off}
              items={() =>
                menuItems("save", {
                  "device.revert": { run: () => key("revert") },
                  "device.factoryReset": { run: factoryReset, danger: true },
                })
              }
            />
          </span>
        )}
        {state.lock?.pinSet && !props.locked ? (
          <button
            type="button"
            class="bar-lock"
            aria-label={state.lock.unlocked ? "Installer lock: open" : "Installer lock: locked"}
            title={state.lock.unlocked ? "Unlocked with the installer's PIN: lock again" : "Locked by the installer: unlock…"}
            onClick={() => (state.lock?.unlocked ? void session.relock() : props.onUnlock())}
          >
            <LockIcon open={state.lock.unlocked} />
          </button>
        ) : null}
      </div>
    ),
  };
}
