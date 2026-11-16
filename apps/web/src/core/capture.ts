// WP P5WCAP: the web console's microphone (PLAN-P5 section 1.1 step 2, S1, S12).
//
// The page opens the measurement microphone and its blocks go, frame-indexed, to the core
// Worker, which pushes them into the core's `measure_capture_push` (P5SES; the Worker's routing
// is P5WMS's, through `bindCapturePort` below). The core does every safety check on them.
//
// The capture graph, by construction:
// - `getUserMedia` with echo cancellation, noise suppression and automatic gain control OFF and
//   one channel, for the device the user chose (labels show only after the permission);
// - an `AudioContext` whose output device is `{type: "none"}` where the browser has `sinkId`
//   (asserted: a context that does not report "none" there is refused), and whose rate is the
//   microphone's own where the browser reports it, except in Firefox, which refuses a source at
//   a rate other than the context's: there the context keeps its default rate;
// - the microphone's source connected to ONE node, an `AudioWorkletNode` with no output
//   (capture-worklet.ts). Nothing is ever connected to the context's destination: nothing is
//   played (S1).
// - Blocks of about 20 ms with the context frame of their first sample (`currentFrame`), the
//   context's rate and a clip flag, posted by the worklet on a MessagePort whose other end the
//   caller transfers to the core Worker. Raw audio goes nowhere else: it is never logged or
//   kept here (S12).
//
// Refusals carry notice keys (assets/notices/notices.json, block "WP P5WCAP"): not a secure
// context, no microphone API, the permission denied, no such microphone, the microphone busy,
// the context that does not start, and the microphone lost while capturing (an event, not an
// error).

import { CoreCallError } from "./protocol";
import { CAPTURE_PROCESSOR, blockFramesFor } from "./capture-worklet";

// ---- notice keys ----

/** The page is not a secure context (https, or a loopback address): browsers offer no microphone. */
export const NOTICE_MIC_INSECURE = "mic_insecure";
/** The browser has no microphone capture or no AudioWorklet. */
export const NOTICE_MIC_UNSUPPORTED = "mic_unsupported";
/** The user or the browser's settings denied the microphone. */
export const NOTICE_MIC_DENIED = "mic_denied";
/** No microphone, or not the one chosen. */
export const NOTICE_MIC_NOT_FOUND = "mic_not_found";
/** The microphone could not be opened (in use elsewhere, or a device error). */
export const NOTICE_MIC_BUSY = "mic_busy";
/** The microphone went away while capturing. */
export const NOTICE_MIC_LOST = "mic_lost";
/** The browser opened the microphone but did not start its audio processing (no audio device, or blocked). */
export const NOTICE_MIC_NOT_RUNNING = "mic_not_running";

export const CAPTURE_NOTICE_KEYS = [
  NOTICE_MIC_INSECURE,
  NOTICE_MIC_UNSUPPORTED,
  NOTICE_MIC_DENIED,
  NOTICE_MIC_NOT_FOUND,
  NOTICE_MIC_BUSY,
  NOTICE_MIC_LOST,
  NOTICE_MIC_NOT_RUNNING,
] as const;

/** A capture refusal: `code` "unsupported", "refused" or "transfer" with the notice key. */
export class CaptureError extends CoreCallError {
  constructor(code: string, message: string, noticeKey: string) {
    super(code, message, noticeKey);
    this.name = "CaptureError";
  }
}

// ---- the browser, as far as the capture uses it (a seam for the unit tests) ----

export interface CaptureTrack {
  readonly kind: string;
  readonly label: string;
  readonly readyState: string;
  stop(): void;
  getSettings(): { deviceId?: string; sampleRate?: number; echoCancellation?: boolean; noiseSuppression?: boolean; autoGainControl?: boolean; channelCount?: number };
  addEventListener(type: "ended", fn: () => void): void;
  removeEventListener(type: "ended", fn: () => void): void;
}
export interface CaptureStream {
  getAudioTracks(): CaptureTrack[];
  getTracks(): CaptureTrack[];
}
export interface CaptureContext {
  readonly sampleRate: number;
  readonly state: string;
  readonly sinkId?: unknown;
  readonly audioWorklet: { addModule(url: string): Promise<void> };
  createMediaStreamSource(stream: CaptureStream): { connect(node: unknown): unknown; disconnect(): void };
  resume(): Promise<void>;
  close(): Promise<void>;
  addEventListener(type: "statechange", fn: () => void): void;
  removeEventListener(type: "statechange", fn: () => void): void;
}
export interface CaptureNode {
  readonly port: MessagePort;
  disconnect(): void;
}
export interface MediaDeviceInfoLike {
  readonly kind: string;
  readonly deviceId: string;
  readonly label: string;
  readonly groupId?: string;
}
export interface CaptureEnv {
  isSecureContext: boolean;
  userAgent: string;
  mediaDevices?: {
    getUserMedia(c: { audio: Record<string, unknown>; video: false }): Promise<CaptureStream>;
    enumerateDevices(): Promise<MediaDeviceInfoLike[]>;
  };
  /** `AudioContext`; `sinkIdSupported` says whether its `sinkId` option is real. */
  AudioContext?: new (options: Record<string, unknown>) => CaptureContext;
  sinkIdSupported: boolean;
  AudioWorkletNode?: new (ctx: CaptureContext, name: string, options: Record<string, unknown>) => CaptureNode;
  MessageChannel: new () => { port1: MessagePort; port2: MessagePort };
}

