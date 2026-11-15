// WP WB6K unit tests: the Files screen's model on the real core (as main.test.ts): filter files
// exported, inspected and imported as one undo step; Save Master Volume and Save Output
// Configuration; the AutoEQ database the page keeps (built-in, Import File, Download through the
// page's own GET with the core reading its answer, Reset to Built-in), its search and entries, the
// favourites; and the `.dspipreset`'s input pair-link mask both ways. A local fixture, no network.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CoreClient } from "../../src/core/client";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { Session } from "../../src/ui/session";
import { inputLinks } from "../../src/screens/inputs/model";
import { SnapshotDesk, restoreLinks } from "../../src/screens/snapshots/model";
import {
  AutoEqLibrary,
  DATABASE_KEY,
  FAVOURITES_KEY,
  MemoryTextStore,
  chosenWires,
  databaseLine,
  downloadAddress,
  exportFilters,
  importFilters,
  inspectFilters,
  inspectProblem,
  masterVolumeWords,
  saveMasterVolume,
  saveOutputConfig,
  type PageResponse,
} from "../../src/screens/files/model";

const FIXTURE = readFileSync(new URL("../../test/pages/autoeq-fixture.json", import.meta.url), "utf8");
const BUILT_IN = JSON.stringify({
  version: 1,
  generatedAt: "2025-01-01T00:00:00.000Z",
  entryCount: 1,
  entries: [{ id: "oratory1990/Built In", manufacturer: "Built", model: "In", source: "oratory1990", preamp: -1, filters: [{ type: "PK", freq: 100, q: 1, gain: 1 }] }],
});

test("WB6K: the small rules (no core)", () => {
  assert.equal(masterVolumeWords(-12.34), "-12.3 dB");
  assert.equal(masterVolumeWords(-128), "−∞ dB (mute)");
  assert.equal(downloadAddress("https://example.invalid/db.json"), "https://example.invalid/db.json");
  assert.equal(downloadAddress("ftp://example.invalid/db.json"), null);
  assert.equal(downloadAddress("javascript:alert(1)"), null);
  assert.deepEqual(chosenWires([{ wire: 0 }, { wire: 3 }, { wire: 1 }], new Set([1, 3])), [3, 1]);
  assert.equal(inspectProblem({ kind: "error", channels: [], error: "too large" }), "too large");
  assert.match(inspectProblem({ kind: "rew", channels: [] }), /No filters/);
  assert.equal(inspectProblem({ kind: "native", channels: [{ wire: 0, label: "L", bands: 10, checked: true }] }), "");
  assert.equal(databaseLine(null), "No AutoEQ database is loaded yet.");
  assert.equal(databaseLine({ hasUserDatabase: true, problem: null, version: 1, generatedAt: "2026-03-04T05:06:07.000Z", entryCount: 3 }), "Current database: 2026-03-04, 3 profiles (your own).");
});

// -- On the real core --

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
const desks: SnapshotDesk[] = [];
after(() => {
  for (const d of desks) d.stop();
  for (const c of made) c.dispose();
});

async function deskOn(access: "control" | "view" = "control") {
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
  const desk = new SnapshotDesk(session, 20);
  desks.push(desk);
  return { session, desk, calls };
}

const band0 = (s: Session, wire: number) => s.state.snapshot!.state.filters[wire]![0]!;

test("WB6K M28: Export Filters, the picker, Import Filters on the chosen channels as one undo step", { skip }, async () => {
  const { session, desk } = await deskOn();
  await session.open({ kind: "demo" });
  const set = (channel: number, gain: number) => session.core.call("filter", { channel, band: 0, filterType: 1, freq: 1000, q: 1, gain } as never);
  await set(0, 5);
  await set(1, 3);
  const text = (await exportFilters(desk))!;
  assert.ok(text.length > 0);
  await set(0, 0);
  await set(1, -1);
  await session.refresh();
  const seen = await inspectFilters(session, text);
  assert.equal(seen.kind, "native");
  assert.equal(inspectProblem(seen), "");
  // Only channel 0 chosen: channel 1 stays as it is now.
  assert.equal(await importFilters(desk, text, "DSPi Filters.txt", [0]), true);
  assert.equal(band0(session, 0).gain, 5);
  assert.equal(band0(session, 1).gain, -1);
  assert.equal(desk.state.outcome?.title, "Imported filters from “DSPi Filters.txt”");
  // One undo step.
  await session.core.call("undo", undefined);
  await session.refresh();
  assert.equal(band0(session, 0).gain, 0);
  // A file with nothing usable is said so before anything is chosen.
  assert.match(inspectProblem(await inspectFilters(session, "hello")), /No filters/);
  // A refused import is the outcome, in the core's words.
  assert.equal(await importFilters(desk, "hello", "x.txt", [0]), false);
  assert.equal(desk.state.outcome?.tone, "error");
});

