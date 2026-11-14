// The main window's presets (WP WB6a): the slot picker with the Console's
// unsaved-changes guard (M42), its slot menu (M43: Save, Rename, Set as
// Default, Copy to, Clear, Clear All) and its Tools entries (M44: Commit,
// Revert, Factory Reset). Every action is the core's (`write` by path, or the
// preset calls), and the picker shows the slot the device reports active.

import { memo } from "preact/compat";
import { useState } from "preact/hooks";
import type { Session } from "../../ui/session";
import { RenameDialog } from "./Dialog";
import type { Ask, Unsaved } from "../../ui/screens";
import { Menu } from "./Menu";
import { PRESET_SLOTS, presetLabel, sameItems } from "./model";

export interface PresetView {
  names: string[];
  occupied: number;
  active: number;
  startupMode: number;
  defaultSlot: number;
}

export type { Unsaved } from "../../ui/screens";

export const PresetsCard = memo(
  function PresetsCard(props: {
    session: Session;
    view: PresetView;
    dirty: boolean;
    disabled: boolean;
    ask: Ask;
    unsaved: () => Promise<Unsaved>;
  }) {
    const { session, view, disabled, ask } = props;
    const [busy, setBusy] = useState(false);
    const [renaming, setRenaming] = useState(false);
    const [note, setNote] = useState("");
    const s = { presetNames: view.names, presetOccupied: view.occupied };
    const label = (slot: number) => presetLabel(s, slot);
    const active = view.active;
    const anyOccupied = view.occupied !== 0;
    const activeOccupied = (view.occupied & (1 << active)) !== 0;

    /** One action at a time; the picker and the menu are off meanwhile. */
    const run = async (f: () => Promise<unknown>) => {
      setBusy(true);
      setNote("");
      try {
        await f();
      } finally {
        setBusy(false);
      }
    };
    // -- WP WB6b -- (c6p WB6a-3) every Save names an unnamed slot "Preset N" first
    const save = (slot: number) => session.savePreset(slot);
    const load = (slot: number) => session.write("preset.load", "trigger", [slot]);

    const switchTo = (slot: number) =>
      run(async () => {
        const a = await props.unsaved();
        if (a === "cancel") return;
        if (a === "save" && !(await save(active))) return;
        await load(slot);
      });

    const copyTo = (dest: number) =>
      run(async () => {
        const a = await props.unsaved();
        if (a === "cancel") return;
        if (a === "save" && !(await save(active))) return;
        if (a === "discard" && !(await session.change("presetRevert", undefined))) return;
        // The destination keeps its name; one without gets the Console's default.
        if (!(view.names[dest] ?? "").trim() && !(await session.write("preset.name", { text: `Preset ${dest + 1}` }, [dest]))) return;
        await session.change("presetCopy", { source: active, dest, token: await session.token("copy_preset") }); // -- WP P1WB2 -- with its token
      });

    const askCopy = async () => {
      let dest = active === 0 ? 1 : 0;
      const a = await ask({
        title: "Copy to…",
        body: (
          <label class="dialog-field">
            Copy the current settings to
            <select
              name="dest"
              onChange={(e) => {
                dest = Number((e.target as HTMLSelectElement).value);
              }}
            >
              {Array.from({ length: PRESET_SLOTS }, (_, i) => i)
                .filter((i) => i !== active)
                .map((i) => (
                  <option key={i} value={i}>
                    {label(i)}
                  </option>
                ))}
            </select>
          </label>
        ),
        buttons: [{ label: "Copy", value: "copy" }, { label: "Cancel", value: "cancel" }],
      });
      if (a === "copy") await copyTo(dest);
    };

    const confirm = async (title: string, text: string, yes: string, danger = false) =>
      (await ask({ title, body: <p>{text}</p>, buttons: [{ label: yes, value: "yes", danger }, { label: "Cancel", value: "cancel" }] })) ===
      "yes";

    const entries = [
      { label: "Save", onSelect: () => void run(() => save(active)) },
      { label: "Rename…", onSelect: () => setRenaming(true) },
      {
        label: "Set as Default",
        disabled: view.startupMode === 0 && view.defaultSlot === active,
        onSelect: () => void run(() => session.write("preset.startup", { bytes: [0, active] })),
      },
      { label: "Copy to…", onSelect: () => void askCopy() },
      ...(activeOccupied
        ? [
            "separator" as const,
            {
              label: `Clear "${label(active)}"…`,
              danger: true,
              onSelect: () =>
                void (async () => {
                  if (await confirm("Clear Preset?", `Clear "${label(active)}" and restore factory defaults? This cannot be undone.`, "Clear", true))
                    await run(() => session.writeConfirmed("delete_preset", "preset.delete", "trigger", [active])); // -- WP P1WB2 --
                })(),
            },
          ]
        : []),
      ...(anyOccupied
        ? [
            "separator" as const,
            {
              label: "Clear All Slots…",
              danger: true,
              onSelect: () =>
                void (async () => {
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
                })(),
            },
          ]
        : []),
      "separator" as const,
      {
        label: "Commit Parameters…",
        onSelect: () =>
          void (async () => {
            if (await confirm("Save Preset", `Save current parameters to preset slot ${active + 1}?`, "Save"))
              await run(async () => {
                if (await save(active)) setNote("Preset saved successfully");
              });
          })(),
      },
      {
        label: "Revert to Saved…",
        onSelect: () =>
          void (async () => {
            if (await confirm("Revert to Saved", "Revert to last saved parameters? Current unsaved changes will be lost.", "Revert"))
              await run(() => session.change("presetRevert", undefined));
          })(),
      },
      {
        label: "Factory Reset…",
        danger: true,
        onSelect: () =>
          void (async () => {
            if (
              await confirm(
                "Factory Reset",
                "Do you wish to clear all active parameters? This will not overwrite your saved parameters unless you run 'Commit Parameters'.",
                "Reset",
                true,
              )
            )
              await run(async () => session.change("factoryReset", { token: await session.token("factory_reset") })); // -- WP P1WB2 --
          })(),
      },
    ];

    return (
      <section class="card presets-card" aria-label="Presets" aria-busy={busy}>
        <div class="card-head">
          <label class="preset-pick">
            Preset
            <select
              value={String(active)}
              disabled={disabled || busy}
              onChange={(e) => {
                const el = e.target as HTMLSelectElement;
                const slot = Number(el.value);
                // The picker shows what the device has active; the load moves it.
                el.value = String(active);
                if (slot !== active) void switchTo(slot);
              }}
            >
              {Array.from({ length: PRESET_SLOTS }, (_, i) => (
                <option key={i} value={i}>
                  {label(i)}
                </option>
              ))}
            </select>
          </label>
          <Menu label="Preset actions" disabled={disabled || busy} entries={entries} />
        </div>
        <p class="preset-status" role="status">
          {props.dirty ? <span class="badge unsaved">Unsaved changes</span> : null}
          {note ? <span class="note">{note}</span> : null}
        </p>
        {renaming ? (
          <RenameDialog
            title="Rename Preset"
            label="Name"
            initial={(view.names[active] ?? "").trim() || `Preset ${active + 1}`}
            onCancel={() => setRenaming(false)}
            onSave={(name) => {
              setRenaming(false);
              void run(() => session.write("preset.name", { text: name }, [active]));
            }}
          />
        ) : null}
      </section>
    );
  },
  (a, b) =>
    a.session === b.session &&
    a.dirty === b.dirty &&
    a.disabled === b.disabled &&
    a.ask === b.ask &&
    a.unsaved === b.unsaved &&
    a.view.active === b.view.active &&
    a.view.occupied === b.view.occupied &&
    a.view.startupMode === b.view.startupMode &&
    a.view.defaultSlot === b.view.defaultSlot &&
    sameItems(a.view.names, b.view.names),
);
