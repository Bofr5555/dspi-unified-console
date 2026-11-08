# DSPi web console

The web client of the unified console (WP WB5, the skeleton). The same Rust core
as every other client runs in a dedicated Worker under JSPI
(`docs/core-merge/WEB-STUDY.md`): `crates/dspi-wasm` (WP WB3), loaded by
`src/core/worker.ts` from `dspi_wasm.wasm` beside it. A small stub core
(`src/core/stub.ts`) remains as the tests' fake only.

**Status:** skeleton. The core's own demo device only. USB (WB4a) and Link to hubs (WB4b) say
"Not available in this version yet". The screens of the other clients arrive in
WB6 slices. Nothing here is published.

## What it is for

The owner's decision (2026-11-08): the main use of a web console is **a hub page
that gathers several DSPi installations**. Single-device use is covered by the
native clients. So the console:

- runs **standalone** (a static page, or served by a hub), and
- drops into a **server-based fleet product** with sign-in and access control,
  without changes to the console itself.

## Shape

```
page ── mount(el, options) ── App (Preact)
          │                     ├─ DevicePicker   installations → sources (demo, USB, hubs)
          │                     ├─ SessionView    one per open device: strip, banner, controls
          │                     └─ About          assets/about.json
          │
          ├─ FleetBackend      who the user is, which installations, what they may do
          │    ├─ LocalBackend      standalone: hub list in this page's storage, full control
          │    └─ (a server's own)  its API, its sign-in, its roles
          │
          └─ CoreClient ×N ──postMessage──▶ Worker ×N (one per session)
                                              SerialCore: one call at a time
                                              WasmCore → dspi_wasm.wasm (the core)
```

- **One Worker per device session.** Sessions share nothing, so a page can hold
  as many as it shows, and two consoles can live on one page. Inside a Worker
  calls run strictly one at a time (`src/core/serial.ts`, and `WasmCore`'s own
  queue for the Worker's timers): the core under JSPI must not be re-entered
  while a call is suspended. The module refuses such a call itself (`busy`).
- **The contract** with the core is `src/core/protocol.ts`: the core's records
  (`DspState` as the core's serde writes it, camelCase), writes by registry path
  with `indices` and a typed value as `Session::write` takes them, the read-back
  as the answer. `crates/dspi-wasm/src/calls.rs` is the other side.
- **The core is caller-driven** on the web (WB2): the Worker drains notifications
  every 100 ms (`poll`) and pushes a `state` event when something changed, and
  turns the analyser (`rtaTick`) at the wait the core asks for. A panic in the
  core ends the module: the session reports `lost`, later calls `dead`.
- **Wording** is the core's: the Link phases come from `link_phase_texts()`
  (generated to `ci/generated/web/link-words.json` by `ci/drift.sh` until the
  binding answers at run time); About is `assets/about.json`.

## Embedding in a fleet server

```js
import { mount } from "/static/dspi-web/dspi-web.js";   // serve dist/ as a directory

const console = mount(document.getElementById("dspi"), {
  backend: myFleetBackend,          // see src/fleet/types.ts
  title: "Acme venues",
  sourceUrl: "https://…/source",    // GPL: link to this build's source
});
// console.unmount();
```

`FleetBackend` (`src/fleet/types.ts`):

| Member | Standalone (`LocalBackend`) | A fleet server |
|---|---|---|
| `installations()` | one, "This console": demo, USB, saved hubs | the user's sites from its API |
| `addHub`, `removeSource` | present | omit them when the server manages the list |
| `access(installation, source, device)` | always `control` | `control` or `view` per role; `view` makes the session read only |
| `user()` | `null` | the signed-in user, shown in the header |
| `hubCredentials(url)` | absent: the user pairs with each hub | a Link token the server holds for the fleet (used from WB4b) |

The console never signs anyone in, never stores a server credential and never
decides access. Styles are scoped under `.dspi-web` (`dist/dspi-web.css`); light
and dark follow the system unless the host sets `data-theme` on `.dspi-web`. A
host page's own element styles can still reach the console's buttons and inputs.

Reaching hubs from a server-hosted page is bounded by the Link spec's `Origin`
rule: a hub accepts only its own pages (WEB-STUDY section 5). A fleet product
therefore talks to hubs through its own server, or serves the console from each
hub; the backend interface allows either, and WB4b/WB7 settle which.

## Build and test

Everything runs in the official Playwright image, pinned by digest in
`ci/container/images.conf` (Node, Chromium, Firefox). Nothing is installed on the
host.

```bash
ci/check.sh --step web                 # the core (cross container), npm ci, typecheck,
                                       # unit (the core's too, under JSPI), e2e
apps/web/tools/ci.sh --no-e2e          # without the browsers
```

Outputs and caches go to `~/.cache/dspi-build/work/web/<branch>/` (`dist/` is there,
not in the tree). `node tools/serve.mjs <dist>` serves a build on loopback for a look.

`dist/` holds `index.html` and `app.js` (the standalone page), `dspi-web.js` (the
library), `worker.js`, `dspi_wasm.wasm` (the core: 638 KB, 227 KB gzip) and
`dspi-web.css`. A host serves `dspi_wasm.wasm` beside `worker.js`. The page needs
JSPI (Chrome 137, Firefox 153, Safari 27); without it every call says so. Preact (MIT) is the one runtime
dependency. Its notice is not yet in the licence inventory (`ci/licence-inventory.py`
covers the Rust and iOS sides); that belongs with WB7, before anything is served.

## Files

| Path | What |
|---|---|
| `src/core/protocol.ts` | the page ↔ Worker contract, the shape WB3 implements |
| `src/core/serial.ts` | one call at a time, with a `busy` guard |
| `src/core/worker.ts` | the Worker: loads the core (`dspi_wasm.wasm`) |
| `src/core/wasm.ts` | the core's glue: imports, JSON calls, the Worker's timers |
| `src/core/client.ts` | the page's side: calls, replies, events |
| `src/core/stub.ts` | the tests' fake core |
| `src/fleet/` | installations, sources, access; `LocalBackend`; what this page can reach |
| `src/ui/` | Preact components and the session model |
| `src/index.tsx` | `mount()` and the public types |
| `src/standalone.ts`, `index.html` | the standalone page (`?readonly` for a display page) |
| `test/unit/` | `node --test` |
| `test/e2e/`, `test/pages/` | Playwright; `embed.html` mounts two consoles with a server-like backend (`?core=stub`: on the fake core, `stub-worker.ts`) |
