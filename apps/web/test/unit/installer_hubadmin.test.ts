// WP HUBADMIN unit tests: the Installer page's rules for the hub's copy (which copy is edited,
// who may edit it, the pinned-mode choosers, the two PIN fields, what the lock covers as the
// profile's `lockedPages`), the session taking the hub's admin events (the lock from the event,
// a new read of the page), and the page's own per-serial copy never applied through a hub. On
// the real core: a demo (direct) DSPi edits this console's copy, and the hub calls say
// `dlink.no_limits` there.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  HUB_ROLES,
  editsHubCopy,
  hubEditable,
  hubLockLine,
  pinPairProblem,
  pinnedModeChoices,
  pinnedModeOf,
  pinnedRoles,
  saveProfile,
  withLockedPage,
} from "../../src/screens/installer/model";
import { CoreCallError, type CoreEvent, type HubAdminEventRecord, type LockStateRecord } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import type { CoreClient } from "../../src/core/client";
import type { KeyValueStore } from "../../src/fleet/local";
import { Session } from "../../src/ui/session";
import { noticeRow } from "../../src/ui/texts";

const hubTarget = (editable: boolean, role: string) => ({
  copy: "hub",
  editable,
  serial: "S1",
  role,
  noticeKey: "installer.hub_copy",
  readOnlyKey: editable ? "" : "installer.hub_role",
});

test("HUBADMIN model: the hub's copy is edited through a hub, by an Admin only (the core's word and the hub's)", () => {
  assert.equal(editsHubCopy(hubTarget(true, "admin")), true);
  assert.equal(editsHubCopy({ copy: "console" }), false);
  assert.equal(editsHubCopy({ copy: "none" }), false);
  assert.equal(editsHubCopy(null), false);
  assert.equal(hubEditable(hubTarget(true, "admin"), null), true, "before the hub's answer: the target's word");
  assert.equal(hubEditable(hubTarget(true, "admin"), { canEdit: true }), true);
  assert.equal(hubEditable(hubTarget(true, "admin"), { canEdit: false }), false, "the hub's live word wins");
  assert.equal(hubEditable(hubTarget(false, "control"), { canEdit: false }), false);
  assert.equal(hubEditable({ copy: "console", editable: true }, null), false, "this console's copy is not the hub's");
});

test("HUBADMIN model: a chooser per role (none, simple, advanced, installer); an Admin sees all three roles", () => {
  const choices = pinnedModeChoices(null);
  assert.deepEqual(
    choices.map((c) => c.value),
    ["", "simple", "advanced", "installer"],
  );
  assert.equal(choices[0]!.label, "None");
  const fromManifest = pinnedModeChoices([
    { id: "simple", label: "Simple" },
    { id: "advanced", label: "Advanced" },
    { id: "installer", label: "Installer" },
  ]);
  assert.deepEqual(fromManifest.map((c) => c.label), ["None", "Simple", "Advanced", "Installer"]);
  const state = { pinnedModes: [{ role: "control", mode: "simple" }] };
  assert.equal(pinnedModeOf(state, "control"), "simple");
  assert.equal(pinnedModeOf(state, "viewer"), "");
  assert.equal(pinnedModeOf(null, "admin"), "");
  assert.deepEqual(pinnedRoles(state, true), [...HUB_ROLES]);
  assert.deepEqual(pinnedRoles({ pinnedModes: [] }, false, "control"), ["control"], "below Admin: its own role");
  assert.deepEqual(pinnedRoles(state, false, "viewer"), ["viewer", "control"]);
});

test("HUBADMIN model: the two PIN fields must match; the core judges the PIN", () => {
  assert.notEqual(pinPairProblem("", ""), "");
  assert.notEqual(pinPairProblem("1234", "1243"), "");
  assert.equal(pinPairProblem("1234", "1234"), "");
  assert.equal(pinPairProblem("12", "12"), "", "too short is the core's refusal, in its words");
});

test("HUBADMIN model: what the lock covers is the profile's lockedPages, in the manifest's order", () => {
  const order = ["main", "channels", "eq", "system"];
  let p = withLockedPage({ note: "n" }, order, "eq", true);
  assert.deepEqual(p.lockedPages, ["eq"]);
  p = withLockedPage(p, order, "main", true);
  assert.deepEqual(p.lockedPages, ["main", "eq"]);
  assert.equal(p.note, "n", "the rest of the profile stays");
  p = withLockedPage({ ...p, lockedPages: [...p.lockedPages!, "future"] }, order, "main", false);
  assert.deepEqual(p.lockedPages, ["eq", "future"], "a page this manifest does not know is kept");
  p = withLockedPage({ lockedPages: ["eq"] }, order, "eq", false);
  assert.equal(p.lockedPages, undefined, "none: left out");
});

test("HUBADMIN model: the hub's lock line", () => {
  assert.equal(hubLockLine(null, 0), "");
  assert.match(hubLockLine({ pinSet: false, locked: false, unlocked: false }, 0), /No installer PIN/);
  assert.match(hubLockLine({ pinSet: true, locked: true, unlocked: false }, 1), /lock is on/);
  assert.match(hubLockLine({ pinSet: true, locked: false, unlocked: true }, 1), /unlocked/);
  assert.match(hubLockLine({ pinSet: true, locked: false, unlocked: false }, 0), /covers nothing/);
});

