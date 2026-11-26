// WP WB6l: the console's keyboard shortcuts in one table (M24), as Qt keeps its
// menus' in one (`qml/main/MenuTable.qml`, QP18): the Console's keys with Ctrl
// for Command (Cmd on a Mac). The keys other files act on are listed here for
// the Keyboard Shortcuts list (`handled: "elsewhere"`): undo and redo
// (ui/undo.tsx), presets, Commit, Revert, Mute, Bypass and A/B (ui/keys.ts,
// P1WB1), the EQ graph's keys (screens/eq/EqGraph.tsx, WB6b). The rows this file
// acts on open a screen of the console: Tools' windows on the Console are the
// web's screens, so a key does what the menu row does there.
//
// A row whose screen this console does not show (another mode, the locked
// front page, a screen not on this build) does nothing and is not listed. A key
// the browser keeps for itself (some take Ctrl+Shift+T, I or P) simply never
// arrives; the screen tabs and the quick strip reach the same places. Opening a
// screen changes nothing on the device, so a shortcut is never a write: the
// writes the keys of ui/keys.ts make go through the core, which refuses them
// while read only or locked.
//
// -- WP WBCLIP -- Ctrl+C and Ctrl+V are the channel clipboard's (M45, the
// Console's Edit menu with no text field focused): Copy and Paste Parameters on
// the channel the EQ screen has chosen (screens/clipboard/model.ts). Paste is
// the one write here, the core's, refused while read only or locked; a page
// text selection or a text field keeps the browser's copy and paste.

/** What a row does: open a screen, at an effect or a tool where the screen has several. */
export interface OpenAction {
  screen: string;
  at?: Record<string, unknown>;
}

export interface ShortcutRow {
  id: string;
  /** The Console's words for the row. */
  text: string;
  /** The menu it sits in on the Console, for the list's headings. */
  menu: "Device" | "Edit" | "File" | "Tools" | "Help" | "EQ graph";
  /** The key as Qt writes it: "Ctrl+Shift+M", "B", "Shift+?". */
  key: string;
  open?: OpenAction;
  /** A web-only key (the shortcut list itself). */
  help?: true;
  handled?: "elsewhere";
  /** Listed only where this screen is shown (the graph keys: the EQ screen). */
  on?: string;
  // -- WP WBCLIP --
  /** The channel clipboard (M45): Copy or Paste Parameters on the channel the EQ screen has chosen. */
  clip?: "copy" | "paste";
}

const effect = (id: string): OpenAction => ({ screen: "effects", at: { effect: id } });

export const SHORTCUTS: ShortcutRow[] = [
  { id: "file.openFile", text: "Open File…", menu: "File", key: "Ctrl+O", open: { screen: "files" } },
  { id: "edit.undo", text: "Undo", menu: "Edit", key: "Ctrl+Z", handled: "elsewhere" },
  { id: "edit.redo", text: "Redo", menu: "Edit", key: "Ctrl+Shift+Z", handled: "elsewhere" },
  // -- WP WBCLIP -- the channel clipboard (M45), on the EQ screen's chosen channel as Qt's on the sidebar's
  { id: "edit.copy", text: "Copy Parameters", menu: "Edit", key: "Ctrl+C", clip: "copy", on: "eq" },
  { id: "edit.paste", text: "Paste Parameters", menu: "Edit", key: "Ctrl+V", clip: "paste", on: "eq" },
  { id: "edit.settings", text: "Settings…", menu: "Edit", key: "Ctrl+,", open: { screen: "settings" } },
  { id: "device.commit", text: "Save to Device…", menu: "Device", key: "Ctrl+S", handled: "elsewhere" }, // -- WP UFW -- (UXP A10, D1) the button's words
  { id: "device.revert", text: "Revert to Saved…", menu: "Device", key: "Ctrl+Shift+R", handled: "elsewhere" },
  { id: "device.preset", text: "Recall Preset 1-9", menu: "Device", key: "Ctrl+1…9", handled: "elsewhere" },
  { id: "device.mute", text: "Mute", menu: "Device", key: "Ctrl+M", handled: "elsewhere" },
  { id: "device.bypass", text: "Bypass Master EQ", menu: "Device", key: "Ctrl+B", handled: "elsewhere" },
  { id: "device.abToggle", text: "Toggle A/B", menu: "Device", key: "B", handled: "elsewhere" },
  { id: "tools.matrix", text: "Matrix Mixer…", menu: "Tools", key: "Ctrl+Shift+M", open: { screen: "matrix" } },
  { id: "tools.loudness", text: "Loudness Compensation…", menu: "Tools", key: "Ctrl+Shift+L", open: effect("loudness") },
  { id: "tools.crossfeed", text: "Headphone Crossfeed…", menu: "Tools", key: "Ctrl+Shift+X", open: effect("crossfeed") },
  { id: "tools.psybass", text: "Psychoacoustic Bass…", menu: "Tools", key: "Ctrl+Shift+P", open: effect("psybass") },
  { id: "tools.subharmonic", text: "Subharmonic Synthesizer…", menu: "Tools", key: "Ctrl+Shift+S", open: effect("subharm") },
  { id: "tools.tube", text: "Tube Modeller…", menu: "Tools", key: "Ctrl+Shift+D", open: effect("tube") },
  { id: "tools.upmixer", text: "Stereo Upmixer…", menu: "Tools", key: "Ctrl+Shift+U", open: effect("upmix") },
  { id: "tools.leveller", text: "Volume Leveller…", menu: "Tools", key: "Ctrl+Shift+V", open: effect("leveller") },
  // The analysis tools' screens (WB6j) take these ids when they come.
  { id: "tools.signalGenerator", text: "Signal Generator…", menu: "Tools", key: "Ctrl+Shift+G", open: { screen: "generator" } },
  { id: "tools.spectrum", text: "Spectrum Analyser…", menu: "Tools", key: "Ctrl+Shift+A", open: { screen: "analyser" } },
  { id: "tools.stats", text: "Stats for nerbs", menu: "Tools", key: "Ctrl+Shift+T", open: { screen: "stats" } },
  { id: "tools.interruptMonitor", text: "Interrupt Monitor…", menu: "Tools", key: "Ctrl+Shift+I", open: { screen: "monitor" } },
  // -- the web's own: the list itself
  { id: "help.shortcuts", text: "Keyboard Shortcuts", menu: "Help", key: "Shift+?", help: true },
  // The EQ graph's keys, while the graph has the focus (WB6b).
  { id: "graph.step", text: "Next or previous band", menu: "EQ graph", key: "Tab, Shift+Tab", handled: "elsewhere", on: "eq" },
  { id: "graph.move", text: "Move the band (Shift: fine)", menu: "EQ graph", key: "Arrows", handled: "elsewhere", on: "eq" },
  { id: "graph.q", text: "Change Q", menu: "EQ graph", key: "Alt+Up, Alt+Down", handled: "elsewhere", on: "eq" },
  { id: "graph.delete", text: "Remove the band", menu: "EQ graph", key: "Delete", handled: "elsewhere", on: "eq" },
  { id: "graph.all", text: "Select all bands", menu: "EQ graph", key: "Ctrl+A", handled: "elsewhere", on: "eq" },
  { id: "graph.type", text: "Type values", menu: "EQ graph", key: "Enter", handled: "elsewhere", on: "eq" },
  { id: "graph.menu", text: "Band menu", menu: "EQ graph", key: "Shift+F10", handled: "elsewhere", on: "eq" },
  { id: "graph.none", text: "Deselect", menu: "EQ graph", key: "Escape", handled: "elsewhere", on: "eq" },
];