/** The page's browser. */
export function browserCaptureEnv(): CaptureEnv {
  const g = globalThis as unknown as {
    isSecureContext?: boolean;
    navigator?: { userAgent?: string; mediaDevices?: CaptureEnv["mediaDevices"] };
    AudioContext?: (new (o: Record<string, unknown>) => CaptureContext) & { prototype: { setSinkId?: unknown } };
    AudioWorkletNode?: CaptureEnv["AudioWorkletNode"];
  };
  return {
    isSecureContext: g.isSecureContext === true,
    userAgent: g.navigator?.userAgent ?? "",
    mediaDevices: g.navigator?.mediaDevices,
    AudioContext: g.AudioContext,
    sinkIdSupported: typeof g.AudioContext?.prototype?.setSinkId === "function",
    AudioWorkletNode: g.AudioWorkletNode,
    MessageChannel,
  };
}

/** Firefox (Gecko): its context must keep its own rate. */
export function isGecko(userAgent: string): boolean {
  return /\bFirefox\//.test(userAgent) && !/\bSeamonkey\//i.test(userAgent);
}

// ---- the input list ----

export interface MicrophoneInfo {
  deviceId: string;
  /** "" until the page has the permission. */
  label: string;
  /** The browser gave a label (the permission was granted once). */
  labelled: boolean;
}

/** The audio inputs the browser lists. Labels are empty until the permission is granted. */
export async function listMicrophones(env: CaptureEnv = browserCaptureEnv()): Promise<MicrophoneInfo[]> {
  checkAvailable(env);
  const md = env.mediaDevices!;
  const all = await md.enumerateDevices();
  return all
    .filter((d) => d.kind === "audioinput" && typeof d.deviceId === "string")
    .map((d) => ({ deviceId: d.deviceId, label: d.label ?? "", labelled: !!d.label }));
}

function checkAvailable(env: CaptureEnv): void {
  if (!env.isSecureContext)
    throw new CaptureError("unsupported", "The microphone needs a secure page (https or a loopback address)", NOTICE_MIC_INSECURE);
  if (!env.mediaDevices || typeof env.mediaDevices.getUserMedia !== "function")
    throw new CaptureError("unsupported", "This browser offers no microphone capture", NOTICE_MIC_UNSUPPORTED);
}

// ---- starting and stopping ----

/** The constraints asked of `getUserMedia`: processing off, mono, the chosen device. */
export function captureConstraints(deviceId?: string): Record<string, unknown> {
  return {
    ...(deviceId ? { deviceId: { exact: deviceId } } : {}),
    echoCancellation: false,
    noiseSuppression: false,
    autoGainControl: false,
    channelCount: 1,
  };
}

/** The `AudioContext` options: no output device where supported, the microphone's rate except in Firefox. */
export function contextOptions(env: CaptureEnv, trackRate: number | undefined): Record<string, unknown> {
  const o: Record<string, unknown> = { latencyHint: "interactive" };
  if (env.sinkIdSupported) o.sinkId = { type: "none" };
  if (!isGecko(env.userAgent) && typeof trackRate === "number" && Number.isInteger(trackRate) && trackRate >= 8000 && trackRate <= 192_000)
    o.sampleRate = trackRate;
  return o;
}

export type CaptureEvent =
  /** The microphone went away (unplugged, revoked); the capture has stopped. */
  | { kind: "lost"; noticeKey: typeof NOTICE_MIC_LOST }
  /** The capture stopped (`stop()` or after `lost`). */
  | { kind: "stopped" };

