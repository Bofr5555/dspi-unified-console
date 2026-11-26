// WP GAP2 unit tests: "Solo this output" on the demo through the real core. The store follows the core's
// solo; the state keeps the user's mutes; a save, a load or the core's own end leave it at the next snapshot.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { soloOf } from "../../src/ui/solo";

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demoSession(): Promise<{ session: Session; core: WasmCore }> {
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
  return { session, core };
}

const settled = () => new Promise((r) => setTimeout(r, 50));
const muted = (s: Session) => (s.state.snapshot?.state.outputMuted ?? []).filter(Boolean).length;

test("GAP2 solo: the store follows the core; the state keeps the user's mutes", { skip, timeout: 60_000 }, async () => {
  const { session, core } = await demoSession();
  const solo = soloOf(session);
  assert.equal(soloOf(session), solo, "one store per session");
  await settled();
  assert.equal(solo.output, null);
  assert.equal(solo.reason, "");
  const before = muted(session);

  await solo.toggle(2);
  assert.equal(solo.output, 2);
  assert.equal((await core.call("soloState", undefined)).output, 2);
  assert.equal(muted(session), before, "no mute of the solo's in the state");
  assert.equal(session.state.unsaved.length, 0, "nothing unsaved");

  await solo.solo(4);
  assert.equal(solo.output, 4, "another output moves it");
  await solo.toggle(4);
  assert.equal(solo.output, null, "the soloed one again ends it");

  // The core ends it by itself (a save, a load): the next snapshot says so
  await solo.solo(1);
  await core.call("soloEnd", undefined);
  await session.refresh();
  await settled();
  assert.equal(solo.output, null);

  // A refusal is the session's error line
  await solo.solo(99);
  assert.equal(solo.output, null);
  assert.match(session.state.error, /out of range/i);
});
