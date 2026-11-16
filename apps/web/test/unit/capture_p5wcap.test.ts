// WP P5WCAP unit tests: the microphone capture without a microphone. The block framing and the
// frame continuity (capture-worklet.ts's BlockFramer and processor on a fake worklet scope,
// capture.ts's FrameContinuity, message checks and port binding over real MessagePorts), and the
// capture graph on a fake browser: processing off, `sinkId` "none", no forced rate in Firefox, a
// worklet with no output and nothing connected to a destination, the refusals as notice keys,
// stop and device loss.

import { after, test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  BlockFramer,
  CAPTURE_PROCESSOR,
  CLIP_LEVEL,
  blockFramesFor,
  registerCaptureProcessor,
  type WorkletBlock,
} from "../../src/core/capture-worklet";
import {
  CAPTURE_NOTICE_KEYS,
  CaptureError,
  FrameContinuity,
  bindCapturePort,
  captureConstraints,
  contextOptions,
  isGecko,
  listMicrophones,
  measurePushParams,
  readCaptureMessage,
  RESUME_WAIT_MS,
  startCapture,
  type CaptureBlock,
  type CaptureEnv,
  type CaptureTrack,
} from "../../src/core/capture";

const CHROME = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
const FIREFOX = "Mozilla/5.0 (X11; Linux x86_64; rv:140.0) Gecko/20100101 Firefox/140.0";
const tick = () => new Promise((r) => setTimeout(r, 10));
/** Node keeps a process alive while a MessagePort listens: every port a test makes is closed at the end. */
const ports: MessagePort[] = [];
after(() => {
  for (const p of ports) p.close();
});
function channel(): MessageChannel {
  const ch = new MessageChannel();
  ports.push(ch.port1, ch.port2);
  return ch;
}
class UnrefChannel {
  readonly port1: MessagePort;
  readonly port2: MessagePort;
  constructor() {
    const ch = channel();
    this.port1 = ch.port1;
    this.port2 = ch.port2;
  }
}

function ramp(from: number, n: number): Float32Array {
  const a = new Float32Array(n);
  for (let i = 0; i < n; i++) a[i] = ((from + i) % 1000) / 2000;
  return a;
}

// ---- framing ----

test("P5WCAP: blocks are about 20 ms, at least a quantum", () => {
  assert.equal(blockFramesFor(48_000), 960);
  assert.equal(blockFramesFor(44_100), 882);
  assert.equal(blockFramesFor(96_000), 1920);
  assert.equal(blockFramesFor(8000), 160);
  assert.equal(blockFramesFor(1000), 128);
  assert.equal(blockFramesFor(Number.NaN), 128);
  assert.equal(blockFramesFor(-1), 128);
});

test("P5WCAP: quanta of 128 become blocks of 960 with contiguous first frames and every sample once", () => {
  const out: WorkletBlock[] = [];
  const f = new BlockFramer(960, 48_000, (b) => out.push(b));
  const start = 4096;
  for (let q = 0; q < 75; q++) f.push(start + q * 128, ramp(q * 128, 128)); // 9600 frames
  assert.equal(out.length, 10);
  for (const [k, b] of out.entries()) {
    assert.equal(b.firstFrame, start + k * 960);
    assert.equal(b.samples.length, 960);
    assert.equal(b.rateHz, 48_000);
    assert.equal(b.clipped, false);
    assert.deepEqual([...b.samples], [...ramp(k * 960, 960)]);
  }
  // A partial block goes out on flush, as it is.
  f.push(start + 9600, ramp(9600, 100));
  f.flush();
  assert.equal(out.length, 11);
  assert.equal(out[10]!.firstFrame, start + 9600);
  assert.equal(out[10]!.samples.length, 100);
  f.flush();
  assert.equal(out.length, 11, "an empty flush posts nothing");
});

