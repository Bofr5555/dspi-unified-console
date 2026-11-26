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
import { AbSegment } from "./AbSegment"; // -- WP UB34 --
import { UndoButtons, appleKeys } from "../undo";
import { runKey, type Ask, type Unsaved } from "../screens";
import type { Session, SessionState } from "../session";
import { LockIcon, SaveIcon } from "./icons";
import { saveMasterVolume, saveOutputConfig } from "../../screens/files/model"; // -- WP WEB7 --

// -- WP UWN -- (UXP A15)
/** Save to Device's accessible name: the count in words. */
export function saveName(n: number): string {
  return n ? `Save to Device, ${n} unsaved ${n === 1 ? "change" : "changes"}` : "Save to Device";
}

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
  /** The locked-user state: what the front page allows (volume, mute, the source, presets), and its own Unlock. */
  locked: boolean;
  onUnlock: () => void;
}): { left: ComponentChildren; right: ComponentChildren } | undefined {
  const { session, state, desk, ask, unsaved } = props;
  const [busy, setBusy] = useState(false);
  const s = state.snapshot?.state;
  if (!s) return undefined;
  const off = !!session.readOnlyReason || busy;
  // -- WP UWN -- (UXP A16) while the link is away nothing is written: Save and its menu wait for it
  const away = !!state.link && state.link.phase !== "bound";
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } finally {
      setBusy(false);
    }
  };
  // -- WP WEB7 -- a store to the device, then what it did in a notice (Qt's NoticeDialog)
  const saveTold = async (f: (d: SnapshotDesk) => Promise<boolean>) => {
    await f(desk);
    const o = desk.state.outcome;
    if (!o) return;
    desk.dismiss();
    await ask({ title: o.title, body: <p>{o.lines.join(" ")}</p>, buttons: [{ label: "OK", value: "ok" }] });
  };
  const key = (action: "commit" | "revert") => void run(() => runKey({ action }, session, desk, ask, unsaved));
  const factoryReset = () => void run(() => askFactoryReset(session, ask)); // -- WP WEBM4 -- the question is session-actions' now
  const n = state.unsaved.length;
  const mod = appleKeys(navigator.platform ?? "") ? "⌘" : "Ctrl+";
  return {
    // -- WP INTD -- the presets as one popover (DL5), in place of the slot picker; -- WP REVEFIX -- (REVE C3) a locked user's Recall only
    // -- WP UB34 -- (UXP B3) then A | B: as saved against now from the first edit
    left: (
      <>
        <PresetPopover session={session} state={state} ask={ask} unsaved={unsaved} disabled={busy} locked={props.locked} />
        <AbSegment desk={desk} unsaved={n} locked={props.locked} />
      </>
    ),
    right: (
      <div class="session-actions" role="group" aria-label="Device actions">
        {/* -- WP UB2 -- (UXP B2) the locked user's undo would be refused: not offered, as Save */}
        {props.locked ? null : <UndoButtons session={session} state={state} />}
        {props.locked ? null : (
          <span class={`save-split${n ? " primary" : ""}`}>
            {/* -- WP UWN -- (UXP A15) one name with the count in words ("Save to Device, 1 unsaved change"), never "Save to Device0" */}
            <button
              type="button"
              class={`bar-save${n ? " primary" : ""}`}
              disabled={off || away}
              aria-label={saveName(n)}
              title={away ? "The DSPi is not connected now" : `Save to the active preset (${mod}S)`}
              onClick={() => key("commit")}
            >
              <SaveIcon />
              <span class="bar-save-text">Save to Device</span>
              <span class={`bar-count${n ? " on" : ""}`} aria-hidden="true">
                {n || 0}
              </span>
            </button>
            <ActionMenuButton
              label="Save options"
              disabled={off || away}
              items={() =>
                menuItems("save", {
                  "device.revert": { run: () => key("revert") },
                  // -- WP WEB7 -- Qt's Save ▾ rows (they were a card on Library › Files); the result as Qt's notice
                  "file.saveMasterVolume": { run: () => void run(() => saveTold(saveMasterVolume)) },
                  "file.saveOutputConfiguration": { run: () => void run(() => saveTold(saveOutputConfig)) },
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
