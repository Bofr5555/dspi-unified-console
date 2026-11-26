// WP WEBM4 unit tests: the menus from assets/actions.json (rows the web does not handle are left out,
// never greyed), the command palette's finding (the desktop's LAY6 rules) and its entries, the
// device's handlers on the real core (every one is a row of actions.json), the arrangement as kept,
// and the notification drawer's log.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { ACTIONS, ITEM_ACTIONS, MENUS, actionShortcut, actionText, isAction, menuItems, ownRow, popupItems, type MenuRow } from "../../src/ui/menus/actions";
import { ActionBoard } from "../../src/ui/menus/board";
import { actionEntry, actionTitle, sessionEntries, sessionHandlers, type SessionContext } from "../../src/ui/menus/session-actions";
import { search, score } from "../../src/ui/palette/search";
import { nextChoosable } from "../../src/ui/palette/Palette";
import { paletteEntries, rowWhere } from "../../src/ui/palette/entries";
import { DEFAULT_ARRANGEMENT, deviceTabsInRail, loadArrangement, saveArrangement, withChoice } from "../../src/ui/arrangement/arrangement";
import { loadNavigation, sectionsFor } from "../../src/ui/shell/sections";
import { KEPT, NoticeLog, waiting } from "../../src/ui/drawer/notices";
import { SnapshotDesk } from "../../src/screens/snapshots/model";
import { SETTINGS_PAGES } from "../../src/screens/settings/SettingsScreen";
import { screensFor, withDeviceScreen } from "../../src/ui/screens";

const rows = (items: ReturnType<typeof menuItems>) => items.map((m) => (m === "separator" ? "|" : "heading" in m ? `#${m.heading}` : m.label));
const run = () => undefined;

test("WEBM4 actions.json: the desktop's rows and menus, every menu row a row", () => {
  assert.equal(ACTIONS.length, 70); // -- WP UFW -- Help › User Manual, Keyboard Shortcuts
  assert.equal(ITEM_ACTIONS.length, 33);
  for (const [menu, groups] of Object.entries(MENUS)) for (const id of groups.flat()) assert.ok(isAction(id), `${menu}: ${id}`);
  // -- INTC -- "files": the desktop Equaliser's Files ▾ (MenuTable's place, DL8 fold)
  assert.deepEqual(Object.keys(MENUS).sort(), ["appMenu", "autoeq", "band", "channel", "device", "files", "graph", "history", "save", "slot"]);
  assert.equal(actionText("help.about"), "About uConsole", "the app's name filled in");
  assert.equal(actionText("channel.mute", true), "Unmute", "words while on");
  assert.equal(actionShortcut("channel.copy"), "Ctrl+C", "the shortcut of the row it is the same as");
  assert.equal(actionShortcut("tools.crossfeed"), "Ctrl+Shift+X");
  assert.equal(actionText("no.such"), "no.such");
});

test("WEBM4 menus: only the rows the web handles, in actions.json's groups; no empty group, no stray separator", () => {
  assert.deepEqual(menuItems("save", {}), [], "nothing handled: no menu");
  // -- WP UFW -- (UXP A10, C1) Factory Reset is a device tab's, not Save ▾'s
  assert.deepEqual(rows(menuItems("save", { "device.revert": { run }, "device.factoryReset": { run, danger: true } })), ["Revert to Saved…"]);
  assert.deepEqual(rows(menuItems("device", { "device.factoryReset": { run, danger: true }, "device.close": { run } })), ["Factory Reset…", "|", "Close"]);
  // The middle group (Save Master Volume, Save Output Configuration) has no web handler: left out, not greyed
  const items = menuItems("channel", { "channel.copy": { run }, "channel.paste": { run, enabled: false, reason: "Nothing copied" }, "channel.clear": { run } });
  assert.deepEqual(rows(items), ["Copy Parameters", "Paste Parameters", "|", "Clear PEQ…"]);
  const paste = items[1] as MenuRow;
  assert.equal(paste.disabled, true);
  assert.equal(paste.reason, "Nothing copied");
  assert.equal(paste.shortcut, "Ctrl+V");
  // On: a row with words for it says them; one without has a check
  const band = menuItems("band", { "band.bypass": { run, on: true } });
  assert.equal((band[0] as MenuRow).label, "Enable");
  assert.equal((band[0] as MenuRow).checked, undefined);
  const mute = menuItems("channel", { "channel.mute": { run, on: true } });
  assert.equal((mute[0] as MenuRow).label, "Unmute");
  const preset = menuItems("graph", { "graph.rta": { run, on: true } });
  assert.equal((preset[0] as MenuRow).label, "Hide the Spectrum");
  // The web's words where it says more
  assert.equal((menuItems("channel", { "channel.paste": { run, text: "Paste Parameters from Den OUT1" } })[0] as MenuRow).label, "Paste Parameters from Den OUT1");
  // The web's own rows: into the group of a row (handled or not), or a group at the end
  const own = ownRow("band.slope.1", "Slope: 6 dB", run, { checked: true });
  const del = ownRow("band.delete", "Delete Band", run, { danger: true });
  assert.deepEqual(rows(menuItems("band", { "band.invert": { run } }, [{ with: "band.slope", rows: [own] }, { rows: [del] }])), ["Slope: 6 dB", "|", "Invert Gain", "|", "Delete Band"]);
  // As the EQ graph's popup takes them
  const pop = popupItems(menuItems("graph", { "graph.selectAll": { run }, "graph.deselect": { run, enabled: false } }));
  assert.deepEqual(pop.map((p) => (typeof p === "string" || "heading" in p ? p : [p.label, p.disabled])), [["Select All Bands", false], ["Deselect All", true]]);
});

