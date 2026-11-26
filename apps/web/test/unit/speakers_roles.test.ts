// WP ROLES unit tests: speaker roles an AI agent set from the person's words, on the web (owner
// 2026-11-23). The layout's "stated" state, the banner and the stamp ("set by AI from your words,
// not checked"), the rows' pills (from your words, tweeter-like, an output not named), the page's
// copy keeping the statement and its checks; then on the real core (the demo): a stated layout
// kept and set again on a fresh core, its roles accepted by the measurement, a corrected role
// made the person's, and Confirm.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  NO_VIEW,
  bannerOf,
  checkOf,
  keepLayout,
  layoutState,
  loadKept,
  loadLayout,
  pillOf,
  saveKept,
  stampWords,
  tidyStatement,
  withRow,
  type LayoutCaller,
  type SpeakerLayoutRecord,
  type StatementRecord,
} from "../../src/screens/settings/speakers/model";
import { initialDraft, setupOf, withLayout } from "../../src/screens/measure/model";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { isLayoutUnconfirmed } from "../../src/screens/settings/speakers/model";

function memory() {
  const m = new Map<string, string>();
  return { map: m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

const AT = new Date(2026, 10, 24, 9, 30).getTime();
const OWNER = "L and r front speakers, both on DSPi named main front, full range three way, tweeter,mid,woofer, l then right, output 1-6";

function statement(over: Partial<StatementRecord> = {}): StatementRecord {
  return { atMs: AT, by: "AI · Claude Code", words: OWNER, whatChanged: "", deviceOutputs: 8, checks: [], ...over };
}

/** The owner's table as the agent sets it: Out 1 L tweeter ... Out 6 R woofer. */
function statedLayout(over: Partial<SpeakerLayoutRecord> = {}): SpeakerLayoutRecord {
  const roles = ["high", "mid", "low", "high", "mid", "low"] as const;
  return {
    serial: "S",
    outputs: roles.map((role, output) => ({ output, role, speaker: output < 3 ? "L" : "R", group: output < 3 ? 0 : 1, source: "stated" as const })),
    confirmedByUser: false,
    updatedMs: AT,
    notes: [],
    stated: statement(),
    ...over,
  };
}

test("wp_roles the stated state: an agent's rows from the person's words, with the words recorded", () => {
  const l = statedLayout();
  assert.equal(layoutState(l), "stated");
  // No words recorded, or a suggestion among the rows: nobody's word
  assert.equal(layoutState({ ...l, stated: null }), "unconfirmed");
  assert.equal(layoutState(withRow(l, 7, { role: "full", source: "suggested" })), "unconfirmed");
  // A row the person set beside them keeps it stated; Confirm makes it confirmed
  const mine = withRow(l, 1, { role: "low" });
  assert.equal(mine.outputs[1]!.source, "user");
  assert.equal(layoutState(mine), "stated");
  assert.equal(layoutState({ ...l, confirmedByUser: true }), "confirmed");
});

test("wp_roles the banner and the stamp: set by AI from your words, not checked", () => {
  const view = { ...NO_VIEW, stored: statedLayout(), imported: false };
  const b = bannerOf(view, AT)!;
  assert.equal(b.kind, "stated");
  assert.equal(b.text, "Your speaker roles were set by AI from your words and are not checked. Check them in Settings › Speakers.");
  assert.match(b.here, /^Set by AI from your words, not checked: Claude Code set these roles today at .+ from what you told it \(“L and r front speakers/);
  assert.match(b.here, /Check each role, correct what is wrong, then confirm\.$/);
  assert.match(stampWords(view.stored, AT), /^Set by AI from your words, not checked, today at /);
  // Long words are cut on the banner, never on the record
  const long = statedLayout({ stated: statement({ words: "w ".repeat(200) }) });
  assert.ok(bannerOf({ ...NO_VIEW, stored: long, imported: false }, AT)!.here.includes("…"));
});

test("wp_roles the rows' pills: from your words, tweeter-like after the check, an output not named", () => {
  const checks = [
    { output: 1, role: "mid" as const, verdict: "topHolds" as const, topFallDb: 0.4, floorHz: 1000, atMs: AT },
    { output: 2, role: "low" as const, verdict: "topFalls" as const, topFallDb: 21, floorHz: 1000, atMs: AT },
  ];
  const l = statedLayout({ stated: statement({ checks }) });
  assert.equal(pillOf(l, l, 0)!.kind, "stated");
  assert.equal(pillOf(l, l, 0)!.word, "from your words");
  assert.equal(pillOf(l, l, 1)!.kind, "statedTweeter");
  assert.equal(pillOf(l, l, 1)!.word, "tweeter-like");
  assert.equal(pillOf(l, l, 2)!.kind, "stated", "a top that fell keeps the stated role");
  assert.equal(pillOf(l, l, 7)!.kind, "notNamed");
  assert.equal(pillOf(l, l, 7)!.word, "unknown, treated as a tweeter");
  // The person's edit is theirs; a check stands for the role it checked only
  const edited = withRow(l, 1, { role: "low" });
  assert.equal(pillOf(edited, l, 1)!.kind, "you");
  assert.equal(checkOf(edited, 1), null);
  assert.equal(checkOf(l, 1)!.verdict, "topHolds");
  // Confirmed: no pills on the rows as kept
  const confirmed = { ...l, confirmedByUser: true, outputs: l.outputs.map((o) => ({ ...o, source: "user" as const })) };
  assert.equal(pillOf(confirmed, confirmed, 0), null);
});

test("wp_roles the page's copy keeps the statement and its checks; what does not read is dropped", () => {
  const store = memory();
  const l = statedLayout({ stated: statement({ checks: [{ output: 1, role: "mid", verdict: "topHolds", topFallDb: 0.4, floorHz: 1000, atMs: AT }] }) });
  assert.equal(saveKept(store, "S", { layout: l, imported: false }), "");
  const back = loadKept(store, "S")!.layout;
  assert.equal(layoutState(back), "stated");
  assert.deepEqual(back.stated, l.stated);
  assert.deepEqual(back.outputs.map((o) => o.source), ["stated", "stated", "stated", "stated", "stated", "stated"]);
  assert.equal(tidyStatement({ words: 3 }), null);
  assert.equal(tidyStatement(null), null);
  const odd = tidyStatement({ words: "w", checks: [{ output: 99, role: "mid" }, { output: 1, role: "mid", verdict: "nonsense" }] })!;
  assert.deepEqual(odd.checks.map((c) => [c.output, c.verdict]), [[1, "unfinished"]]);
});

// ------------------------------------------------------------------ on the real core (demo device)

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => {
  for (const c of made) c.dispose();
});

async function demo(): Promise<{ core: WasmCore; serial: string; outputs: number }> {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  const snap = await core.call("open", { kind: "demo", blank: true }); // -- WP UB12 -- a new board
  return { core, serial: snap.device.serial, outputs: snap.state.numOutputChannels };
}

const caller = (core: WasmCore) => core as unknown as LayoutCaller;

test("wp_roles on the demo: a stated layout kept and set again, its roles trusted, corrected and confirmed", { skip, timeout: 60_000 }, async () => {
  const store = memory();
  const a = await demo();
  const v0 = await loadLayout(caller(a.core), a.serial, store);
  const l = statedLayout({ serial: a.serial, outputs: statedLayout().outputs.slice(0, 2), stated: statement({ deviceOutputs: a.outputs }) });
  const v1 = await keepLayout(caller(a.core), a.serial, store, l, false, v0);
  assert.equal(layoutState(v1.stored), "stated", "the core keeps the sources and the words");
  assert.equal(v1.stored!.stated!.words, OWNER);
  assert.equal(bannerOf(v1)!.kind, "stated");
  // The page reopened: a fresh core gets the copy, still stated.
  const b = await demo();
  const v2 = await loadLayout(caller(b.core), b.serial, store);
  assert.equal(layoutState(v2.stored), "stated");
  assert.equal(v2.imported, false);
  // Its roles are trusted: the measurement does not refuse it for want of a confirmed layout.
  const setup = { ...setupOf({ ...withLayout(initialDraft([0, 1]), v2.stored), placement: "listeningPosition" }, 48_000), rolesFromLayout: true };
  const err = await b.core.call("measurePreflight" as never, { setup } as never).then(() => null, (e: unknown) => e);
  assert.ok(!isLayoutUnconfirmed(err), `not refused as unconfirmed: ${String(err)}`);
  // A corrected role is the person's; kept unconfirmed, the layout stays stated.
  const v3 = await keepLayout(caller(b.core), b.serial, store, withRow(v2.stored, 1, { role: "low" }), false, v2);
  assert.deepEqual(v3.stored!.outputs.map((o) => [o.role, o.source]), [["high", "stated"], ["low", "user"]]);
  assert.equal(layoutState(v3.stored), "stated");
  // Confirm makes every row the person's: no banner, the stamp says confirmed.
  const v4 = await keepLayout(caller(b.core), b.serial, store, v3.stored!, true, v3);
  assert.equal(layoutState(v4.stored), "confirmed");
  assert.deepEqual(v4.stored!.outputs.map((o) => o.source), ["user", "user"]);
  assert.equal(bannerOf(v4), null);
  assert.match(stampWords(v4.stored), /^Confirmed today at /);
});

// -- WP INTD -- (REVDFIX B1, the ratchet) the rows a statement replaced are the person's own; a
// tweeter among them stays one for agents. The page's copy keeps them, so a fresh core (a reload)
// gets them back with the statement; without them the reload would drop the ratchet.
const REPLACED = [{ output: 2, role: "high" as const, speaker: "L", group: 0, source: "user" as const }];

test("wp_intd the page's copy keeps the rows a statement replaced and a check's hidden top", () => {
  const store = memory();
  const l = statedLayout({
    stated: statement({ replaced: REPLACED, checks: [{ output: 1, role: "mid", verdict: "unfinished", topFallDb: null, floorHz: 1000, atMs: AT, filtersHideTop: true }] }),
  });
  assert.equal(saveKept(store, "S", { layout: l, imported: false }), "");
  const back = loadKept(store, "S")!.layout;
  assert.deepEqual(back.stated!.replaced, REPLACED);
  assert.equal(back.stated!.checks[0]!.filtersHideTop, true);
  // A row that does not read is dropped; none left, none kept
  assert.equal(tidyStatement({ words: "w", replaced: [{ output: 40, role: "high" }] })!.replaced, undefined);
});

test("wp_intd on the demo: a statement's replaced rows come back on a fresh core", { skip, timeout: 60_000 }, async () => {
  const store = memory();
  const a = await demo();
  const v0 = await loadLayout(caller(a.core), a.serial, store);
  const l = statedLayout({ serial: a.serial, outputs: statedLayout().outputs.slice(0, 3), stated: statement({ deviceOutputs: a.outputs, replaced: REPLACED }) });
  const v1 = await keepLayout(caller(a.core), a.serial, store, l, false, v0);
  assert.deepEqual(v1.stored!.stated!.replaced?.map((o) => [o.output, o.role]), [[2, "high"]]);
  // The page reopened: the core has no statement on record and takes the copy's, the ratchet with it.
  const b = await demo();
  const v2 = await loadLayout(caller(b.core), b.serial, store);
  assert.equal(layoutState(v2.stored), "stated");
  assert.deepEqual(v2.stored!.stated!.replaced?.map((o) => [o.output, o.role]), [[2, "high"]]);
});
