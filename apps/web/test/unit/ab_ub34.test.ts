// WP UB34 unit tests (UXP B3, owner D7): the top bar's A | B on the web. Its rules and words, and
// on the real core (as snapshots_p1wb1.test.ts) "as saved" against "now" from the first edit: the
// B key starts it on A, B goes back, a save on B ends it.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { SnapshotDesk, mayStartSaved, type DeskState } from "../../src/screens/snapshots/model";
import { abTitle } from "../../src/ui/shell/AbSegment";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import { Session } from "../../src/ui/session";

const desk = (o: Partial<DeskState> = {}): DeskState =>
  ({
    docs: [],
    ab: { active: false, side: null, aName: "", bName: "", recall: null },
    guard: null,
    guardDefault: false,
    busy: "",
    outcome: null,
    abDiff: null,
    timeoutS: 30,
    rollback: false,
    waitingText: "",
    backstopText: "",
    ...o,
  }) as DeskState;

test("UB34: as saved against now is offered from the first edit, never with a compare, a waiting guard or read-only", () => {
  assert.equal(mayStartSaved(desk(), false, 0), false, "nothing unsaved: A and B are the same");
  assert.equal(mayStartSaved(desk(), false, 1), true);
  assert.equal(mayStartSaved(desk(), true, 3), false, "a viewer");
  assert.equal(mayStartSaved(desk({ busy: "Compare" }), false, 3), false);
  assert.equal(mayStartSaved(desk({ guard: { phase: "waiting" } as DeskState["guard"] }), false, 3), false);
  assert.equal(mayStartSaved(desk({ ab: { active: true, side: "a", aName: "x", bName: "y", recall: null } }), false, 3), false);
});

test("UB34: the tooltip says what A and B are", () => {
  assert.equal(abTitle(desk(), 0, "B"), "");
  assert.match(abTitle(desk(), 2, "B"), /^A\/B compare: A is the preset as saved on the device, B is now, with your unsaved changes\. Click A, or press B,/);
  const on = (side: "a" | "b", saved: boolean) => desk({ ab: { active: true, side, aName: "Loud", bName: "Soft", recall: null, saved } });
  assert.match(abTitle(on("a", true), 0, "B"), /you hear A, the preset as saved on the device/);
  assert.match(abTitle(on("b", true), 2, "B"), /you hear B, now, with your changes/);
  assert.match(abTitle(on("a", false), 0, "B"), /A is “Loud”, B is “Soft”/);
});

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
const desks: SnapshotDesk[] = [];
after(() => {
  for (const d of desks) d.stop();
  for (const c of made) c.dispose();
});

async function deskOn() {
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
  const d = new SnapshotDesk(session, 20);
  desks.push(d);
  return { session, desk: d };
}

const gain0 = (s: Session) => s.state.snapshot!.state.outputGainDb[0];
const volume = (s: Session) => s.state.snapshot!.state.userVolumeDb;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until(what: string, f: () => boolean, ms = 8000) {
  const end = Date.now() + ms;
  while (!f()) {
    if (Date.now() > end) assert.fail(`timed out waiting for ${what}`);
    await sleep(20);
  }
}

test("UB34: the B key with an edit hears the saved preset, B goes back, a save on B ends the compare", { skip }, async () => {
  const { session, desk: d } = await deskOn();
  await session.open({ kind: "demo" });
  await d.start();
  const saved = gain0(session);
  assert.equal(await d.abToggle(), false, "nothing unsaved: the key does nothing");
  assert.equal(await session.write("vol.user", { float: -20 }), true);
  assert.equal(await session.write("out.gain", { float: saved - 6 }, [0]), true);
  await until("the edit counted", () => session.state.unsaved.length > 0);
  assert.equal(await d.abToggle(), true, "the B key starts as saved against now");
  assert.equal(d.state.ab.active && d.state.ab.saved && d.state.ab.side, "a");
  assert.equal(gain0(session), saved, "A is the saved gain");
  assert.equal(volume(session), -20, "the listening volume stays");
  assert.equal(await d.abToggle(), true);
  assert.equal(d.state.ab.side, "b");
  assert.equal(gain0(session), saved - 6, "B is the edit");
  await session.savePreset(session.state.snapshot!.state.activePresetSlot);
  await until("a save on B ends it", () => !d.state.ab.active);
  assert.equal(gain0(session), saved - 6);
});
