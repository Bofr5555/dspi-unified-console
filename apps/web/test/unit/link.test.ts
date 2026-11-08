// WP WB4b unit tests: the Worker's WebSocket shim (src/core/ws.ts) on a fake
// socket, and the real core (crates/dspi-wasm) over Node's own WebSocket against
// dspi-link's fake hub ($DSPI_FAKEHUB, a binary the CI builds), through the
// Worker's glue: connect, pair, list, open, a write with read-back, a drop and the
// core's own reconnect. Without the binary or the module those are skipped, unless
// $DSPI_FAKEHUB_REQUIRED is set (CI).

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { spawn, type ChildProcess } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { createInterface } from "node:readline";
import { CoreCallError, type CoreEvent } from "../../src/core/protocol";
import { WasmCore, loadWasmModule } from "../../src/core/wasm";
import { WsHost, type WebSocketLike } from "../../src/core/ws";

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------- the shim

/** A WebSocket that opens at once and records what is sent. */
class FakeSocket {
  static last: FakeSocket | null = null;
  binaryType = "blob";
  readyState = 0;
  protocol = "";
  sent: (string | Uint8Array)[] = [];
  closedWith: number | null = null;
  onopen: (() => void) | null = null;
  onmessage: ((e: { data: unknown }) => void) | null = null;
  onclose: ((e: { code: number; wasClean: boolean }) => void) | null = null;
  onerror: (() => void) | null = null;
  constructor(
    readonly url: string,
    proto: string,
  ) {
    FakeSocket.last = this;
    setTimeout(() => {
      this.protocol = proto;
      this.readyState = 1;
      this.onopen?.();
    }, 1);
  }
  send(d: string | Uint8Array) {
    this.sent.push(d);
  }
  close(code?: number) {
    if (code !== undefined && code !== 1000 && (code < 3000 || code > 4999)) throw new Error("InvalidAccessError");
    this.closedWith = code ?? 1000;
  }
}

/** The shim's imports over a scratch memory, as the module calls them. */
function shim() {
  const memory = new WebAssembly.Memory({ initial: 1 });
  const host = new WsHost(FakeSocket as unknown as WebSocketLike);
  host.bindMemory(() => memory);
  class Suspending {
    constructor(readonly f: (...a: number[]) => Promise<number>) {}
  }
  const im = host.imports(Suspending as never) as Record<string, ((...a: number[]) => number) | Suspending>;
  const call = (name: string, ...a: number[]) => {
    const f = im[name]!;
    return f instanceof Suspending ? f.f(...a) : Promise.resolve(f(...a));
  };
  const bytes = new Uint8Array(memory.buffer);
  const put = (at: number, s: string) => {
    const b = new TextEncoder().encode(s);
    bytes.set(b, at);
    return b.length;
  };
  return { host, call, bytes, put };
}

test("WB4b: the shim opens with the protocol, queues messages and wakes a waiting read", async () => {
  const { host, call, bytes, put } = shim();
  let woken = 0;
  host.onActivity(() => woken++);
  const ul = put(0, "ws://hub:1/dspi/v1");
  const pl = put(100, "dspi-link-1");
  const sock = await call("dspi_ws_open", 0, ul, 100, pl);
  assert.ok(sock > 0);
  const ws = FakeSocket.last!;
  assert.equal(ws.binaryType, "arraybuffer");
  assert.equal(await call("dspi_ws_poll", sock), 0, "nothing yet");

  // Nobody reading: the message waits, and the Worker is asked for a turn.
  ws.onmessage!({ data: '{"t":"x"}' });
  assert.equal(woken, 1);
  assert.equal(await call("dspi_ws_poll", sock), 1, "text");
  assert.equal(await call("dspi_ws_len", sock), 9);
  assert.equal(await call("dspi_ws_take", sock, 200, 9), 9);
  assert.equal(new TextDecoder().decode(bytes.subarray(200, 209)), '{"t":"x"}');

  // A read that waits is woken by the next message, not the Worker.
  const waiting = call("dspi_ws_wait", sock, 5000);
  ws.onmessage!({ data: new Uint8Array([1, 2, 3]).buffer });
  assert.equal(await waiting, 2, "binary");
  assert.equal(woken, 1);
  assert.equal(await call("dspi_ws_take", sock, 300, 3), 3);
  assert.deepEqual([...bytes.subarray(300, 303)], [1, 2, 3]);

  // Sends are whole messages; a ping is the core's own text.
  put(400, "hello");
  assert.equal(await call("dspi_ws_send", sock, 0, 400, 5), 0);
  assert.equal(await call("dspi_ws_send", sock, 1, 400, 2), 0);
  assert.equal(ws.sent[0], "hello");
  assert.deepEqual([...(ws.sent[1] as Uint8Array)], [104, 101]);

  // A wait with nothing coming ends after its time.
  assert.equal(await call("dspi_ws_wait", sock, 5), 0);

  // A drop (no close frame) has no code; the core reconnects after it. A hub's
  // own code stands even when the browser calls the close unclean.
  const sock2 = await call("dspi_ws_open", 0, ul, 100, pl);
  FakeSocket.last!.onclose!({ code: 4000, wasClean: false });
  assert.equal(await call("dspi_ws_closed", sock2), 4000);
  ws.onclose!({ code: 1006, wasClean: false });
  assert.equal(await call("dspi_ws_poll", sock), 3);
  assert.equal(await call("dspi_ws_closed", sock), -1);
  // A page may close only with 1000 or 3000-4999.
  await call("dspi_ws_close", sock, 1001);
  assert.equal(ws.closedWith, 1000);
  assert.equal(await call("dspi_ws_send", sock, 0, 400, 5), -1, "gone");
});