// -- The session --

const lock = (over: Partial<LockStateRecord>): LockStateRecord => ({
  serial: "S1",
  pinSet: false,
  unlocked: false,
  locked: false,
  installerOpen: true,
  covers: [],
  allowedWrites: [],
  wrongTries: 0,
  retryAfterMs: 0,
  recovered: false,
  note: "",
  contact: "",
  scopeNoticeKey: "",
  detail: "",
  heldBy: "",
  hubPinnedMode: "",
  ...over,
});

function stub(copy: string) {
  const calls: { method: string; params: unknown }[] = [];
  let push: (e: CoreEvent) => void = () => undefined;
  const client = {
    call: (method: string, params: unknown) => {
      calls.push({ method, params });
      if (method === "open") return Promise.resolve({ device: { serial: "S1", readOnly: false, readOnlyReason: "" }, state: {} });
      if (method === "installerTarget") return Promise.resolve({ ...hubTarget(true, "admin"), copy });
      if (method === "installerLockRefresh") return Promise.resolve(lock({}));
      if (method === "limitsApply") return Promise.resolve({ lines: [], held: [], notes: [] });
      return Promise.reject(new CoreCallError("unsupported", "not in this stub"));
    },
    subscribe: (fn: (e: CoreEvent) => void) => {
      push = fn;
      return () => undefined;
    },
    dispose: async () => undefined,
  } as unknown as CoreClient;
  return { calls, client, push: (e: CoreEvent) => push(e) };
}

/** A store for the page's per-serial copy (as `safeLocalStorage` gives one). */
function withStorage(fn: () => Promise<void>): Promise<void> {
  const data = new Map<string, string>();
  const store: KeyValueStore = { getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v) };
  const g = globalThis as unknown as { localStorage?: unknown };
  const before = g.localStorage;
  g.localStorage = store;
  saveProfile(store, "S1", JSON.stringify({ schemaVersion: 1, meta: { kind: "limits" }, limits: { maxPeqBoostDb: 3 } }));
  return fn().finally(() => {
    g.localStorage = before;
  });
}

test("HUBADMIN session: this page's per-serial copy is applied to a direct DSPi only, never through a hub", async () => {
  await withStorage(async () => {
    const direct = stub("console");
    await new Session("a", "i", "Here", { kind: "demo" }, "control", direct.client).open({ kind: "demo" });
    assert.ok(direct.calls.some((c) => c.method === "limitsApply"), "USB: as before");
    const viaHub = stub("hub");
    await new Session("b", "i", "Here", { kind: "demo" }, "control", viaHub.client).open({ kind: "demo" });
    assert.ok(!viaHub.calls.some((c) => c.method === "limitsApply"), "through a hub: the hub's copy is in force");
  });
});

test("HUBADMIN session: a hub admin event gives the lock from the event and a new read of the page", async () => {
  const s = stub("hub");
  const session = new Session("c", "i", "Here", { kind: "demo" }, "control", s.client);
  await session.open({ kind: "demo" });
  const rev = session.state.hubAdminRev ?? 0;
  const lockRev = session.state.lockRev ?? 0;
  const ev = (serial: string, l: LockStateRecord, kind = "limits.changed"): HubAdminEventRecord => ({ kind, serial, rows: [], role: "", mode: "", lock: l });
  // Another device's event: nothing.
  s.push({ event: "hubAdmin", events: [ev("OTHER", lock({ pinSet: true, locked: true }))] });
  assert.equal(session.state.hubAdminRev ?? 0, rev);
  // This device's: the last event's lock, and a new read.
  s.push({ event: "hubAdmin", events: [ev("S1", lock({ pinSet: true })), ev("S1", lock({ pinSet: true, locked: true, heldBy: "hub" }), "limits.held")] });
  assert.equal(session.state.hubAdminRev, rev + 1);
  assert.equal(session.state.lockRev, lockRev + 1, "the mode is resolved again");
  assert.equal(session.state.lock?.locked, true);
  assert.equal(session.state.lock?.heldBy, "hub");
  assert.equal(session.state.hubAdminEvent?.kind, "limits.held");
});

// -- On the real core --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

test("HUBADMIN core: the demo (direct) edits this console's copy; the hub calls say the hub keeps no limits", { skip }, async () => {
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
  const t = await session.core.call("installerTarget", undefined);
  assert.equal(t.copy, "console");
  assert.equal(t.editable, true);
  assert.equal(t.noticeKey, "installer.console_copy");
  for (const key of ["installer.hub_copy", "installer.console_copy", "installer.hub_role", "dlink.no_limits", "installer_lock.hub_recovery"]) {
    assert.ok(noticeRow(key), `the core's row ${key}`);
  }
  await assert.rejects(session.core.call("hubLimitsGet", undefined), (e: unknown) => e instanceof CoreCallError && e.noticeKey === "dlink.no_limits");
  assert.deepEqual(await session.core.call("hubAdminEvents", undefined), []);
});
