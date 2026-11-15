// WP WB6f unit tests: the status rail's health panel. What is on (clipping, limiter
// gain reduction, the two cores' load, a lost DSPi Link) from the core's readings;
// problems stay until acknowledged; the feed reads through the session's core and
// leaves what it could not read as it was. With the core module (DSPI_WASM_DIR):
// the demo's `limiterMeter` and a demo clip through the real core.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  HealthTracker,
  LIMITER_ON_DB,
  LOAD_HIGH,
  conditions,
  healthSummary,
  readKinds,
  type Condition,
  type HubAlarmSource,
  type Reading,
} from "../../src/ui/status-rail/health";
import { HealthFeed } from "../../src/ui/status-rail/feed";
import { linkText } from "../../src/ui/texts";
import { CoreCallError, type DspState, type LinkPhase, type SystemStatus } from "../../src/core/protocol";
import type { Session } from "../../src/ui/session";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";

const none: Reading = { clip: null, load: null, limiter: null, dlink: null };
const keys = (c: Condition[]) => c.map((x) => x.key);

test("WB6F: clipping, by the channel's name, from the clip bits", () => {
  const c = conditions({ ...none, clip: { mask: 0b101, names: ["USB L", "USB R", "Out 1"] } }, new Set());
  assert.deepEqual(keys(c), ["clip:0", "clip:2"]);
  assert.equal(c[1]!.text, "Clipping on Out 1");
  assert.equal(c[0]!.severity, "error");
  // Bit 31 is a channel too, not a sign.
  assert.deepEqual(keys(conditions({ ...none, clip: { mask: 0x80000000 >>> 0, names: [] } }, new Set())), ["clip:31"]);
  assert.equal(conditions({ ...none, clip: { mask: 0, names: [] } }, new Set()).length, 0);
});

test("WB6F: a core's load above the Console's red mark; it ends only once well below", () => {
  const at = (a: number, b: number, on = new Set<string>()) => keys(conditions({ ...none, load: [a, b] }, on));
  assert.deepEqual(at(LOAD_HIGH, 10), [], "at the mark is not over it");
  assert.deepEqual(at(LOAD_HIGH + 1, 10), ["load:0"]);
  assert.deepEqual(at(20, 97), ["load:1"]);
  // Hovering: 88 % keeps an on problem on, raises no new one.
  assert.deepEqual(at(88, 88, new Set(["load:0"])), ["load:0"]);
  assert.deepEqual(at(80, 10, new Set(["load:0"])), []);
  const [c] = conditions({ ...none, load: [95, 0] }, new Set());
  assert.equal(c!.value, 95);
  assert.match(c!.text, /^Core 0 load high: up to \{worst\}%$/);
});

test("WB6F: limiter gain reduction counts on outputs whose limiter is on, above the threshold", () => {
  const lim = (gr: number[], enabled: boolean[], on = new Set<string>()) =>
    conditions({ ...none, limiter: { gr, names: ["Out 1", "Out 2", "Sub"], enabled } }, on);
  assert.deepEqual(keys(lim([0, 3.04, 6], [true, true, false])), ["limiter:1"], "the sub's limiter is off");
  assert.equal(lim([0, 3.04, 0], [true, true, true])[0]!.value, 3);
  assert.deepEqual(keys(lim([LIMITER_ON_DB - 0.1, 0, 0], [true, true, true])), []);
  assert.deepEqual(keys(lim([0.7, 0, 0], [true, true, true], new Set(["limiter:0"]))), ["limiter:0"], "on until below the lower mark");
  assert.deepEqual(keys(lim([120, NaN, 0], [true, true, true])), [], "the display floor and junk are no reading");
});

test("WB6F: a lost DSPi Link in CG10's words; bound and no hub are fine", () => {
  for (const phase of ["reconnecting", "closed", "refused"] as LinkPhase[]) {
    const [c] = conditions({ ...none, dlink: phase }, new Set());
    assert.equal(c!.key, "dlink");
    assert.equal(c!.text, `DSPi Link: ${linkText(phase).text}`);
    assert.doesNotMatch(c!.text, /\blink\b/, "never a bare link");
  }
  assert.equal(conditions({ ...none, dlink: "bound" }, new Set()).length, 0);
  assert.equal(conditions(none, new Set()).length, 0);
});

test("WB6F: no host-device nouns in the panel's words", () => {
  const all = conditions(
    {
      clip: { mask: 1, names: ["USB L"] },
      load: [99, 99],
      limiter: { gr: [5], names: ["Out 1"], enabled: [true] },
      dlink: "reconnecting",
    },
    new Set(),
  );
  for (const c of all) assert.doesNotMatch(c.text, /\b(phone|computer|tablet|laptop|mac|pc|device)\b/i, c.text);
});

