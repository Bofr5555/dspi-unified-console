// The standalone page (WP WB5): index.html loads this. Query flags, for testing
// and for a display-only page: `?readonly`.

import { mount } from "./index";
// -- WP P2WB1 -- served by a hub: the hub's backend (its inventory, its roles)
import { CoreClient } from "./core/client";
import { pageBackend } from "./fleet/env";

const params = new URLSearchParams(location.search);
// A page served by a hub over http(s) is that hub's own page: list it first.
const served = location.protocol === "http:" || location.protocol === "https:";
const meta = (name: string) => (served ? document.querySelector<HTMLMetaElement>(`meta[name="${name}"]`)?.content || null : null);
const servingHub = meta("dspi-hub");

const workerUrl = new URL("./worker.js", location.href);
mount(document.getElementById("console")!, {
  backend: pageBackend({ servingHub, spawnCore: () => CoreClient.spawn(workerUrl) }), // -- WP P2WB1 --
  workerUrl,
  readOnly: params.has("readonly"),
  // -- WP WB7 -- the hub's own page, beside which a hub serves this one
  hubPage: meta("dspi-hub-page"),
});
