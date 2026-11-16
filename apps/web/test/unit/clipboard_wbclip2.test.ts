// WP WBCLIP2 unit tests: one channel clip per PAGE (review-p3c N9), on the real
// core. A channel copied on one session's DSPi pastes onto another session's
// through that session's core (one undo step there, the source untouched); the
// Paste entry names the source device and channel when the clip came from
// another session; Paste stays off before a copy and while read only; the
// target's core judges the clip (read only writes nothing, a bad channel is
// the core's refusal).

import { after, beforeEach, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { channelClipboard, clearPageClip, clipEntries, clipKey } from "../../src/screens/clipboard/model";
import { inputLinks } from "../../src/screens/inputs/model";

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});
beforeEach(() => clearPageClip());

/** A demo session on its own core, as each session of a fleet page has. */
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
const deviceName = (s: Session) => {
  const d = s.state.snapshot!.device;
  return d.serial && d.serial !== d.name ? `${d.name} (${d.serial})` : d.name;
};

test("WBCLIP2 cross-session: OUT1 copied on one DSPi pastes onto another's OUT2, one undo step there, the source untouched", { skip }, async () => {
  const a = await demoSession();
  const b = await demoSession();
  const ni = st(a).numInputChannels;
  assert.equal(await a.write("out.gain", { float: -7 }, [0]), true, a.state.error);
  assert.equal(await a.write("out.delay", { float: 3 }, [0]), true, a.state.error);
  assert.equal(await a.change("filter", { channel: ni, band: 3, ...bell }), true, a.state.error);
  const aBefore = structuredClone(st(a));
  const bBefore = { gain: st(b).outputGainDb[1], delay: st(b).outputDelayMs[1], bands: structuredClone(st(b).filters[ni + 1]) };

  const ba = channelClipboard(a);
  const bb = channelClipboard(b);
  assert.notEqual(ba, bb, "each session its own chosen channel");
  assert.equal(await ba.copy(ni), true, a.state.error);
  assert.equal(bb.hasClip, true, "the clip is the page's");
  assert.equal(bb.canPaste, true);
  assert.equal(await bb.paste(ni + 1), true, b.state.error);
  const s = st(b);
  assert.equal(s.outputGainDb[1], -7);
  assert.equal(s.outputDelayMs[1], 3);
  assert.equal(s.filters[ni + 1]![3]!.freq, 315);
  assert.deepEqual(st(a).filters, aBefore.filters, "the source device untouched");
  assert.deepEqual(st(a).outputGainDb, aBefore.outputGainDb);
  assert.equal(a.state.history.canUndo, true, "only the source's own edits in its journal");

  await b.undo();
  const u = st(b);
  assert.equal(u.outputGainDb[1], bBefore.gain);
  assert.equal(u.outputDelayMs[1], bBefore.delay);
  assert.deepEqual(u.filters[ni + 1], bBefore.bands, "one undo step on the target puts it all back");
});

test("WBCLIP2 cross-session keys: Ctrl+C on one session, Ctrl+V on another; a linked input there gets its partner too", { skip }, async () => {
  const a = await demoSession();
  const b = await demoSession();
  const ni = st(a).numInputChannels;
  assert.equal(await a.change("filter", { channel: ni, band: 2, ...bell }), true, a.state.error);
  assert.equal(inputLinks(b).partner(0), 1, "IN1/IN2 linked by default");
  const ba = channelClipboard(a);
  const bb = channelClipboard(b);
  ba.selected = ni;
  bb.selected = 0;
  assert.equal(clipKey(ba, "copy"), true);
  assert.equal(bb.copyPending, true, "a copy under way on another session is seen");
  assert.equal(clipKey(bb, "paste"), true, "a paste right after waits for it");
  // The key's paste runs on; wait for it to land on the target's mirror.
  for (let i = 0; i < 200 && st(b).filters[1]![2]!.freq !== 315; i++) await new Promise((r) => setTimeout(r, 10));
  for (const ch of [0, 1]) assert.equal(st(b).filters[ch]![2]!.freq, 315, `IN${ch + 1} on the target`);
  assert.notEqual(st(a).filters[0]![2]!.freq, 315, "the source's inputs untouched");
});

test("WBCLIP2 source naming: the Paste entry names another device's clip, not its own", { skip }, async () => {
  const a = await demoSession();
  const b = await demoSession();
  const ni = st(a).numInputChannels;
  const ba = channelClipboard(a);
  const bb = channelClipboard(b);
  assert.equal(clipEntries(bb, ni, false)[1]!.label, "Paste Parameters", "nothing copied: the plain words");
  assert.equal(await ba.copy(ni), true);
  assert.equal(ba.fromElsewhere, false);
  assert.equal(clipEntries(ba, ni + 1, false)[1]!.label, "Paste Parameters", "its own clip: the plain words");
  assert.equal(bb.fromElsewhere, true);
  assert.equal(clipEntries(bb, ni + 1, false)[1]!.label, `Paste Parameters from ${deviceName(a)} OUT1`);
  assert.equal(bb.clip!.channel, "OUT1");
  assert.equal(await ba.copy(1), true);
  assert.equal(clipEntries(bb, 0, false)[1]!.label, `Paste Parameters from ${deviceName(a)} IN2`, "an input's descriptor");
  // B copies: now A's entry names B.
  assert.equal(await bb.copy(ni + 2), true);
  assert.equal(clipEntries(ba, 0, false)[1]!.label, `Paste Parameters from ${deviceName(b)} OUT3`);
  assert.equal(clipEntries(bb, 0, false)[1]!.label, "Paste Parameters");
  // Every board re-renders on a copy anywhere on the page.
  let seen = 0;
  const stop = ba.watch(() => seen++);
  assert.equal(await bb.copy(ni), true);
  stop();
  assert.equal(seen, 1);
});

test("WBCLIP2 guards: Paste off before a copy and on a read-only target; the target's core judges the clip", { skip }, async () => {
  const a = await demoSession();
  const view = await demoSession("view");
  const ni = st(a).numInputChannels;
  const ba = channelClipboard(a);
  const bv = channelClipboard(view);
  assert.equal(clipEntries(bv, ni, true)[1]!.disabled, true, "nothing copied");
  bv.selected = ni;
  assert.equal(clipKey(bv, "paste"), false, "nothing copied: the browser's key");
  assert.equal(await ba.copy(ni), true);
  assert.equal(bv.canPaste, false, "read only");
  assert.equal(clipEntries(bv, ni + 1, true)[1]!.disabled, true, "Paste off on a read-only target");
  const before = structuredClone(st(view).filters[ni + 1]);
  assert.equal(await bv.paste(ni + 1), false);
  assert.equal(view.state.error, view.readOnlyReason, "the target's own refusal");
  assert.deepEqual(st(view).filters[ni + 1], before, "nothing written");
  // A writable target: a channel it lacks is the core's refusal, nothing written, nothing to undo.
  const b = await demoSession();
  const bb = channelClipboard(b);
  assert.equal(await bb.paste(250), false);
  assert.match(b.state.error, /\S/, "the core's words");
  assert.equal(b.state.history.canUndo, false);
  // A failed copy keeps the page's clip as it was.
  assert.equal(await bb.copy(250), false);
  assert.equal(bb.clip!.channel, "OUT1");
  assert.equal(bb.fromElsewhere, true);
});
