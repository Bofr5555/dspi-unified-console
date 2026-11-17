// WP P5WCAP: the P5SPIKE page (PLAN-P5 section 2, P5SPIKE): development only, never part of
// the built console (tools/spike/build.mjs builds it beside a copy of dist/).
//
// It opens a DSPi through the console's own core Worker (USB in Chromium, or a hub, which is
// how Firefox reaches one: `uconsole web`), opens the microphone with the console's capture
// (src/core/capture.ts), runs P5SES's `measure_start` with the setup in the text box (its
// ceiling at the start level: the owner present, PLAN-P5 section 7), pushes every capture block
// with its frame index into `measure_capture_push`, turns `measure_tick`, and shows and
// downloads what the timing says (the clock ratio, every cycle's residual, the verdict) with the
// frame gaps this page saw. The page itself plays nothing; the record holds no audio (S12).
//
// The measure calls are P5SES's, by the names its web binding gives them (crates/dspi-wasm/src/
// measure.rs: `measurePreflight` and `measureStart` take `{setup}`, `measureCapturePush` the
// block). Review P5-B wired them here (`SpikeCore`), with the setup the session really takes;
// `startLevelOnly` is forced on whatever the text box says, and the record is built from the
// session's state when the run ends, whether or not it has a result.

import { CoreClient } from "../../src/core/client";
import {
  bindCapturePort,
  listMicrophones,
  measurePushParams,
  startCapture,
  type Capture,
  type CaptureBlock,
  type FrameContinuity,
} from "../../src/core/capture";
import { DSPI_FILTERS } from "../../src/core/usb-ids";
import type { LinkDeviceRecord, LinkHelloRecord } from "../../src/core/protocol";

