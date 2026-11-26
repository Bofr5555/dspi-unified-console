// WP ADETW unit tests: the speaker detection on the web (AUTODES 3.1, 5.2). The layout's three
// states, the banner and stamp of an auto-detected layout, the rows' pills (detected, detected?,
// uncertain, unknown, nothing heard, off, you, suggested), the evidence in words, the page's copy
// keeping the detection, the main screen's read-only peek, and the presence strip's detection;
// then on the real core (the demo): an auto-detected layout set, kept and set again on a fresh
// core, its roles accepted by the measurement, a corrected role made the person's, and Confirm.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  NO_VIEW,
  bannerOf,
  evidenceWords,
  keepLayout,
  layoutState,
  loadKept,
  loadLayout,
  peekLayout,
  pillOf,
  saveKept,
  stampWords,
  tidyDetection,
  withRow,
  type DetectedOutputRecord,
  type DetectionRecord,
  type LayoutCaller,
  type LayoutView,
  type SpeakerLayoutRecord,
} from "../../src/screens/settings/speakers/model";
import { DETECTING_HERE, detectingSpeakers } from "../../src/ui/presence";
import { initialDraft, setupOf, withLayout } from "../../src/screens/measure/model";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { isLayoutUnconfirmed } from "../../src/screens/settings/speakers/model";

function memory() {
  const m = new Map<string, string>();
  return { map: m, getItem: (k: string) => m.get(k) ?? null, setItem: (k: string, v: string) => void m.set(k, v) };
}

const AT = new Date(2026, 10, 23, 14, 2).getTime();

function found(output: number, over: Partial<DetectedOutputRecord> = {}): DetectedOutputRecord {
  return {
    output,
    class: "woofer",
    confidence: "high",
    role: "low",
    floorHz: 1000,
    heard: "top",
    levelDbfs: -40,
    topFallDb: 21,
    lowTurnDb: null,
    fLowHz: 45,
    lowestHz: 20,
    stoppedAtHz: null,
    note: "detect.woofer",
    probedMs: AT,
    ...over,
  };
}

function detection(outputs: DetectedOutputRecord[], over: Partial<DetectionRecord> = {}): DetectionRecord {
  return { atMs: AT, by: "Claude Code", placement: "listeningPosition", floorHz: 1000, microphone: "", outputs, stopped: false, summary: "1 tweeter, 1 woofer; nothing heard on Out 3; Out 4 off, not probed.", deviceOutputs: 4, ...over };
}

/** Out 1 woofer, Out 2 tweeter (detected), Out 3 nothing, Out 4 off. */
function detectedLayout(over: Partial<SpeakerLayoutRecord> = {}): SpeakerLayoutRecord {
  return {
    serial: "S1",
    outputs: [
      { output: 0, role: "low", speaker: "L", group: 0, source: "detected" },
      { output: 1, role: "high", speaker: "L", group: 0, source: "detected" },
    ],
    confirmedByUser: false,
    updatedMs: AT,
    notes: [],
    detected: detection([found(0), found(1, { class: "tweeter", role: "high", note: "detect.tweeter", topFallDb: 1 }), found(2, { class: "nothing", role: null, heard: "never", note: "detect.nothing" }), found(3, { class: "off", role: null, note: "detect.off" })]),
    ...over,
  };
}

const view = (stored: SpeakerLayoutRecord | null, imported = false): LayoutView => ({ ...NO_VIEW, stored, imported });

test("wp_adetw the three states: confirmed, auto-detected (every row detected or the person's), unconfirmed", () => {
  const l = detectedLayout();
  assert.equal(layoutState(l), "autoDetected");
  assert.equal(layoutState({ ...l, confirmedByUser: true }), "confirmed");
  assert.equal(layoutState({ ...l, detected: null }), "unconfirmed", "no detection");
  assert.equal(layoutState(withRow(l, 0, { role: "sub" })), "autoDetected", "a corrected row is the person's; the detection stands for the rest");
  assert.equal(withRow(l, 0, { role: "sub" }).outputs[0]!.source, "user");
  assert.equal(withRow(l, 0, { speaker: "R" }).outputs[0]!.source, "detected", "a new name is not a new role");
  assert.equal(layoutState({ ...l, outputs: [...l.outputs, { output: 2, role: "full", speaker: "", group: 0, source: "suggested" }] }), "unconfirmed", "a suggestion among the rows");
  assert.equal(layoutState({ ...l, outputs: [{ output: 0, role: "low", speaker: "", group: 0 }] }), "unconfirmed", "a row kept before the detection has no source");
  assert.equal(layoutState({ ...l, outputs: [] }), "unconfirmed");
  assert.equal(layoutState(null), "unconfirmed");
});

