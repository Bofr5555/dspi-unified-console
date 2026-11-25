// -- WP WEB7 -- Library › Presets, as Qt Modern's (LibraryPresets.qml, DL3a/INTC/REVDFIX): every slot
// of the device as a row in the 760 px column (its number, the power icon on the startup slot, the
// name or a quiet "Empty", what it is: Active, with unsaved changes, Startup); Recall on a stored row
// and Save on the active one, with the slot's menu (actions.json's `slot`) beside them, showing on
// hover, focus and the active row; a double click recalls. The page's one tool is Save to Preset N,
// a secondary button (Save to device in the bar stays the view's primary). The session bar's preset
// popover stays; both do the same with the same questions. What is offered is ./model.ts's.

import { useState } from "preact/hooks";
import { EmptyState, Glyph } from "../../ui/design";
import { ActionMenuButton } from "../../ui/menus/ActionMenu";
import { menuItems, type Handler } from "../../ui/menus/actions";
import { useSessionState } from "../../ui/components";
import type { ScreenProps } from "../../ui/screens";
import { PageTools } from "../../ui/shell/Parts";
import { RenameDialog } from "../main/Dialog";
import { PRESET_SLOTS } from "../main/model";
import { recall } from "../scenebar/model";
import { copyTargets, knownActive, slotActions, slotLabel, slotRows, startupWords } from "./model";
import "../snapshots/rows.css";
import "./presets.css";

