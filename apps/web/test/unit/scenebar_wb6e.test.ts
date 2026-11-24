// WP WB6e unit tests (the verbs the session bar's preset popover uses since INTD): what a recall
// asks first, the Store question; and on the real core the two verbs through the session:
// Recall's unsaved-changes prompt (Cancel, Discard, Save), the reload of the active slot,
// Store into an empty and an occupied slot (with its token), and nothing while read only.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { recall, recallStep, store, storeText } from "../../src/screens/scenebar/model";
import type { Question, Unsaved } from "../../src/ui/screens";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";

const slots = (active: number, occupied: number, names: string[] = []) => ({
  activePresetSlot: active,
  presetOccupied: occupied,
  presetNames: Array.from({ length: 10 }, (_, i) => names[i] ?? ""),
});

test("WB6e: a recall asks only when something unsaved would be lost", () => {
  assert.equal(recallStep(0, 0, false), "nothing");
  assert.equal(recallStep(0, 0, true), "confirm-reload");
  assert.equal(recallStep(3, 0, false), "load");
  assert.equal(recallStep(3, 0, true), "ask-unsaved");
});

test("WB6e: Store says what it replaces", () => {
  const s = slots(0, 0b11, ["Demo", "Stage"]);
  assert.equal(storeText(s, 4), "Store the current settings in slot 5?");
  assert.equal(storeText(s, 0), "Store the current settings in slot 1, “Demo”? What it held is replaced.");
  assert.equal(storeText(s, 1), "Store the current settings in slot 2? “Stage” is replaced; this cannot be undone.");
});

// -- The verbs on the real core (as main.test.ts) --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(access: "control" | "view" = "control") {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const calls: { m: string; p: unknown }[] = [];
  const client = {
    call: (m: never, p: never) => {
      calls.push({ m, p });
      return core.call(m, p);
    },
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, access, client);
  await session.open({ kind: "demo" });
  return { session, calls };
}

/** Answers the questions in order, recording their titles. */
function asker(answers: string[]) {
  const asked: Question[] = [];
  return { asked, ask: async (q: Question) => (asked.push(q), answers.shift() ?? "cancel") };
}

async function settle(session: Session) {
  // The session asks the core what is unsaved after each snapshot; let that answer land.
  for (let i = 0; i < 20 && session.dirty !== ((await session.core.call("presetChanges", undefined)).dirty); i++)
    await new Promise((r) => setTimeout(r, 5));
}

test("WB6e: Recall loads another slot through the core; with unsaved changes the prompt decides", { skip }, async () => {
  const { session, calls } = await sessionOn();
  const st = () => session.state.snapshot!.state;
  const start = st().activePresetSlot;
  const other = start === 1 ? 2 : 1;
  const token = await session.core.call("confirmToken", { op: "copy_preset" });
  assert.equal(await session.change("presetCopy", { source: start, dest: other, token }), true);
  const { ask, asked } = asker([]);

  // Nothing unsaved: it loads with no question.
  const loads = () => calls.filter((c) => c.m === "write" && (c.p as { path: string }).path === "preset.load").length;
  let prompted = 0;
  const choose = (a: Unsaved) => async () => (prompted++, a);
  assert.equal(await recall({ session, ask, unsaved: choose("clean") }, other), true);
  assert.equal(st().activePresetSlot, other);
  assert.equal(prompted, 0);
  assert.equal(loads(), 1);
  // The active slot, clean: nothing to do, nothing written.
  assert.equal(await recall({ session, ask, unsaved: choose("clean") }, other), false);
  assert.equal(loads(), 1);

  // An unsaved change: Cancel keeps it and the slot.
  assert.equal(await session.write("out.gain", { float: -6 }, [0]), true);
  await settle(session);
  assert.equal(session.dirty, true);
  assert.equal(await recall({ session, ask, unsaved: choose("cancel") }, start), false);
  assert.equal(prompted, 1);
  assert.equal(st().activePresetSlot, other);
  assert.equal(st().outputGainDb[0], -6);

  // Save: stored in the slot left (with its token), then the load.
  assert.equal(await recall({ session, ask, unsaved: choose("save") }, start), true);
  assert.equal(st().activePresetSlot, start);
  assert.ok(calls.some((c) => c.m === "write" && (c.p as { path: string; token?: string }).path === "preset.save" && !!(c.p as { token?: string }).token));
  assert.equal(st().outputGainDb[0], 0, "the slot recalled has its own gain");

  // Discard: the change goes with the load.
  assert.equal(await session.write("out.gain", { float: -3 }, [0]), true);
  await settle(session);
  assert.equal(await recall({ session, ask, unsaved: choose("discard") }, other), true);
  assert.equal(st().activePresetSlot, other);
  assert.equal(st().outputGainDb[0], -6, "what Save stored in that slot");
  assert.equal(asked.length, 0, "another slot asks through the unsaved-changes prompt only");
});

