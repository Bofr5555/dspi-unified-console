// -- FIX016 -- (REVG F3) a solo outlives a reload or a closed tab on a hub or a USB DSPi (the core cannot lift it
// on a connection lost without a close), so while one stands the page holds the browser's leave question and a
// `pagehide` ends it. Over the real core's demo; the page's window is an EventTarget of the test's own (each
// unit file runs in its own Node process).

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

const page = new EventTarget();
(globalThis as unknown as { window: EventTarget }).window = page;

async function demoSession(): Promise<{ session: Session; core: WasmCore; asked: string[] }> {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const asked: string[] = [];
  const client = {
    call: (m: never, p: never) => {
      asked.push(m);
      return core.call(m, p);
    },
    subscribe: (fn: never) => {
      core.onEvent(fn);
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  const session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo" });
  return { session, core, asked };
}

const settled = () => new Promise((r) => setTimeout(r, 50));

/** Whether leaving the page now is held (the browser would ask). */
function held(): boolean {
  const e = new Event("beforeunload", { cancelable: true });
  page.dispatchEvent(e);
  return e.defaultPrevented;
}

test("FIX016 F3: leaving is held only while soloed; a pagehide during a solo ends it", { skip, timeout: 60_000 }, async () => {
  const { session, core, asked } = await demoSession();
  const solo = soloOf(session);
  await settled();
  assert.equal(solo.output, null);
  assert.equal(held(), false, "no solo: the page goes without a question");

  await solo.solo(2);
  assert.equal(solo.output, 2);
  assert.equal(held(), true, "soloed: the browser asks first");

  // The page goes all the same: the solo ends on the way out
  asked.length = 0;
  page.dispatchEvent(new Event("pagehide"));
  assert.ok(asked.includes("soloEnd"), `soloEnd asked: ${asked.join(", ")}`);
  await settled();
  assert.equal((await core.call("soloState", undefined)).output, null, "the core's solo is over");

  // Its next snapshot says so; nothing holds the page any more
  await session.refresh();
  await settled();
  assert.equal(solo.output, null);
  assert.equal(held(), false, "the solo is over: no question");
  asked.length = 0;
  page.dispatchEvent(new Event("pagehide"));
  assert.ok(!asked.includes("soloEnd"), "no solo, nothing to end");

  // Ended in the page: the hold goes too
  await solo.solo(1);
  assert.equal(held(), true);
  await solo.end();
  assert.equal(held(), false);
});