test("WB4b: a failed open says why, in words that name the Origin rule", async () => {
  class Refused extends FakeSocket {
    constructor(url: string, proto: string) {
      super(url, proto);
      // Before the open: what a refused upgrade looks like to a page.
      this.onopen = null;
      Promise.resolve().then(() => this.onclose?.({ code: 1006, wasClean: false }));
    }
  }
  const mem = new WebAssembly.Memory({ initial: 1 });
  const host = new WsHost(Refused as unknown as WebSocketLike);
  host.bindMemory(() => mem);
  class Suspending {
    constructor(readonly f: (...a: number[]) => Promise<number>) {}
  }
  const im = host.imports(Suspending as never) as Record<string, unknown>;
  const b = new Uint8Array(mem.buffer);
  const u = new TextEncoder().encode("ws://hub:1/dspi/v1");
  b.set(u, 0);
  assert.equal(await (im.dspi_ws_open as Suspending).f(0, u.length, 0, 0), -1);
  const n = (im.dspi_ws_error as (o: number, c: number) => number)(500, 256);
  assert.match(new TextDecoder().decode(b.subarray(500, 500 + n)), /a hub accepts only pages it serves itself/);
  assert.equal(host.openCount, 0);
});

// ---------------------------------------------------------------- the real core

const dir = process.env.DSPI_WASM_DIR ?? "";
const pageModule = dir ? join(dir, "dspi_wasm.wasm") : "";
const fakehub = process.env.DSPI_FAKEHUB ?? "";
const have = !!pageModule && existsSync(pageModule) && !!fakehub && existsSync(fakehub);
const skip = have ? false : "no core module or fake hub (DSPI_WASM_DIR, DSPI_FAKEHUB)";
if (process.env.DSPI_FAKEHUB_REQUIRED && !have) {
  test("WB4b: the fake hub and the module are there", () => assert.fail(`no fake hub "${fakehub}" or module in "${dir}"`));
}

const hubs: ChildProcess[] = [];
const cores: WasmCore[] = [];
after(() => {
  for (const c of cores) c.dispose();
  for (const h of hubs) h.kill();
});

/** A fake hub on a free port: its address, and a way to send it a fault line. */
async function startHub(args: string[]): Promise<{ url: string; fault: (line: string) => Promise<string> }> {
  const child = spawn(fakehub, ["--bind", "127.0.0.1:0", ...args], { stdio: ["pipe", "pipe", "inherit"] });
  hubs.push(child);
  const lines = createInterface({ input: child.stdout! });
  const waiting: ((l: string) => void)[] = [];
  const port = await new Promise<number>((ok) => {
    lines.on("line", (line) => {
      const m = / on 127\.0\.0\.1:(\d+) /.exec(line);
      if (m) ok(Number(m[1]));
      else waiting.shift()?.(line.trim());
    });
  });
  return {
    url: `ws://127.0.0.1:${port}`,
    fault: (line) =>
      new Promise((ok) => {
        waiting.push(ok);
        child.stdin!.write(`${line}\n`);
      }),
  };
}

test("WB4b: the core over Node's WebSocket pairs, opens, writes with read-back and reconnects by itself", { skip }, async () => {
  const hub = await startHub(["--auth", "fixed:424242"]);
  const core = new WasmCore(await loadWasmModule(readFileSync(pageModule)));
  cores.push(core);
  const phases: string[] = [];
  core.onEvent((e: CoreEvent) => {
    if (e.event === "link") phases.push(e.phase);
  });
  const until = async (phase: string) => {
    const end = Date.now() + 15000;
    while (phases.at(-1) !== phase) {
      if (Date.now() > end) assert.fail(`no ${phase}: ${phases.join(" ")}`);
      await sleep(20);
    }
  };

  const hello = await core.call("linkConnect", { url: hub.url });
  assert.equal(hello.needsPairing, true);
  assert.equal(hello.pinSource, "fixed");
  await assert.rejects(core.call("linkPair", { pin: "000000" }), (e: unknown) => e instanceof CoreCallError && e.code === "wrong_pin");
  const { token } = await core.call("linkPair", { pin: "424242" });
  assert.ok(token.length > 0);
  await until("choose_device");
  const devices = await core.call("linkDevices", undefined);
  assert.equal(devices.length, 1);

  const snap = await core.call("open", { kind: "hub", url: hub.url, deviceId: devices[0]!.serial });
  assert.equal(snap.device.serial, devices[0]!.serial);
  assert.deepEqual(await core.call("write", { path: "vol.master", value: { float: -7.5 } }), { float: -7.5 });
  await until("bound");

  assert.equal(await hub.fault("drop"), "ok");
  await until("reconnecting");
  await until("bound");
  assert.equal((await core.call("state", undefined)).state.masterVolumeDb, -7.5);
  assert.deepEqual(await core.call("write", { path: "vol.master", value: { float: -4 } }), { float: -4 });
  await core.call("close", undefined);
});