test("WB6e: recalling the active slot with unsaved changes asks, then reloads it", { skip }, async () => {
  const { session } = await sessionOn();
  const st = () => session.state.snapshot!.state;
  const active = st().activePresetSlot;
  assert.equal(await session.write("out.gain", { float: -9 }, [0]), true);
  await settle(session);
  const never = async (): Promise<Unsaved> => {
    throw new Error("not the prompt");
  };
  const no = asker(["cancel"]);
  assert.equal(await recall({ session, ask: no.ask, unsaved: never }, active), false);
  assert.equal(no.asked[0]!.title, "Recall Preset");
  assert.equal(st().outputGainDb[0], -9);
  const yes = asker(["recall"]);
  assert.equal(await recall({ session, ask: yes.ask, unsaved: never }, active), true);
  assert.equal(st().outputGainDb[0], 0);
  assert.equal(st().activePresetSlot, active);
});

test("WB6e: Store asks, names an empty slot, stores the live settings and makes it active", { skip }, async () => {
  const { session, calls } = await sessionOn();
  const st = () => session.state.snapshot!.state;
  const unsaved = async (): Promise<Unsaved> => "clean";
  const empty = [...Array(10).keys()].find((i) => (st().presetOccupied & (1 << i)) === 0)!;
  assert.equal(await session.write("out.gain", { float: -4.5 }, [1]), true);

  const no = asker(["cancel"]);
  assert.equal(await store({ session, ask: no.ask, unsaved }, empty), false);
  assert.equal(no.asked[0]!.title, "Store Preset");
  assert.equal(st().presetOccupied & (1 << empty), 0);

  const yes = asker(["store"]);
  assert.equal(await store({ session, ask: yes.ask, unsaved }, empty), true);
  assert.notEqual(st().presetOccupied & (1 << empty), 0);
  assert.equal(st().activePresetSlot, empty);
  assert.equal(st().presetNames[empty], `Preset ${empty + 1}`);
  const save = calls.find((c) => c.m === "write" && (c.p as { path: string }).path === "preset.save");
  assert.ok(save && (save.p as { token?: string }).token, "the save carries its confirmation token");
  assert.equal((await session.core.call("presetChanges", undefined)).dirty, false);

  // Into an occupied slot: replaced, and the question says so.
  const other = empty === 0 ? 1 : 0;
  const over = asker(["store"]);
  assert.equal(await store({ session, ask: over.ask, unsaved }, other), true);
  assert.match(String(over.asked[0]!.body), /is replaced/);
  assert.equal(st().activePresetSlot, other);
});

test("WB6e: read only, neither verb reaches the core", { skip }, async () => {
  const { session, calls } = await sessionOn("view");
  assert.ok(session.readOnlyReason);
  const before = calls.length;
  const a = asker(["store", "recall"]);
  const unsaved = async (): Promise<Unsaved> => "discard";
  assert.equal(await recall({ session, ask: a.ask, unsaved }, 3), false);
  assert.equal(await store({ session, ask: a.ask, unsaved }, 3), false);
  assert.equal(a.asked.length, 0);
  assert.equal(calls.length, before);
});