test("WB6F: a problem stays until acknowledged, also once it has ended", () => {
  const t = new HealthTracker();
  const clip: Condition = { key: "clip:0", kind: "clip", severity: "error", text: "Clipping on USB L" };
  const kinds = readKinds({ ...none, clip: { mask: 0, names: [] } });
  assert.equal(t.update(1000, [clip], kinds), true);
  assert.equal(t.update(1500, [clip], kinds), false, "still on: nothing new");
  t.update(2000, [], kinds);
  let [p] = t.list();
  assert.equal(p!.ended, 2000, "ended by itself, still listed");
  assert.equal(p!.since, 1000);
  assert.equal(p!.acknowledged, false);
  // On again before anyone looked: the same entry, counted.
  t.update(3000, [clip], kinds);
  [p] = t.list();
  assert.equal(p!.count, 2);
  assert.equal(p!.ended, null);
  t.update(4000, [], kinds);
  assert.equal(t.acknowledge("clip:0"), true);
  assert.deepEqual(t.list(), [], "ended and acknowledged: gone");
  assert.equal(t.acknowledge("clip:0"), false);
});

test("WB6F: acknowledged while on, it stays marked and goes when it ends; back on, it asks again", () => {
  const t = new HealthTracker();
  const kinds = readKinds({ ...none, load: [0, 0] });
  const load = (v: number): Condition => ({ key: "load:1", kind: "load", severity: "warning", text: "Core 1 load high: up to {worst}%", value: v });
  t.update(1, [load(95)], kinds);
  t.acknowledge("load:1");
  t.update(2, [load(99)], kinds);
  let [p] = t.list();
  assert.equal(p!.acknowledged, true);
  assert.equal(p!.worst, 99);
  assert.equal(p!.text, "Core 1 load high: up to 99%");
  t.update(3, [load(92)], kinds);
  assert.equal(t.list()[0]!.worst, 99, "the worst is kept");
  t.update(4, [], kinds);
  assert.deepEqual(t.list(), []);
  t.update(5, [load(93)], kinds);
  [p] = t.list();
  assert.equal(p!.acknowledged, false);
  assert.equal(p!.count, 1);
});

test("WB6F: a reading that does not cover a kind ends nothing of it", () => {
  const t = new HealthTracker();
  const clip: Condition = { key: "clip:0", kind: "clip", severity: "error", text: "Clipping on USB L" };
  t.update(1, [clip], readKinds({ ...none, clip: { mask: 1, names: [] } }));
  // The meters not read (stale or failed): only the DSPi Link phase counts.
  t.update(2, [], readKinds(none));
  assert.equal(t.list()[0]!.ended, null);
});

test("WB6F: acknowledge all; the list order and the rail's summary", () => {
  const t = new HealthTracker();
  const all = readKinds({ clip: { mask: 0, names: [] }, load: [0, 0], limiter: { gr: [], names: [], enabled: [] }, dlink: null });
  assert.deepEqual(healthSummary(t.list()), { text: "OK", tone: "ok" });
  t.update(1, [{ key: "load:0", kind: "load", severity: "warning", text: "w", value: 95 }], all);
  assert.deepEqual(healthSummary(t.list()), { text: "1 problem", tone: "warn" });
  t.update(2, [
    { key: "load:0", kind: "load", severity: "warning", text: "w", value: 95 },
    { key: "dlink", kind: "dlink", severity: "error", text: "e" },
  ], all);
  assert.deepEqual(t.list().map((p) => p.key), ["dlink", "load:0"], "errors first");
  assert.deepEqual(healthSummary(t.list()), { text: "2 problems", tone: "bad" });
  assert.equal(t.acknowledgeAll(), true);
  assert.deepEqual(healthSummary(t.list()), { text: "2 acknowledged", tone: "warn" });
});

// ---------------------------------------------------------------- the feed

const STATE = {
  numChannels: 4,
  numInputChannels: 2,
  numOutputChannels: 2,
  channelNames: ["USB L", "USB R", "Out 1", "Out 2"],
  limiterEnabled: [false, true],
} as unknown as DspState;

function status(over: Partial<SystemStatus> = {}): SystemStatus {
  return { peaks: [], cpu0: 10, cpu1: 20, clipFlags: 0, activeInputChannels: 2, numChannels: 4, stale: false, ageMs: 0, peakHold: [], peakMax: [], clipHold: 0, ...over };
}

/** A session as the feed sees it, on a core whose answers the test sets. */
function fakeSession() {
  const calls: string[] = [];
  const watchers = new Set<() => void>();
  const s = {
    meters: status(),
    gr: [0, 0] as number[] | CoreCallError,
    state: { status: "open", snapshot: { device: { name: "D" }, state: structuredClone(STATE) }, link: null as { phase: LinkPhase } | null },
    document: null,
    watch(fn: () => void) {
      watchers.add(fn);
      return () => watchers.delete(fn);
    },
    setLink(phase: LinkPhase) {
      s.state = { ...s.state, link: { phase } };
      for (const w of watchers) w();
    },
    core: {
      call: async (method: string) => {
        calls.push(method);
        if (method === "meters") return s.meters;
        if (method === "limiterMeter") {
          if (s.gr instanceof CoreCallError) throw s.gr;
          return s.gr;
        }
        throw new CoreCallError("unknown_method", method);
      },
    },
    calls,
  };
  return s;
}