/** The calls this page makes beyond the console's typed ones (P5SES's names). */
interface SpikeCore {
  preflight(setup: unknown): Promise<unknown>;
  start(setup: unknown): Promise<unknown>;
  push(b: CaptureBlock): Promise<unknown>;
  tick(): Promise<unknown>;
  state(): Promise<unknown>;
  stop(): Promise<unknown>;
  result(): Promise<unknown>;
  generator(): Promise<unknown>;
  // -- WP P5DEMO -- the demo's simulated microphone
  sim(on: boolean): Promise<unknown>;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
const logEl = $("log");
function log(line: string): void {
  logEl.textContent = `${new Date().toISOString().slice(11, 23)} ${line}\n${logEl.textContent ?? ""}`.slice(0, 20_000);
}
const show = (id: string, v: unknown) => ($(id).textContent = typeof v === "string" ? v : JSON.stringify(v, null, 2));
const errText = (e: unknown) => {
  const x = e as { code?: string; noticeKey?: string; message?: string };
  return `${x.code ?? "error"}${x.noticeKey ? ` [${x.noticeKey}]` : ""}: ${x.message ?? String(e)}`;
};

const workerUrl = new URL("../worker.js", location.href);
const workletUrl = new URL("../capture-worklet.js", location.href);
let client: CoreClient | null = null;
let core: SpikeCore | null = null;
let capture: Capture | null = null;
let continuity: FrameContinuity | null = null;
const discontinuities: { atBlock: number; kind: string; detail: unknown }[] = [];
let pushFailures = 0;
let ticker: ReturnType<typeof setInterval> | null = null;
let record: unknown = null;
// -- WP P5DEMO -- the demo's simulated microphone is on: the core hears the generator itself
// (P5SIM's room), the page opens no microphone and pushes nothing, the tick feeds the session.
let simulated: { label?: string; on?: boolean } | null = null;

function spikeCore(c: CoreClient): SpikeCore {
  const call = (m: string, p: unknown) => c.call(m as never, p as never) as Promise<unknown>;
  return {
    preflight: (setup) => call("measurePreflight", { setup }),
    start: (setup) => call("measureStart", { setup }),
    push: (b) => call("measureCapturePush", measurePushParams(b)),
    tick: () => call("measureTick", undefined),
    state: () => call("measureState", undefined),
    stop: () => call("measureStop", undefined),
    result: () => call("measureResult", undefined),
    generator: () => call("siggenStatus", undefined),
    sim: (on) => call("measureSimSet", { on, seed: 1 }),
  };
}

function freshClient(): CoreClient {
  simulated = null; // -- WP P5DEMO --
  void client?.dispose();
  client = CoreClient.spawn(workerUrl);
  core = spikeCore(client);
  return client;
}

// ---- 1. the DSPi ----

const servingHub = document.querySelector<HTMLMetaElement>('meta[name="dspi-hub"]')?.content ?? "";
if (servingHub) ($("hub-url") as HTMLInputElement).value = servingHub;
const usb = (navigator as unknown as { usb?: { requestDevice(o: unknown): Promise<{ serialNumber?: string }> } }).usb;
($("usb") as HTMLButtonElement).disabled = !usb;

$("usb").addEventListener("click", async () => {
  try {
    const d = await usb!.requestDevice({ filters: DSPI_FILTERS });
    const c = freshClient();
    const snap = await c.call("open", { kind: "usb", serial: d.serialNumber ?? "" });
    show("device", `Open on USB: ${JSON.stringify(snap).slice(0, 200)}…`);
    log("USB device open");
  } catch (e) {
    show("device", errText(e));
  }
});

$("demo").addEventListener("click", async () => {
  try {
    const c = freshClient();
    await c.call("open", { kind: "demo" });
    // The demo's volume is above what a measurement starts at (S2). It is a simulated device,
    // so the page lowers it; a real DSPi is never written to (its preflight says what to change).
    await c.call("write" as never, { path: "vol.user", value: { float: -20 } } as never);
    show("device", "The demo DSPi is open (simulated; nothing is played anywhere). Its volume is set to -20 dB.");
    log("demo device open");
  } catch (e) {
    show("device", errText(e));
  }
});

let hubUrl = "";
$("hub").addEventListener("click", async () => {
  hubUrl = ($("hub-url") as HTMLInputElement).value.trim();
  try {
    const c = freshClient();
    const hello: LinkHelloRecord = await c.call("linkConnect", { url: hubUrl, clientName: "P5SPIKE" });
    if (hello.needsPairing) {
      const pin = ($("hub-pin") as HTMLInputElement).value.trim();
      if (!pin) throw new Error("The hub wants a PIN: type it and connect again");
      await c.call("linkPair", { pin });
    }
    const devices: LinkDeviceRecord[] = await c.call("linkDevices", undefined);
    const sel = $("hub-devices") as HTMLSelectElement;
    sel.replaceChildren(
      ...devices.map((d) => {
        const o = document.createElement("option");
        o.value = d.serial;
        o.textContent = `${d.name} · ${d.serial} · ${d.state}`;
        return o;
      }),
    );
    ($("hub-open") as HTMLButtonElement).disabled = devices.length === 0;
    show("device", `Hub ${hello.hubName}: ${devices.length} DSPi`);
  } catch (e) {
    show("device", errText(e));
  }
});

$("hub-open").addEventListener("click", async () => {
  const deviceId = ($("hub-devices") as HTMLSelectElement).value;
  try {
    await client!.call("open", { kind: "hub", url: hubUrl, deviceId });
    show("device", `Open through the hub: ${deviceId}`);
    log(`hub device ${deviceId} open`);
  } catch (e) {
    show("device", errText(e));
  }
});

// ---- 2. the microphone ----

async function refreshMics(): Promise<void> {
  try {
    const mics = await listMicrophones();
    const sel = $("mics") as HTMLSelectElement;
    const chosen = sel.value;
    const def = document.createElement("option");
    def.value = "";
    def.textContent = "The browser's default";
    sel.replaceChildren(
      def,
      ...mics.map((m) => {
        const o = document.createElement("option");
        o.value = m.deviceId;
        o.textContent = m.labelled ? m.label : `(name shown after the permission) ${m.deviceId.slice(0, 8)}`;
        return o;
      }),
    );
    sel.value = chosen;
  } catch (e) {
    show("mic", errText(e));
  }
}
void refreshMics();

$("mic-start").addEventListener("click", async () => {
  if (capture && !capture.stopped) return;
  if (!core) {
    show("mic", "Open a DSPi first: the blocks go to its core.");
    return;
  }
  const c = core;
  try {
    capture = await startCapture({ deviceId: ($("mics") as HTMLSelectElement).value || undefined, workletUrl });
  } catch (e) {
    show("mic", errText(e));
    return;
  }
  discontinuities.length = 0;
  pushFailures = 0;
  const bound = bindCapturePort(capture.port, {
    push: async (b) => void (await c.push(b)),
    end: () => log("capture ended"),
    issue: (i) => {
      if (i.kind === "push_failed") {
        // The core refusing a push (no run, or the run aborted) is its answer: count it.
        if (pushFailures++ < 3) log(`push refused: ${errText(i.error)}`);
        return;
      }
      discontinuities.push({ atBlock: bound.continuity.blocks, kind: i.kind, detail: i });
      log(`capture ${i.kind}: ${JSON.stringify(i)}`);
    },
  });
  continuity = bound.continuity;
  capture.onEvent((e) => log(`microphone ${e.kind}`));
  show("mic", capture.report);
  // The default setup follows the microphone's rate until the user edits it.
  const box = $("setup") as HTMLTextAreaElement;
  if (box.value === defaultSetup(48_000)) box.value = defaultSetup(capture.report.rateHz);
  void refreshMics();
});

$("mic-stop").addEventListener("click", () => void capture?.stop());

// -- WP P5DEMO -- the demo's simulated microphone (refused on a real DSPi: `unsupported`)
$("mic-sim").addEventListener("click", async () => {
  if (!core) {
    show("mic", "Open the demo DSPi first: the simulated microphone is the demo's.");
    return;
  }
  try {
    await capture?.stop();
    simulated = (await core.sim(true)) as { label?: string; on?: boolean };
    show("mic", { simulatedMicrophone: simulated, rateHz: 48_000 });
    log(`${simulated.label ?? "simulated microphone"} on`);
  } catch (e) {
    simulated = null;
    show("mic", errText(e));
  }
});

// ---- 3. the run ----

/**
 * The session's setup (`MeasureSetup`): two outputs walked, three passes, on a coupler (the
 * start level is -60 dBFS; a speaker placement's is -50), the start level only.
 */
function defaultSetup(rateHz: number): string {
  return JSON.stringify(
    {
      kind: "walked",
      outputs: [
        { output: 0, role: "full", speaker: "L" },
        { output: 1, role: "full", speaker: "L" },
      ],
      placement: "headphones",
      coupler: "flatPlate",
      passes: 3,
      startLevelOnly: true,
      captureRateHz: rateHz,
      name: "P5SPIKE (web)",
    },
    null,
    2,
  );
}
($("setup") as HTMLTextAreaElement).value = defaultSetup(48_000);

/** The text box's setup; the start level only, whatever it says (the page never plays above it). */
function setup(): Record<string, unknown> {
  const s = JSON.parse(($("setup") as HTMLTextAreaElement).value) as Record<string, unknown>;
  return { ...s, startLevelOnly: true };
}

$("preflight").addEventListener("click", async () => {
  try {
    show("state", await core!.preflight(setup()));
  } catch (e) {
    show("state", errText(e));
  }
});

$("run").addEventListener("click", async () => {
  if ((!capture || capture.stopped) && !simulated?.on) {
    show("state", "Start the microphone first: the run needs its capture from the pre-roll on.");
    return;
  }
  try {
    show("state", await core!.start(setup()));
    log("run started");
  } catch (e) {
    show("state", errText(e));
    return;
  }
  if (ticker) clearInterval(ticker);
  phases.length = 0;
  maxLevelDb = null;
  stoppedBy = "";
  record = null;
  delete $("timing").dataset.record;
  ($("download") as HTMLButtonElement).disabled = true;
  let busy = false;
  ticker = setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      const st = (await core!.tick()) as SessionState;
      notePhase(st);
      show("state", st);
      if (st.active === false) {
        if (ticker) clearInterval(ticker);
        ticker = null;
        await buildRecord();
      }
    } catch (e) {
      show("state", errText(e));
    } finally {
      busy = false;
    }
  }, 50);
});