test("P5WCAP: a discontinuity in the frame count ends the block, and the next starts at its own frame", () => {
  const out: WorkletBlock[] = [];
  const f = new BlockFramer(960, 48_000, (b) => out.push(b));
  f.push(0, ramp(0, 128));
  f.push(128, ramp(128, 128));
  f.push(512, ramp(512, 128)); // 256 frames never delivered
  f.push(640, ramp(640, 128));
  f.push(512, ramp(512, 128)); // a repeat
  f.flush();
  assert.deepEqual(
    out.map((b) => [b.firstFrame, b.samples.length]),
    [
      [0, 256],
      [512, 256],
      [512, 128],
    ],
  );
  // The receiving end sees exactly that.
  const c = new FrameContinuity();
  const seen = out.map((b) => c.accept({ firstFrame: b.firstFrame, samples: b.samples, rateHz: b.rateHz, clipped: b.clipped }).kind);
  assert.deepEqual(seen, ["first", "gap", "repeat"]);
  assert.equal(c.gaps, 1);
  assert.equal(c.repeats, 1);
});

test("P5WCAP: the clip flag: full scale or a non-number in the block (posted as 0); it does not carry over", () => {
  const out: WorkletBlock[] = [];
  const f = new BlockFramer(256, 48_000, (b) => out.push(b));
  const a = new Float32Array(128);
  a[5] = -CLIP_LEVEL;
  f.push(0, a);
  f.push(128, new Float32Array(128));
  const b = new Float32Array(128);
  b[1] = Number.NaN;
  b[2] = Number.POSITIVE_INFINITY;
  f.push(256, b);
  f.push(384, new Float32Array(128));
  f.push(512, new Float32Array(128).fill(0.999));
  f.push(640, new Float32Array(128));
  assert.deepEqual(
    out.map((x) => x.clipped),
    [true, true, false],
  );
  assert.equal(out[1]!.samples[1], 0);
  assert.equal(out[1]!.samples[2], 0);
  assert.ok(out.every((x) => x.samples.every(Number.isFinite)));
});

test("P5WCAP: the processor on a fake worklet scope: blocks on the handed port with currentFrame, stop flushes and ends", async () => {
  const registered: { name: string; ctor: unknown }[] = [];
  class Base {
    readonly port: MessagePort;
    static last: Base;
    constructor() {
      const ch = channel();
      this.port = ch.port1;
      (this as unknown as { page: MessagePort }).page = ch.port2;
      Base.last = this;
    }
  }
  const scope = {
    registerProcessor: (name: string, ctor: unknown) => registered.push({ name, ctor }),
    AudioWorkletProcessor: Base as unknown as new () => { port: MessagePort },
    currentFrame: 0,
    sampleRate: 48_000,
  };
  assert.equal(registerCaptureProcessor({}), false, "nothing outside a worklet scope");
  assert.equal(registerCaptureProcessor(scope), true);
  assert.equal(registered[0]!.name, CAPTURE_PROCESSOR);
  const Proc = registered[0]!.ctor as new (o: unknown) => { process(i: Float32Array[][]): boolean };
  const p = new Proc({ processorOptions: { blockFrames: 256 } });
  const page = (Base.last as unknown as { page: MessagePort }).page;
  const toCore = channel();
  const got: unknown[] = [];
  toCore.port2.onmessage = (ev) => got.push(ev.data);
  const acks: unknown[] = [];
  page.onmessage = (ev) => acks.push(ev.data);

  // Before the port arrives nothing is posted.
  scope.currentFrame = 0;
  assert.equal(p.process([[ramp(0, 128)]]), true);
  page.postMessage({ kind: "port", port: toCore.port1 }, [toCore.port1]);
  await tick();
  for (let q = 1; q <= 4; q++) {
    scope.currentFrame = q * 128;
    p.process([[ramp(q * 128, 128)]]);
  }
  scope.currentFrame = 640;
  p.process([[]]); // an input without channels
  p.process([]);
  scope.currentFrame = 768;
  p.process([[ramp(768, 128)]]);
  page.postMessage({ kind: "stop" });
  await tick();
  assert.equal(p.process([[ramp(896, 128)]]), false, "stopped");
  await tick();
  const blocks = got.filter((m) => (m as { kind: string }).kind === "block") as WorkletBlock[];
  assert.deepEqual(
    blocks.map((b) => [b.firstFrame, b.samples.length]),
    [
      [128, 256],
      [384, 256],
      [768, 128],
    ],
  );
  assert.deepEqual([...blocks[0]!.samples], [...ramp(128, 256)]);
  assert.deepEqual(got.at(-1), { kind: "end" });
  assert.deepEqual(acks, [{ kind: "stopped" }]);
  for (const m of got) assert.ok(m === got.at(-1) || readCaptureMessage(m) !== null);
  page.close();
  (p as unknown as { port: MessagePort }).port.close();
  toCore.port2.close();
});

