// WP P1WB2 unit tests: the Installer page's rules (the limits profile this page keeps, the
// power-on bytes, the lock's line), the Installer tab in the Installer mode only, the stale
// meters' words, AutoEQ's pairs; and on the real core (as main.test.ts): tokens on from the
// open, a gated call without one refused in the notice table's words, limits narrowed below
// the device and the clamp report, a write past a limit refused with its notice, Clear All as
// one undo group, protect all outputs, AutoEQ's preview writing nothing, the pinned mode, the
// hub backstop's row.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  limitText,
  limitValue,
  limitsSomething,
  loadProfile,
  lockLine,
  parseProfile,
  powerOnSlot,
  profileDocument,
  saveProfile,
  startupBytes,
  tidyProfile,
  withField,
  withOutputLimit,
} from "../../src/screens/installer/model";
import { lockedScreens, screensFor } from "../../src/ui/screens";
import { staleText } from "../../src/ui/components";
import { autoEqPairs, levelLine } from "../../src/screens/eq/AutoEq";
import { hasLimits as revefixHasLimits, limitDefault as revefixDefault, stepLimit as revefixStep } from "../../src/screens/installer/model"; // -- WP REVEFIX --
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { CoreCallError, type LockStateRecord } from "../../src/core/protocol";
import { Session } from "../../src/ui/session";
import { noticeRow, resetNotices } from "../../src/ui/texts";
import { GUARD_BACKSTOP_KEY } from "../../src/screens/snapshots/model";

function memory() {
  const m = new Map<string, string>();
  return {
    getItem: (k: string) => m.get(k) ?? null,
    setItem: (k: string, v: string) => void m.set(k, v),
    removeItem: (k: string) => void m.delete(k),
  };
}

test("P1WB2 limits: a field set, emptied, and the document round trip; no zero for an empty field", () => {
  let p = withOutputLimit({}, 2, "maxGainDb", 3);
  assert.deepEqual(p, { outputs: [{ output: 2, maxGainDb: 3 }] });
  p = withOutputLimit(p, 2, "limiterCeilingDb", -1);
  p = withOutputLimit(p, 0, "highpassMinHz", 80);
  assert.deepEqual(
    p.outputs!.map((o) => o.output),
    [0, 2],
    "outputs in order",
  );
  p = withOutputLimit(p, 0, "highpassMinHz", undefined);
  assert.deepEqual(p.outputs, [{ output: 2, maxGainDb: 3, limiterCeilingDb: -1 }], "an output with no limit left is dropped");
  p = withField(p, "note", "Venue A: call before changing");
  p = withField(p, "contact", "");
  assert.equal(p.contact, undefined);
  p = withField(p, "maxPeqBoostDb", 6);
  const doc = profileDocument({ ...p, serial: "DEMO" });
  const back = JSON.parse(doc) as { meta: { kind: string }; limits: unknown };
  assert.equal(back.meta.kind, "limits");
  assert.deepEqual(parseProfile(doc), { ...p, serial: "DEMO" });
  // A document carrying it in meta.limits (an export) reads too; anything else is none.
  assert.deepEqual(parseProfile(JSON.stringify({ meta: { kind: "preset", limits: { maxPeqBoostDb: 4 } } })), { maxPeqBoostDb: 4 });
  assert.equal(parseProfile(JSON.stringify({ meta: { kind: "preset" } })), null);
  assert.equal(parseProfile("{"), null);
  assert.equal(parseProfile(""), null);
  assert.equal(limitsSomething(p), true);
  assert.equal(limitsSomething(tidyProfile({ note: "only words" })), false);
  // Fields: "" is no limit; a number kept in range.
  const gain = { min: -60, max: 20 };
  assert.equal(limitValue("", gain), undefined);
  assert.equal(limitValue("abc", gain), undefined);
  assert.equal(limitValue("25", gain), 20, "kept in the range");
  assert.equal(limitValue("2,5", gain), 2.5);
  assert.equal(limitValue("500", null), 500, "before the core's table is read: as typed (the core clamps)");
  assert.equal(limitText(undefined), "");
  assert.equal(limitText(-1.234), "-1.23");
});