/** What the page reads of the session's state (`MeasureState`). */
interface SessionState {
  phase?: string;
  active?: boolean;
  levelDb?: number | null;
  frames?: number;
  resultReady?: boolean;
  timing?: { reliable?: boolean } | null;
  issue?: { key?: string; message?: string } | null;
}
const phases: { phase: string; atMs: number; frames: number }[] = [];
let maxLevelDb: number | null = null;
let stoppedBy = "";
let runT0 = 0;
function notePhase(st: SessionState): void {
  if (typeof st.levelDb === "number") maxLevelDb = maxLevelDb === null ? st.levelDb : Math.max(maxLevelDb, st.levelDb);
  if (!st.phase || phases[phases.length - 1]?.phase === st.phase) return;
  if (phases.length === 0) runT0 = performance.now();
  phases.push({ phase: st.phase, atMs: Math.round(performance.now() - runT0), frames: st.frames ?? 0 });
  log(`session: ${st.phase}`);
}

async function stopAll(): Promise<void> {
  const wasRunning = ticker !== null;
  if (ticker) clearInterval(ticker);
  ticker = null;
  try {
    const st = (await core?.stop()) as SessionState | undefined;
    if (st) notePhase(st);
  } catch (e) {
    log(`stop: ${errText(e)}`);
  }
  if (wasRunning) {
    stoppedBy = "user";
    await buildRecord();
  }
  await capture?.stop();
  log("stopped");
}
$("run-stop").addEventListener("click", () => void stopAll());
$("stop-all").addEventListener("click", () => void stopAll());