// ---- the receiving end ----

const block = (firstFrame: number, n = 960, rateHz = 48_000, clipped = false): CaptureBlock => ({
  firstFrame,
  samples: new Float32Array(n),
  rateHz,
  clipped,
});

test("P5WCAP: frame continuity: next, gap, repeat, a rate change", () => {
  const c = new FrameContinuity();
  assert.deepEqual(c.accept(block(1000)), { kind: "first" });
  assert.deepEqual(c.accept(block(1960)), { kind: "next" });
  assert.deepEqual(c.accept(block(3000)), { kind: "gap", missing: 80 });
  assert.deepEqual(c.accept(block(3500)), { kind: "repeat", overlap: 460 });
  assert.deepEqual(c.accept(block(4460, 960, 44_100)), { kind: "rate", from: 48_000, to: 44_100 });
  assert.deepEqual(c.accept(block(5420, 960, 44_100, true)), { kind: "next" });
  assert.equal(c.blocks, 6);
  assert.equal(c.frames, 6 * 960);
  assert.equal(c.gaps, 1);
  assert.equal(c.repeats, 1);
  assert.equal(c.clippedBlocks, 1);
});

test("P5WCAP: a capture message is checked: anything malformed is dropped", () => {
  const ok = { kind: "block", firstFrame: 0, samples: new Float32Array(4), rateHz: 48_000, clipped: false };
  assert.deepEqual(readCaptureMessage(ok), { firstFrame: 0, samples: ok.samples, rateHz: 48_000, clipped: false });
  assert.equal(readCaptureMessage({ kind: "end" }), "end");
  for (const bad of [
    null,
    undefined,
    7,
    "block",
    {},
    { ...ok, kind: "other" },
    { ...ok, firstFrame: -1 },
    { ...ok, firstFrame: 1.5 },
    { ...ok, firstFrame: 2 ** 53 },
    { ...ok, firstFrame: "0" },
    { ...ok, samples: [0, 0] },
    { ...ok, samples: new Float64Array(4) },
    { ...ok, samples: new Float32Array(0) },
    { ...ok, samples: new Float32Array((1 << 15) + 1) },
    { ...ok, rateHz: 0 },
    { ...ok, rateHz: Number.NaN },
    { ...ok, rateHz: 400_000 },
    { ...ok, clipped: 1 },
  ])
    assert.equal(readCaptureMessage(bad), null, JSON.stringify(bad));
});

test("P5WCAP: a bound port feeds the sink in order, awaits each push, reports gaps and push failures, then ends", async () => {
  const ch = channel();
  const pushed: number[] = [];
  const issues: string[] = [];
  let ended = false;
  let slow = true;
  const bound = bindCapturePort(ch.port2, {
    async push(b) {
      if (slow) {
        slow = false;
        await new Promise((r) => setTimeout(r, 20));
      }
      if (b.firstFrame === 4000) throw new Error("core said no");
      pushed.push(b.firstFrame);
    },
    end() {
      ended = true;
    },
    issue(c) {
      issues.push(c.kind);
    },
  });
  const send = (b: CaptureBlock) => ch.port1.postMessage({ kind: "block", ...b });
  send(block(0));
  send(block(960));
  ch.port1.postMessage({ kind: "nonsense" });
  send(block(4000));
  send(block(4960));
  ch.port1.postMessage({ kind: "end" });
  send(block(5920)); // after the end: ignored
  await new Promise((r) => setTimeout(r, 80));
  assert.deepEqual(pushed, [0, 960, 4960]);
  assert.deepEqual(issues, ["gap", "push_failed"]);
  assert.equal(ended, true);
  assert.equal(bound.continuity.blocks, 4);
  bound.unbind();
  ch.port1.close();
});

