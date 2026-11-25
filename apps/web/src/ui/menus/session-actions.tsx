// -- WP WEBM4 -- What a device's view can do, as actions.json's rows (ui/menus/actions.ts): the
// handlers the app menu, Save ▾ and the command palette run, and the palette's other entries for
// it (its sections and pages, its channels, the Settings pages). Every handler is what a button or
// a key of the view already does, with the same questions: the keys' (`runKey`), the undo buttons',
// a page opened as its rail and tabs open it. A row whose page this device does not show has no
// handler, so it is not offered.

import { inputRows, liveInputs, outputRows, PRESET_SLOTS, presetLabel } from "../../screens/main/model";
import { mayToggle, type SnapshotDesk } from "../../screens/snapshots/model";
import { SHORTCUTS } from "../../screens/shortcuts/table";
import { runKey, type Ask, type Unsaved } from "../screens";
import type { Session } from "../session";
import { CONNECT, type Section } from "../shell/sections";
import { actionDef, actionShortcut, actionText, type Handler, type Handlers } from "./actions";
import type { PaletteEntry } from "./board";

export interface SessionContext {
  session: Session;
  desk: SnapshotDesk;
  ask: Ask;
  unsaved: () => Promise<Unsaved>;
  /** The screens the device shows now. */
  shown: ReadonlySet<string>;
  /** Show a screen, at an effect or a channel (`ChainTarget`'s fields). */
  open: (screen: string, at?: Record<string, unknown>) => void;
  sections: readonly Section[];
  /** The locked-user front page: presets, mute and volume only. */
  locked: boolean;
}

/** The rows that open a page of this device, by row: screens/shortcuts/table.ts's, and those Modern has a page for. */
const PAGE_ROWS: Record<string, { screen: string; at?: Record<string, unknown> }> = {
  ...Object.fromEntries(SHORTCUTS.filter((r) => r.open).map((r) => [r.id, r.open!])),
  "edit.history": { screen: "history" },
  "device.snapshots": { screen: "snapshots" },
  "device.events": { screen: "events" },
  "device.firmwareUpdate": { screen: "device" },
  "tools.measure": { screen: "measure" },
  "window.system": { screen: "system" },
  "help.saveDiagnostics": { screen: "events" },
  "file.newFromTemplate": { screen: "files" },
  "file.saveFile": { screen: "files" },
  "file.saveAsTemplate": { screen: "files" },
  "file.importConfiguration": { screen: "files" },
  "file.exportConfiguration": { screen: "files" },
};

/** Factory Reset…, as Save ▾ has asked it (WEBM1): the live parameters only. */
export async function factoryReset(session: Session, ask: Ask): Promise<void> {
  const a = await ask({
    title: "Factory Reset",
    body: <p>Do you wish to clear all active parameters? This will not overwrite your saved parameters unless you use Save to Device.</p>, // -- WP UWN -- (UXP D1) one word for Save
    buttons: [
      { label: "Reset", value: "yes", danger: true },
      { label: "Cancel", value: "cancel" },
    ],
  });
  if (a === "yes") await session.change("factoryReset", { token: await session.token("factory_reset") });
}

export function sessionHandlers(c: SessionContext): Handlers {
  const { session, desk, ask, unsaved } = c;
  const st = session.state;
  const s = st.snapshot?.state;
  const out: Record<string, Handler> = {};
  if (!s || st.status !== "open") return out;
  const ro = session.readOnlyReason;
  const write = (run: () => unknown, more: Partial<Handler> = {}): Handler => ({
    run: () => void run(),
    enabled: !ro,
    reason: ro || undefined,
    ...more,
  });
  const key = (k: Parameters<typeof runKey>[0]) => () => runKey(k, session, desk, ask, unsaved);

  // What the front page allows when locked: presets and mute (WB6b's locked state)
  for (let n = 1; n <= 9 && n <= PRESET_SLOTS; ++n)
    out[`device.preset${n}`] = write(key({ action: "preset", slot: n - 1 }), { on: s.activePresetSlot === n - 1 });
  out["device.mute"] = write(key({ action: "mute" }), { on: s.userMute });
  if (c.locked) return out;

  out["edit.undo"] = { run: () => void session.undo(), enabled: st.history.canUndo && !ro, reason: ro || (st.history.canUndo ? undefined : "Nothing to undo") };
  out["edit.redo"] = { run: () => void session.redo(), enabled: st.history.canRedo && !ro, reason: ro || (st.history.canRedo ? undefined : "Nothing to redo") };
  out["device.commit"] = write(key({ action: "commit" }));
  out["device.revert"] = write(key({ action: "revert" }));
  out["device.factoryReset"] = write(() => factoryReset(session, ask), { danger: true });
  out["device.bypass"] = write(key({ action: "bypass" }), { on: s.bypass });
  const ab = mayToggle(desk.state, !!ro);
  out["device.abToggle"] = { run: () => void runKey({ action: "ab" }, session, desk, ask, unsaved), enabled: ab, reason: ab ? undefined : ro || "No A/B compare runs (Library › Snapshots)" };
  for (const [id, p] of Object.entries(PAGE_ROWS)) if (c.shown.has(p.screen)) out[id] = { run: () => c.open(p.screen, p.at) };
  return out;
}

