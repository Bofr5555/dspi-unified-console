// WP WB5 unit tests: the serial rule, the stub's contract, the client's RPC and
// the standalone backend. Run with `npm run test:unit` (node --test).

import { test } from "node:test";
import assert from "node:assert/strict";
import { SerialCore } from "../../src/core/serial";
import { StubCore } from "../../src/core/stub";
import { CoreClient, type WorkerLike } from "../../src/core/client";
import {
  CoreCallError,
  type CoreEvent,
  type CoreImpl,
  type CoreMethods,
  type FromWorker,
  type Method,
  type Request,
} from "../../src/core/protocol";
import { LocalBackend, normaliseHubUrl, type KeyValueStore } from "../../src/fleet/local";
import { pageEnv } from "../../src/fleet/env";

/** A core that records how many calls run at once. */
class CountingCore implements CoreImpl {
  running = 0;
  max = 0;
  order: string[] = [];
  onEvent(): void {}
  async call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    this.running++;
    this.max = Math.max(this.max, this.running);
    this.order.push(`start ${String((params as { value?: number })?.value ?? method)}`);
    await new Promise((r) => setTimeout(r, 5));
    this.running--;
    this.order.push(`end ${String((params as { value?: number })?.value ?? method)}`);
    if ((params as { value?: number })?.value === 2) throw new CoreCallError("refused", "two is refused");
    return 0 as CoreMethods[M]["result"];
  }
}

test("calls never overlap, run in arrival order, and a failure does not stop the queue", async () => {
  const core = new CountingCore();
  const s = new SerialCore(core);
  const results = await Promise.allSettled([1, 2, 3, 4].map((v) => s.call("write", { path: "vol.user", value: v })));
  assert.equal(core.max, 1);
  assert.deepEqual(core.order, ["start 1", "end 1", "start 2", "end 2", "start 3", "end 3", "start 4", "end 4"]);
  assert.deepEqual(
    results.map((r) => r.status),
    ["fulfilled", "rejected", "fulfilled", "fulfilled"],
  );
});

test("a call made while one is suspended inside the core waits for it", async () => {
  const stub = new StubCore({ latencyMs: 10 });
  const s = new SerialCore(stub);
  await s.call("open", { kind: "demo" });
  const write = s.call("write", { path: "vol.user", value: -12 });
  const read = s.call("state", undefined); // arrives while the write is suspended
  assert.equal(await write, -12);
  assert.equal((await read).volume.db, -12, "the read ran after the write, not inside it");
});

test("stub: write reads back the kept value; out of range and read-only are refused", async () => {
  const c = new StubCore();
  await assert.rejects(c.call("state", undefined), (e: CoreCallError) => e.code === "not_open");
  await c.call("open", { kind: "demo" });
  assert.equal(await c.call("write", { path: "vol.user", value: -12.3 }), -12.5);
  await assert.rejects(c.call("write", { path: "vol.user", value: 3 }), (e: CoreCallError) => e.code === "out_of_range");
  assert.equal((await c.call("state", undefined)).volume.db, -12.5, "a refused write changes nothing");

  const ro = new StubCore();
  const snap = await ro.call("open", { kind: "demo", readOnly: true });
  assert.equal(snap.readOnly, true);
  assert.ok(snap.readOnlyReason.length > 0);
  await assert.rejects(ro.call("write", { path: "vol.user", value: -10 }), (e: CoreCallError) => e.code === "read_only");
});

test("stub: only the demo opens until the transports land", async () => {
  const c = new StubCore();
  await assert.rejects(c.call("open", { kind: "usb" }), (e: CoreCallError) => e.code === "refused");
  await assert.rejects(c.call("open", { kind: "hub", url: "ws://h:1", deviceId: "" }), (e: CoreCallError) => e.code === "refused");
});

/** A Worker in this process: the same code path as worker.ts, without a thread. */
function inProcessWorker(core: CoreImpl): WorkerLike {
  const serial = new SerialCore(core);
  const w: WorkerLike = {
    onmessage: null,
    postMessage(req: Request) {
      serial.call(req.method, req.params).then(
        (value) => deliver({ reply: { id: req.id, ok: true, value } }),
        (e: CoreCallError) => deliver({ reply: { id: req.id, ok: false, error: { code: e.code, message: e.message } } }),
      );
    },
    terminate() {
      w.onmessage = null;
    },
  };
  const deliver = (m: FromWorker) => queueMicrotask(() => w.onmessage?.({ data: m } as MessageEvent<FromWorker>));
  core.onEvent((e) => deliver({ push: e }));
  return w;
}

