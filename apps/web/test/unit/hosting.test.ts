// WP WB7 unit tests: the c6m nits on the page's side. Hub tokens (N4, the
// reviewer's probe adopted), a replaced hub's stale token (N2), and the shim
// keeping the first failed open's words (N3).

import { test } from "node:test";
import assert from "node:assert/strict";
import { CoreClient, type WorkerLike } from "../../src/core/client";
import type { FromWorker, Request } from "../../src/core/protocol";
import { WsHost, type WebSocketLike } from "../../src/core/ws";
import { FleetHubTokens, LocalHubTokens, hubTokensFor } from "../../src/fleet/hub-tokens";
import type { FleetBackend } from "../../src/fleet/types";
import { Session } from "../../src/ui/session";

class Store {
  m = new Map<string, string>();
  getItem(k: string) {
    return this.m.get(k) ?? null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, v);
  }
}

// ---------------------------------------------------------------- N4: hub tokens

test("WB7 (c6m N4): local tokens survive a reload, forget removes, garbage is ignored", async () => {
  const s = new Store();
  const a = new LocalHubTokens(s);
  a.save("ws://h1/dspi/v1", { token: "T1", hubId: "H1" });
  a.save("ws://h2/dspi/v1", { token: "", hubId: "H2" }); // empty token: not kept
  const b = new LocalHubTokens(s);
  assert.deepEqual(await b.get("ws://h1/dspi/v1"), { token: "T1", hubId: "H1" });
  assert.equal(await b.get("ws://h2/dspi/v1"), null);
  b.forget("ws://h1/dspi/v1");
  assert.equal(await new LocalHubTokens(s).get("ws://h1/dspi/v1"), null);
  s.setItem("dspi-web.hub-tokens.v1", "not json");
  assert.equal(await new LocalHubTokens(s).get("ws://h1/dspi/v1"), null);
  s.setItem("dspi-web.hub-tokens.v1", JSON.stringify({ "ws://x": { token: 5, hubId: "y" }, "ws://y": { token: "ok", hubId: "z" } }));
  const c = new LocalHubTokens(s);
  assert.equal(await c.get("ws://x"), null);
  assert.deepEqual(await c.get("ws://y"), { token: "ok", hubId: "z" });
  // Storage refused: memory only, no throw.
  const d = new LocalHubTokens(null);
  d.save("ws://m", { token: "M", hubId: "" });
  assert.deepEqual(await d.get("ws://m"), { token: "M", hubId: "" });
});

test("WB7 (c6m N4): fleet tokens: the server's first, refused once per visit, a paired one in memory only", async () => {
  let asked = 0;
  const backend = {
    kind: "f",
    installations: async () => [],
    access: async () => "control",
    hubCredentials: async (u: string) => {
      asked++;
      return u === "ws://a" ? { token: "SRV", hubId: "HA" } : null;
    },
  } as unknown as FleetBackend;
  const s = new Store();
  const t = hubTokensFor(backend, s);
  assert.ok(t instanceof FleetHubTokens);
  assert.deepEqual(await t.get("ws://a"), { token: "SRV", hubId: "HA" });
  assert.equal(await t.get("ws://b"), null);
  // The hub refused the server's token: not offered again this visit.
  t.forget("ws://a");
  assert.equal(await t.get("ws://a"), null);
  const before = asked;
  await t.get("ws://a");
  assert.equal(asked, before, "the backend is not asked again");
  // The user pairs: kept in memory, wins over the server, never stored.
  t.save("ws://a", { token: "MINE", hubId: "HA" });
  assert.deepEqual(await t.get("ws://a"), { token: "MINE", hubId: "HA" });
  assert.equal(s.m.size, 0, "nothing in page storage");
  t.forget("ws://a");
  assert.equal(await t.get("ws://a"), null);
  // Without hubCredentials: the page's own storage.
  assert.ok(hubTokensFor({ ...backend, hubCredentials: undefined } as FleetBackend, s) instanceof LocalHubTokens);
});

// ---------------------------------------------------------------- N2: a replaced hub

/** A Worker whose core answers from `answer`, in process; records the calls. */
function fakeCore(answer: (r: Request) => { ok: true; value: unknown } | { ok: false; code: string; message: string }) {
  const calls: Request[] = [];
  const w: WorkerLike = {
    onmessage: null,
    postMessage(r: Request) {
      calls.push(r);
      const a = answer(r);
      const reply: FromWorker = a.ok
        ? { reply: { id: r.id, ok: true, value: a.value } }
        : { reply: { id: r.id, ok: false, error: { code: a.code, message: a.message } } };
      queueMicrotask(() => w.onmessage?.({ data: reply } as MessageEvent<FromWorker>));
    },
    terminate() {},
  };
  return { client: new CoreClient(w), calls };
}