test("WB6K M28: a viewer cannot import filters; nothing reaches the core", { skip }, async () => {
  const { session, desk, calls } = await deskOn("view");
  await session.open({ kind: "demo" });
  assert.ok(desk.readOnly);
  assert.equal(await importFilters(desk, "Filter 1: ON PK Fc 100 Hz Gain 2 dB Q 1", "a.txt", [0]), false);
  assert.ok(!calls.some((c) => c.m === "filtersImport"));
  assert.equal(desk.state.outcome?.tone, "error");
});

test("WB6K M30: Save Master Volume and Save Output Configuration (with its token)", { skip }, async () => {
  const { session, desk, calls } = await deskOn();
  await session.open({ kind: "demo" });
  assert.equal(await saveMasterVolume(desk), true);
  assert.match(desk.state.outcome!.lines[0]!, /^Master volume saved \(.* dB\)\. It will be applied on next boot\.$/);
  assert.equal(await saveOutputConfig(desk), true);
  assert.equal(desk.state.outcome!.lines[0], "Output configuration saved. It will be applied on next boot.");
  const w = calls.find((c) => c.m === "write" && (c.p as { path: string }).path === "dev.save.io");
  assert.ok((w!.p as { token?: string }).token, "the gated save carries its token");
  // Without its token the core refuses the save, and the page says so.
  await assert.rejects(session.core.call("write", { path: "dev.save.io", value: "trigger" }), /confirm/i);
  // A viewer may not save; nothing reaches the core.
  const viewer = await deskOn("view");
  await viewer.session.open({ kind: "demo" });
  assert.equal(await saveMasterVolume(viewer.desk), false);
  assert.ok(!viewer.calls.some((c) => c.m === "write"));
});

class Prefs {
  map = new Map<string, string>();
  getItem(k: string) {
    return this.map.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.map.set(k, v);
  }
}