// ---- 4. timing ----

/** Every value under a key that names timing, wherever it is in `v`. */
function timingParts(v: unknown, path = "", out: Record<string, unknown> = {}, depth = 0): Record<string, unknown> {
  if (depth > 12 || v === null || typeof v !== "object") return out;
  for (const [k, x] of Object.entries(v as Record<string, unknown>)) {
    const p = path ? `${path}.${k}` : k;
    if (/residual|ratio|ppm|drift|timing|arrival|onset|cycle|gate|reliable/i.test(k)) out[p] = x;
    else timingParts(x, p, out, depth + 1);
  }
  return out;
}

/**
 * The record of a run, from the session's state (and its document when it has one): shown, and
 * ready to download. Built when the session ends, by itself or by Stop.
 */
async function buildRecord(): Promise<void> {
  try {
    const state = (await core!.state()) as SessionState;
    notePhase(state);
    let doc: unknown = null;
    if (state.resultReady) {
      const raw = await core!.result();
      doc = typeof raw === "string" ? JSON.parse(raw) : raw;
    }
    const generatorAfter = await core!.generator().catch((e) => errText(e));
    const verdict = state.timing
      ? state.timing.reliable
        ? "timing reliable"
        : "timing unreliable"
      : `no timing: ${state.phase ?? ""} ${state.issue?.key ?? ""} ${state.issue?.message ?? ""}`.trim();
    record = {
      kind: "p5spike",
      client: "web",
      savedUtc: new Date().toISOString(),
      userAgent: navigator.userAgent,
      capture: capture?.report ?? null,
      simulatedMicrophone: simulated, // -- WP P5DEMO --
      frames: continuity && { blocks: continuity.blocks, frames: continuity.frames, gaps: continuity.gaps, repeats: continuity.repeats, clippedBlocks: continuity.clippedBlocks },
      discontinuities,
      pushFailures,
      setup: setup(),
      phases,
      maxLevelDb,
      stoppedBy,
      verdict,
      timing: state.timing ?? null,
      timingParts: { ...timingParts(state), ...timingParts(doc) },
      state,
      generatorAfter,
      result: doc,
    };
    show("timing", { verdict, frames: (record as { frames: unknown }).frames, discontinuities, timing: state.timing ?? null });
    $("timing").dataset.record = JSON.stringify(record, (k, v) => (k === "result" ? undefined : v));
    ($("download") as HTMLButtonElement).disabled = false;
    log(`record ready: ${verdict}`);
  } catch (e) {
    show("timing", errText(e));
  }
}

$("result").addEventListener("click", () => void buildRecord());

$("download").addEventListener("click", () => {
  if (!record) return;
  const blob = new Blob([JSON.stringify(record, null, 2)], { type: "application/json" });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  const browser = /Firefox\//.test(navigator.userAgent) ? "firefox" : "chromium";
  a.download = `p5spike-${browser}-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
  a.click();
  setTimeout(() => URL.revokeObjectURL(a.href), 1000);
});

addEventListener("pagehide", () => void stopAll());