test("P1WB2 limits: kept per device in this page's storage; a refused storage keeps nothing and throws nothing", () => {
  const st = memory();
  saveProfile(st, "A", "doc-a");
  saveProfile(st, "B", "doc-b");
  assert.equal(loadProfile(st, "A"), "doc-a");
  saveProfile(st, "A", "");
  assert.equal(loadProfile(st, "A"), "");
  assert.equal(loadProfile(st, "B"), "doc-b");
  assert.equal(loadProfile(null, "B"), "");
  const refusing = { getItem: () => "not json", setItem: () => { throw new Error("quota"); }, removeItem: () => undefined };
  saveProfile(refusing, "A", "x");
  assert.equal(loadProfile(refusing, "A"), "");
});

test("P1WB2 power-on: the startup bytes, the slot Test power-on loads", () => {
  assert.deepEqual(startupBytes(false, 3), { bytes: [0, 3] });
  assert.deepEqual(startupBytes(true, 3), { bytes: [1, 3] });
  assert.deepEqual(startupBytes(false, 42), { bytes: [0, 9] });
  assert.equal(powerOnSlot(0, 4, 1), 4, "the startup slot");
  assert.equal(powerOnSlot(1, 4, 1), 1, "the last one used: the one active now");
});

test("P1WB2 lock: one line for each state; no PIN, nothing locked", () => {
  const lock = (o: Partial<LockStateRecord>) => ({ pinSet: true, heldBy: "hub", locked: false, unlocked: false, covers: ["channels"], ...o });
  assert.equal(lockLine(null), "No installer PIN is set: nothing is locked.");
  assert.equal(lockLine(lock({ pinSet: false })), "No installer PIN is set: nothing is locked.");
  resetNotices();
  assert.equal(lockLine(lock({ locked: true })), "Locked by the installer", "the row's title before the table is read (P1EFIX)");
  assert.equal(lockLine(lock({ unlocked: true, heldBy: "console" })), "Unlocked on this console for this session.");
  assert.match(lockLine(lock({ covers: [] })), /covers nothing yet/);
});

test("P1WB2 screens: the Installer page in the Installer mode only, last; the locked user has the front page", () => {
  const ids = (m: Parameters<typeof screensFor>[0], hub = false) => screensFor(m, hub).map((x) => x.id);
  // -- WP WB6I -- Settings comes after it, in every mode
  assert.equal(ids("installer").at(-2), "installer");
  assert.equal(ids("installer", true).at(-2), "installer");
  assert.ok(!ids("advanced").includes("installer"));
  assert.ok(!ids("simple").includes("installer"));
  assert.deepEqual(
    lockedScreens().map((x) => x.id),
    ["home"], // -- WP UB2 -- (UXP B2) the front page is Home, the remote

  );
});

test("P1WB2 meters: stale since a time of day, and how long ago", () => {
  const t = new Date(2026, 10, 14, 9, 5, 7).getTime();
  assert.equal(staleText(t, t + 12_400), "09:05:07 (12 s ago)");
  assert.equal(staleText(t, t - 5), "09:05:07 (0 s ago)");
});

test("P1WB2 AutoEQ: pairs of inputs and of outputs, never across; the level in words", () => {
  const ch = (wire: number, output: boolean) => ({ wire, descriptor: `${output ? "OUT" : "IN"}${wire}`, name: "", output });
  const pairs = autoEqPairs([ch(0, false), ch(1, false), ch(2, true), ch(3, true), ch(4, true)]);
  assert.deepEqual(
    pairs.map((p) => [p.left.wire, p.right.wire]),
    [
      [0, 1],
      [2, 3],
    ],
  );
  const p = { profileId: "x", channels: [], levelTarget: "none", suggestedTrimDb: -4, held: [], unsupported: [], dropped: 0, notes: [] };
  assert.equal(levelLine(p), "The outputs' gain is kept. Suggested trim: -4.0 dB, if the correction clips.");
  assert.equal(levelLine({ ...p, levelTarget: "input_preamp" }), "The inputs' preamp is set to the profile's -4.0 dB.");
});

// -- On the real core --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(): Promise<Session> {
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

