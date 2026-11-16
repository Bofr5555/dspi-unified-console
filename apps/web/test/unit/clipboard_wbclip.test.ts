// WP WBCLIP unit tests: the channel clipboard on the web (M45). The keys in the
// one shortcut table (Ctrl/Cmd+C and +V, listed with the EQ screen), the
// clipboard's guards (nothing selected, nothing copied, read only), and on the
// real core: a pasted output is one undo step, a linked input pastes onto its
// partner too, and the core's refusals reach the session's error line.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { SHORTCUTS, listed, shortcutFor } from "../../src/screens/shortcuts/table";
import { channelClipboard, clipEntries, clipKey } from "../../src/screens/clipboard/model";
import { inputLinks } from "../../src/screens/inputs/model";

const ev = (key: string, mods: { ctrl?: boolean; meta?: boolean; shift?: boolean; alt?: boolean } = {}, code = "") => ({
  key,
  code,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  shiftKey: !!mods.shift,
  altKey: !!mods.alt,
});

test("WBCLIP keys: Ctrl+C and Ctrl+V (Cmd on a Mac) are the clipboard's rows, listed with the EQ screen", () => {
  assert.equal(shortcutFor(ev("c", { ctrl: true }), false)?.clip, "copy");
  assert.equal(shortcutFor(ev("v", { ctrl: true }), false)?.clip, "paste");
  assert.equal(shortcutFor(ev("c", { meta: true }), true)?.clip, "copy");
  assert.equal(shortcutFor(ev("c", { ctrl: true }), true), null, "Ctrl is not Cmd on a Mac");
  assert.equal(shortcutFor(ev("C", { ctrl: true, shift: true }), false), null);
  assert.equal(shortcutFor(ev("c", { ctrl: true, alt: true }), false), null);
  assert.equal(shortcutFor(ev("с", { ctrl: true }, "KeyC"), false)?.id, "edit.copy", "another layout: the key's place");
  assert.equal(shortcutFor(ev("V", { ctrl: true, shift: true }), false)?.id, "tools.leveller", "Ctrl+Shift+V stays the leveller");
  const ids = (screens: string[]) => listed(new Set(screens)).flatMap((g) => g.rows.map((r) => r.id));
  assert.ok(ids(["main", "eq"]).includes("edit.copy") && ids(["main", "eq"]).includes("edit.paste"));
  assert.ok(!ids(["main"]).includes("edit.copy"), "no EQ screen, no channel to act on");
  const acted = SHORTCUTS.filter((r) => (r.open || r.help || r.clip) && !r.handled).map((r) => r.key);
  assert.equal(new Set(acted).size, acted.length, "one row per key");
});

// -- On the real core (as inputs_wb6g.test.ts) --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demoSession(access: "control" | "view" = "control") {
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
  const session = new Session("s", "i", "Here", { kind: "demo" }, access, client);
  await session.open({ kind: "demo" });
  return session;
}

const st = (s: Session) => s.state.snapshot!.state;
const bell = { filterType: 1, bypass: false, freq: 315, q: 1.4, gain: 5, qp: 0.707 };

test("WBCLIP guards: nothing selected or nothing copied leaves the key to the browser; Paste off until a copy and while read only", { skip }, async () => {
  const session = await demoSession();
  const board = channelClipboard(session);
  assert.equal(board.selected, null);
  assert.equal(clipKey(board, "copy"), false, "no channel chosen");
  board.selected = 0;
  assert.equal(clipKey(board, "paste"), false, "nothing copied");
  assert.equal(board.hasClip, false);
  assert.equal(clipEntries(board, 0, false)[1]!.disabled, true, "Paste off before a copy");
  assert.equal(await board.paste(0), false);
  assert.equal(await board.copy(0), true, session.state.error);
  assert.equal(board.hasClip, true);
  assert.equal(board.canPaste, true);
  assert.equal(clipEntries(board, 0, false)[1]!.disabled, false);
  assert.equal(clipEntries(board, 0, true)[1]!.disabled, true, "Paste off while read only");
  assert.equal(clipEntries(board, 0, true)[0]!.disabled, undefined, "Copy stays on");
  assert.notEqual(channelClipboard(await demoSession()), board, "one clipboard per session");
});

