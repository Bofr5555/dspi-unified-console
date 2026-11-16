// WP P5WCAP: the microphone's AudioWorklet (PLAN-P5 section 1.1 step 2). Built as its own
// file (dist/capture-worklet.js, tools/build.mjs) and loaded with `audioWorklet.addModule`:
// the console's CSP allows scripts from its own origin only, so never from a blob.
//
// The processor (`dspi-capture`) has one input and NO output (`numberOfOutputs: 0`): it
// renders nothing and nothing is connected after it, so nothing can ever be played. It cuts
// the microphone's render quanta (128 frames) into blocks of about 20 ms and posts each, with
// the context frame of its first sample (the worklet scope's `currentFrame`), the context's
// rate and a clip flag, on the MessagePort the page hands it (`{kind: "port"}`): that port's
// other end goes to the core Worker, so blocks never pass through the page's thread.
//
// Frame indices are honest: a quantum that does not follow the previous one (the context
// skipped, or the input had no channels for a while) ends the open block early and starts the
// next at its own frame, so the gap shows in the indices for the core to judge (S5). Nothing is
// invented to fill it.
//
// `BlockFramer` is the pure part, imported by the unit tests (test/unit/capture_p5wcap.test.ts);
// the processor class is defined only inside an AudioWorkletGlobalScope.

/** One block as the worklet posts it (`capture.ts` checks it again on arrival). */
export interface WorkletBlock {
  kind: "block";
  /** The context frame of `samples[0]`. */
  firstFrame: number;
  samples: Float32Array;
  /** The context's sample rate, Hz. */
  rateHz: number;
  /** A sample reached full scale or was not a number (then it was posted as 0). */
  clipped: boolean;
}

/**
 * A sample at or above this magnitude is "clipped": the largest 16-bit code, so a converter
 * that clipped is caught whatever its word length. The core's own -1 dBFS abort (S4) reads
 * the samples themselves.
 */
export const CLIP_LEVEL = 32767 / 32768;

/** About 20 ms at `rateHz` (960 frames at 48 kHz), at least one render quantum. */
export function blockFramesFor(rateHz: number): number {
  if (!Number.isFinite(rateHz) || rateHz <= 0) return 128;
  return Math.max(128, Math.min(1 << 15, Math.round(rateHz * 0.02)));
}

/** Cuts quanta into blocks of `blockFrames`, splitting at any discontinuity in the frame count. */
export class BlockFramer {
  private buf: Float32Array;
  private fill = 0;
  private first = 0;
  private clipped = false;
  /** The frame the next quantum should start at, or null before the first and after a flush. */
  private expected: number | null = null;

  constructor(
    readonly blockFrames: number,
    readonly rateHz: number,
    private readonly emit: (b: WorkletBlock) => void,
  ) {
    this.buf = new Float32Array(blockFrames);
  }

  /** One quantum of mono samples starting at context frame `frame`. */
  push(frame: number, samples: Float32Array): void {
    if (samples.length === 0) return;
    if (this.expected !== null && frame !== this.expected) this.flush();
    let i = 0;
    while (i < samples.length) {
      if (this.fill === 0) this.first = frame + i;
      const n = Math.min(samples.length - i, this.blockFrames - this.fill);
      for (let k = 0; k < n; k++) {
        let v = samples[i + k] as number;
        if (!Number.isFinite(v)) {
          v = 0;
          this.clipped = true;
        } else if (Math.abs(v) >= CLIP_LEVEL) this.clipped = true;
        this.buf[this.fill + k] = v;
      }
      this.fill += n;
      i += n;
      if (this.fill === this.blockFrames) this.flush();
    }
    this.expected = frame + samples.length;
  }

  /** Posts the open block, if any, as it is (shorter than `blockFrames`). */
  flush(): void {
    if (this.fill > 0) {
      const samples = this.buf.slice(0, this.fill);
      this.emit({ kind: "block", firstFrame: this.first, samples, rateHz: this.rateHz, clipped: this.clipped });
    }
    this.fill = 0;
    this.clipped = false;
    this.expected = null;
  }
}

// ---- the processor, registered only inside an AudioWorkletGlobalScope ----

export interface ProcessorBase {
  readonly port: MessagePort;
}
/** What the processor uses of an AudioWorkletGlobalScope (a fake one in the unit tests). */
export interface WorkletScope {
  registerProcessor?: (name: string, ctor: unknown) => void;
  AudioWorkletProcessor?: new (options?: unknown) => ProcessorBase;
  currentFrame?: number;
  sampleRate?: number;
}

/** The processor's registered name. */
export const CAPTURE_PROCESSOR = "dspi-capture";

/** Defines and registers the processor in `scope`; false outside a worklet scope. */
export function registerCaptureProcessor(scope: WorkletScope): boolean {
  if (typeof scope.registerProcessor !== "function" || typeof scope.AudioWorkletProcessor !== "function") return false;
  const Base = scope.AudioWorkletProcessor;
  class CaptureProcessor extends Base {
    private out: MessagePort | null = null;
    private running = true;
    private readonly framer: BlockFramer;

    constructor(options?: { processorOptions?: { blockFrames?: unknown } }) {
      super(options);
      const rate = scope.sampleRate ?? 48_000;
      const asked = options?.processorOptions?.blockFrames;
      const frames = typeof asked === "number" && asked >= 128 && asked <= 1 << 15 ? Math.floor(asked) : blockFramesFor(rate);
      this.framer = new BlockFramer(frames, rate, (b) => {
        // The block's buffer goes to the Worker, not copied.
        this.out?.postMessage(b, [b.samples.buffer]);
      });
      this.port.onmessage = (ev: MessageEvent<{ kind?: string; port?: MessagePort }>) => {
        const m = ev.data;
        if (m?.kind === "port" && m.port) this.out = m.port;
        else if (m?.kind === "stop") {
          this.framer.flush();
          this.out?.postMessage({ kind: "end" });
          this.out?.close();
          this.out = null;
          this.running = false;
          this.port.postMessage({ kind: "stopped" });
        }
      };
    }

    process(inputs: Float32Array[][]): boolean {
      if (!this.running) return false;
      const ch = inputs[0]?.[0];
      // No channel: the source has nothing this quantum; the next one starts a new block.
      if (ch && this.out) this.framer.push(scope.currentFrame ?? 0, ch);
      return true;
    }
  }
  scope.registerProcessor(CAPTURE_PROCESSOR, CaptureProcessor);
  return true;
}

registerCaptureProcessor(globalThis as unknown as WorkletScope);