export function PresetsScreen(props: ScreenProps) {
  const { session, ask, unsaved } = props;
  const st = useSessionState(session);
  const [busy, setBusy] = useState(false);
  const [renaming, setRenaming] = useState<number | null>(null);
  const s = st.snapshot?.state;
  if (!s || session.document)
    return (
      <section class="lib-page presets-screen" aria-label="Presets">
        <EmptyState icon="unplug" title="No Device Connected" text="Presets live on the device. Open one to see its slots." />
      </section>
    );
  const ro = session.readOnlyReason;
  const dirty = st.unsaved.length > 0 && !ro;
  const active = knownActive(s);
  const opts = { ro: !!ro, free: !busy, dirty };
  const verbs = { session, ask, unsaved };

  /** One action at a time; the page's controls are off meanwhile. */
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

  const save = () => (active === null ? Promise.resolve(false) : run(() => session.savePreset(active)));

  /**
   * The active slot's settings into `dest` (REVE B1: the core's copy saves the LIVE settings into
   * both slots, so the source is always the active one). Unsaved changes are asked about first.
   */
  const copyHere = (dest: number) =>
    run(async () => {
      const source = knownActive(s);
      if (source === null || dest === source || ro) return;
      const a = await unsaved();
      if (a === "cancel") return;
      if (a === "save" && !(await session.savePreset(source))) return;
      if (a === "discard" && !(await session.change("presetRevert", undefined))) return;
      // The destination keeps its name; one without gets the Console's default.
      if (!(s.presetNames[dest] ?? "").trim() && !(await session.write("preset.name", { text: `Preset ${dest + 1}` }, [dest]))) return;
      await session.change("presetCopy", { source, dest, token: await session.token("copy_preset") });
    });
  const askCopyTo = async () => {
    const targets = copyTargets(s);
    if (!targets.length || active === null) return;
    let dest = targets[0]!;
    const a = await ask({
      title: "Copy to…",
      body: (
        <label class="dialog-field">
          Copy the current settings of “{slotLabel(s, active)}” to
          <select name="dest" onChange={(e) => (dest = Number((e.target as HTMLSelectElement).value))}>
            {targets.map((i) => (
              <option key={i} value={i}>
                {slotLabel(s, i)}
              </option>
            ))}
          </select>
        </label>
      ),
      buttons: [
        { label: "Copy", value: "copy" },
        { label: "Cancel", value: "cancel" },
      ],
    });
    if (a === "copy") await copyHere(dest);
  };
  const clear = async (slot: number) => {
    if (await confirm("Clear Preset?", `Clear "${slotLabel(s, slot)}" and restore factory defaults? This cannot be undone.`, "Clear", true))
      await run(() => session.writeConfirmed("delete_preset", "preset.delete", "trigger", [slot]));
  };
  const clearAll = async () => {
    if (await confirm("Clear All Presets?", "This will erase all preset data and names, restoring every slot to factory defaults. This cannot be undone.", "Clear All", true))
      await run(async () => {
        for (let i = 0; i < PRESET_SLOTS; i++) {
          if ((session.state.snapshot?.state.presetOccupied ?? 0) & (1 << i)) {
            if (!(await session.writeConfirmed("delete_preset", "preset.delete", "trigger", [i]))) return;
          }
        }
      });
  };

  /** The slot's menu: actions.json's `slot` rows, as Qt's ContextMenus.slot offers them. */
  const slotMenu = (slot: number) => {
    const a = slotActions(s, slot, opts);
    const h = (o: { shown: boolean; enabled: boolean }, f: () => unknown, more: Partial<Handler> = {}): Handler | undefined =>
      o.shown ? { run: () => void f(), enabled: o.enabled, reason: ro || undefined, ...more } : undefined;
    const handlers: Record<string, Handler | undefined> = {
      "slot.recall": h({ shown: true, enabled: a.recall.shown && a.recall.enabled }, () => run(() => recall(verbs, slot))),
      "slot.save": h(a.saveHere, () => (a.saveHere.copy ? copyHere(slot) : save()), { text: a.saveHere.text }),
      "slot.rename": h(a.rename, () => setRenaming(slot)),
      "slot.startup": h(a.startup, () => run(() => session.write("preset.startup", { bytes: [0, slot] }))),
      "slot.copyTo": h(a.copyTo, askCopyTo),
      "slot.clear": h(a.clear, () => clear(slot), { text: a.clear.text }),
      "slot.clearAll": h(a.clearAll, clearAll),
    };
    return menuItems("slot", Object.fromEntries(Object.entries(handlers).filter(([, v]) => v)) as Record<string, Handler>);
  };

  const rows = slotRows(s, dirty);
  return (
    <section class="lib-page presets-screen" aria-label="Presets">
      {active !== null ? (
        <PageTools label="Presets tools">
          <button type="button" class="tool-icon-button" disabled={!!ro || busy} title={ro || "Save the current settings in this slot"} onClick={() => void save()}>
            <Glyph name="save" />
            Save to Preset {active + 1}
          </button>
        </PageTools>
      ) : null}
      <p class="lib-caption">{startupWords(s)}</p>
      <ul class="lib-rows" aria-label="Preset slots" aria-busy={busy}>
        {rows.map((r) => {
          const a = slotActions(s, r.slot, opts);
          const label = `${r.slot + 1}: ${r.title}`;
          return (
            <li
              key={r.slot}
              class={`lib-row${r.active ? " selected" : ""}`}
              aria-label={r.note ? `${label}, ${r.note}` : label}
              aria-current={r.active ? "true" : undefined}
              onDblClick={() => a.recall.shown && a.recall.enabled && void run(() => recall(verbs, r.slot))}
            >
              <span class="lib-num">{r.slot + 1}</span>
              <span class="lib-lead">{r.startup ? <Glyph name="power" /> : null}</span>
              <span class="lib-text">
                <span class={`lib-title${r.muted ? " muted" : ""}`}>{r.title}</span>
                {r.note ? <span class="lib-sub">{r.note}</span> : null}
              </span>
              <span class="lib-actions">
                {a.recall.shown ? (
                  <button type="button" aria-label={`Recall ${label}`} disabled={!a.recall.enabled} title={ro || undefined} onClick={() => void run(() => recall(verbs, r.slot))}>
                    Recall
                  </button>
                ) : null}
                {a.save.shown ? (
                  <button type="button" disabled={!a.save.enabled} title={ro || "Save the current settings in this slot"} onClick={() => void save()}>
                    Save
                  </button>
                ) : null}
                <ActionMenuButton label={`More, ${label}`} items={() => slotMenu(r.slot)}>
                  <span class="visually-hidden">More, {label}</span>
                </ActionMenuButton>
              </span>
            </li>
          );
        })}
      </ul>
      {renaming !== null ? (
        <RenameDialog
          title="Rename Preset"
          label="Name"
          initial={(s.presetNames[renaming] ?? "").trim() || `Preset ${renaming + 1}`}
          onCancel={() => setRenaming(null)}
          onSave={(name) => {
            const slot = renaming;
            setRenaming(null);
            if (name) void run(() => session.write("preset.name", { text: name }, [slot]));
          }}
        />
      ) : null}
    </section>
  );
}
