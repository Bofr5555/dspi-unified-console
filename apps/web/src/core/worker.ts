// The core Worker (WP WB5, WB3): one per device session. Calls arrive as
// messages and run one at a time (serial.ts) on the real core, crates/dspi-wasm
// (wasm.ts), loaded from beside this file. Messages that arrive while it loads
// wait for it. If it cannot load, every call answers why: there is no stand-in
// core in a page (the stub is the unit tests' fake only).

import { CoreCallError, type CoreEvent, type CoreImpl, type FromWorker, type Request } from "./protocol";
import { SerialCore } from "./serial";
import { WasmCore, loadWasmModule } from "./wasm";
// -- WP P5WMS -- the measurement's capture port and ticks (measure-protocol.ts)
import { MeasureWorkerRoute } from "./measure-protocol";
import { SystemWorkerRoute } from "./measure-system"; // -- WP ATWEB --

declare const self: DedicatedWorkerGlobalScope;

const post = (m: FromWorker) => self.postMessage(m);

const ready: Promise<WasmCore> = loadWasmModule(new URL("./dspi_wasm.wasm", import.meta.url)).then((m) => {
  const core = new WasmCore(m);
  core.onEvent((e: CoreEvent) => post({ push: e }));
  return core;
});
// Seen by the first call; keeps an unhandled rejection out of the console.
ready.catch(() => undefined);

/** The core once it has loaded; a failed load is every call's answer. */
const deferred: CoreImpl = {
  async call(method, params) {
    let core: WasmCore;
    try {
      core = await ready;
    } catch (e) {
      throw new CoreCallError("internal", `The DSPi core could not be loaded: ${e instanceof Error ? e.message : String(e)}`);
    }
    return core.call(method, params);
  },
  onEvent() {},
};

const serial = new SerialCore(deferred);

// -- WP P5WMS -- the capture worklet's blocks go straight from its port into `measureCapturePush`,
// and a running measurement is turned here; its states are pushed as `measure` events. The
// route's own trouble never holds a reply back.
const route = new MeasureWorkerRoute(
  (m, p) => serial.call(m as never, p as never),
  (e) => self.postMessage({ push: e }),
  { MessageChannel, workletUrl: new URL("./capture-worklet.js", self.location.href).href, setTimeout, clearTimeout },
);
const measure = {
  handles: (m: string) => route.handles(m),
  answer: (m: string, p: unknown) => route.answer(m, p),
  after(m: string, ok: boolean, v: unknown) {
    try {
      route.after(m, ok, v);
    } catch {
      // Ticks stopped; the session's state says the rest.
    }
  },
};

// -- WP ATWEB -- the system measurement's capture port and ticks (measure-system.ts)
const system = new SystemWorkerRoute(
  (m, p) => serial.call(m as never, p as never),
  (e) => self.postMessage({ push: e }),
  { MessageChannel, workletUrl: new URL("./capture-worklet.js", self.location.href).href, setTimeout, clearTimeout },
);
// -- end WP ATWEB --

self.onmessage = (ev: MessageEvent<Request>) => {
  const { id, method, params } = ev.data;
  // -- WP P5WMS -- the Worker's own methods: a port (transferred), never a core call
  if (measure.handles(method)) {
    const a = measure.answer(method, params);
    self.postMessage({ reply: { id, ok: true, value: a.value } }, a.transfer);
    return;
  }
  // -- WP ATWEB -- the system measurement's own: its capture port
  if (system.handles(method)) {
    const a = system.answer(method);
    self.postMessage({ reply: { id, ok: true, value: a.value } }, a.transfer);
    return;
  }
  serial.call(method, params).then(
    (value) => {
      measure.after(method, true, value); // -- WP P5WMS --
      system.after(method, true, value); // -- WP ATWEB --
      post({ reply: { id, ok: true, value } });
    },
    (err: unknown) => {
      measure.after(method, false, null); // -- WP P5WMS --
      system.after(method, false, null); // -- WP ATWEB --
      const e =
        err instanceof CoreCallError
          ? { code: err.code, message: err.message, ...(err.noticeKey ? { noticeKey: err.noticeKey } : {}), ...(err.text ? { text: err.text } : {}) }
          : { code: "internal", message: err instanceof Error ? err.message : String(err) };
      post({ reply: { id, ok: false, error: e } });
    },
  );
};
