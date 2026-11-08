// One call at a time (WP WB5). The core under JSPI must never be entered while a
// call is suspended (WEB-STUDY section 2, "JSPI's two sharp edges"). A Worker
// receives messages whenever the page sends them, so calls are chained here and
// each starts only after the one before it has settled, failed or not. The
// `inFlight` check is a second line: should the chain ever be bypassed, the call
// is refused with `busy`, not run.

import { CoreCallError, type CoreImpl, type Method, type CoreMethods } from "./protocol";

export class SerialCore {
  private tail: Promise<unknown> = Promise.resolve();
  private inFlight = false;

  constructor(private readonly core: CoreImpl) {}

  call<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    const result = this.tail.then(() => this.enter(method, params));
    this.tail = result.catch(() => undefined);
    return result;
  }

  private async enter<M extends Method>(method: M, params: CoreMethods[M]["params"]): Promise<CoreMethods[M]["result"]> {
    if (this.inFlight) throw new CoreCallError("busy", "the core is already running a call");
    this.inFlight = true;
    try {
      return await this.core.call(method, params);
    } finally {
      this.inFlight = false;
    }
  }
}