/** The palette's title for an action: a preset's row says its slot's name, a submenu's row its submenu. */
export function actionTitle(id: string, session: Session | null): string {
  const d = actionDef(id);
  const text = actionText(id);
  const n = id.startsWith("device.preset") ? Number(id.slice("device.preset".length)) : 0;
  const s = session?.state.snapshot?.state;
  if (n > 0 && s) return `${d?.submenu ?? "Recall Preset"}: ${presetLabel(s, n - 1)}`;
  return d?.submenu ? `${d.submenu}: ${text}` : text;
}

/** The names Modern shows for a row besides its own (LAY6's), and the web's. */
export const ALIASES: Record<string, string[]> = {
  "device.commit": ["Save to Device"],
  "device.devices": ["Open a DSPi…", "Switch Device"],
  "autoeq.browse": ["AutoEQ…", "Headphone Profile"], // -- WP UWN -- (UXP A17) the page tool says AutoEQ…
  "tools.spectrum": ["Spectrum Analyzer"],
};

/** The device's sections and pages, its channels and the Settings pages, for the palette. */
export function sessionEntries(c: SessionContext, settingsPages: readonly { id: string; title: string; group: string }[]): PaletteEntry[] {
  const out: PaletteEntry[] = [];
  // The pages a row opens are found by that row
  const byRow = new Set(Object.values(PAGE_ROWS).filter((p) => !p.at).map((p) => p.screen));
  for (const sec of c.sections) {
    const only = sec.pages.length === 1 ? sec.pages[0]! : null;
    if (!only || only.label !== sec.title || !byRow.has(only.id))
      out.push({
        id: `section:${sec.id}`,
        kind: "section",
        title: `Go to ${sec.title}`,
        aliases: [sec.title],
        where: "Section",
        keys: "",
        enabled: true,
        run: () => c.open(sec.pages[0]!.id),
      });
    if (only) continue;
    for (const p of sec.pages)
      if (!byRow.has(p.id) || p.id === CONNECT)
        out.push({ id: `page:${p.id}`, kind: "page", title: p.label, where: sec.title, keys: "", enabled: true, run: () => c.open(p.id) });
  }
  const s = c.session.state.snapshot?.state;
  if (s && !c.locked && c.shown.has("eq")) {
    const rows = [...inputRows(s, liveInputs(s, null)), ...outputRows(s, c.session.state.snapshot?.outputLabels)];
    for (const r of rows)
      out.push({
        id: `channel:${r.wire}`,
        kind: "channel",
        title: r.name,
        where: r.descriptor === r.name ? "Channel" : `Channel · ${r.descriptor}`,
        keywords: [r.descriptor],
        keys: "",
        enabled: true,
        run: () => c.open("eq", { wire: r.wire }),
      });
  }
  if (c.shown.has("settings"))
    for (const p of settingsPages)
      out.push({
        id: `settings:${p.id}`,
        kind: "settings",
        title: p.title,
        where: `Settings › ${p.group}`,
        keywords: ["Settings"],
        keys: "",
        enabled: true,
        run: () => c.open("settings", { page: p.id }),
      });
  return out;
}

/** An action's palette entry; null for a row the console has no handler for. */
export function actionEntry(id: string, h: Handler | undefined, session: Session | null, where: (id: string) => string): PaletteEntry | null {
  if (!h) return null;
  const d = actionDef(id);
  return {
    id,
    kind: "action",
    title: h.text ?? actionTitle(id, session), // -- WP WEBM5 -- a handler's own words, as its menu row has them
    aliases: [...(ALIASES[id] ?? []), ...(h.text ? [actionText(id)] : [])],
    keywords: [d?.text ?? "", d?.menu ?? "", actionShortcut(id)].filter(Boolean),
    where: where(id),
    keys: actionShortcut(id),
    enabled: h.enabled !== false,
    reason: h.enabled === false ? h.reason : undefined,
    run: h.run,
  };
}
