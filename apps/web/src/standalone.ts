// The standalone page (WP WB5): index.html loads this. Query flags, for testing
// and for a display-only page: `?readonly`.

import { LocalBackend } from "./fleet/local";
import { mount } from "./index";

const params = new URLSearchParams(location.search);
// A page served by a hub over http(s) is that hub's own page: list it first.
const servingHub =
  location.protocol === "http:" || location.protocol === "https:"
    ? document.querySelector<HTMLMetaElement>('meta[name="dspi-hub"]')?.content || null
    : null;

mount(document.getElementById("console")!, {
  backend: new LocalBackend({ servingHub }),
  workerUrl: new URL("./worker.js", location.href),
  readOnly: params.has("readonly"),
});