/** What the browser reports it did with the asked settings: for the screens and the spike's record. */
export interface CaptureReport {
  deviceId: string;
  label: string;
  rateHz: number;
  blockFrames: number;
  /** The context's rate was set to the microphone's (not in Firefox). */
  rateForced: boolean;
  /** The context reports output device "none" (false where the browser has no `sinkId`). */
  sinkNone: boolean;
  /** As the track reports them; `undefined` where the browser does not say. */
  echoCancellation?: boolean;
  noiseSuppression?: boolean;
  autoGainControl?: boolean;
  channelCount?: number;
}

export interface CaptureOptions {
  /** The input to open; the browser's default without one. */
  deviceId?: string;
  /** Where capture-worklet.js is (beside the console's other scripts). */
  workletUrl: string | URL;
  env?: CaptureEnv;
}

/** A running capture. */
export interface Capture {
  /** Blocks (`WorkletBlock`, then `{kind: "end"}`): transfer this port to the core Worker. */
  readonly port: MessagePort;
  readonly report: CaptureReport;
  onEvent(fn: (e: CaptureEvent) => void): () => void;
  /** Ends the capture: the last block and the end mark are posted, the track stopped, the context closed. */
  stop(): Promise<void>;
  readonly stopped: boolean;
}

function sinkIsNone(ctx: CaptureContext): boolean {
  const s = ctx.sinkId as { type?: unknown } | string | undefined;
  return typeof s === "object" && s !== null && s.type === "none";
}

function mediaError(e: unknown): CaptureError {
  const name = (e as { name?: unknown })?.name;
  const msg = e instanceof Error ? e.message : String(e);
  switch (name) {
    case "NotAllowedError":
    case "SecurityError":
    case "PermissionDeniedError":
      return new CaptureError("refused", `The microphone was denied: ${msg}`, NOTICE_MIC_DENIED);
    case "NotFoundError":
    case "OverconstrainedError":
    case "DevicesNotFoundError":
      return new CaptureError("refused", `No such microphone: ${msg}`, NOTICE_MIC_NOT_FOUND);
    case "NotReadableError":
    case "AbortError":
    case "TrackStartError":
      return new CaptureError("transfer", `The microphone could not be opened: ${msg}`, NOTICE_MIC_BUSY);
    default:
      return new CaptureError("transfer", `The microphone could not be opened: ${msg}`, NOTICE_MIC_BUSY);
  }
}

/**
 * Opens the microphone and starts the capture graph. Asks the browser's permission (call it
 * from the user's action). Throws a `CaptureError` with a notice key; on any failure everything
 * opened so far is closed again.
 */
export async function startCapture(opts: CaptureOptions): Promise<Capture> {
  const env = opts.env ?? browserCaptureEnv();
  checkAvailable(env);
  if (!env.AudioContext || !env.AudioWorkletNode)
    throw new CaptureError("unsupported", "This browser has no AudioWorklet", NOTICE_MIC_UNSUPPORTED);

  let stream: CaptureStream;
  try {
    stream = await env.mediaDevices!.getUserMedia({ audio: captureConstraints(opts.deviceId), video: false });
  } catch (e) {
    throw mediaError(e);
  }
  const stopTracks = () => {
    for (const t of stream.getTracks()) t.stop();
  };
  const track = stream.getAudioTracks()[0];
  if (!track) {
    stopTracks();
    throw new CaptureError("refused", "The browser gave no audio track", NOTICE_MIC_NOT_FOUND);
  }
  const settings = track.getSettings();

  let ctx: CaptureContext | null = null;
  try {
    const options = contextOptions(env, settings.sampleRate);
    ctx = new env.AudioContext(options);
    const sinkNone = sinkIsNone(ctx);
    if (env.sinkIdSupported && !sinkNone)
      throw new CaptureError("unsupported", "The browser did not keep the capture off every output device", NOTICE_MIC_UNSUPPORTED);
    await ctx.audioWorklet.addModule(String(opts.workletUrl));
    const rateHz = ctx.sampleRate;
    const blockFrames = blockFramesFor(rateHz);
    const source = ctx.createMediaStreamSource(stream);
    const node = new env.AudioWorkletNode(ctx, CAPTURE_PROCESSOR, {
      numberOfInputs: 1,
      numberOfOutputs: 0,
      channelCount: 1,
      // The device's first channel as it is, not a mix of its channels.
      channelCountMode: "explicit",
      channelInterpretation: "discrete",
      processorOptions: { blockFrames },
    });
    const channel = new env.MessageChannel();
    node.port.postMessage({ kind: "port", port: channel.port1 }, [channel.port1]);
    // The ONLY connection of the graph: the microphone into the worklet. Nothing reaches the destination.
    source.connect(node);
    // A context that never starts (Firefox without any audio device, or its autoplay rules)
    // would capture nothing: refused after a wait, not left hanging.
    let timer: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([ctx.resume(), new Promise((r) => (timer = setTimeout(r, RESUME_WAIT_MS)))]).finally(() => clearTimeout(timer));
    if (ctx.state !== "running")
      throw new CaptureError("unsupported", `The browser did not start its audio processing (${ctx.state})`, NOTICE_MIC_NOT_RUNNING);

    const report: CaptureReport = {
      deviceId: settings.deviceId ?? opts.deviceId ?? "",
      label: track.label ?? "",
      rateHz,
      blockFrames,
      rateForced: "sampleRate" in options,
      sinkNone,
      echoCancellation: settings.echoCancellation,
      noiseSuppression: settings.noiseSuppression,
      autoGainControl: settings.autoGainControl,
      channelCount: settings.channelCount,
    };
    return new RunningCapture(ctx, source, node, track, stopTracks, channel.port2, report);
  } catch (e) {
    stopTracks();
    if (ctx) await ctx.close().catch(() => undefined);
    if (e instanceof CaptureError) throw e;
    throw new CaptureError("transfer", `The capture could not start: ${e instanceof Error ? e.message : String(e)}`, NOTICE_MIC_BUSY);
  }
}

