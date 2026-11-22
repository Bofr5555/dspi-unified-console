// The standalone page (WP WB5): index.html loads this. Query flags, for testing
// and for a display-only page: `?readonly`; -- WP WEBM1 -- `?nav=tabs` (or `rail`) for how the
// shell is arranged.

import { mount } from "./index";
// -- WP P2WB1 -- served by a hub: the hub's backend (its inventory, its roles)
import { CoreClient } from "./core/client";
import { pageBackend } from "./fleet/env";
import { isNavigation } from "./ui/shell/sections"; // -- WP WEBM1 --

const params = new URLSearchParams(location.search);
// A page served by a hub over http(s) is that hub's own page: list it first.
const served = location.protocol === "http:" || location.protocol === "https:";
const meta = (name: string) => (served ? document.querySelector<HTMLMetaElement>(`meta[name="${name}"]`)?.content || null : null);
const servingHub = meta("dspi-hub");

const nav = params.get("nav"); // -- WP WEBM1 --
const workerUrl = new URL("./worker.js", location.href);
mount(document.getElementById("console")!, {
  backend: pageBackend({ servingHub, spawnCore: () => CoreClient.spawn(workerUrl) }), // -- WP P2WB1 --
  workerUrl,
  readOnly: params.has("readonly"),
  // -- WP WB7 -- the hub's own page, beside which a hub serves this one
  hubPage: meta("dspi-hub-page"),
  navigation: isNavigation(nav) ? nav : undefined, // -- WP WEBM1 --
});
