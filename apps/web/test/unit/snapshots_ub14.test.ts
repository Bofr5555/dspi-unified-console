// WP UB14 (B14, D14): held snapshots outlive the page: kept per DSPi serial in the browser's store
// (IndexedDB on the page; a memory text store stands in here), found again by the next desk (a reload),
// another DSPi's list apart, Delete lets one go for good; a store that refuses keeps them for the visit
// and says so. Capture no longer downloads (the screen's; the e2e checks it), Export… does.

import { test } from "node:test";
import assert from "node:assert/strict";
import { MemoryTextStore } from "../../src/screens/files/model";
import { SnapshotDesk } from "../../src/screens/snapshots/model";
import { SNAPSHOTS_KEY, SnapshotStore, memorySnapshotStore } from "../../src/screens/snapshots/store";
import type { Session } from "../../src/ui/session";

const doc = (name: string) => JSON.stringify({ meta: { kind: "snapshot", name, savedUtc: "2026-11-26T18:00:00Z" }, snapshot: { note: `${name} note` } });

function fakeSession(serial: string) {
  const watchers = new Set<() => void>();
  const s = {
    // -- INT016 -- with the state UB34's "as saved" watch reads (activePresetSlot)
    state: { status: "open", snapshot: { device: { serial }, state: { activePresetSlot: 0 } } } as Record<string, unknown>,
    deviceId: "",
    readOnlyReason: "",
    refresh: async () => undefined,
    watch(fn: () => void) {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
    /** Another DSPi in the same session (a hub's chooser). */
    open(next: string) {
      s.state = { status: "open", snapshot: { device: { serial: next }, state: { activePresetSlot: 0 } } };
      for (const w of [...watchers]) w();
    },
    core: {
      call: async (method: string, params: { name?: string } | undefined) => {
        if (method === "snapshotCapture") return { text: doc(params?.name ?? "Snapshot") };
        if (method === "configDiff") return { count: 0, rows: [] };
        if (method === "abState") return { active: false, side: null, aName: "", bName: "", recall: null };
        if (method === "applyGuardDefault") return false;
        if (method === "applyStatus") return null;
        throw new Error(`no ${method}`);
      },
    },
  };
  return s;
}

const settle = () => new Promise((r) => setTimeout(r, 0));

async function deskOn(serial: string, store: SnapshotStore) {
  const s = fakeSession(serial);
  const desk = new SnapshotDesk(s as unknown as Session, 20, store);
  await desk.start();
  await desk.loaded();
  return { s, desk };
}

test("UB14: a captured snapshot is there again after a reload (the next desk on the same DSPi)", async () => {
  const store = new SnapshotStore(new MemoryTextStore(), true);
  const one = await deskOn("E66138", store);
  assert.equal(one.desk.state.kept, "browser");
  await one.desk.capture("Evening", "warmer");
  await one.desk.addFile(doc("From a file"), "file.dspipreset");
  one.desk.stop();

  const two = await deskOn("E66138", store);
  assert.deepEqual(two.desk.state.docs.map((d) => [d.name, d.note, d.origin]), [
    ["Evening", "Evening note", "captured"],
    ["From a file", "From a file note", "file"],
  ]);
  // Delete is for good: the next visit does not find it either.
  await two.desk.remove(two.desk.state.docs[0]!.id);
  two.desk.stop();
  const three = await deskOn("E66138", store);
  assert.deepEqual(three.desk.state.docs.map((d) => d.name), ["From a file"]);
  three.desk.stop();
});

test("UB14: each DSPi has its own list; the session's DSPi changing shows the other's", async () => {
  const store = new SnapshotStore(new MemoryTextStore(), true);
  const { s, desk } = await deskOn("AAA", store);
  await desk.capture("Hall", "");
  s.open("BBB");
  await desk.loaded();
  assert.equal(desk.state.docs.length, 0);
  await desk.capture("Kitchen", "");
  s.open("AAA");
  await desk.loaded();
  assert.deepEqual(desk.state.docs.map((d) => d.name), ["Hall"]);
  desk.stop();
  assert.deepEqual((await store.list("BBB")).map((d) => d.origin), ["captured"]);
});

test("UB14: two tabs on one DSPi add to one list", async () => {
  const store = new SnapshotStore(new MemoryTextStore(), true);
  const a = await deskOn("E66138", store);
  const b = await deskOn("E66138", store);
  await a.desk.capture("A", "");
  await b.desk.capture("B", "");
  a.desk.stop();
  b.desk.stop();
  assert.deepEqual((await deskOn("E66138", store)).desk.state.docs.map((d) => d.name), ["A", "B"]);
});

test("UB14: where the browser keeps nothing, the snapshots last for the visit and the page says so", async () => {
  const mem = await deskOn("E66138", memorySnapshotStore());
  assert.equal(mem.desk.state.kept, "page");
  mem.desk.stop();
  // A store that refuses (quota, blocked site data): the snapshot is held anyway, the words change.
  const refusing = new MemoryTextStore();
  refusing.set = async () => {
    throw new Error("QuotaExceededError");
  };
  const { desk } = await deskOn("E66138", new SnapshotStore(refusing, true));
  assert.equal(desk.state.kept, "browser");
  assert.ok(await desk.capture("Evening", ""));
  assert.deepEqual(desk.state.docs.map((d) => d.name), ["Evening"]);
  assert.equal(desk.state.kept, "page");
  desk.stop();
});

test("UB14: a list the store cannot read is no list, not a failure", async () => {
  const texts = new MemoryTextStore();
  await texts.set(SNAPSHOTS_KEY + "E66138", "{not json");
  const store = new SnapshotStore(texts, true);
  assert.deepEqual(await store.list("E66138"), []);
  await settle();
});
