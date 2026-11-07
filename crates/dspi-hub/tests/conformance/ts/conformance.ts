// WP HB7: upstream's TypeScript client (`@dspi/link` 1.2.0, WeebLabs/DSPi-Link
// 86f10e3, unmodified) through the shared scenario list (../README.md), against
// `serve::conformance_serve_for_other_clients`. Run by ../run-clients.sh, which
// mounts the client's source at /upstream and answers the `@ctl` lines.
//
// Environment: HB7_URL, HB7_PIN (the first window's PIN), HB7_HUB_ID.
// Output: one `PASS <id>` or `FAIL <id>: <why>` per scenario, then `DONE`.

import { createInterface } from "node:readline";
import {
  LinkClient,
  LinkProtocolError,
  LinkStatus,
  type CmdResponseFrame,
  type LinkCloseInfo,
  type NotifyFrame,
  type PollFrame,
} from "/upstream/js/packages/dspi-link/src/index.ts";

const URL = process.env.HB7_URL ?? "";
const HUB_ID = process.env.HB7_HUB_ID ?? "";
let pin = process.env.HB7_PIN ?? "";

const REQ_GET_PLATFORM = 0x7f;
const REQ_GET_STATUS = 0x50;
const REQ_SET_MASTER_VOLUME = 0xd2;
const WAIT_MS = 10000;

// ---- The driver ----

const lines = createInterface({ input: process.stdin })[Symbol.asyncIterator]();

/** Ask the driver for what only the hub's side can do. */
async function ctl(command: string): Promise<Record<string, unknown>> {
  console.log(`@ctl ${command}`);
  const next = await lines.next();
  if (next.done) throw new Error("the driver went away");
  return JSON.parse(next.value) as Record<string, unknown>;
}

async function newPin(): Promise<string> {
  pin = String((await ctl("pin")).pin);
  return pin;
}

// ---- Helpers ----

function fail(why: string): never {
  throw new Error(why);
}

function expect(cond: boolean, why: string): void {
  if (!cond) fail(why);
}

function client(): LinkClient {
  return new LinkClient({ webSocket: WebSocket as never });
}

/** The next `event` that `match` accepts, or a timeout. */
function next<T>(c: LinkClient, event: "notify" | "poll" | "close", match: (v: T) => boolean = () => true): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      off();
      reject(new Error(`no ${event} in ${WAIT_MS} ms`));
    }, WAIT_MS);
    const off = c.on(event as never, ((v: T) => {
      if (!match(v)) return;
      clearTimeout(timer);
      off();
      resolve(v);
    }) as never);
  });
}

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function getPlatform(c: LinkClient, handle = 0): Promise<CmdResponseFrame> {
  return c.command({ handle, direction: "get", bRequest: REQ_GET_PLATFORM, wValue: 0, wIndex: 2, wLength: 7 });
}

function setVolume(c: LinkClient, db: number): Promise<CmdResponseFrame> {
  const payload = new Uint8Array(4);
  new DataView(payload.buffer).setFloat32(0, db, true);
  return c.command({ handle: 0, direction: "set", bRequest: REQ_SET_MASTER_VOLUME, wValue: 0, wIndex: 2, payload });
}

/** A paired session, with a fresh window's PIN unless this is the first. */
async function paired(name: string, role: "admin" | "control" | "viewer"): Promise<{ c: LinkClient; token: string }> {
  const c = client();
  await c.connect(URL, { clientName: name, app: "wp_hb7 ts", version: "0" });
  const token = await c.pair(await newPin(), name, role);
  expect(c.state === "ready", `state ${c.state} after pairing`);
  return { c, token: token ?? fail("no token") };
}

async function withToken(name: string, token: string): Promise<LinkClient> {
  const c = client();
  await c.connect(URL, { clientName: name, token, app: "wp_hb7 ts", version: "0" });
  return c;
}

// ---- The scenarios ----

