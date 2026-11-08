// The core Worker (WP WB5): one per device session. Calls arrive as messages and
// run one at a time (serial.ts). Until WB3 the core is the stub; WB3 replaces
// `loadCore` with the wasm module and keeps everything else.

import { CoreCallError, type CoreImpl, type FromWorker, type Request } from "./protocol";
import { SerialCore } from "./serial";
import { StubCore } from "./stub";

declare const self: DedicatedWorkerGlobalScope;

function loadCore(): CoreImpl {
  return new StubCore();
}

const core = loadCore();
const serial = new SerialCore(core);
const post = (m: FromWorker) => self.postMessage(m);

core.onEvent((e) => post({ push: e }));

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