test("WBCLIP M45: a pasted output is one undo step; Undo puts every parameter back", { skip }, async () => {
  const session = await demoSession();
  const board = channelClipboard(session);
  const ni = st(session).numInputChannels;
  const [o1, o2] = [ni, ni + 1];
  assert.equal(await session.write("out.gain", { float: -7 }, [0]), true, session.state.error);
  assert.equal(await session.write("out.delay", { float: 3 }, [0]), true, session.state.error);
  assert.equal(await session.change("filter", { channel: o1, band: 3, ...bell }), true, session.state.error);
  const before = { gain: st(session).outputGainDb[1], delay: st(session).outputDelayMs[1], bands: structuredClone(st(session).filters[o2]) };
  // A paste right after the copy (Ctrl+C, Ctrl+V) waits for it.
  const copied = board.copy(o1);
  assert.equal(board.copyPending, true);
  assert.equal(await board.paste(o2), true, session.state.error);
  assert.equal(await copied, true);
  assert.equal(board.copyPending, false);
  const s = st(session);
  assert.equal(s.outputGainDb[1], -7);
  assert.equal(s.outputDelayMs[1], 3);
  assert.deepEqual(s.filters[o2]![3], s.filters[o1]![3]);
  assert.equal(s.filters[o2]![3]!.freq, 315);
  assert.equal(session.state.history.canUndo, true);
  await session.undo();
  const u = st(session);
  assert.equal(u.outputGainDb[1], before.gain);
  assert.equal(u.outputDelayMs[1], before.delay);
  assert.deepEqual(u.filters[o2], before.bands, "one step puts the whole paste back");
  assert.equal(u.outputGainDb[0], -7, "the source is untouched");
});

test("WBCLIP M45: a linked input pastes onto its partner too, one undo step; unlinked, onto itself alone", { skip }, async () => {
  const session = await demoSession();
  const board = channelClipboard(session);
  const links = inputLinks(session);
  assert.equal(links.partner(0), 1, "1/2 linked by default");
  const ni = st(session).numInputChannels;
  assert.equal(await session.change("filter", { channel: ni, band: 2, ...bell }), true, session.state.error);
  const was = [structuredClone(st(session).filters[0]), structuredClone(st(session).filters[1])];
  assert.equal(await board.copy(ni), true, session.state.error);
  assert.equal(await board.paste(0), true, session.state.error);
  for (const ch of [0, 1]) assert.equal(st(session).filters[ch]![2]!.freq, 315, `IN${ch + 1} has the band`);
  await session.undo();
  assert.deepEqual([st(session).filters[0], st(session).filters[1]], was, "one step takes both back");
  links.set(0, false);
  assert.equal(await board.paste(1), true, session.state.error);
  assert.equal(st(session).filters[1]![2]!.freq, 315);
  assert.deepEqual(st(session).filters[0], was[0], "IN1 alone untouched");
});

test("WBCLIP refusals: read only writes nothing and says why; the core's refusal is its own text", { skip }, async () => {
  const view = await demoSession("view");
  const vb = channelClipboard(view);
  assert.ok(view.readOnlyReason);
  assert.equal(await vb.copy(0), true, "copying reads the mirror: allowed read only");
  assert.equal(vb.canPaste, false);
  const before = structuredClone(st(view).filters[1]);
  assert.equal(await vb.paste(1), false);
  assert.equal(view.state.error, view.readOnlyReason);
  assert.deepEqual(st(view).filters[1], before);

  const session = await demoSession();
  const board = channelClipboard(session);
  board.clear(); // -- WP WBCLIP2 -- the clip is the page's: forget the view session's copy
  assert.equal(await board.copy(250), false);
  assert.match(session.state.error, /\S/, "the core's words for a channel the device lacks");
  assert.equal(board.hasClip, false, "a failed copy keeps nothing");
  // Another clip version: the core refuses, nothing written.
  const { clip } = await session.core.call("copyChannel", { ch: 0 });
  const r = await session.change("pasteChannel", { clip: { ...clip, version: 0 }, ch: 1 });
  assert.equal(r, false);
  assert.match(session.state.error, /\S/);
  assert.equal(session.state.history.canUndo, false, "nothing to undo");
});
