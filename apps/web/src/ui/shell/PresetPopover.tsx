// -- WP INTD -- The session bar's preset control as one popover, the web's DL5 (Qt's
// modern/PresetPopover.qml): the button names the active slot (a dot while it has unsaved
// changes); the popover lists every slot of the device as a row (number, name or "Empty", what it
// is: Active, with unsaved changes, Startup) with its actions: Recall on a stored slot, Save on the
// active one, Copy Here on an empty one, and the slot's menu (actions.json's `slot`: Recall, Save
// Current Settings Here, Rename…, Set as Startup Preset, Copy to…, Clear…, Clear All Slots…).
// It takes the place of the Overview's preset card and of the slot strip above Overview and
// Library. Every action is the core's, with the questions the card and the strip asked.

import { useState } from "preact/hooks";
import { RenameDialog } from "../../screens/main/Dialog";
import { PRESET_SLOTS, occupied, presetLabel, presetName } from "../../screens/main/model";
import { recall, store } from "../../screens/scenebar/model";
import { ActionMenuButton } from "../menus/ActionMenu";
import { menuItems } from "../menus/actions";
import { Popover } from "../Popover";
import type { Ask, Unsaved } from "../screens";
import type { Session, SessionState } from "../session";

export function PresetPopover(props: { session: Session; state: SessionState; ask: Ask; unsaved: () => Promise<Unsaved>; disabled: boolean }) {
  const { session, state, ask, unsaved } = props;
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<number | null>(null);
  const s = state.snapshot?.state;
  if (!s) return null;
  const ro = session.readOnlyReason;
  const active = s.activePresetSlot;
  const dirty = state.unsaved.length > 0 && !ro;
  const off = !!ro || busy || props.disabled;
  const startupSlot = s.presetStartupMode === 0 ? s.presetDefaultSlot : -1;
  const verbs = { session, ask, unsaved };

  /** One action at a time; the popover's controls are off meanwhile. */
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await f();
    } finally {
      setBusy(false);
    }
  };
  const confirm = async (title: string, text: string, yes: string, danger = false) =>
    (await ask({ title, body: <p>{text}</p>, buttons: [{ label: yes, value: "yes", danger }, { label: "Cancel", value: "cancel" }] })) === "yes";

  /** Copy slot `source` into `dest`; from the active slot with unsaved changes, asked first (Save, Discard, Cancel). */
  const copy = (source: number, dest: number) =>
    run(async () => {
      if (source === active) {
        const a = await unsaved();
        if (a === "cancel") return;
        if (a === "save" && !(await session.savePreset(active))) return;
        if (a === "discard" && !(await session.change("presetRevert", undefined))) return;
      }
      // The destination keeps its name; one without gets the Console's default.
      if (!(s.presetNames[dest] ?? "").trim() && !(await session.write("preset.name", { text: `Preset ${dest + 1}` }, [dest]))) return;
      await session.change("presetCopy", { source, dest, token: await session.token("copy_preset") }); // -- WP P1WB2 -- with its token
    });
  const askCopy = async (source: number) => {
    let dest = source === 0 ? 1 : 0;
    const a = await ask({
      title: "Copy to…",
      body: (
        <label class="dialog-field">
          Copy “{presetName(s, source)}” to
          <select
            name="dest"
            onChange={(e) => {
              dest = Number((e.target as HTMLSelectElement).value);
            }}
          >
            {Array.from({ length: PRESET_SLOTS }, (_, i) => i)
              .filter((i) => i !== source)
              .map((i) => (
                <option key={i} value={i}>
                  {presetLabel(s, i)}
                </option>
              ))}
          </select>
        </label>
      ),
      buttons: [{ label: "Copy", value: "copy" }, { label: "Cancel", value: "cancel" }],
    });
    if (a === "copy") await copy(source, dest);
  };
  const clear = async (slot: number) => {
    if (await confirm("Clear Preset?", `Clear "${presetLabel(s, slot)}" and restore factory defaults? This cannot be undone.`, "Clear", true))
      await run(() => session.writeConfirmed("delete_preset", "preset.delete", "trigger", [slot])); // -- WP P1WB2 --
  };
  const clearAll = async () => {
    if (
      await confirm(
        "Clear All Presets?",
        "This will erase all preset data and names, restoring every slot to factory defaults. This cannot be undone.",
        "Clear All",
        true,
      )
    )
      await run(async () => {
        for (let i = 0; i < PRESET_SLOTS; i++) {
          if ((session.state.snapshot?.state.presetOccupied ?? 0) & (1 << i)) {
            if (!(await session.writeConfirmed("delete_preset", "preset.delete", "trigger", [i]))) return; // -- WP P1WB2 --
          }
        }
      });
  };

  /** The slot's menu (actions.json's `slot`); each row closes the popover first, as Qt's does. */
  const slotMenu = (slot: number, close: (refocus?: boolean) => void) => {
    const stored = occupied(s, slot);
    const w = (f: () => unknown, enabled = true) => ({
      run: () => {
        close(false);
        void f();
      },
      enabled: !off && enabled,
      reason: ro || undefined,
    });
    return menuItems("slot", {
      "slot.recall": w(() => run(() => recall(verbs, slot)), stored && (slot !== active || dirty)),
      "slot.save": w(() => run(() => store(verbs, slot))),
      "slot.rename": w(() => setRenaming(slot), stored),
      "slot.startup": w(() => run(() => session.write("preset.startup", { bytes: [0, slot] })), stored && startupSlot !== slot),
      "slot.copyTo": w(() => askCopy(slot), stored),
      "slot.clear": { ...w(() => clear(slot), stored), danger: true },
      "slot.clearAll": { ...w(() => clearAll(), s.presetOccupied !== 0), danger: true },
    });
  };

  const activeText = presetLabel(s, active);
  return (
    <>
      <Popover
        label="Presets"
        class="preset-control"
        buttonClass="bar-preset"
        buttonLabel={`Presets: ${activeText}${dirty ? ", unsaved changes" : ""}`}
        title={dirty ? "Preset (unsaved changes)" : "Preset"}
        button={
          <>
            <span class="bar-preset-text">{activeText}</span>
            <span class={`bar-preset-dot${dirty ? " on" : ""}`} aria-hidden="true" />
            <span class="bar-preset-chevron" aria-hidden="true">
              ▾
            </span>
          </>
        }
      >
        {(close) => (
          <>
            <ul class="preset-rows" aria-label="Preset slots" aria-busy={busy}>
              {Array.from({ length: PRESET_SLOTS }, (_, slot) => {
                const stored = occupied(s, slot);
                const isActive = slot === active;
                const empty = !stored && !isActive;
                const name = empty ? "Empty" : presetName(s, slot);
                const note = [isActive ? (dirty ? "Active, with unsaved changes" : "Active") : "", slot === startupSlot ? "Startup" : ""]
                  .filter(Boolean)
                  .join(" · ");
                const label = `${slot + 1}: ${name}`;
                return (
                  <li key={slot} class={`preset-row${isActive ? " active" : ""}${empty ? " empty" : ""}`} aria-label={note ? `${label}, ${note}` : label} aria-current={isActive ? "true" : undefined}>
                    <span class="pr-n">{slot + 1}</span>
                    <span class="pr-text">
                      <span class="pr-name">{name}</span>
                      {note ? <span class="pr-note">{note}</span> : null}
                    </span>
                    {isActive ? (
                      <button
                        type="button"
                        class={dirty ? "primary" : undefined}
                        disabled={off}
                        title={ro || "Save the current settings in this slot"}
                        onClick={() => {
                          close(false);
                          void run(() => session.savePreset(slot));
                        }}
                      >
                        Save
                      </button>
                    ) : stored ? (
                      <button
                        type="button"
                        aria-label={`Recall ${label}`}
                        disabled={off}
                        title={ro || undefined}
                        onClick={() => {
                          close(false);
                          void run(() => recall(verbs, slot));
                        }}
                      >
                        Recall
                      </button>
                    ) : (
                      <button
                        type="button"
                        disabled={off}
                        title={ro || `Copy “${presetLabel(s, active)}” into this slot`}
                        onClick={() => {
                          close(false);
                          void copy(active, slot);
                        }}
                      >
                        Copy Here
                      </button>
                    )}
                    <ActionMenuButton label={`More, ${label}`} items={() => slotMenu(slot, close)}>
                      <span class="visually-hidden">More</span>
                    </ActionMenuButton>
                  </li>
                );
              })}
            </ul>
            <p class="preset-startup">
              {startupSlot >= 0 ? `At power-on the device loads preset ${startupSlot + 1}.` : "At power-on the device loads the preset it was last on."}
            </p>
          </>
        )}
      </Popover>
      {renaming !== null ? (
        <RenameDialog
          title="Rename Preset"
          label="Name"
          initial={(s.presetNames[renaming] ?? "").trim() || `Preset ${renaming + 1}`}
          onCancel={() => setRenaming(null)}
          onSave={(name) => {
            const slot = renaming;
            setRenaming(null);
            void run(() => session.write("preset.name", { text: name }, [slot]));
          }}
        />
      ) : null}
    </>
  );
}