test("WEBM4 palette: found as the desktop finds (start, word start, inside, initials, letters left out; any order; no case or accents)", () => {
  const e = (title: string, more: Partial<{ aliases: string[]; keywords: string[]; enabled: boolean }> = {}) => ({ title, enabled: true, ...more });
  const all = [
    e("Signal Generator…"),
    e("Headphone Crossfeed…"),
    e("Spectrum Analyser…", { aliases: ["Spectrum Analyzer"] }),
    e("Stats for nerbs"),
    e("Commit Parameters…", { aliases: ["Save to Device"] }),
    e("Interrupt Monitor…"),
    e("Équaliser"),
    e("Mute", { enabled: false }),
    e("Mute All"),
  ];
  const first = (q: string) => search(all, q)[0]?.title;
  assert.equal(first("sig"), "Signal Generator…", "the start");
  assert.equal(first("gen"), "Signal Generator…", "a word's start");
  assert.equal(first("sg"), "Signal Generator…", "initials");
  assert.equal(first("cfeed"), "Headphone Crossfeed…", "letters left out");
  assert.equal(first("generator signal"), "Signal Generator…", "any order");
  assert.equal(first("EQUAL"), "Équaliser", "no case, no accents");
  assert.equal(first("analyzer"), "Spectrum Analyser…", "an alias");
  assert.equal(search(all, "save device")[0]!.matched, "Save to Device", "found by its alias, which it says");
  assert.equal(search(all, "q").length, 0, "a single letter is not looked for inside words");
  assert.equal(search(all, "zzz").length, 0);
  assert.equal(search(all, "").length, all.length, "all, in their order");
  assert.deepEqual(search(all, "").map((x) => x.title), all.map((x) => x.title));
  // Of equals a greyed one comes after
  assert.deepEqual(search(all, "mute").map((x) => x.title), ["Mute All", "Mute"]);
  assert.ok(score(e("Stats for nerbs"), "nerbs").score > 0);
  assert.equal(score(e("Matrix", { keywords: ["Routing"] }), "rout").matched, "Routing", "a keyword, as spelt");
});

test("WEBM4 palette: the next entry that can be chosen, round the end", () => {
  const r = [{ enabled: false }, { enabled: true }, { enabled: false }, { enabled: true }];
  assert.equal(nextChoosable(r, -1, 1), 1);
  assert.equal(nextChoosable(r, 1, 1), 3);
  assert.equal(nextChoosable(r, 3, 1), 1, "round the end");
  assert.equal(nextChoosable(r, 1, -1), 3);
  assert.equal(nextChoosable([{ enabled: false }], -1, 1), -1);
  assert.equal(nextChoosable([], -1, 1), -1);
});