const open: LinkClient[] = [];
const keep = <T extends LinkClient>(c: T): T => (open.push(c), c);

const scenarios: [string, () => Promise<void>][] = [
  ["C1 hello", async () => {
    const c = keep(client());
    await c.connect(URL, { clientName: "ts c1" });
    expect(c.state === "awaiting_pairing", `state ${c.state}`);
    const hub = c.hub ?? fail("no hello");
    expect(hub.proto.major === 1, `proto ${JSON.stringify(hub.proto)}`);
    expect(c.hubId === HUB_ID, `hub id ${c.hubId}`);
    expect(hub.auth === "pin", `auth ${hub.auth}`);
    expect(c.hasCap("cmd") && !c.hasCap("fw_install"), `caps ${c.caps}`);
    expect((c.limits?.max_frame ?? 0) > 0, "no max_frame");
  }],
  ["C2 a wrong PIN is typed and a right one then works", async () => {
    const right = await newPin();
    const c = keep(client());
    await c.connect(URL, { clientName: "ts c2" });
    const wrong = right === "000000" ? "111111" : "000000";
    const e = await c.pair(wrong, "ts c2", "control").then(() => null, (e: unknown) => e);
    expect(e instanceof LinkProtocolError && e.code === "unauthenticated", `wrong PIN gave ${e}`);
    expect(c.state === "awaiting_pairing", `state ${c.state}`);
    await c.pair(right, "ts c2", "control");
    expect(c.state === "ready", `state ${c.state}`);
  }],
  ["C3 pair lists the device", async () => {
    const { c } = await paired("ts c3", "admin");
    keep(c);
    expect(c.role === "admin", `role ${c.role}`);
    const devices = await c.refreshDevices();
    expect(devices.length === 1 && devices[0].handle === 0, JSON.stringify(devices));
    expect(devices[0].state === "online" && devices[0].serial.length > 0, JSON.stringify(devices[0]));
  }],
  ["C4 the token reconnects without a PIN", async () => {
    const { c, token } = await paired("ts c4", "control");
    keep(c);
    const again = keep(await withToken("ts c4", token));
    expect(again.state === "ready", `state ${again.state}`);
    expect(again.session !== c.session, "the same session id");
    expect(again.role === "control", `role ${again.role}`);
  }],
  ["C5 commands", async () => {
    const { c } = await paired("ts c5", "control");
    keep(c);
    const platform = await getPlatform(c);
    expect(platform.status === LinkStatus.OK && platform.payload.length === 7, `status ${platform.status}`);
    const missing = await getPlatform(c, 9);
    expect(missing.status === LinkStatus.NO_DEVICE, `status ${missing.status}`);
  }],
  ["C6 a write is heard with its origin", async () => {
    const { c, token } = await paired("ts c6", "control");
    keep(c);
    const other = keep(await withToken("ts c6", token));
    const own = next<NotifyFrame>(c, "notify");
    const heard = next<NotifyFrame>(other, "notify");
    const set = await setVolume(c, -12);
    expect(set.status === LinkStatus.OK, `status ${set.status}`);
    const echo = await own;
    expect(echo.origin === c.session, `echo origin ${echo.origin}, session ${c.session}`);
    const foreign = await heard;
    expect(foreign.origin === c.session, `foreign origin ${foreign.origin}`);
    expect(foreign.packet[0] === 2, "not a v2 packet");
  }],
  ["C7 a change at the device has origin 0", async () => {
    const { c } = await paired("ts c7", "viewer");
    keep(c);
    const heard = next<NotifyFrame>(c, "notify");
    await ctl("change");
    expect((await heard).origin === 0, "origin not 0");
  }],
  ["C8 snapshot", async () => {
    const { c } = await paired("ts c8", "viewer");
    keep(c);
    const snap = await c.snapshot(0);
    expect(snap.handle === 0 && (snap.bulk?.length ?? 0) > 0, `bulk ${snap.bulk?.length}`);
  }],
  ["C9 polls arrive and stop", async () => {
    const { c } = await paired("ts c9", "viewer");
    keep(c);
    let count = 0;
    const off = c.on("poll", (p: PollFrame) => {
      if (p.handle === 0 && p.slot === 1 && p.payload.length > 0) count++;
    });
    const grants = await c.subscribePolls(0, [{ slot: 1, req: REQ_GET_STATUS, val: 9, idx: 2, len: 41, hz: 10 }]);
    expect(grants.length === 1 && grants[0].slot === 1, JSON.stringify(grants));
    const start = Date.now();
    while (count < 3 && Date.now() - start < WAIT_MS) await sleep(20);
    expect(count >= 3, `${count} polls`);
    await c.unsubscribePolls(0, [1]);
    await sleep(300);
    const before = count;
    await sleep(500);
    off();
    expect(count === before, "polls after the unsubscribe");
  }],
  ["C10 a lock holds others off", async () => {
    const { c, token } = await paired("ts c10", "control");
    keep(c);
    const other = keep(await withToken("ts c10", token));
    await c.acquireLock(0, "conformance", 5000);
    expect((await setVolume(other, -20)).status === LinkStatus.LOCKED, "not LOCKED");
    expect((await setVolume(c, -20)).status === LinkStatus.OK, "the holder could not write");
    await c.releaseLock(0);
    expect((await setVolume(other, -21)).status === LinkStatus.OK, "still locked");
  }],
  ["C11 a viewer reads and may not write", async () => {
    const { c } = await paired("ts c11", "viewer");
    keep(c);
    expect(c.role === "viewer", `role ${c.role}`);
    expect((await getPlatform(c)).status === LinkStatus.OK, "read refused");
    expect((await setVolume(c, -6)).status === LinkStatus.DENIED, "not DENIED");
  }],
  ["C12 a revoke closes 4002 and the token is refused", async () => {
    const { c, token } = await paired("ts c12 revoked", "viewer");
    const closed = next<LinkCloseInfo>(c, "close");
    expect((await ctl("revoke ts c12 revoked")).revoked === true, "not revoked");
    const info = await closed;
    expect(info.code === 4002 && info.reason === "revoked" && !info.willReconnect, JSON.stringify(info));
    const again = keep(client());
    let rejected = false;
    again.on("tokenRejected", () => (rejected = true));
    await again.connect(URL, { clientName: "ts c12 revoked", token });
    expect(again.state === "awaiting_pairing", `state ${again.state}`);
    expect(rejected, "no tokenRejected");
  }],
  ["K1 the browser keepalive is answered", async () => {
    // Node's WebSocket has no ping(), so this client keeps alive the way a
    // browser does: a `ping` request, which a hub answers `unknown_type`.
    const { token } = await paired("ts k1", "viewer");
    const c = keep(new LinkClient({ webSocket: WebSocket as never, pingIntervalMs: 200 }));
    await c.connect(URL, { clientName: "ts k1", token });
    const errors: Error[] = [];
    c.on("error", (e) => errors.push(e));
    await sleep(1500);
    expect(c.state === "ready", `state ${c.state}`);
    expect(errors.length === 0, `errors ${errors}`);
  }],
  ["C13 a stop closes 4003", async () => {
    const { c } = await paired("ts c13", "viewer");
    const closed = next<LinkCloseInfo>(c, "close");
    await ctl("stop");
    const info = await closed;
    expect(info.code === 4003 && info.reason === "shutting_down", JSON.stringify(info));
  }],
];

let failed = 0;
for (const [id, run] of scenarios) {
  try {
    await run();
    console.log(`PASS ${id}`);
  } catch (e) {
    failed++;
    console.log(`FAIL ${id}: ${e instanceof Error ? e.message : e}`);
  }
  for (const c of open.splice(0)) c.disconnect();
}
console.log(`DONE ${scenarios.length - failed}/${scenarios.length}`);
process.exit(failed === 0 ? 0 : 1);
