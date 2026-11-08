// The standalone page (WP WB5): index.html loads this. Query flags, for testing
// and for a display-only page: `?readonly`.

import { LocalBackend } from "./fleet/local";
import { mount } from "./index";

const params = new URLSearchParams(location.search);
// A page served by a hub over http(s) is that hub's own page: list it first.
const served = location.protocol === "http:" || location.protocol === "https:";
const meta = (name: string) => (served ? document.querySelector<HTMLMetaElement>(`meta[name="${name}"]`)?.content || null : null);
const servingHub = meta("dspi-hub");

mount(document.getElementById("console")!, {
  backend: new LocalBackend({ servingHub }),
  workerUrl: new URL("./worker.js", location.href),
  readOnly: params.has("readonly"),
  // -- WP WB7 -- the hub's own page, beside which a hub serves this one
  hubPage: meta("dspi-hub-page"),
});
