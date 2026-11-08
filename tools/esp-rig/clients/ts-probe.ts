// WP LK3: upstream's TypeScript Link client (@dspi/link, DSPi-Link at the
// conformance pin, unmodified source) against the emulated ESP32 bridge.
// Run by tools/esp-rig/rig.sh in the official Node image, which mounts the
// client's source at /upstream and loads it as run-clients.sh does
// (--experimental-transform-types and crates/dspi-hub/tests/conformance/ts/register.mjs).
//
// LK3_URL      ws://10.77.0.2:11915/dspi/v1
// LK3_MODE     meters (default) | config
// LK3_SECONDS  how long to keep the meters running (meters), default 60
// LK3_REREAD_S a 0xA0 bulk re-read every this many seconds while meters run (0: none)
// LK3_CLIENTS  sessions at once (meters), default 1
// LK3_BAUD     config: the UART rate to save, with detection off
// LK3_HZ       the meter poll's rate (meters), default 10 (WP LK4)
//
// Output: `LK3 ...` lines, then `LK3 summary ...` per session.

import {
  LinkClient,
  type DeviceInfo,
  type LinkCloseInfo,
  type PollFrame,
} from "/upstream/js/packages/dspi-link/src/index.ts";

const URL = process.env.LK3_URL ?? "ws://10.77.0.2:11915/dspi/v1";
const MODE = process.env.LK3_MODE ?? "meters";
const SECONDS = Number(process.env.LK3_SECONDS ?? "60");
const REREAD_S = Number(process.env.LK3_REREAD_S ?? "0");
const CLIENTS = Number(process.env.LK3_CLIENTS ?? "1");
const HZ = Number(process.env.LK3_HZ ?? "10");
const REQ_GET_STATUS = 0x50;
const REQ_GET_ALL_PARAMS = 0xa0;
/** A gap between live meter frames longer than this is a freeze. */
const FREEZE_MS = 1000;

const t0 = Date.now();
const now = () => ((Date.now() - t0) / 1000).toFixed(2);
const say = (s: string) => console.log(`LK3 t=${now()}s ${s}`);
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function client(): LinkClient {
  return new LinkClient({ webSocket: WebSocket as never });
}

async function firstDevice(c: LinkClient, limitMs: number): Promise<DeviceInfo | null> {
  const until = Date.now() + limitMs;
  while (Date.now() < until) {
    const d = c.deviceList.find((x) => x.state === "online");
    if (d) return d;
    await sleep(50);
  }
  return null;
}

/** Bytes of the combined meter frame (REQ_GET_STATUS wValue 9): 2n+7 for n channels. */
function meterLen(d: DeviceInfo): number {
  const n = (d.inputs ?? 8) + (d.outputs ?? 9);
  return 2 * n + 7;
}

async function config(): Promise<void> {
  const baud = Number(process.env.LK3_BAUD ?? "0");
  const c = client();
  await c.connect(URL, { clientName: "lk3 config" });
  const before = await c.getHubConfig();
  say(`config before: ${JSON.stringify(before.config.uart)}`);
  const after = await c.setHubConfig({ uart: { baud, detect_baud: false } } as never);
  say(`config after: ${JSON.stringify((after as Record<string, unknown>).uart ?? after)}`);
  c.disconnect();
}

interface Stats {
  connects: number;
  closes: string[];
  firstMeterS: number | null;
  /** WP LK4: the snapshot (every setting) in hand: the reference client's usable UI. */
  usableS: number | null;
  frames: number;
  freezes: number;
  longestGapMs: number;
  rereads: number;
  rereadFailed: number;
  rereadMs: number[];
  cmdFailed: number;
}

