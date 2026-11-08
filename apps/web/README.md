# DSPi web console

The web client of the unified console (WP WB5, the skeleton). The same Rust core
as every other client will run in a dedicated Worker under JSPI
(`docs/core-merge/WEB-STUDY.md`); until WB3 builds that binding, a small stub core
answers the same calls, so the page and its tests are already written against the
real contract.

**Status:** skeleton. Demo device only. USB (WB4a) and Link to hubs (WB4b) say
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
                                              StubCore now → dspi-wasm (WB3)
```

- **One Worker per device session.** Sessions share nothing, so a page can hold
  as many as it shows, and two consoles can live on one page. Inside a Worker
  calls run strictly one at a time (`src/core/serial.ts`): the core under JSPI
  must not be re-entered while a call is suspended.
- **The contract** with the core is `src/core/protocol.ts`. Its shapes follow the
  core's records so WB3 answers them from `dspi-core` directly.
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
ci/check.sh --step web                 # npm ci, typecheck, unit, e2e (Chromium, Firefox, 375 px)
apps/web/tools/ci.sh --no-e2e          # without the browsers
```

Outputs and caches go to `~/.cache/dspi-build/work/web/<branch>/` (`dist/` is there,
not in the tree). `node tools/serve.mjs <dist>` serves a build on loopback for a look.

`dist/` holds `index.html` and `app.js` (the standalone page), `dspi-web.js` (the
library), `worker.js` and `dspi-web.css`. Preact (MIT) is the one runtime
dependency. Its notice is not yet in the licence inventory (`ci/licence-inventory.py`
covers the Rust and iOS sides); that belongs with WB7, before anything is served.

## Files

| Path | What |
|---|---|
| `src/core/protocol.ts` | the page ↔ Worker contract, the shape WB3 implements |
| `src/core/serial.ts` | one call at a time, with a `busy` guard |
| `src/core/worker.ts` | the Worker: loads the core (stub now) |
| `src/core/client.ts` | the page's side: calls, replies, events |
| `src/core/stub.ts` | the stand-in core; later the tests' fake |
| `src/fleet/` | installations, sources, access; `LocalBackend`; what this page can reach |
| `src/ui/` | Preact components and the session model |
| `src/index.tsx` | `mount()` and the public types |
| `src/standalone.ts`, `index.html` | the standalone page (`?readonly` for a display page) |
| `test/unit/` | `node --test` |
| `test/e2e/`, `test/pages/` | Playwright; `embed.html` mounts two consoles with a server-like backend |
