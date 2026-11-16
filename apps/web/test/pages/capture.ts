// WP P5WCAP: the microphone capture's e2e page. It runs the real capture (src/core/capture.ts)
// on the browser's fake microphone, keeps what arrives on the capture port (here, in a test page
// only: the console never keeps raw audio, S12), and records every `connect` the page's Web Audio
// graph makes, so the test can show nothing reaches a destination. A refusal shows the core's
// notice row (the real core's table, through its Worker).

import { CoreClient } from "../../src/core/client";
import { bindCapturePort, browserCaptureEnv, listMicrophones, startCapture, type Capture, type CaptureEvent, type CaptureStream } from "../../src/core/capture";
import { loadNotices, noticeOf } from "../../src/ui/texts";
import table from "../../../../assets/notices/notices.json";

interface Connection {
  from: string;
  to: string;
  toDestination: boolean;
}
const connections: Connection[] = [];
const nameOf = (n: unknown) => (n as { constructor?: { name?: string } })?.constructor?.name ?? typeof n;
const realConnect = AudioNode.prototype.connect as (this: AudioNode, ...a: unknown[]) => unknown;
(AudioNode.prototype as unknown as { connect: (...a: unknown[]) => unknown }).connect = function (this: AudioNode, ...a: unknown[]) {
  const to = a[0];
  connections.push({ from: nameOf(this), to: nameOf(to), toDestination: to instanceof AudioDestinationNode });
  return realConnect.apply(this, a);
};

const state = {
  report: null as Capture["report"] | null,
  blocks: [] as { firstFrame: number; len: number; rateHz: number; clipped: boolean }[],
  samples: [] as number[],
  ended: false,
  events: [] as CaptureEvent["kind"][],
  error: null as { code: string; noticeKey: string | null; message: string } | null,
  trackStates: [] as string[],
  microphones: [] as { deviceId: string; label: string; labelled: boolean }[],
};
let capture: Capture | null = null;
let stream: CaptureStream | null = null;
let unbind: (() => void) | null = null;
let continuity: { frames: number; blocks: number; gaps: number; repeats: number } | null = null;

const core = CoreClient.spawn(new URL("../worker.js", location.href));
const show = (s: string) => (document.getElementById("state")!.textContent = s);

async function start(deviceId?: string): Promise<void> {
  const env = browserCaptureEnv();
  const gum = env.mediaDevices!.getUserMedia.bind(env.mediaDevices);
  env.mediaDevices = {
    enumerateDevices: env.mediaDevices!.enumerateDevices.bind(env.mediaDevices),
    getUserMedia: async (c) => (stream = await gum(c)),
  };
  try {
    capture = await startCapture({ deviceId, workletUrl: new URL("../capture-worklet.js", location.href), env });
  } catch (e) {
    const err = e as { code?: string; noticeKey?: string; message?: string };
    state.error = { code: err.code ?? "", noticeKey: err.noticeKey ?? null, message: err.message ?? String(e) };
    // The core's table; where the core cannot load (Firefox without JSPI), the same asset.
    await Promise.race([loadNotices(core), new Promise((r) => setTimeout(r, 3000))]);
    const n = noticeOf(e) ?? (table as { notices: { key: string; title: string; reason: string; action: string }[] }).notices.find((r) => r.key === err.noticeKey) ?? null;
    if (n) {
      document.getElementById("notice-title")!.textContent = n.title;
      document.getElementById("notice-reason")!.textContent = n.reason;
      document.getElementById("notice-action")!.textContent = n.action;
      document.getElementById("notice")!.hidden = false;
    }
    state.trackStates = stream?.getTracks().map((t) => t.readyState) ?? [];
    show("refused");
    return;
  }
  state.report = capture.report;
  capture.onEvent((e) => state.events.push(e.kind));
  const bound = bindCapturePort(capture.port, {
    push(b) {
      state.blocks.push({ firstFrame: b.firstFrame, len: b.samples.length, rateHz: b.rateHz, clipped: b.clipped });
      if (state.samples.length < 48_000 * 6) for (const v of b.samples) state.samples.push(v);
    },
    end() {
      state.ended = true;
    },
  });
  unbind = bound.unbind;
  continuity = bound.continuity;
  state.microphones = await listMicrophones(env);
  show("capturing");
}

async function stop(): Promise<void> {
  await capture?.stop();
  state.trackStates = stream?.getTracks().map((t) => t.readyState) ?? [];
  show("stopped");
}

document.getElementById("start")!.addEventListener("click", () => void start());
document.getElementById("stop")!.addEventListener("click", () => void stop());

(globalThis as unknown as { captureTest: unknown }).captureTest = {
  start,
  stop,
  /** Everything but the samples. */
  info: () => ({ ...state, samples: state.samples.length, connections, continuity: continuity && { ...continuity } }),
  samples: (from: number, n: number) => state.samples.slice(from, from + n),
  unbind: () => unbind?.(),
};
