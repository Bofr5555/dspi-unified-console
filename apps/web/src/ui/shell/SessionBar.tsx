// -- WP WEBM1 -- The actions of one device, above its pages: the presets (INTD: DL5's popover), Undo and Redo, Save
// to device with what belongs to it, and the installer's lock. Every action is the core's and the
// one other places already run (the scene bar's recall, the keys' Commit and Revert), with the
// same questions.

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
import { saveMasterVolume, saveOutputConfig } from "../../screens/files/model"; // -- WP WEB7 --

export function SessionBar(props: {
  session: Session;
  state: SessionState;
  desk: SnapshotDesk;
  ask: Ask;
  unsaved: () => Promise<Unsaved>;
  /** The locked-user state: what the front page allows (volume, mute, presets), and its own Unlock. */
  locked: boolean;
  onUnlock: () => void;
}) {
  const { session, state, desk, ask, unsaved } = props;
  const [busy, setBusy] = useState(false);
  const s = state.snapshot?.state;
  if (!s) return null;
  const off = !!session.readOnlyReason || busy;
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
  return (
    <div class="session-actions" role="group" aria-label="Device actions">
      {/* -- WP INTD -- the presets as one popover (DL5), in place of the slot picker */}
      <PresetPopover session={session} state={state} ask={ask} unsaved={unsaved} disabled={busy} />
      <UndoButtons session={session} state={state} />
      {props.locked ? null : (
        <span class="save-split">
          <button type="button" class="bar-save" disabled={off} title={`Save to the active preset (${mod}S)`} onClick={() => key("commit")}>
            <SaveIcon />
            <span class="bar-save-text">Save to device</span>
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
  );
}
