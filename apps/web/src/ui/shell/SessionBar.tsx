// -- WP WEBM1 -- The actions of one device, above its pages: the preset picker, Undo and Redo, Save
// to device with what belongs to it, and the installer's lock. Every action is the core's and the
// one other places already run (the scene bar's recall, the keys' Commit and Revert), with the
// same questions.

import { useState } from "preact/hooks";
import type { SnapshotDesk } from "../../screens/snapshots/model";
import { Menu } from "../../screens/main/Menu";
import { PRESET_SLOTS, presetLabel } from "../../screens/main/model";
import { recall } from "../../screens/scenebar/model";
import { UndoButtons, appleKeys } from "../undo";
import { runKey, type Ask, type Unsaved } from "../screens";
import type { Session, SessionState } from "../session";
import { LockIcon, SaveIcon } from "./icons";

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
  const key = (action: "commit" | "revert") => void run(() => runKey({ action }, session, desk, ask, unsaved));
  const factoryReset = () =>
    void (async () => {
      const a = await ask({
        title: "Factory Reset",
        body: <p>Do you wish to clear all active parameters? This will not overwrite your saved parameters unless you run 'Commit Parameters'.</p>,
        buttons: [
          { label: "Reset", value: "yes", danger: true },
          { label: "Cancel", value: "cancel" },
        ],
      });
      if (a === "yes") await run(async () => session.change("factoryReset", { token: await session.token("factory_reset") }));
    })();
  const n = state.unsaved.length;
  const mod = appleKeys(navigator.platform ?? "") ? "⌘" : "Ctrl+";
  return (
    <div class="session-actions" role="group" aria-label="Device actions">
      <select
        class="bar-preset"
        aria-label="Recall preset"
        title={state.unsaved.length ? "Preset (unsaved changes)" : "Preset"}
        value={String(s.activePresetSlot)}
        disabled={off}
        onChange={(e) => {
          const el = e.target as HTMLSelectElement;
          const slot = Number(el.value);
          // The picker shows what the device has active; the load moves it.
          el.value = String(s.activePresetSlot);
          void run(() => recall({ session, ask, unsaved }, slot));
        }}
      >
        {Array.from({ length: PRESET_SLOTS }, (_, i) => (
          <option key={i} value={i}>
            {presetLabel(s, i)}
          </option>
        ))}
      </select>
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
          <Menu
            label="Save options"
            disabled={off}
            entries={[
              { label: "Revert to Saved…", onSelect: () => key("revert") },
              "separator",
              { label: "Factory Reset…", danger: true, onSelect: factoryReset },
            ]}
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