async function session(id: number, deadline: number): Promise<Stats> {
  const st: Stats = {
    connects: 0,
    closes: [],
    firstMeterS: null,
    usableS: null,
    frames: 0,
    freezes: 0,
    longestGapMs: 0,
    rereads: 0,
    rereadFailed: 0,
    rereadMs: [],
    cmdFailed: 0,
  };
  let lastFrame = 0;
  while (Date.now() < deadline) {
    const c = client();
    let closed: LinkCloseInfo | null = null;
    c.on("close", (info: LinkCloseInfo) => {
      closed = info;
    });
    c.on("poll", (_p: PollFrame) => {
      const t = Date.now();
      if (st.firstMeterS === null) {
        st.firstMeterS = (t - t0) / 1000;
        say(`c${id} first live meter`);
      }
      if (lastFrame) {
        const gap = t - lastFrame;
        st.longestGapMs = Math.max(st.longestGapMs, gap);
        if (gap > FREEZE_MS) {
          st.freezes += 1;
          say(`c${id} meter freeze ${gap} ms`);
        }
      }
      lastFrame = t;
      st.frames += 1;
    });
    try {
      await c.connect(URL, { clientName: `lk3 ts c${id}` });
      st.connects += 1;
      say(`c${id} connected (session ${st.connects})`);
      const d = await firstDevice(c, 30000);
      if (!d) throw new Error("no device in 30 s");
      try {
        const snap = await c.snapshot(d.handle);
        if (st.usableS === null && snap.bulk) st.usableS = (Date.now() - t0) / 1000;
        say(`c${id} snapshot ${snap.bulk?.length ?? 0} bytes`);
      } catch (e) {
        say(`c${id} snapshot failed: ${(e as Error).message}`);
      }
      const grants = await c.subscribePolls(d.handle, [
        { slot: 1, req: REQ_GET_STATUS, val: 9, idx: 2, len: meterLen(d), hz: HZ },
      ]);
      say(`c${id} poll granted ${JSON.stringify(grants)}`);
      let nextReread = REREAD_S > 0 ? Date.now() + REREAD_S * 1000 : Infinity;
      while (Date.now() < deadline && closed === null) {
        await sleep(100);
        if (Date.now() >= nextReread && closed === null) {
          nextReread = Date.now() + REREAD_S * 1000;
          const t = Date.now();
          st.rereads += 1;
          try {
            await c.commandOk({
              handle: d.handle,
              direction: "get",
              bRequest: REQ_GET_ALL_PARAMS,
              wValue: 0,
              wIndex: 2,
              wLength: 6136,
            });
            st.rereadMs.push(Date.now() - t);
          } catch (e) {
            st.rereadFailed += 1;
            say(`c${id} re-read failed after ${Date.now() - t} ms: ${(e as Error).message}`);
          }
        }
      }
    } catch (e) {
      say(`c${id} error: ${(e as Error).message}`);
      st.cmdFailed += 1;
    }
    if (closed !== null) {
      const info = closed as LinkCloseInfo;
      st.closes.push(`${info.code}:${info.reason}`);
      say(`c${id} closed by hub/link: code=${info.code} reason=${info.reason}`);
    } else {
      c.disconnect();
    }
    if (Date.now() < deadline) await sleep(1000);
  }
  return st;
}

async function meters(): Promise<void> {
  const deadline = Date.now() + SECONDS * 1000;
  const all = await Promise.all(Array.from({ length: CLIENTS }, (_, i) => session(i + 1, deadline)));
  all.forEach((st, i) => {
    const rr = st.rereadMs.length ? Math.max(...st.rereadMs) : 0;
    console.log(
      `LK3 summary client=ts c=${i + 1} first_meter=${st.firstMeterS?.toFixed(2) ?? "never"} ` +
        `usable=${st.usableS?.toFixed(2) ?? "never"} ` +
        `frames=${st.frames} freezes=${st.freezes} longest_gap_ms=${st.longestGapMs} ` +
        `sessions=${st.connects} closes=${st.closes.length} [${st.closes.join(",")}] ` +
        `rereads=${st.rereads} reread_failed=${st.rereadFailed} reread_max_ms=${rr} errors=${st.cmdFailed}`,
    );
  });
}

(MODE === "config" ? config() : meters())
  .then(() => process.exit(0))
  .catch((e) => {
    console.log(`LK3 fatal ${(e as Error).stack ?? e}`);
    process.exit(1);
  });
