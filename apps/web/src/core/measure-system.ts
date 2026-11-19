// WP ATWEB: the Worker's part of a system measurement across units (PLAN-AT 1.4, 1.5; the calls
// are crates/dspi-wasm/src/measure_system.rs, typed in protocol.ts).
//
// - `measureSystemCaptureOpen` makes a MessageChannel in the Worker and answers with one end (and
//   where capture-worklet.js is). The page hands it to the capture worklet, so the microphone's
//   blocks go from the worklet straight to the Worker, which pushes each into
//   `measureSystemCapturePush` in order. Raw audio goes nowhere else (S12).
// - After a `measureSystemStart`, `measureSystemSumCheck` or `measureSystemVerify` that answers an
//   active state, the Worker turns the session (`measureSystemTick`) about every 50 ms until it is
//   no longer active, pushing each state to the page as a `measureSystem` event. A Worker's timers
//   are not slowed in a background tab, so the hub's lease is renewed while the console is not in
//   front. `measureSystemStop`, `close` and `open` end the turns (one last state is pushed).
// Neither method reaches the core; nothing here plays anything.

import { bindCapturePort, measurePushParams } from "./capture";
import type { MeasureSystemStateRecord } from "./protocol";

export const SYSTEM_CAPTURE_OPEN = "measureSystemCaptureOpen";
export const SYSTEM_CAPTURE_CLOSE = "measureSystemCaptureClose";

/** How often the Worker turns a running system session, ms (the core asks for about 50 ms). */
export const SYSTEM_TICK_MS = 50;

/** The event the Worker pushes with each state. */
export type MeasureSystemEvent = { event: "measureSystem"; state: MeasureSystemStateRecord };

export function isMeasureSystemEvent(e: unknown): e is MeasureSystemEvent {
  const x = e as { event?: unknown; state?: unknown } | null;
  return !!x && x.event === "measureSystem" && typeof x.state === "object" && x.state !== null;
}

/** The calls after which the Worker turns the session. */
const STARTS = new Set(["measureSystemStart", "measureSystemSumCheck", "measureSystemVerify"]);
/** The calls that end the turns. */
const ENDS = new Set(["measureSystemStop", "close", "open", "documentOpen"]);

export class SystemWorkerRoute {
  private unbind: (() => void) | null = null;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private on = false;

  constructor(
    private readonly call: (method: string, params: unknown) => Promise<unknown>,
    private readonly post: (e: MeasureSystemEvent) => void,
    private readonly env: {
      MessageChannel: new () => { port1: MessagePort; port2: MessagePort };
      workletUrl: string;
      setTimeout: typeof setTimeout;
      clearTimeout: typeof clearTimeout;
    },
  ) {}

  /** The method is the Worker's own: `answer` it here, never in the core. */
  handles(method: string): boolean {
    return method === SYSTEM_CAPTURE_OPEN || method === SYSTEM_CAPTURE_CLOSE;
  }

  /** A new port (the earlier one closed), or every port closed. */
  answer(method: string): { value: { port: MessagePort; workletUrl: string } | null; transfer: MessagePort[] } {
    this.closeCapture();
    if (method !== SYSTEM_CAPTURE_OPEN) return { value: null, transfer: [] };
    const ch = new this.env.MessageChannel();
    const bound = bindCapturePort(ch.port1, {
      push: async (b) => {
        await this.call("measureSystemCapturePush", measurePushParams(b));
      },
      end: () => {
        if (this.unbind === unbind) this.unbind = null;
      },
      // A gap or a repeat is the core's to judge (S5): it sees the frame indices itself.
    });
    const unbind = () => bound.unbind();
    this.unbind = unbind;
    return { value: { port: ch.port2, workletUrl: this.env.workletUrl }, transfer: [ch.port2] };
  }

  closeCapture(): void {
    const u = this.unbind;
    this.unbind = null;
    u?.();
  }

  get ticking(): boolean {
    return this.on;
  }

  /** After a core call answered: start or end the turns. */
  after(method: string, ok: boolean, value: unknown): void {
    if (ok && STARTS.has(method)) {
      if ((value as MeasureSystemStateRecord | null)?.active) this.start();
      return;
    }
    if (ENDS.has(method)) {
      this.stop();
      if (method === "measureSystemStop" && ok && value) this.post({ event: "measureSystem", state: value as MeasureSystemStateRecord });
      if (method !== "measureSystemStop") this.closeCapture();
    }
  }

  // A browser's timer functions refuse another `this` ("Illegal invocation"): called unbound.
  private schedule(): void {
    const st = this.env.setTimeout;
    this.timer = st(() => void this.turn(), SYSTEM_TICK_MS);
  }

  private clear(): void {
    const ct = this.env.clearTimeout;
    ct(this.timer);
  }

  private start(): void {
    this.clear();
    this.on = true;
    this.schedule();
  }

  private stop(): void {
    this.clear();
    this.timer = undefined;
    this.on = false;
  }

  private async turn(): Promise<void> {
    if (!this.on) return;
    let active = false;
    try {
      const st = (await this.call("measureSystemTick", undefined)) as MeasureSystemStateRecord;
      this.post({ event: "measureSystem", state: st });
      active = !!st.active;
    } catch {
      // A failed tick (a lost session) ends the turns; the session's own state says why.
    }
    if (!this.on) return;
    this.clear();
    if (active) this.schedule();
    else this.stop();
  }
}