test("WB6K M31 M32: the database the page keeps: built-in, Import File, Download, Reset to Built-in; search; favourites", { skip }, async () => {
  const { session } = await deskOn();
  const store = new MemoryTextStore();
  const prefs = new Prefs();
  const asked: string[] = [];
  const answers: Record<string, PageResponse> = {
    "builtin.json": { status: 200, body: BUILT_IN },
    "https://example.invalid/db.json": { status: 200, body: FIXTURE },
    "https://example.invalid/missing.json": { status: 404, body: "" },
    "https://example.invalid/limited.json": { status: 403, body: "", rateLimitRemaining: 0, retryAfterS: 120 },
    "https://example.invalid/page.html": { status: 200, body: "<html>" },
  };
  const get = async (url: string) => {
    asked.push(url);
    const a = answers[url];
    if (!a) throw new TypeError("Failed to fetch");
    return a;
  };
  const lib = new AutoEqLibrary("builtin.json", store, prefs, get);

  // No database kept: the built-in one, fetched once.
  assert.equal(await lib.ready(session), true);
  assert.equal(lib.state.info?.hasUserDatabase, false);
  assert.equal(lib.state.info?.entryCount, 1);
  assert.equal(await lib.ready(session), true);
  assert.deepEqual(asked, ["builtin.json"]);
  assert.equal((await lib.search(session, "built")).total, 1);

  // Download: the page's GET, the core reads the answer.
  await assert.rejects(lib.download(session, "https://example.invalid/missing.json"), /status 404/);
  await assert.rejects(lib.download(session, "https://example.invalid/limited.json"), /rate limit.*about 2 minutes/);
  await assert.rejects(lib.download(session, "https://example.invalid/page.html"), /not usable|could not read/i);
  await assert.rejects(lib.download(session, "https://example.invalid/none.json"), /no answer came/);
  assert.equal(await store.get(DATABASE_KEY), null, "nothing kept after a failure");
  assert.equal(lib.state.info?.hasUserDatabase, false);
  const info = await lib.download(session, "https://example.invalid/db.json");
  assert.equal(info.hasUserDatabase, true);
  assert.equal(await store.get(DATABASE_KEY), FIXTURE);
  const found = await lib.search(session, "hd 600");
  assert.equal(found.hits[0]?.name, "Sennheiser HD 600");
  const entry = (await lib.entry(session, found.hits[0]!.id)) as { manufacturer: string };
  assert.equal(entry.manufacturer, "Sennheiser");
  // The entry is the panel's profile.
  await session.open({ kind: "demo" });
  const pv = await session.core.call("autoeqPreview", { profile: entry, left: 0, right: 1 });
  assert.equal(pv.levelTarget, "input_preamp");

  // Import File: a file that is no database is refused and the one before stays.
  await assert.rejects(lib.importFile(session, "{}"), /no profiles/);
  assert.equal(await store.get(DATABASE_KEY), FIXTURE);
  await lib.importFile(session, BUILT_IN);
  assert.equal(lib.state.info?.hasUserDatabase, true);
  assert.equal(lib.state.info?.entryCount, 1);

  // A new session of the page (another core) takes the kept database.
  const other = await deskOn();
  assert.equal(await lib.ready(other.session), true);
  assert.equal((await lib.search(other.session, "built")).total, 1);

  // Reset to Built-in: the kept file goes.
  const back = await lib.resetToBuiltIn(session);
  assert.equal(back.hasUserDatabase, false);
  assert.equal(await store.get(DATABASE_KEY), null);

  // Favourites: the heart, kept in the page's storage as the core writes the file.
  await lib.toggleFavourite(session, "oratory1990/Built In");
  await lib.toggleFavourite(session, "gone/Not Here");
  assert.deepEqual(lib.state.favourites, ["oratory1990/Built In", "gone/Not Here"]);
  assert.deepEqual(JSON.parse(prefs.getItem(FAVOURITES_KEY)!), ["oratory1990/Built In", "gone/Not Here"]);
  assert.deepEqual((await lib.favouriteHits(session)).map((h) => h.name), ["Built In"]);
  await lib.toggleFavourite(session, "gone/Not Here");
  assert.deepEqual(lib.state.favourites, ["oratory1990/Built In"]);
  await lib.clearFavourites(session);
  assert.deepEqual(lib.state.favourites, []);
  assert.deepEqual(JSON.parse(prefs.getItem(FAVOURITES_KEY)!), []);

  // A kept file that no longer reads: the built-in one is used, and Reset stays offered.
  await store.set(DATABASE_KEY, "{");
  const fresh = new AutoEqLibrary("builtin.json", store, prefs, get);
  assert.equal(await fresh.ready(session), true);
  assert.equal(fresh.state.info?.hasUserDatabase, true);
  assert.ok(fresh.state.info?.problem);
  assert.equal(fresh.state.info?.entryCount, 1);
});

test("WB6K: the .dspipreset's input pair-link mask: exported from the links, restored by an import", { skip }, async () => {
  const { session, desk } = await deskOn();
  await session.open({ kind: "demo" });
  const links = inputLinks(session);
  links.set(0, false);
  links.set(2, true);
  const text = (await desk.exportDoc("Links"))!;
  assert.deepEqual(JSON.parse(text).global.inputPairLinked, [false, true, false, false]);
  links.set(0, true);
  links.set(2, false);
  assert.equal(links.mask, 1);
  // A plain import with routing restores the document's links.
  assert.equal(await desk.importDoc(text, "Links", 0x1ff, 0xffff_ffff, false), true);
  assert.equal(links.mask, 2, JSON.stringify(desk.state.outcome));
  restoreLinks(session, null);
  assert.equal(links.mask, 2, "a document without links leaves them");
  restoreLinks(session, 0b0101);
  assert.equal(links.mask, 0b0101);
});
