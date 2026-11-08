// The core Worker (WP WB5, WB3): one per device session. Calls arrive as
// messages and run one at a time (serial.ts) on the real core, crates/dspi-wasm
// (wasm.ts), loaded from beside this file. Messages that arrive while it loads
// wait for it. If it cannot load, every call answers why: there is no stand-in
// core in a page (the stub is the unit tests' fake only).

import { CoreCallError, type CoreEvent, type CoreImpl, type FromWorker, type Request } from "./protocol";
import { SerialCore } from "./serial";
import { WasmCore, loadWasmModule } from "./wasm";

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

self.onmessage = (ev: MessageEvent<Request>) => {
  const { id, method, params } = ev.data;
  serial.call(method, params).then(
    (value) => post({ reply: { id, ok: true, value } }),
    (err: unknown) => {
      const e =
        err instanceof CoreCallError
          ? { code: err.code, message: err.message }
          : { code: "internal", message: err instanceof Error ? err.message : String(err) };
      post({ reply: { id, ok: false, error: e } });
    },
  );
};