test("P1WB2 tokens: on from the open; a gated call without one is refused in the table's words; the page's own prompts carry one", { skip }, async () => {
  const s = await sessionOn();
  // -- WP P1TZ -- required by the core from the start: the switch is gone
  await assert.rejects(
    s.core.call("write", { path: "preset.save", indices: [0], value: "trigger" }),
    (e: unknown) => e instanceof CoreCallError && e.noticeKey === "confirmation_required",
  );
  // Session.write shows it with its notice row.
  assert.equal(await s.write("preset.save", "trigger", [0]), false);
  assert.equal(s.state.errorNotice?.key, "confirmation_required");
  assert.equal(s.state.errorNotice?.title, noticeRow("confirmation_required")!.title);
  // The page's Save, Copy to and Factory Reset mint theirs.
  assert.equal(await s.savePreset(0), true);
  assert.equal(s.state.errorNotice, null);
  assert.equal(await s.change("presetCopy", { source: 0, dest: 1, token: await s.token("copy_preset") }), true);
  assert.equal(await s.change("factoryReset", { token: await s.token("factory_reset") }), true);
  // A token for another operation is refused.
  assert.equal(await s.change("factoryReset", { token: await s.token("copy_preset") }), false);
});

test("P1WB2 limits: narrowed below the device it is held and reported; a write past it is refused with its notice", { skip }, async () => {
  const s = await sessionOn();
  assert.equal(await s.write("out.gain", { float: 8 }, [1]), true);
  const doc = profileDocument(withOutputLimit({}, 1, "maxGainDb", 2));
  const r = await s.core.call("limitsApply", { text: doc });
  assert.deepEqual(r.held, ["out.gain[1] 8 → 2 dB"]);
  await s.refresh();
  assert.equal(s.state.snapshot!.state.outputGainDb[1], 2);
  assert.equal(await s.write("out.gain", { float: 5 }, [1]), false);
  assert.equal(s.state.errorNotice?.key, "limit_exceeded");
  assert.ok(s.state.error.length > 0, "the core's own words as the detail");
  assert.equal(await s.write("out.gain", { float: 1 }, [1]), true);
  assert.equal(s.state.errorNotice, null);
  await s.core.call("limitsClear", undefined);
  assert.equal(await s.write("out.gain", { float: 5 }, [1]), true);
});

test("P1WB2 groups: a Clear All of ten bands is one undo step; Protect all outputs is one too (its ceilings back, the limiters stay on)", { skip }, async () => {
  const s = await sessionOn();
  const band = (b: number) => ({ channel: 0, band: b, filterType: 1, freq: 200 + b * 100, q: 1, gain: 3, bypass: false, qp: 1 });
  for (let b = 0; b < 10; b++) await s.core.call("filter", band(b));
  await s.core.call("endGroup", undefined);
  await s.group("Clear All Bands", async () => {
    for (let b = 0; b < 10; b++) await s.core.call("filter", { channel: 0, band: b, filterType: 0, freq: 1000, q: 0.707, gain: 0, bypass: false, qp: 0.707 });
  });
  await s.refresh();
  assert.ok(s.state.snapshot!.state.filters[0]!.slice(0, 10).every((p) => p.filterType === 0));
  await s.undo();
  assert.ok(
    s.state.snapshot!.state.filters[0]!.slice(0, 10).every((p) => p.filterType === 1),
    "one undo puts every band back",
  );
  // Protect all: every limiter on, nothing left without one; one undo takes it back.
  assert.equal(await s.write("limit.on", { bool: false }, [255]), true);
  assert.equal(await s.write("limit.threshold", { float: -6 }, [255]), true);
  assert.equal(await s.write("out.gain", { float: 0 }, [0]), true);
  assert.ok((await s.core.call("outputsWithoutLimiter", undefined)).includes(0), "a loud output without a limiter is named");
  const n = s.state.snapshot!.state.numOutputChannels;
  assert.equal(await s.change("protectAllOutputs", undefined), true);
  assert.deepEqual(await s.core.call("outputsWithoutLimiter", undefined), []);
  assert.ok(s.state.snapshot!.state.limiterEnabled.slice(0, n).every(Boolean));
  assert.ok(s.state.snapshot!.state.limiterThresholdDb.slice(0, n).every((t) => t === -1), "every ceiling at -1 dBFS");
  // One undo puts every threshold (and link group) back; switching a limiter on is never undone (the registry's rule).
  await s.undo();
  const after = s.state.snapshot!.state.limiterThresholdDb.slice(0, n);
  assert.ok(after.every((t) => t === -6), `one undo: ${JSON.stringify(after)}`);
});