const feeds: HealthFeed[] = [];
after(() => feeds.forEach((f) => f.stop()));
function feedOn(s: ReturnType<typeof fakeSession>, opts: ConstructorParameters<typeof HealthFeed>[1] = {}) {
  let now = 1_000;
  const f = new HealthFeed(s as unknown as Session, { periodMs: 60_000, now: () => (now += 1000), ...opts });
  feeds.push(f);
  return f;
}

test("WB6F feed: clipping and load from the meters, gain reduction while a limiter is on", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.tick();
  assert.equal(f.view.problems.length, 0);
  assert.deepEqual(f.view.load, [10, 20]);
  assert.ok(s.calls.includes("limiterMeter"), "Out 2's limiter is on");
  s.meters = status({ clipHold: 0b100, cpu1: 96 });
  s.gr = [0, 4.5];
  await f.tick();
  assert.deepEqual(f.view.problems.map((p) => p.text).sort(), ["Clipping on Out 1", "Core 1 load high: up to 96%", "Limiter reducing Out 2: up to 4.5 dB"]);
  // All back to normal: every one is still listed, ended.
  s.meters = status();
  s.gr = [0, 0];
  await f.tick();
  assert.equal(f.view.problems.length, 3);
  assert.ok(f.view.problems.every((p) => p.ended !== null));
  f.acknowledgeAll();
  assert.equal(f.view.problems.length, 0);
});

test("WB6F feed: a stale or failed reading raises and ends nothing; no limiter on, no limiter read", async () => {
  const s = fakeSession();
  s.state.snapshot.state.limiterEnabled = [false, false];
  const f = feedOn(s);
  f.start();
  await f.tick(); // the first read, which start() began
  s.meters = status({ clipHold: 1 });
  await f.tick();
  assert.equal(f.view.problems.length, 1);
  assert.ok(!s.calls.includes("limiterMeter"));
  s.meters = status({ stale: true, ageMs: 3000 });
  await f.tick();
  assert.equal(f.view.problems[0]!.ended, null, "stale: not a reading");
  s.meters = status({ stale: true, clipHold: 2 });
  await f.tick();
  assert.equal(f.view.problems.length, 1, "nor does a stale clip raise one");
});

test("WB6F feed: a firmware without the limiter is asked once", async () => {
  const s = fakeSession();
  s.gr = new CoreCallError("unsupported", "no limiter");
  const f = feedOn(s);
  f.start();
  await f.tick();
  await f.tick();
  assert.equal(s.calls.filter((c) => c === "limiterMeter").length, 1);
});

test("WB6F feed: a lost DSPi Link is raised at once and stays after the reconnect until acknowledged", async () => {
  const s = fakeSession();
  const f = feedOn(s);
  f.start();
  await f.tick();
  let shown = 0;
  f.subscribe(() => shown++);
  s.setLink("reconnecting");
  assert.equal(f.view.problems[0]?.text, `DSPi Link: ${linkText("reconnecting").text}`);
  assert.ok(shown > 0, "the rail was told");
  s.setLink("bound");
  assert.notEqual(f.view.problems[0]!.ended, null);
  f.acknowledge("dlink");
  assert.equal(f.view.problems.length, 0);
});

test("WB6F feed: the hub's alarms join through the typed seam (phase 2), acknowledged back to it", async () => {
  const s = fakeSession();
  let push: (on: Condition[]) => void = () => {};
  const acked: string[] = [];
  const hub: HubAlarmSource = {
    subscribe(fn) {
      push = fn;
      return () => (push = () => {});
    },
    acknowledge: async (key) => void acked.push(key),
  };
  const f = feedOn(s, { hubAlarms: hub });
  f.start();
  push([{ key: "hub:7", kind: "hub", severity: "warning", text: "Amplifier too warm" }]);
  assert.equal(f.view.problems[0]?.key, "hub:7");
  f.acknowledge("hub:7");
  assert.deepEqual(acked, ["hub:7"]);
});

// ---------------------------------------------------------------- the real core

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const skip = dir && existsSync(pageModule) ? false : "no core module (set DSPI_WASM_DIR)";
const made: WasmCore[] = [];
after(() => made.forEach((c) => c.dispose()));

test("WB6F core: the demo's limiterMeter, and a demo clip in the meters' clip hold", { skip }, async () => {
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  made.push(core);
  await assert.rejects(core.call("limiterMeter", undefined), (e: unknown) => e instanceof CoreCallError && e.code === "not_open");
  const snap = await core.call("open", { kind: "demo" });
  const gr = await core.call("limiterMeter", undefined);
  assert.ok(gr.length >= snap.state.numOutputChannels);
  assert.ok(gr.every((v) => v === 0), "the demo limits nothing");
  await core.call("demo", { action: "clip", mask: 0b10 });
  const m = await core.call("meters", undefined);
  const c = conditions({ ...none, clip: { mask: m.clipHold >>> 0, names: snap.state.channelNames } }, new Set());
  assert.deepEqual(keys(c), ["clip:1"]);
  core.dispose();
});
