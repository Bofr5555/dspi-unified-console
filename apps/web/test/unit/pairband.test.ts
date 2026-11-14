// WP PAIRBAND unit tests on the real core (as installer_p1wb2.test.ts): a linked
// input pair's band through `inputPairFilter` is one undo step that moves both
// sides, a drag's ticks join one step, and the bypass through
// `inputPairBandBypass` is one step too. The web has no Link Inputs control yet;
// these are the calls it will use.
import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import type { FilterParams } from "../../src/core/protocol";
import { Session } from "../../src/ui/session";

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

const sides = (s: Session, b: number): [FilterParams, FilterParams] => {
  const f = s.state.snapshot!.state.filters;
  return [f[0]![b]!, f[1]![b]!];
};

test("PAIRBAND: a linked pair's band edit, its drag and its bypass are one undo step each, both sides together", { skip }, async () => {
  const s = await sessionOn();
  const band = (freq: number, gain: number) => ({ input: 0, band: 1, filterType: 1, freq, q: 1, gain, bypass: false, qp: 0.707 });
  await s.refresh();
  const before = sides(s, 1);
  assert.equal(await s.change("inputPairFilter", band(1200, 4)), true);
  const [l, r] = sides(s, 1);
  assert.equal(l.filterType, 1);
  assert.deepEqual(l, r, "both sides written");
  await s.undo();
  assert.deepEqual(sides(s, 1), before, "one undo, both sides back");

  // A drag: every tick through the pair call, one step
  assert.equal(await s.change("inputPairFilter", band(1000, 0)), true);
  const start = sides(s, 1);
  await s.core.call("setDragging", { dragging: true });
  for (let i = 1; i <= 5; i++) assert.equal(await s.change("inputPairFilter", band(1000 + 100 * i, i)), true);
  await s.core.call("setDragging", { dragging: false });
  assert.equal(sides(s, 1)[1].freq, 1500);
  await s.undo();
  assert.deepEqual(sides(s, 1), start, "one undo puts the whole drag back on both sides");

  // The bypass
  assert.equal(await s.change("inputPairBandBypass", { input: 1, band: 1, bypass: true }), true);
  assert.ok(sides(s, 1).every((p) => p.bypass));
  await s.undo();
  assert.ok(sides(s, 1).every((p) => !p.bypass), "one undo enables both sides");
});
