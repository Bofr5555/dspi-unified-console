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
// The measure_* calls are P5SES's (same wave). Until it is merged the core answers that it
// does not know them; the names and the push's parameters are this page's typed seam
// (`SpikeCore`), the only place to change if P5SES names them otherwise.

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

function spikeCore(c: CoreClient): SpikeCore {
  const call = (m: string, p: unknown) => c.call(m as never, p as never) as Promise<unknown>;
  return {
    preflight: (setup) => call("measure_preflight", setup),
    start: (setup) => call("measure_start", setup),
    push: (b) => call("measure_capture_push", measurePushParams(b)),
    tick: () => call("measure_tick", undefined),
    state: () => call("measure_state", undefined),
    stop: () => call("measure_stop", undefined),
    result: () => call("measure_result", undefined),
  };
}

function freshClient(): CoreClient {
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
    await freshClient().call("open", { kind: "demo" });
    show("device", "The demo DSPi is open (simulated; nothing is played anywhere).");
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

// ---- 3. the run ----

/** A first guess at P5SES's setup: two outputs walked, three passes, the ceiling at the start level. */
function defaultSetup(rateHz: number): string {
  return JSON.stringify(
    {
      kind: "walked",
      outputs: [
        { output: 0, role: "Low" },
        { output: 1, role: "High" },
      ],
      alignmentGroups: [[0, 1]],
      placement: { mode: "listeningPosition" },
      passes: 3,
      ceilingDb: -60,
      captureRateHz: rateHz,
    },
    null,
    2,
  );
}
($("setup") as HTMLTextAreaElement).value = defaultSetup(48_000);

function setup(): unknown {
  return JSON.parse(($("setup") as HTMLTextAreaElement).value);
}

$("preflight").addEventListener("click", async () => {
  try {
    show("state", await core!.preflight(setup()));
  } catch (e) {
    show("state", errText(e));
  }
});

$("run").addEventListener("click", async () => {
  if (!capture || capture.stopped) {
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
  let n = 0;
  let busy = false;
  ticker = setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      await core!.tick();
      if (n++ % 3 === 0) show("state", await core!.state());
    } catch (e) {
      show("state", errText(e));
    } finally {
      busy = false;
    }
  }, 100);
});

async function stopAll(): Promise<void> {
  if (ticker) clearInterval(ticker);
  ticker = null;
  try {
    await core?.stop();
  } catch (e) {
    log(`stop: ${errText(e)}`);
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

$("result").addEventListener("click", async () => {
  try {
    const raw = await core!.result();
    const doc = typeof raw === "string" ? JSON.parse(raw) : raw;
    const state = await core!.state().catch((e) => errText(e));
    const timing = { ...timingParts(state), ...timingParts(doc) };
    record = {
      kind: "p5spike",
      savedUtc: new Date().toISOString(),
      userAgent: navigator.userAgent,
      capture: capture?.report ?? null,
      frames: continuity && { blocks: continuity.blocks, frames: continuity.frames, gaps: continuity.gaps, repeats: continuity.repeats, clippedBlocks: continuity.clippedBlocks },
      discontinuities,
      pushFailures,
      setup: setup(),
      timing,
      state,
      result: doc,
    };
    show("timing", { frames: (record as { frames: unknown }).frames, discontinuities, timing });
    ($("download") as HTMLButtonElement).disabled = false;
  } catch (e) {
    show("timing", errText(e));
  }
});

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
