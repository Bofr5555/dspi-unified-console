// WP WB3: a Worker on the tests' fake core (src/core/stub.ts), for the e2e
// tests the core's demo cannot drive yet: the lost-link banner needs Link
// phases, which arrive with WB4b. The page's own Worker (src/core/worker.ts)
// runs the real core.

import { CoreCallError, type FromWorker, type Request } from "../../src/core/protocol";
import { SerialCore } from "../../src/core/serial";
import { StubCore } from "../../src/core/stub";

declare const self: DedicatedWorkerGlobalScope;

const core = new StubCore();
const serial = new SerialCore(core);
const post = (m: FromWorker) => self.postMessage(m);
core.onEvent((e) => post({ push: e }));

self.onmessage = (ev: MessageEvent<Request>) => {
  const { id, method, params } = ev.data;
  serial.call(method, params).then(
    (value) => post({ reply: { id, ok: true, value } }),
    (err: unknown) => {
      const e = err instanceof CoreCallError ? { code: err.code, message: err.message } : { code: "internal", message: String(err) };
      post({ reply: { id, ok: false, error: e } });
    },
  );
};