test("client: replies match their calls, errors keep their code, events reach subscribers", async () => {
  const client = new CoreClient(inProcessWorker(new StubCore()));
  const events: CoreEvent[] = [];
  client.subscribe((e) => events.push(e));
  const snap = await client.call("open", { kind: "demo" });
  assert.equal(snap.name, "DSPi demo");
  const [a, b] = await Promise.allSettled([
    client.call("write", { path: "vol.user", value: -6 }),
    client.call("write", { path: "vol.user", value: 6 }),
  ]);
  assert.deepEqual(a, { status: "fulfilled", value: -6 });
  assert.equal(b.status, "rejected");
  assert.equal((b as PromiseRejectedResult).reason.code, "out_of_range");
  await client.call("simulate", { what: "link-lost" });
  await new Promise((r) => setTimeout(r, 0));
  assert.ok(events.some((e) => e.event === "state"));
  assert.ok(events.some((e) => e.event === "link" && e.phase === "reconnecting"));
  await client.dispose();
  await assert.rejects(client.call("state", undefined), (e: CoreCallError) => e.code === "not_open");
});

test("two sessions are independent", async () => {
  const one = new CoreClient(inProcessWorker(new StubCore()));
  const two = new CoreClient(inProcessWorker(new StubCore()));
  await one.call("open", { kind: "demo" });
  await two.call("open", { kind: "demo" });
  await one.call("write", { path: "vol.user", value: -3 });
  assert.equal((await two.call("state", undefined)).volume.db, -20);
});

class MapStore implements KeyValueStore {
  m = new Map<string, string>();
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
}

test("local backend: hubs persist, are normalised and not duplicated; the serving hub is first", async () => {
  const store = new MapStore();
  const b = new LocalBackend({ storage: store, servingHub: "ws://hub.local:11915" });
  await b.addHub(null, "studio.local:11915", "Studio");
  await b.addHub(null, "http://studio.local:11915/", "");
  const again = new LocalBackend({ storage: store, servingHub: "ws://hub.local:11915" });
  const [inst] = await again.installations();
  assert.deepEqual(inst!.sources, [
    { kind: "demo" },
    { kind: "usb" },
    { kind: "hub", url: "ws://hub.local:11915" },
    { kind: "hub", url: "ws://studio.local:11915" },
  ]);
  assert.equal(await again.access(), "control");
  assert.equal(await again.user(), null);
  await again.removeSource("local", { kind: "hub", url: "ws://studio.local:11915" });
  assert.equal((await new LocalBackend({ storage: store }).installations())[0]!.sources.length, 2);
});

test("local backend: works with no storage and with broken storage data", async () => {
  const none = new LocalBackend({ storage: null });
  await none.addHub(null, "a.local:1", "");
  assert.equal((await none.installations())[0]!.sources.length, 3);
  const bad = new MapStore();
  bad.setItem("dspi-web.hubs.v1", "{not json");
  assert.equal((await new LocalBackend({ storage: bad }).installations())[0]!.sources.length, 2);
});

test("hub addresses normalise to ws:// or wss://", () => {
  assert.equal(normaliseHubUrl(" hub.local:11915 "), "ws://hub.local:11915");
  assert.equal(normaliseHubUrl("https://hub.example:443/x"), "wss://hub.example");
  assert.equal(normaliseHubUrl("ws://192.168.1.5:11915"), "ws://192.168.1.5:11915");
  assert.throws(() => normaliseHubUrl("not a url with spaces://"));
});

test("page env: USB needs a secure page and WebUSB; the reasons name no device", () => {
  const insecure = pageEnv({ isSecureContext: false, navigator: { usb: {} } });
  assert.equal(insecure.usb.ok, false);
  assert.match(insecure.usb.reason, /secure page/);
  const noUsb = pageEnv({ isSecureContext: true, navigator: {} });
  assert.match(noUsb.usb.reason, /Chromium/);
  for (const r of [insecure.usb.reason, noUsb.usb.reason, noUsb.link.reason]) {
    assert.doesNotMatch(r, /\b(computer|phone|tablet|laptop|desktop|mac|pc)\b/i);
  }
});
