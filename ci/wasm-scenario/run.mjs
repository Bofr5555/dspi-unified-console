// The wasm gate's scenario (WP WB2): dspi-core on the demo device under Node and
// JSPI, two sessions at once, with no threads. `node run.mjs dspi_wasm_scenario.wasm`
// prints one line per step and exits 1 on the first failure.
//
// The imports are the page's: the clock seam (dspi_transport::clock, dspi-link's
// clock), with the sleep a JSPI `Suspending` timer, and getrandom's custom
// backend. Every export is wrapped in `WebAssembly.promising` and awaited before
// the next call: one call at a time, as the binding's queue will (WEB-STUDY 2).
import { readFileSync } from 'node:fs';
import { webcrypto } from 'node:crypto';

if (typeof WebAssembly.Suspending !== 'function' || typeof WebAssembly.promising !== 'function') {
  console.error('wasm-scenario: this Node has no JSPI (WebAssembly.Suspending)');
  process.exit(2);
}

let memory;
let timersFired = 0;
const imports = {
  dspi: {
    dspi_now_ms: () => performance.now(),
    dspi_unix_ms: () => Date.now(),
    dspi_sleep_ms: new WebAssembly.Suspending(
      (ms) => new Promise((resolve) => setTimeout(resolve, ms))),
    dspi_random_fill: (ptr, len) => {
      for (let at = 0; at < len; at += 65536) {
        webcrypto.getRandomValues(new Uint8Array(memory.buffer, ptr + at, Math.min(65536, len - at)));
      }
      return 0;
    },
  },
};

const { instance } = await WebAssembly.instantiate(readFileSync(process.argv[2]), imports);
memory = instance.exports.memory;
const x = {};
for (const [name, f] of Object.entries(instance.exports)) {
  if (typeof f === 'function') x[name] = WebAssembly.promising(f);
}
const importNames = WebAssembly.Module.imports(new WebAssembly.Module(readFileSync(process.argv[2])))
  .map((i) => `${i.module}.${i.name}`);

function lastError() {
  const p = instance.exports.last_error_ptr();
  const n = instance.exports.last_error_len();
  return new TextDecoder().decode(new Uint8Array(memory.buffer, p, n));
}

async function call(name, ...args) {
  const r = await x[name](...args);
  if (r === -1) throw new Error(`${name}: ${lastError()}`);
  return r;
}