type KeyEv = Pick<KeyboardEvent, "key" | "code" | "ctrlKey" | "metaKey" | "shiftKey" | "altKey">;

/** A row's key as `{cmd, shift, key}`, for the rows this file acts on; null for the others. */
function parse(key: string): { cmd: boolean; shift: boolean; k: string } | null {
  const parts = key.split("+");
  const k = parts.pop() ?? "";
  if (!k) return null;
  return { cmd: parts.includes("Ctrl"), shift: parts.includes("Shift"), k: k.toLowerCase() };
}

/**
 * The row of this file's a key press asks for, or null. The command key is Ctrl,
 * Cmd on a Mac; Alt and the other platform's key never. A letter is matched by
 * its key, or by its place on the keyboard when a layout gives another one.
 */
export function shortcutFor(e: KeyEv, apple: boolean, rows: ShortcutRow[] = SHORTCUTS): ShortcutRow | null {
  const cmd = apple ? e.metaKey : e.ctrlKey;
  const other = apple ? e.ctrlKey : e.metaKey;
  if (e.altKey || other) return null;
  const pressed = e.key.toLowerCase();
  const byCode = /^Key[A-Z]$/.test(e.code ?? "") ? e.code.slice(3).toLowerCase() : "";
  for (const r of rows) {
    if (r.handled || (!r.open && !r.help && !r.clip)) continue; // -- WP WBCLIP -- clip
    const p = parse(r.key);
    if (!p || p.cmd !== cmd) continue;
    if (r.help) {
      // "?" is Shift and a key on most layouts: the character decides.
      if (pressed === "?" && !cmd) return r;
      continue;
    }
    if (p.shift !== e.shiftKey) continue;
    if (pressed === p.k || (p.k.length === 1 && /[a-z]/.test(p.k) && byCode === p.k)) return r;
  }
  return null;
}

/** A key as this platform writes it: "Ctrl+Shift+M", or "⇧⌘M" on a Mac. */
export function keyText(key: string, apple: boolean): string {
  if (!apple) return key;
  return key
    .split(", ")
    .map((one) => {
      const parts = one.split("+");
      const k = parts.pop() ?? "";
      if (parts.length === 0) return k;
      const mods = (parts.includes("Ctrl") ? "⌘" : "") + (parts.includes("Alt") ? "⌥" : "");
      return (parts.includes("Shift") ? "⇧" : "") + mods + k;
    })
    .join(", ");
}

/** The key as `aria-keyshortcuts` names it ("Control+Shift+M", "Meta+Shift+M"). */
export function ariaKeys(key: string, apple: boolean): string {
  return key.replace(/Ctrl/g, apple ? "Meta" : "Control");
}

/** The rows a console with these screens lists, by menu, in the table's order. */
export function listed(screens: Set<string>): { menu: string; rows: ShortcutRow[] }[] {
  const out: { menu: string; rows: ShortcutRow[] }[] = [];
  for (const r of SHORTCUTS) {
    if (r.open && !screens.has(r.open.screen)) continue;
    if (r.on && !screens.has(r.on)) continue;
    let g = out.find((x) => x.menu === r.menu);
    if (!g) out.push((g = { menu: r.menu, rows: [] }));
    g.rows.push(r);
  }
  return out;
}

/** The tool row of a quick-strip effect, for its title: the screen it opens and its key. */
export function toolRow(effectId: string): ShortcutRow | undefined {
  return SHORTCUTS.find((r) => r.open?.screen === "effects" && r.open.at?.effect === effectId);
}
