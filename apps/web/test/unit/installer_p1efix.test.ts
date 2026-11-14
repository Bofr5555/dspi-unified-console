// WP P1EFIX unit tests (review p1e-a fixups): tokens are required before the session shows as
// open (F5); on the real core, the locked-user line is the core's row `installer_lock.banner`
// (F2), the limits editor's ranges are the core's one table (F7), and undoing Protect all
// outputs says the limiters stay on.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { LOCK_BANNER_KEY, fieldRange, lockLine, resetFieldRanges } from "../../src/screens/installer/model";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";
import { noticeRow } from "../../src/ui/texts";

test("P1EFIX tokens: required before the session shows as open (a device and a document)", async () => {
  const seen: { method: string; status: string }[] = [];
  let session: Session | null = null;
  const snapshot = { device: { serial: "X", readOnly: false, readOnlyReason: "" }, state: {} };
  const client = {
    call: (method: string) => {
      seen.push({ method, status: session?.state.status ?? "" });
      if (method === "open") return Promise.resolve(snapshot);
      if (method === "documentOpen") return Promise.resolve({ ...snapshot, document: { fileName: "a.dspipreset" } });
      if (method === "requireConfirmation") return Promise.resolve(true);
      return Promise.reject(new Error("not in this stub"));
    },
    subscribe: () => () => undefined,
    dispose: async () => undefined,
  } as unknown as CoreClient;
  session = new Session("s", "i", "Here", { kind: "demo" }, "control", client);
  await session.open({ kind: "demo" });
  assert.equal(session.state.status, "open");
  const req = seen.find((c) => c.method === "requireConfirmation");
  assert.ok(req, "tokens required");
  assert.notEqual(req.status, "open", "before the session shows as open");

  seen.length = 0;
  const doc = new Session("d", "i", "Here", { kind: "demo" }, "control", client);
  session = doc;
  await doc.openDocument("{}", "a.dspipreset");
  assert.equal(doc.state.status, "open");
  const reqDoc = seen.find((c) => c.method === "requireConfirmation");
  assert.ok(reqDoc && reqDoc.status !== "open", "a document too");
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

test("P1EFIX lock: the locked-user line is the core's banner row", { skip }, async () => {
  await sessionOn();
  const row = noticeRow(LOCK_BANNER_KEY);
  assert.equal(row?.reason, "Locked by the installer: only volume, mute and presets can be changed.");
  const line = lockLine({ pinSet: true, heldBy: "hub", locked: true, unlocked: false, covers: ["channels"] });
  assert.equal(line, row?.reason);
  assert.ok(!/\p{Extended_Pictographic}/u.test(line), "no glyphs");
});

test("P1EFIX limits: the editor's ranges are the core's table, read after the open", { skip }, async () => {
  resetFieldRanges();
  assert.equal(fieldRange("maxGainDb"), null);
  const s = await sessionOn();
  const table = await s.core.call("limitsFieldRanges", undefined);
  for (const r of table) assert.deepEqual(fieldRange(r.key), r);
  assert.equal(fieldRange("maxGainDb")?.max, 20);
  assert.equal(fieldRange("masterVolumeMaxDb")?.min, -128);
  assert.equal(fieldRange("delayMaxMs")?.unit, "ms");
});

test("P1EFIX protect all: one undo says the limiters stay on", { skip }, async () => {
  const s = await sessionOn();
  assert.equal(await s.write("limit.on", { bool: false }, [255]), true);
  assert.equal(await s.change("protectAllOutputs", undefined), true);
  await s.undo();
  assert.match(s.state.notice?.text ?? "", /^The limiters stay on/);
  const n = s.state.snapshot!.state.numOutputChannels;
  assert.ok(s.state.snapshot!.state.limiterEnabled.slice(0, n).every(Boolean));
});