test("P1WB2 AutoEQ: a preview writes nothing; apply changes the pair and keeps the state before", { skip }, async () => {
  const s = await sessionOn();
  const profile = { id: "o/Sony A", manufacturer: "Sony", model: "A", preamp: -4, filters: [{ type: "PK", freq: 105, q: 0.7, gain: 3 }] };
  const before = JSON.stringify(s.state.snapshot!.state.filters);
  const pv = await s.core.call("autoeqPreview", { profile, left: 0, right: 1 });
  assert.equal(pv.channels.length, 2);
  assert.ok(pv.channels[0]!.curve.length > 100, "a curve on the graph's grid");
  await s.refresh();
  assert.equal(JSON.stringify(s.state.snapshot!.state.filters), before, "nothing written");
  const done = await s.core.call("autoeqApply", { profile, left: 0, right: 1 });
  await s.refresh();
  assert.notEqual(JSON.stringify(s.state.snapshot!.state.filters), before);
  assert.match(done.snapshotDocument, /"snapshot"/);
});

test("P1WB2 modes: a pinned mode wins over the user's and is not switchable; no PIN, no lock, no prompt", { skip }, async () => {
  const s = await sessionOn();
  assert.equal(s.state.lock?.pinSet, false);
  assert.equal(s.state.lock?.locked, false);
  const free = await s.core.call("modeResolve", { platform: "web", chosen: "installer" });
  assert.equal(free.mode, "installer");
  assert.equal(free.needsPin, false);
  assert.equal(free.switchable, true);
  // A fleet's pin, as the session sets it after reading the lock.
  s.fleetMode = "simple";
  await s.refreshLock();
  const pinned = await s.core.call("modeResolve", { platform: "web", chosen: "advanced" });
  assert.equal(pinned.mode, "simple");
  assert.equal(pinned.reason, "pinned");
  assert.equal(pinned.switchable, false);
  // The web keeps no PIN.
  await assert.rejects(s.core.call("installerPinSet", { pin: "1234" }), (e: unknown) => e instanceof CoreCallError && e.code === "unsupported");
  // The hub's backstop beside the countdown is the core's row.
  const n = await s.core.call("noticeFor", { key: GUARD_BACKSTOP_KEY });
  assert.equal(n.reason, "The hub puts them back too if this console goes away.");
});

test("P1WB2 power-on: the DAC mute as the page writes it (fetch, change one field, read back)", { skip }, async () => {
  const s = await sessionOn();
  const d = await s.core.call("dacMuteFetch", undefined);
  assert.equal(d.supported, true, JSON.stringify(d));
  await s.core.call("dacMuteSet", { enabled: true, activeLow: d.activeLow, pin: d.pin, holdMs: d.holdMs, releaseMs: d.releaseMs });
  await new Promise((r) => setTimeout(r, 200));
  const back = await s.core.call("dacMuteFetch", undefined);
  assert.equal(back.enabled, true, `${JSON.stringify(d)} -> ${JSON.stringify(back)}`);
  await s.core.call("dacMuteSet", { ...back, holdMs: 25 });
  assert.equal((await s.core.call("dacMuteFetch", undefined)).holdMs, 25);
});

// -- WP REVEFIX -- (REVE §3.1) Qt's check box and stepper per limit

test("REVEFIX: a limit's default is Qt's, in range; the stepper steps by the range's step and stays in it; Remove All needs a limit", () => {
  assert.equal(revefixDefault("masterVolumeMaxDb", { min: -128, max: 0 }), -10);
  assert.equal(revefixDefault("highpassMinHz", { min: 100, max: 500 }), 100);
  assert.equal(revefixDefault("unknown", null), 0);
  assert.equal(revefixStep(-10, 1, { min: -128, max: 0, step: 0.5 }), -9.5);
  assert.equal(revefixStep(-0.2, 1, { min: -128, max: 0, step: 0.5 }), 0);
  assert.equal(revefixStep(0.01, -1, { min: 0, max: 85, step: 0.01 }), 0);
  assert.equal(revefixStep(5, 1, null), 6);
  assert.equal(revefixHasLimits({}), false);
  assert.equal(revefixHasLimits({ outputs: [{ output: 1 }] }), false);
  assert.equal(revefixHasLimits({ outputs: [{ output: 1, maxGainDb: 2 }] }), true);
  assert.equal(revefixHasLimits({ maxPeqBoostDb: 3 }), true);
});