test("wp_adetw the banner and the stamp: detected by AI, not confirmed; the app's own; then confirmed", () => {
  const now = AT + 3 * 86_400_000;
  const b = bannerOf(view(detectedLayout()), now)!;
  assert.equal(b.kind, "autoDetected");
  assert.equal(b.text, "Your speaker layout was detected by AI and is not confirmed. Check it in Settings › Speakers.");
  assert.match(b.here, /^Detected by AI, not confirmed: Claude Code probed this DSPi's outputs .+ at .+ \(microphone at the listening position\)\. Check each role, correct what is wrong, then confirm\.$/);
  assert.doesNotMatch(b.here, /Settings/, "on the page itself it does not send the person there");
  assert.match(bannerOf(view(detectedLayout()), AT + 60_000)!.here, / outputs today at /);
  // Auto-detected wins over "imported" (an import carries no detection; the core drops it).
  assert.equal(bannerOf(view(detectedLayout(), true), now)!.kind, "autoDetected");
  const app = detectedLayout({ detected: detection(detectedLayout().detected!.outputs, { by: "", placement: "nearField" }) });
  assert.match(bannerOf(view(app), now)!.here, /^Detected, not confirmed: the speaker detection probed .*\(microphone close to each driver\)/);
  assert.doesNotMatch(bannerOf(view(app), now)!.text, /AI/);
  assert.equal(bannerOf(view(detectedLayout({ confirmedByUser: true }))), null);
  // A detection whose rows are no longer all trusted is an unconfirmed layout's banner.
  assert.equal(bannerOf(view(detectedLayout({ outputs: [{ output: 0, role: "low", speaker: "", group: 0, source: "suggested" }] })))!.kind, "unconfirmed");
  assert.match(stampWords(detectedLayout(), AT + 60_000), /^Detected by AI, not confirmed, today at /);
  assert.match(stampWords(detectedLayout(), now), /^Detected by AI, not confirmed, on .+\.$/);
  assert.equal(stampWords(detectedLayout({ detected: { ...detectedLayout().detected!, atMs: 0 } })), "Detected by AI, not confirmed.");
  assert.match(stampWords(app, now), /^Detected, not confirmed, on /);
  assert.match(stampWords(detectedLayout({ confirmedByUser: true, updatedMs: now }), now), /^Confirmed today at /);
  assert.match(stampWords({ ...detectedLayout(), detected: null, updatedMs: now }, now), /^Kept, not confirmed, today at /);
});

test("wp_adetw the rows' pills: how sure the detection was, nothing heard and off, the person's and suggestions", () => {
  const l = detectedLayout();
  const kind = (draft: SpeakerLayoutRecord | null, stored: SpeakerLayoutRecord | null, o: number) => pillOf(draft, stored, o)?.kind ?? null;
  assert.equal(kind(l, l, 0), "detected");
  assert.equal(pillOf(l, l, 0)!.word, "detected");
  assert.equal(pillOf(l, l, 0)!.note, "detect.woofer", "the hover text is the notice's reason");
  assert.equal(pillOf(l, l, 2)!.word, "nothing heard");
  assert.equal(pillOf(l, l, 2)!.note, "detect.nothing");
  assert.equal(pillOf(l, l, 3)!.word, "off, not probed");
  const unsure = detectedLayout({ detected: detection([found(0, { confidence: "medium" }), found(1, { confidence: "low" }), found(2, { class: "unknown", role: "high", note: null })]) });
  unsure.outputs.push({ output: 2, role: "high", speaker: "", group: 0, source: "detected" });
  assert.equal(pillOf(unsure, unsure, 0)!.word, "detected?");
  assert.equal(pillOf(unsure, unsure, 1)!.word, "uncertain");
  assert.equal(pillOf(unsure, unsure, 2)!.word, "unknown, treated as a tweeter");
  assert.equal(pillOf(unsure, unsure, 2)!.note, "detect.unknown");
  // A corrected role turns the pill to the person's; a row checked in too.
  const corrected = withRow(l, 1, { role: "mid" });
  assert.equal(kind(corrected, l, 1), "you");
  assert.equal(kind(withRow(l, 2, { role: "full" }), l, 2), "you", "the person includes an output nothing was heard from");
  // Confirmed and as kept: no pills; an edit of it still says so.
  const c = { ...l, confirmedByUser: true };
  assert.equal(kind(c, c, 0), null);
  assert.equal(kind(withRow(c, 0, { role: "sub" }), c, 0), "you");
  // A plain unconfirmed layout's rows are suggestions; with no layout and no detection, nothing.
  const plain: SpeakerLayoutRecord = { serial: "", outputs: [{ output: 0, role: "full", speaker: "", group: 0, source: "suggested" }], confirmedByUser: false, updatedMs: AT, notes: [] };
  assert.equal(kind(plain, plain, 0), "suggested");
  assert.equal(kind(plain, null, 0), null, "a proposal before anything is kept");
  assert.equal(kind(plain, plain, 5), null, "an output with no row and no detection");
});

test("wp_adetw the evidence in words", () => {
  assert.deepEqual(evidenceWords(found(0)), { heard: "4–16 kHz", topFall: "21.0 dB", fLow: "45 Hz", lowest: "20 Hz" });
  assert.deepEqual(evidenceWords(found(1, { heard: "aboveFloor", topFallDb: null, fLowHz: 1414, lowestHz: 1000 })), { heard: "above the floor", topFall: "—", fLow: "1.41 kHz", lowest: "1 kHz" });
  assert.equal(evidenceWords(found(3, { class: "off", heard: "never" })).heard, "not probed");
  assert.equal(evidenceWords(found(2, { class: "nothing", heard: "never" })).heard, "never");
});

test("wp_adetw the page's copy keeps the detection and the rows' sources; what does not read is dropped", () => {
  const s = memory();
  saveKept(s, "S1", { layout: detectedLayout(), imported: false });
  const k = loadKept(s, "S1")!;
  assert.deepEqual(k.layout.outputs.map((o) => o.source), ["detected", "detected"]);
  assert.deepEqual(k.layout.detected, detectedLayout().detected);
  assert.equal(layoutState(k.layout), "autoDetected");
  // A copy kept before the detection (no source, no detection) still reads.
  s.map.set("dspi.speakerLayouts", JSON.stringify({ S1: { layout: { outputs: [{ output: 0, role: "low", speaker: "", group: 0 }], confirmedByUser: true, updatedMs: 1 } } }));
  const old = loadKept(s, "S1")!;
  assert.equal(old.layout.outputs[0]!.source, undefined);
  assert.equal(old.layout.detected, null);
  // A bad source is dropped, a detection without readable outputs too, bad fields fall back.
  s.map.set("dspi.speakerLayouts", JSON.stringify({ S1: { layout: { outputs: [{ output: 0, role: "low", speaker: "", group: 0, source: "agent" }], detected: { outputs: [{ output: 99 }] } } } }));
  assert.equal(loadKept(s, "S1")!.layout.outputs[0]!.source, undefined);
  assert.equal(loadKept(s, "S1")!.layout.detected, null);
  assert.equal(tidyDetection("x"), null);
  const t = tidyDetection({ outputs: [{ output: 1, class: "loud", confidence: 7, heard: "top", role: "tweeter", topFallDb: "x" }], placement: "moon", deviceOutputs: 999 })!;
  assert.deepEqual([t.outputs[0]!.class, t.outputs[0]!.confidence, t.outputs[0]!.role, t.outputs[0]!.topFallDb], ["unknown", "low", null, null]);
  assert.equal(t.placement, "listeningPosition");
  assert.equal(t.deviceOutputs, 0);
});

test("wp_adetw the main screen's peek: the core's layout, else the copy; never a write", async () => {
  const calls: string[] = [];
  const core = (stored: SpeakerLayoutRecord | null) => ({
    call: async (m: string) => {
      calls.push(m);
      if (m !== "speakerLayout") throw new Error("only reads");
      return stored;
    },
  }) as unknown as LayoutCaller;
  const s = memory();
  assert.equal((await peekLayout(core(null), "S1", s)).stored, null);
  saveKept(s, "S1", { layout: detectedLayout(), imported: false });
  const v = await peekLayout(core(null), "S1", s);
  assert.equal(bannerOf(v)!.kind, "autoDetected", "the copy before the open sets it again");
  assert.equal(bannerOf(await peekLayout(core(detectedLayout()), "S1", s))!.kind, "autoDetected");
  const plain = { ...detectedLayout(), detected: null, updatedMs: AT + 1 };
  assert.equal(bannerOf(await peekLayout(core(plain), "S1", s))!.kind, "imported", "a layout this page did not keep");
  assert.equal(bannerOf(await peekLayout(core({ ...plain, confirmedByUser: true }), "S1", s)), null);
  assert.deepEqual([...new Set(calls)], ["speakerLayout"]);
  assert.equal(JSON.parse(s.map.get("dspi.speakerLayouts")!).S1.layout.updatedMs, AT, "the copy untouched");
});

test("wp_adetw the presence strip: an agent detecting the speakers", () => {
  assert.ok(detectingSpeakers("Detecting speakers: Out 3"));
  assert.ok(detectingSpeakers("Detecting the speakers: Out 3 of 6"));
  assert.ok(detectingSpeakers("‮Detecting speakers"));
  assert.ok(!detectingSpeakers("Changing the gain: OUT1"));
  assert.ok(!detectingSpeakers("Measuring the speakers"));
  assert.match(DETECTING_HERE, /Settings › Speakers/);
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

test("wp_adetw on the demo: an auto-detected layout kept and set again, its roles trusted, corrected and confirmed", { skip, timeout: 60_000 }, async () => {
  const store = memory();
  const a = await demo();
  const v0 = await loadLayout(caller(a.core), a.serial, store);
  const det = detectedLayout({ serial: a.serial, detected: { ...detectedLayout().detected!, deviceOutputs: a.outputs } });
  // The layout as a detection leaves it (the core's own job is ADET0's; the web never runs one).
  const v1 = await keepLayout(caller(a.core), a.serial, store, det, false, v0);
  assert.equal(layoutState(v1.stored), "autoDetected", "the core keeps the sources and the detection");
  assert.deepEqual(v1.stored!.detected!.outputs.map((o) => o.class), ["woofer", "tweeter", "nothing", "off"]);
  assert.equal(bannerOf(v1)!.kind, "autoDetected");
  // The page reopened: a fresh core gets the copy, still auto-detected.
  const b = await demo();
  const v2 = await loadLayout(caller(b.core), b.serial, store);
  assert.equal(layoutState(v2.stored), "autoDetected");
  assert.equal(v2.imported, false);
  assert.equal(bannerOf(v2)!.kind, "autoDetected");
  // Its roles are trusted: the measurement does not refuse it for want of a confirmed layout.
  const setup = { ...setupOf({ ...withLayout(initialDraft([0, 1]), v2.stored), placement: "listeningPosition" }, 48_000), rolesFromLayout: true };
  const err = await b.core.call("measurePreflight" as never, { setup } as never).then(() => null, (e: unknown) => e);
  assert.ok(!isLayoutUnconfirmed(err), `not refused as unconfirmed: ${String(err)}`);
  // A corrected role is the person's; kept unconfirmed, the layout stays auto-detected.
  const v3 = await keepLayout(caller(b.core), b.serial, store, withRow(v2.stored, 1, { role: "mid" }), false, v2);
  assert.deepEqual(v3.stored!.outputs.map((o) => [o.role, o.source]), [["low", "detected"], ["mid", "user"]]);
  assert.equal(layoutState(v3.stored), "autoDetected");
  // Confirm makes it the person's layout: no banner, the stamp says confirmed.
  const v4 = await keepLayout(caller(b.core), b.serial, store, v3.stored!, true, v3);
  assert.equal(layoutState(v4.stored), "confirmed");
  assert.equal(bannerOf(v4), null);
  assert.match(stampWords(v4.stored), /^Confirmed today at /);
  assert.equal(layoutState(loadKept(store, b.serial)!.layout), "confirmed");
});

test("wp_adetw on the demo: a detection of another output count reads as suggestions", { skip, timeout: 60_000 }, async () => {
  const store = memory();
  const a = await demo();
  const v0 = await loadLayout(caller(a.core), a.serial, store);
  const det = detectedLayout({ serial: a.serial, detected: { ...detectedLayout().detected!, deviceOutputs: a.outputs + 2 } });
  await keepLayout(caller(a.core), a.serial, store, det, false, v0);
  const again = await loadLayout(caller(a.core), a.serial, store);
  assert.equal(layoutState(again.stored), "unconfirmed", `rows: ${JSON.stringify(again.stored?.outputs.map((o) => o.source))}`);
  assert.notEqual(bannerOf(again)!.kind, "autoDetected");
});