let failed = false;
function check(what, ok, detail = '') {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}${detail ? `: ${detail}` : ''}`);
  if (!ok) failed = true;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

try {
  check('imports are the seam', importNames.every((n) => n.startsWith('dspi.')), importNames.join(' '));

  // JSPI: a sleep inside the core suspends it, and the event loop runs meanwhile.
  const t = setInterval(() => { timersFired += 1; }, 5);
  const before = performance.now();
  await x.clock_sleep(60);
  clearInterval(t);
  const slept = performance.now() - before;
  check('clock::sleep suspends under JSPI', slept >= 59 && timersFired >= 3,
    `${slept.toFixed(1)} ms, ${timersFired} timer ticks meanwhile`);

  // Two device sessions at once, the hub's many installations: one whose core
  // reads the demo's notification endpoint itself (drained on the caller's
  // calls), one whose page reads the endpoint and pushes the packets in, as a
  // WebUSB page does.
  const a = await x.session_new();
  const b = await x.session_new();
  check('two sessions', a !== b, `handles ${a} and ${b}`);
  await call('session_open_demo', a);
  await call('session_open_demo_endpoint', b);
  for (const h of [a, b]) {
    check(`session ${h}: demo connected`, (await call('session_connected', h)) === 1);
  }

  // A write with read-back from the device, different on each session.
  const ra = await call('session_write_read_back', a, -17.5);
  const rb = await call('session_write_read_back', b, -6.25);
  check(`session ${a}: write read back`, ra === -1750, `${ra / 100} dB`);
  check(`session ${b}: write read back`, rb === -625, `${rb / 100} dB`);

  // Notifications: another host renames each device.
  let lastSeq = -2;
  async function pump(h) {
    if (h !== b) return;
    const s = await call('session_endpoint_pump', h);
    if (s >= 0) lastSeq = s;
  }
  for (const h of [a, b]) {
    await call('session_rename_elsewhere', h);
    let flags = 0;
    for (let i = 0; i < 20 && !(flags & (1 << 8)); i += 1) {
      await pump(h);
      flags |= await call('session_process_notifications', h);
      await sleep(5);
    }
    check(`session ${h}: a foreign rename arrives as a notification`, (flags & (1 << 8)) !== 0,
      `flags 0x${flags.toString(16)}`);
    check(`session ${h}: the mirror has the new name`, (await call('session_renamed', h)) === 1);
  }
  check(`session ${b}: the page's endpoint loop saw the device's packets`, lastSeq >= 0, `last seq ${lastSeq}`);

  // Packets the page made itself, pushed in: a PARAM_CHANGED of the master
  // volume from a control surface. The next sequence number is patched in;
  // one after a gap reads as a loss, and the image is read again.
  async function pushMaster(h, seq, db) {
    const pkt = new Uint8Array(16);
    const dv = new DataView(pkt.buffer);
    pkt.set([2, 0x02, 0, seq & 0xff]); // v2, NOTIFY_EVT_PARAM_CHANGED, seq
    dv.setUint16(4, instance.exports.off_master_volume(), true);
    dv.setUint16(6, 4, true);
    pkt.set([5, 0, 0, 0], 8); // PARAM_SRC_GPIO: a knob, not this host
    dv.setFloat32(12, db, true);
    const p = instance.exports.buf_alloc(pkt.length);
    new Uint8Array(memory.buffer, p, pkt.length).set(pkt);
    try {
      await call('session_push_notification', h, p, pkt.length);
    } finally {
      instance.exports.buf_free(p, pkt.length);
    }
    return call('session_process_notifications', h);
  }
  const patched = await pushMaster(b, lastSeq + 1, -23.5);
  const mv = await call('session_master_volume', b);
  check(`session ${b}: a pushed packet is patched in`,
    (patched & 1) !== 0 && (patched & 4) === 0 && mv === -2350,
    `flags 0x${patched.toString(16)}, master ${mv / 100} dB`);
  const lost = await pushMaster(b, lastSeq + 10, -30);
  const mvLost = await call('session_master_volume', b);
  check(`session ${b}: a pushed packet after a gap re-reads the device`,
    (lost & 4) !== 0 && mvLost !== -3000, `flags 0x${lost.toString(16)}, master ${mvLost / 100} dB`);
  check(`session ${a}: untouched by the other's packets`,
    (await call('session_master_volume', a)) !== -2350);

  // RTA frames, both analysers driven by the page's timers, interleaved.
  for (const h of [a, b]) await call('session_rta_start', h);
  const due = { [a]: 0, [b]: 0 };
  const frames = { [a]: 0, [b]: 0 };
  let ticks = 0;
  const start = performance.now();
  while ((frames[a] < 1000 || frames[b] < 1000) && performance.now() - start < 10000) {
    const h = due[a] <= due[b] ? a : b;
    const wait = due[h] - performance.now();
    if (wait > 0) await sleep(wait);
    const next = await call('session_rta_tick', h);
    ticks += 1;
    if (next === -2) break;
    due[h] = performance.now() + next;
    frames[h] = await call('session_rta_frames', h);
  }
  for (const h of [a, b]) {
    check(`session ${h}: analyser probed`, (await call('session_rta_supported', h)) === 1);
    const f = frames[h];
    check(`session ${h}: RTA frames`, f >= 1000 && f % 1000 > 0,
      `${Math.floor(f / 1000)} channels, ${f % 1000} bands, after ${ticks} ticks in all`);
  }

  // An AutoEQ rebuild with no thread and no file: each poll is one step, its
  // fetches suspended under JSPI, the database handed back as bytes.
  await x.rebuild_start();
  let polls = 0;
  let entries = -3;
  while (entries === -3 && polls < 100) {
    entries = await call('rebuild_poll');
    polls += 1;
    await sleep(1);
  }
  check('AutoEQ rebuild, caller-driven, into bytes', entries === 1, `${entries} profile after ${polls} polls`);

  for (const h of [a, b]) {
    await call('session_free', h);
  }
  check('sessions closed', true);
} catch (e) {
  check('scenario', false, e.stack || String(e));
}

process.exit(failed ? 1 : 0);