test("P5WCAP: the push parameters are the block's, samples as plain numbers", () => {
  const b = block(42, 3, 44_100, true);
  b.samples.set([0.5, -0.25, 0]);
  assert.deepEqual(measurePushParams(b), { firstFrame: 42, samples: [0.5, -0.25, 0], rateHz: 44_100, clipped: true });
});

// ---- the graph, on a fake browser ----

interface FakeWorld {
  env: CaptureEnv;
  gum: { audio: Record<string, unknown> }[];
  contexts: FakeContext[];
  nodes: { name: string; options: Record<string, unknown> }[];
  tracks: FakeTrack[];
}
class FakeTrack {
  kind = "audio";
  readyState = "live";
  label = "Measurement mic";
  private ended = new Set<() => void>();
  constructor(private readonly settings: ReturnType<CaptureTrack["getSettings"]>) {}
  stop() {
    this.readyState = "ended";
  }
  getSettings() {
    return this.settings;
  }
  addEventListener(_: "ended", fn: () => void) {
    this.ended.add(fn);
  }
  removeEventListener(_: "ended", fn: () => void) {
    this.ended.delete(fn);
  }
  /** The device went away. */
  unplug() {
    this.readyState = "ended";
    for (const f of this.ended) f();
  }
}
class FakeContext {
  state = "suspended";
  sinkId: unknown = "";
  sampleRate: number;
  modules: string[] = [];
  connections: [string, string][] = [];
  readonly destination = { name: "destination" };
  readonly audioWorklet = { addModule: async (u: string) => void this.modules.push(u) };
  constructor(readonly options: Record<string, unknown>, sinkSupported: boolean) {
    this.sampleRate = typeof options.sampleRate === "number" ? options.sampleRate : 44_100;
    if (sinkSupported) this.sinkId = options.sinkId ?? "";
  }
  createMediaStreamSource() {
    return {
      connect: (n: unknown) => {
        this.connections.push(["source", n === this.destination ? "destination" : (n as { name: string }).name]);
        return n;
      },
      disconnect: () => void (this.connections = []),
    };
  }
  async resume() {
    this.state = "running";
  }
  async close() {
    this.state = "closed";
  }
  addEventListener() {}
  removeEventListener() {}
}

function fakeWorld(opts: { userAgent?: string; secure?: boolean; sink?: boolean; sinkAnswer?: unknown; trackRate?: number; gumError?: string; noMedia?: boolean; neverRuns?: boolean } = {}): FakeWorld {
  const w: FakeWorld = { gum: [], contexts: [], nodes: [], tracks: [], env: undefined as unknown as CaptureEnv };
  const sink = opts.sink ?? true;
  w.env = {
    isSecureContext: opts.secure ?? true,
    userAgent: opts.userAgent ?? CHROME,
    mediaDevices: opts.noMedia
      ? undefined
      : {
          async getUserMedia(c) {
            w.gum.push(c);
            if (opts.gumError) throw Object.assign(new Error("no"), { name: opts.gumError });
            const t = new FakeTrack({ deviceId: "mic-1", sampleRate: opts.trackRate, echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 });
            w.tracks.push(t);
            return { getAudioTracks: () => [t], getTracks: () => [t] };
          },
          async enumerateDevices() {
            return [
              { kind: "audioinput", deviceId: "mic-1", label: "" },
              { kind: "videoinput", deviceId: "cam", label: "" },
              { kind: "audiooutput", deviceId: "spk", label: "Speakers" },
              { kind: "audioinput", deviceId: "mic-2", label: "UMIK-1" },
            ];
          },
        },
    AudioContext: class {
      constructor(o: Record<string, unknown>) {
        const c = new FakeContext(o, sink);
        if (opts.sinkAnswer !== undefined) c.sinkId = opts.sinkAnswer;
        if (opts.neverRuns) c.resume = () => new Promise(() => undefined);
        w.contexts.push(c);
        return c as unknown as object;
      }
    } as unknown as CaptureEnv["AudioContext"],
    sinkIdSupported: sink,
    AudioWorkletNode: class {
      readonly name = "worklet";
      readonly port: MessagePort;
      constructor(_: unknown, name: string, options: Record<string, unknown>) {
        const ch = channel();
        this.port = ch.port1;
        // The processor's side: acknowledge a stop at once.
        ch.port2.onmessage = (ev) => {
          if (ev.data?.kind === "stop") ch.port2.postMessage({ kind: "stopped" });
        };
        w.nodes.push({ name, options });
      }
      disconnect() {}
    } as unknown as CaptureEnv["AudioWorkletNode"],
    MessageChannel: UnrefChannel as unknown as CaptureEnv["MessageChannel"],
  };
  return w;
}

