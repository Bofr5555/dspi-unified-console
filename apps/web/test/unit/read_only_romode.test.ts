// WP ROMODE unit tests: a view-only session (`?readonly`, a fleet's view access) puts its core
// in the client read-only mode before anything opens, a control session does not; on the real
// core a write that got past the page is refused by the core with the mode's notice row, and
// reads go on.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { CoreCallError } from "../../src/core/protocol";
import { Session } from "../../src/ui/session";

/** A client that records each call and answers nothing useful (the open fails). */
function recorder() {
  const calls: { method: string; params: unknown }[] = [];
  const client = {
    call: async (method: string, params: unknown) => {
      calls.push({ method, params });
      throw new CoreCallError("unknown_method", `unknown method ${method}`);
    },
    subscribe: () => () => undefined,
    dispose: async () => undefined,
  } as unknown as CoreClient;
  return { client, calls };
}

test("ROMODE: a view session turns the core's read-only mode on first, a control session never", async () => {
  for (const [access, want] of [
    ["view", true],
    ["control", false],
  ] as const) {
    const device = recorder();
    await new Session("s", "i", "Here", { kind: "demo" }, access, device.client).open({ kind: "demo" });
    assert.equal(device.calls[0]?.method === "setClientReadOnly", want, `${access}: ${JSON.stringify(device.calls)}`);
    if (want) assert.deepEqual(device.calls[0]?.params, { on: true });
    assert.equal(
      device.calls.some((c) => c.method === "setClientReadOnly"),
      want,
    );
    // A document opened with no device: the same rule.
    const doc = recorder();
    await new Session("d", "files", "File", { kind: "document", fileName: "x" }, access, doc.client).openDocument("{}", "x");
    assert.equal(doc.calls[0]?.method === "setClientReadOnly", want, `${access} document`);
  }
});

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function sessionOn(access: "control" | "view"): Promise<Session> {
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

test("ROMODE: on the real core a viewer's write is refused by the core too, in the mode's words; reads go on", { skip }, async () => {
  const s = await sessionOn("view");
  assert.equal(s.state.status, "open");
  assert.deepEqual(await s.core.call("clientReadOnly", undefined), { on: true, refusals: 0 });
  const before = (await s.core.call("state", undefined)).state.userVolumeDb;
  await assert.rejects(
    s.core.call("write", { path: "vol.user", value: { float: -10 } }),
    (e: unknown) => e instanceof CoreCallError && e.code === "read_only" && e.noticeKey === "read_only.client",
  );
  assert.equal((await s.core.call("state", undefined)).state.userVolumeDb, before, "the device did not change");
  assert.equal((await s.core.call("clientReadOnly", undefined)).refusals, 1);
  await s.core.call("meters", undefined);

  const c = await sessionOn("control");
  assert.deepEqual(await c.core.call("clientReadOnly", undefined), { on: false, refusals: 0 });
  assert.equal(await c.write("vol.user", { float: -10 }), true);
});