test("WEBM4 palette: a row says where it is; the page's rows and the device's", () => {
  assert.equal(rowWhere("tools.crossfeed"), "Effects", "its page's section");
  assert.equal(rowWhere("help.about"), "Help", "its Classic menu");
  assert.equal(rowWhere("device.commit"), "Device");
  const board = new ActionBoard();
  board.app = { "help.about": { run }, "device.devices": { run } };
  assert.deepEqual(Object.keys(board.handlers(null)).sort(), ["device.devices", "help.about"]);
  board.setSession("a", { handlers: { "help.about": { run, text: "mine" }, "device.mute": { run } }, entries: [] });
  assert.equal(board.handlers("a")["help.about"]!.text, "mine", "the device's over the page's");
  assert.equal(board.handlers("b")["device.mute"], undefined, "another device's view says nothing for this one");
  board.clearSession("b");
  assert.ok(board.session("a"), "only its own view takes it back");
  board.clearSession("a");
  assert.equal(board.session("a"), null);
  const entries = paletteEntries({ current: null, session: null, handlers: board.handlers(null), others: [], extra: [] });
  assert.deepEqual(entries.map((x) => x.id).sort(), ["device.devices", "help.about"], "no handler, no entry");
  assert.ok(entries.find((x) => x.id === "device.devices")!.aliases!.includes("Open a DSPi…"));
  assert.equal(actionEntry("device.mute", undefined, null, rowWhere), null);
  const off = actionEntry("device.mute", { run, enabled: false, reason: "Read only" }, null, rowWhere)!;
  assert.equal(off.enabled, false);
  assert.equal(off.reason, "Read only");
  assert.equal(off.keys, "Ctrl+M");
  assert.equal(actionTitle("autoeq.clearFavorites", null), "Favorite Profiles: Clear Favorites", "a submenu's row says its submenu");
});

test("WEBM4 arrangement: as kept (WEBM1's key, which held the sections alone), defaults for the rest", () => {
  const store = new Map<string, string>();
  const kv = { getItem: (k: string) => store.get(k) ?? null, setItem: (k: string, v: string) => void store.set(k, v) };
  assert.deepEqual(loadArrangement(kv), DEFAULT_ARRANGEMENT);
  assert.deepEqual(loadArrangement(null), DEFAULT_ARRANGEMENT);
  store.set("dspi-web.arrangement", JSON.stringify({ navigation: "tabs" }));
  assert.deepEqual(loadArrangement(kv), { ...DEFAULT_ARRANGEMENT, navigation: "tabs" });
  store.set("dspi-web.arrangement", "{not json");
  assert.deepEqual(loadArrangement(kv), DEFAULT_ARRANGEMENT);
  const a = withChoice(withChoice(DEFAULT_ARRANGEMENT, "theme", "dark")!, "deviceTabs", "rail")!;
  assert.equal(withChoice(a, "theme", "purple"), null, "not one of its choices");
  saveArrangement(kv, a);
  assert.deepEqual(loadArrangement(kv), { navigation: "rail", deviceTabs: "rail", theme: "dark" });
  assert.equal(loadNavigation(kv), "rail", "WEBM1's reading of it still works");
  saveArrangement(kv, withChoice(a, "navigation", "tabs")!);
  assert.equal(loadNavigation(kv), "tabs");
  assert.equal(deviceTabsInRail(a), true);
  assert.equal(deviceTabsInRail({ ...a, navigation: "tabs" }), false, "no rail, no device tabs in it");
  const throwing = { getItem: () => null, setItem: () => { throw new Error("quota"); } };
  saveArrangement(throwing, a); // a blocked store: the choice lasts as long as the page
});