test("P5WCAP: the graph: processing off, mono, the chosen device; sinkId none; the microphone's rate; a worklet with no output; nothing to the destination", async () => {
  const w = fakeWorld({ trackRate: 48_000 });
  const cap = await startCapture({ deviceId: "mic-1", workletUrl: "https://h/console/capture-worklet.js", env: w.env });
  assert.deepEqual(w.gum, [
    { audio: { deviceId: { exact: "mic-1" }, echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 }, video: false },
  ]);
  const ctx = w.contexts[0]!;
  assert.deepEqual(ctx.options, { latencyHint: "interactive", sinkId: { type: "none" }, sampleRate: 48_000 });
  assert.deepEqual(ctx.modules, ["https://h/console/capture-worklet.js"]);
  assert.equal(ctx.state, "running");
  assert.equal(w.nodes.length, 1);
  assert.equal(w.nodes[0]!.name, CAPTURE_PROCESSOR);
  assert.equal(w.nodes[0]!.options.numberOfOutputs, 0);
  assert.equal(w.nodes[0]!.options.numberOfInputs, 1);
  assert.equal(w.nodes[0]!.options.channelCount, 1);
  assert.deepEqual(w.nodes[0]!.options.processorOptions, { blockFrames: 960 });
  assert.deepEqual(ctx.connections, [["source", "worklet"]], "one connection, none to the destination");
  assert.deepEqual(cap.report, {
    deviceId: "mic-1",
    label: "Measurement mic",
    rateHz: 48_000,
    blockFrames: 960,
    rateForced: true,
    sinkNone: true,
    echoCancellation: false,
    noiseSuppression: false,
    autoGainControl: false,
    channelCount: 1,
  });
  const events: string[] = [];
  cap.onEvent((e) => events.push(e.kind));
  await cap.stop();
  assert.equal(cap.stopped, true);
  assert.equal(w.tracks[0]!.readyState, "ended");
  assert.equal(ctx.state, "closed");
  assert.deepEqual(events, ["stopped"]);
  await cap.stop();
  assert.deepEqual(events, ["stopped"], "a second stop does nothing");
  cap.port.close();
});

test("P5WCAP: Firefox: no forced rate, no sinkId; the default device without deviceId", async () => {
  assert.equal(isGecko(FIREFOX), true);
  assert.equal(isGecko(CHROME), false);
  const w = fakeWorld({ userAgent: FIREFOX, sink: false, trackRate: 48_000 });
  const cap = await startCapture({ workletUrl: "capture-worklet.js", env: w.env });
  assert.deepEqual(w.contexts[0]!.options, { latencyHint: "interactive" });
  assert.equal("deviceId" in w.gum[0]!.audio, false);
  assert.equal(cap.report.rateHz, 44_100, "the context's own rate");
  assert.equal(cap.report.rateForced, false);
  assert.equal(cap.report.sinkNone, false);
  assert.deepEqual(w.contexts[0]!.connections, [["source", "worklet"]]);
  await cap.stop();
  cap.port.close();
  // No reported or a nonsense rate is never forced either.
  for (const r of [undefined, 0, 44_100.5, 1e9]) assert.equal("sampleRate" in contextOptions(fakeWorld().env, r), false, String(r));
  assert.deepEqual(captureConstraints(), { echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 });
});

test("P5WCAP: a context that does not report sinkId none where the browser has it is refused, and the microphone released", async () => {
  const w = fakeWorld({ sinkAnswer: "" });
  await assert.rejects(startCapture({ workletUrl: "w.js", env: w.env }), (e: unknown) => e instanceof CaptureError && e.noticeKey === "mic_unsupported");
  assert.equal(w.tracks[0]!.readyState, "ended");
  assert.equal(w.contexts[0]!.state, "closed");
  assert.deepEqual(w.contexts[0]!.connections, []);
});