/** How long `startCapture` waits for the context to run, ms. */
export const RESUME_WAIT_MS = 3000;

/** How long `stop` waits for the worklet's last block before closing the context, ms. */
const STOP_WAIT_MS = 250;

class RunningCapture implements Capture {
  private readonly listeners = new Set<(e: CaptureEvent) => void>();
  private done = false;
  private readonly onEnded = () => void this.end(true);
  private readonly onState = () => {
    if (this.ctx.state === "closed" || this.ctx.state === "interrupted") void this.end(true);
  };

  constructor(
    private readonly ctx: CaptureContext,
    private readonly source: { disconnect(): void },
    private readonly node: CaptureNode,
    private readonly track: CaptureTrack,
    private readonly stopTracks: () => void,
    readonly port: MessagePort,
    readonly report: CaptureReport,
  ) {
    track.addEventListener("ended", this.onEnded);
    ctx.addEventListener("statechange", this.onState);
  }

  get stopped(): boolean {
    return this.done;
  }

  onEvent(fn: (e: CaptureEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  stop(): Promise<void> {
    return this.end(false);
  }

  private async end(lost: boolean): Promise<void> {
    if (this.done) return;
    this.done = true;
    this.track.removeEventListener("ended", this.onEnded);
    this.ctx.removeEventListener("statechange", this.onState);
    // The worklet posts its open block and the end mark, then acknowledges.
    await new Promise<void>((resolve) => {
      const timer = setTimeout(resolve, STOP_WAIT_MS);
      this.node.port.onmessage = (ev: MessageEvent<{ kind?: string }>) => {
        if (ev.data?.kind === "stopped") {
          clearTimeout(timer);
          resolve();
        }
      };
      try {
        this.node.port.postMessage({ kind: "stop" });
      } catch {
        clearTimeout(timer);
        resolve();
      }
    });
    try {
      this.source.disconnect();
      this.node.disconnect();
    } catch {
      // Already gone with the context.
    }
    this.stopTracks();
    if (this.ctx.state !== "closed") await this.ctx.close().catch(() => undefined);
    if (lost) this.emit({ kind: "lost", noticeKey: NOTICE_MIC_LOST });
    this.emit({ kind: "stopped" });
    this.listeners.clear();
  }

  private emit(e: CaptureEvent): void {
    for (const l of [...this.listeners]) {
      try {
        l(e);
      } catch {
        // A listener's failure is its own.
      }
    }
  }
}

// ---- the receiving end (the core Worker, or a page that reads the blocks itself) ----

/** A block as it arrives, checked: the arguments of `measure_capture_push`. */
export interface CaptureBlock {
  firstFrame: number;
  samples: Float32Array;
  rateHz: number;
  clipped: boolean;
}

/** Most frames a block may have (the worklet's are about 20 ms). */
export const MAX_BLOCK_FRAMES = 1 << 15;

/**
 * A message from the capture port: a block, the end mark, or null for anything else (the
 * port is the page's own, but its data is checked like any other input).
 */
export function readCaptureMessage(data: unknown): CaptureBlock | "end" | null {
  if (typeof data !== "object" || data === null) return null;
  const m = data as Record<string, unknown>;
  if (m.kind === "end") return "end";
  if (m.kind !== "block") return null;
  const { firstFrame, samples, rateHz, clipped } = m;
  if (typeof firstFrame !== "number" || !Number.isSafeInteger(firstFrame) || firstFrame < 0) return null;
  if (!(samples instanceof Float32Array) || samples.length === 0 || samples.length > MAX_BLOCK_FRAMES) return null;
  if (typeof rateHz !== "number" || !Number.isFinite(rateHz) || rateHz < 8000 || rateHz > 192_000) return null;
  if (typeof clipped !== "boolean") return null;
  return { firstFrame, samples, rateHz, clipped };
}

/** What a block says about the frame count so far. */
export type Continuity =
  | { kind: "first" }
  | { kind: "next" }
  /** `missing` frames were never delivered before this block. */
  | { kind: "gap"; missing: number }
  /** This block starts `overlap` frames before the previous one ended. */
  | { kind: "repeat"; overlap: number }
  /** The rate changed mid-capture. */
  | { kind: "rate"; from: number; to: number };

/** Follows the frame indices of one capture (the core judges them again, S5). */
export class FrameContinuity {
  private next: number | null = null;
  private rate: number | null = null;
  /** Frames delivered, blocks, and discontinuities seen. */
  frames = 0;
  blocks = 0;
  gaps = 0;
  repeats = 0;
  clippedBlocks = 0;

  accept(b: CaptureBlock): Continuity {
    let c: Continuity;
    if (this.next === null || this.rate === null) c = { kind: "first" };
    else if (b.rateHz !== this.rate) c = { kind: "rate", from: this.rate, to: b.rateHz };
    else if (b.firstFrame === this.next) c = { kind: "next" };
    else if (b.firstFrame > this.next) c = { kind: "gap", missing: b.firstFrame - this.next };
    else c = { kind: "repeat", overlap: this.next - b.firstFrame };
    if (c.kind === "gap") this.gaps++;
    if (c.kind === "repeat") this.repeats++;
    if (b.clipped) this.clippedBlocks++;
    this.next = b.firstFrame + b.samples.length;
    this.rate = b.rateHz;
    this.frames += b.samples.length;
    this.blocks++;
    return c;
  }
}

/**
 * The typed seam to the core: what `measure_capture_push(first_frame, samples, rate_hz,
 * clipped)` takes (P5SES owns the call and its wasm binding; until it is merged a caller
 * passes its own push). Errors from it are the core's, reported to `issue`.
 */
export interface CaptureSink {
  push(b: CaptureBlock): void | Promise<void>;
  /** The capture ended (stop or loss). */
  end?(): void;
  /** A block broke the frame count, or the push failed; the core's own checks decide what follows. */
  issue?(c: Exclude<Continuity, { kind: "first" } | { kind: "next" }> | { kind: "push_failed"; error: unknown }): void;
}

/**
 * Feeds the blocks arriving on `port` to `sink`, in order (a push that returns a promise is
 * awaited before the next block goes in). Returns the unbind. Malformed messages are dropped.
 */
export function bindCapturePort(port: MessagePort, sink: CaptureSink): { unbind(): void; continuity: FrameContinuity } {
  const continuity = new FrameContinuity();
  let chain: Promise<void> = Promise.resolve();
  let open = true;
  port.onmessage = (ev: MessageEvent) => {
    const m = readCaptureMessage(ev.data);
    if (m === null || !open) return;
    if (m === "end") {
      open = false;
      chain = chain.then(() => sink.end?.());
      port.close();
      return;
    }
    const c = continuity.accept(m);
    if (c.kind !== "first" && c.kind !== "next") sink.issue?.(c);
    chain = chain.then(async () => {
      try {
        await sink.push(m);
      } catch (error) {
        sink.issue?.({ kind: "push_failed", error });
      }
    });
  };
  port.start?.();
  return {
    unbind() {
      open = false;
      port.onmessage = null;
      port.close();
    },
    continuity,
  };
}

/**
 * `measure_capture_push`'s parameters as JSON for the wasm call (`dspi_call` takes JSON):
 * camelCase as every web call. The binding is P5SES's; this is the shape this file proposes.
 */
export interface MeasureCapturePushParams {
  firstFrame: number;
  samples: number[];
  rateHz: number;
  clipped: boolean;
}

export function measurePushParams(b: CaptureBlock): MeasureCapturePushParams {
  return { firstFrame: b.firstFrame, samples: Array.from(b.samples), rateHz: b.rateHz, clipped: b.clipped };
}