test("WEBM4 drawer: what was said, newest first, at most 50; what waits on each device", () => {
  const log = new NoticeLog();
  let seen = 0;
  const stop = log.watch(() => ++seen);
  log.say("one", "info");
  log.say("", "info");
  log.say("two", "error", "Den");
  assert.deepEqual(log.earlier.map((n) => [n.text, n.kind, n.device]), [["two", "error", "Den"], ["one", "info", ""]]);
  assert.equal(seen, 2, "nothing said, nothing changed");
  for (let i = 0; i < 60; ++i) log.say(`n${i}`, "info");
  assert.equal(log.earlier.length, KEPT);
  assert.equal(log.earlier[0]!.text, "n59");
  log.clear();
  assert.equal(log.earlier.length, 0);
  stop();

  // A session's error and notice, as they come
  type Fake = { id: string; installationName: string; state: Record<string, unknown>; watch: (f: () => void) => () => void; report: (t: string) => void; dismissNotice: () => void };
  const fns: (() => void)[] = [];
  const s: Fake = {
    id: "s",
    installationName: "Here",
    state: { error: "", notice: null, snapshot: { device: { name: "Den" } } },
    watch: (f) => (fns.push(f), () => fns.splice(fns.indexOf(f), 1)),
    report: (t) => ((s.state.error = t), fns.forEach((f) => f())),
    dismissNotice: () => ((s.state.notice = null), fns.forEach((f) => f())),
  };
  const session = s as unknown as Session;
  log.track([session]);
  s.report("The link was lost");
  s.state.notice = { text: "Undid Gain", tone: "info" };
  fns.forEach((f) => f());
  assert.deepEqual(log.earlier.map((n) => [n.text, n.device]), [["Undid Gain", "Den"], ["The link was lost", "Den"]]);
  assert.deepEqual(waiting([session]).map((w) => [w.kind, w.text]), [["error", "The link was lost"], ["info", "Undid Gain"]]);
  waiting([session])[0]!.dismiss();
  assert.deepEqual(waiting([session]).map((w) => w.kind), ["info"]);
  fns.forEach((f) => f());
  assert.equal(log.earlier.length, 2, "the same again is not said twice");
  log.track([]);
  assert.equal(fns.length, 0, "a closed session is no longer followed");
});

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demoSession(): Promise<Session> {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const client = {
    call: (m: never, p: never) => core.call(m, p),
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo" });
  return session;
}

test("WEBM4 on the demo: the device's handlers are actions.json's rows, its pages and channels in the palette, a row runs", { skip, timeout: 60_000 }, async () => {
  const session = await demoSession();
  const shown = withDeviceScreen(screensFor("advanced", false));
  const opened: [string, unknown][] = [];
  const asked: string[] = [];
  const ctx: SessionContext = {
    session,
    desk: new SnapshotDesk(session),
    ask: async (q) => (asked.push(q.title), "cancel"),
    unsaved: async () => "clean",
    shown: new Set(shown.map((x) => x.id)),
    sections: sectionsFor(shown),
    locked: false,
    open: (screen, at) => void opened.push([screen, at]),
  };
  const h = sessionHandlers(ctx);
  for (const id of Object.keys(h)) assert.ok(isAction(id), `${id} is a row of actions.json`);
  for (const id of ["device.commit", "device.revert", "device.factoryReset", "device.mute", "device.bypass", "edit.undo", "edit.settings", "tools.matrix", "tools.crossfeed", "tools.measure", "device.preset1", "device.preset9"])
    assert.ok(h[id], id);
  assert.equal(h["device.abToggle"]!.enabled, false, "no compare runs");
  assert.ok(h["device.abToggle"]!.reason);
  assert.equal(h["edit.undo"]!.enabled, false, "nothing to undo yet");
  assert.equal(h["device.preset1"]!.on, true, "the active slot");
  // A page row opens its page, at its effect
  h["tools.crossfeed"]!.run();
  assert.deepEqual(opened.pop(), ["effects", { effect: "crossfeed" }]);
  // A write row runs the core's write: Mute flips the mute
  const was = session.state.snapshot!.state.userMute;
  h["device.mute"]!.run();
  for (let i = 0; i < 50 && session.state.snapshot!.state.userMute === was; ++i) await new Promise((r) => setTimeout(r, 20));
  assert.equal(session.state.snapshot!.state.userMute, !was);
  // Commit asks first
  h["device.commit"]!.run();
  await new Promise((r) => setTimeout(r, 20));
  assert.deepEqual(asked, ["Save Preset"]);
  // Locked: presets and mute only
  assert.deepEqual(Object.keys(sessionHandlers({ ...ctx, locked: true })).filter((k) => !k.startsWith("device.preset")), ["device.mute"]);
  // A shown page no row opens is in the palette by itself; a channel; a Settings page
  const entries = sessionEntries(ctx, SETTINGS_PAGES);
  const ids = entries.map((x) => x.id);
  assert.ok(ids.includes("section:equaliser"));
  assert.ok(ids.includes("page:xover"), "Crossover: no row opens it");
  assert.ok(ids.includes("section:routing") && !ids.includes("page:matrix"), "Routing's only page: the section, and Matrix Mixer…'s row");
  assert.ok(ids.includes("settings:appearance"));
  const ch = entries.find((x) => x.kind === "channel" && x.where.includes("OUT1"))!;
  ch.run();
  assert.deepEqual(opened.pop(), ["eq", { wire: session.state.snapshot!.state.numInputChannels }]);
  // The palette as it opens on this device: its preset rows by slot name
  const all = paletteEntries({ current: session, session: { handlers: h, entries }, handlers: h, others: [], extra: [] });
  assert.match(all.find((x) => x.id === "device.preset1")!.title, /^Recall Preset: /);
  assert.equal(search(all, "crossfeed")[0]!.id, "tools.crossfeed");
  assert.equal(search(all, "appearance")[0]!.id, "settings:appearance");
});