const hello = { hubId: "NEW", hubName: "Hub", hubKind: "console", auth: "pin", pinSource: "window", needsPairing: true, tokenRefused: false, caps: [], maxInflight: 1 };

test("WB7 (c6m N2): another hub at a stored address: the token is forgotten and the hub asks for its PIN", async () => {
  const s = new Store();
  const tokens = new LocalHubTokens(s);
  tokens.save("ws://hub:11915", { token: "OLD", hubId: "GONE" });
  const { client, calls } = fakeCore((r) => {
    if (r.method !== "linkConnect") return { ok: true, value: null };
    const p = r.params as { token: string };
    return p.token ? { ok: false, code: "wrong_hub", message: "Another hub answers at this address." } : { ok: true, value: hello };
  });
  const session = new Session("s", "local", "This console", { kind: "hub", url: "ws://hub:11915" }, "control", client, tokens);
  await session.open({ kind: "hub", url: "ws://hub:11915", deviceId: "" });
  assert.equal(session.state.status, "pairing", session.state.error);
  const connects = calls.filter((c) => c.method === "linkConnect").map((c) => (c.params as { token: string }).token);
  assert.deepEqual(connects, ["OLD", ""], "once with the stale token, then without");
  assert.equal(await new LocalHubTokens(s).get("ws://hub:11915"), null, "forgotten in storage too");
});

test("WB7 (c6m N2): wrong_hub with no token offered is the session's failure, not a loop", async () => {
  const { client, calls } = fakeCore((r) =>
    r.method === "linkConnect" ? { ok: false, code: "wrong_hub", message: "Another hub answers at this address." } : { ok: true, value: null },
  );
  const session = new Session("s", "local", "This console", { kind: "hub", url: "ws://hub" }, "control", client, new LocalHubTokens(null));
  await session.open({ kind: "hub", url: "ws://hub", deviceId: "" });
  assert.equal(session.state.status, "failed");
  assert.equal(session.state.error, "Another hub answers at this address.");
  assert.equal(calls.filter((c) => c.method === "linkConnect").length, 1);
});

// ---------------------------------------------------------------- N3: the shim's error text

test("WB7 (c6m N3): a later close of an open socket keeps the first failed open's words", async () => {
  const sockets: { onopen: (() => void) | null; onclose: ((e: { code: number }) => void) | null; protocol: string }[] = [];
  let refuse = true;
  class Sock {
    binaryType = "";
    protocol = "";
    onopen: (() => void) | null = null;
    onmessage: unknown = null;
    onclose: ((e: { code: number }) => void) | null = null;
    onerror: unknown = null;
    constructor(_u: string, p: string) {
      sockets.push(this);
      const refused = refuse;
      setTimeout(() => {
        if (refused) this.onclose?.({ code: 1006 });
        else {
          this.protocol = p;
          this.onopen?.();
        }
      }, 1);
    }
    close() {}
  }
  const memory = new WebAssembly.Memory({ initial: 1 });
  const host = new WsHost(Sock as unknown as WebSocketLike);
  host.bindMemory(() => memory);
  class Suspending {
    constructor(readonly f: (...a: number[]) => Promise<number>) {}
  }
  const im = host.imports(Suspending as never) as Record<string, unknown>;
  const bytes = new Uint8Array(memory.buffer);
  const proto = new TextEncoder().encode("dspi-link-1");
  bytes.set(proto, 100);
  const open = (address: string) => {
    const url = new TextEncoder().encode(address);
    bytes.set(url, 0);
    return (im.dspi_ws_open as Suspending).f(0, url.length, 100, proto.length);
  };
  const error = () => {
    const n = (im.dspi_ws_error as (o: number, c: number) => number)(500, 400);
    return new TextDecoder().decode(bytes.subarray(500, 500 + n));
  };
  assert.equal(await open("ws://hub:1/dspi/v1"), -1);
  const first = error();
  assert.match(first, /Cannot connect to the hub at hub:1/);
  refuse = false;
  const sock = await open("ws://other:2/dspi/v1");
  assert.ok(sock > 0);
  sockets.at(-1)!.onclose!({ code: 1006 }); // the open socket drops later
  assert.equal(error(), first, "the drop does not rewrite the failed open's text");
});