test("P5WCAP: a context that never runs (no audio device) is refused after a wait, the microphone released", async () => {
  const w = fakeWorld({ neverRuns: true, userAgent: FIREFOX, sink: false });
  const t0 = Date.now();
  await assert.rejects(startCapture({ workletUrl: "w.js", env: w.env }), (e: unknown) => e instanceof CaptureError && e.noticeKey === "mic_not_running");
  assert.ok(Date.now() - t0 >= RESUME_WAIT_MS - 50);
  assert.equal(w.tracks[0]!.readyState, "ended");
  assert.equal(w.contexts[0]!.state, "closed");
});

test("P5WCAP: refusals are notice keys: not secure, no API, denied, not found, busy", async () => {
  const cases: [Parameters<typeof fakeWorld>[0], string, string][] = [
    [{ secure: false }, "mic_insecure", "unsupported"],
    [{ noMedia: true }, "mic_unsupported", "unsupported"],
    [{ gumError: "NotAllowedError" }, "mic_denied", "refused"],
    [{ gumError: "SecurityError" }, "mic_denied", "refused"],
    [{ gumError: "NotFoundError" }, "mic_not_found", "refused"],
    [{ gumError: "OverconstrainedError" }, "mic_not_found", "refused"],
    [{ gumError: "NotReadableError" }, "mic_busy", "transfer"],
    [{ gumError: "SomethingElse" }, "mic_busy", "transfer"],
  ];
  for (const [o, key, code] of cases) {
    const w = fakeWorld(o);
    await assert.rejects(
      startCapture({ workletUrl: "w.js", env: w.env }),
      (e: unknown) => e instanceof CaptureError && e.noticeKey === key && e.code === code,
      key,
    );
    assert.equal(w.contexts.length, 0, "no context was made");
  }
  // No AudioWorklet.
  const w = fakeWorld();
  w.env.AudioWorkletNode = undefined;
  await assert.rejects(startCapture({ workletUrl: "w.js", env: w.env }), (e: unknown) => (e as CaptureError).noticeKey === "mic_unsupported");
  assert.equal(w.gum.length, 0, "the permission is not even asked");
  await assert.rejects(listMicrophones(fakeWorld({ secure: false }).env), (e: unknown) => (e as CaptureError).noticeKey === "mic_insecure");
});

test("P5WCAP: the device list: audio inputs only, labelled once the permission was given", async () => {
  assert.deepEqual(await listMicrophones(fakeWorld().env), [
    { deviceId: "mic-1", label: "", labelled: false },
    { deviceId: "mic-2", label: "UMIK-1", labelled: true },
  ]);
});

test("P5WCAP: the microphone lost while capturing: a lost event, then stopped, everything closed", async () => {
  const w = fakeWorld({ trackRate: 48_000 });
  const cap = await startCapture({ workletUrl: "w.js", env: w.env });
  const events: unknown[] = [];
  cap.onEvent((e) => events.push(e));
  w.tracks[0]!.unplug();
  await new Promise((r) => setTimeout(r, 30));
  assert.deepEqual(events, [{ kind: "lost", noticeKey: "mic_lost" }, { kind: "stopped" }]);
  assert.equal(cap.stopped, true);
  assert.equal(w.contexts[0]!.state, "closed");
  cap.port.close();
});

test("P5WCAP: every capture notice key has a row in the core's table, without host nouns", () => {
  const raw = JSON.parse(readFileSync(join(process.cwd(), "../../assets/notices/notices.json"), "utf8")) as {
    notices: { key: string; kind: string; title: string; reason: string; action: string }[];
  };
  const rows = new Map(raw.notices.map((n) => [n.key, n]));
  const host = ["computer", "phone", "tablet", "desktop", "laptop", "mac", "pc", "iphone", "ipad"];
  for (const k of CAPTURE_NOTICE_KEYS) {
    const r = rows.get(k);
    assert.ok(r, k);
    for (const t of [r.title, r.reason, r.action]) {
      const words = t.toLowerCase().split(/[^a-z0-9']+/);
      for (const h of host) assert.ok(!words.includes(h) && !words.includes(`${h}s`), `${k}: ${t}`);
    }
  }
});
