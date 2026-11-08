# DSPi web console

The web client of the unified console (WP WB5, the skeleton). The same Rust core
as every other client runs in a dedicated Worker under JSPI
(`docs/core-merge/WEB-STUDY.md`): `crates/dspi-wasm` (WP WB3), loaded by
`src/core/worker.ts` from `dspi_wasm.wasm` beside it. A small stub core
(`src/core/stub.ts`) remains as the tests' fake only.

**Status:** skeleton. The core's own demo device, a DSPi on USB through
WebUSB (WB4a, Chromium only; tested on a WebUSB mock, real hardware is WB8),
and DSPis behind hubs over DSPi Link (WB4b; tested against dspi-link's fake hub
behind a host that enforces the hub's `Origin` rule, in Chromium and Firefox),
served by a real hub, by `dspi web` on this machine and by any static host (WB7,
[`DEPLOY.md`](DEPLOY.md)). The screens of the other clients arrive in WB6 slices.
Nothing here is published.

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

## USB (WebUSB)

WP WB4a. **Chromium-based browsers only** (Chrome, Edge, Opera; Firefox and
Safari have no WebUSB), and only on a **secure page**: `https://`, or
`http://localhost` / `http://127.0.0.1`. Elsewhere the picker says which of the
two is missing. USB MIDI cannot go through WebUSB (Chromium will not let a page
claim an audio-class interface); the console uses the vendor interface 2, which
every DSPi firmware since 1.1.5 has, stock and bofr-main alike.

- **Choosing a DSPi.** "DSPi on USB" → Connect opens the browser's own device
  picker (it needs the click). DSPis this page was allowed before and that are
  plugged in are listed as entries of their own ("DSPi on USB · 1B8B4E3A", the
  serial's last 8 characters), kept current as they are plugged in and out.
- **Several at once.** Each DSPi opens in its own session and Worker; the
  Worker finds the chosen device again by serial (the permission is the
  page's origin's, which a Worker shares), opens it and claims interface 2.
  A DSPi held by another tab or program cannot be claimed; the session says so.
- **Unplugged** (or gone mid-transfer): the session ends with the core's words
  ("Device not connected"); the other sessions go on.
- **How it runs:** the core's control transfers are the module's imports
  `dspi_usb_in` / `dspi_usb_out`, Promises under JSPI, each bounded by the core's
  1 s transfer time (WebUSB has none of its own); the Worker reads the
  notification endpoint and pushes the packets in before its next `poll`
  (`src/core/usb.ts`, `crates/dspi-wasm/src/usb.rs`).

**Linux: the udev rule.** Chromium opens the device through usbfs, so without a
rule the open fails ("Access denied") and the session says the udev rule must be
installed. It is the same rule as for the other consoles:
`xtask/70-dspi.rules` (installed by `sudo dspi --install-udev`) or
`apps/desktop/packaging/linux/70-dspi.rules`, then `sudo udevadm control
--reload` and replug. A Flatpak or Snap Chromium may also need its own USB
permission. ChromeOS and Android Chrome: WB8 checks them.

**Windows: no Zadig.** The firmware's MS OS 2.0 descriptors bind WinUSB to
interface 2 by themselves since 1.1.5 (WEB-STUDY section 6). Zadig is only the
blunt fix for a machine that cached another driver for that device (it saw
older firmware, or someone bound it by hand): then pick "WinUSB" for interface 2
in Zadig, or remove the device in Device Manager with its driver and replug.
That this needs no step on a clean machine is reasoned, not yet tested: WB8.

**macOS:** no step expected; quit any other program holding the DSPi. Reasoned
from WEB-STUDY section 6, not yet run (WB8).

## Link (hubs)

WP WB4b. The client is the core's own, the same as every other console's:
`dspi-link`'s Rust client under the core's Link session and supervisor, built
into `dspi_wasm.wasm`. Only the socket is the browser's: `src/core/ws.ts` hands
whole WebSocket messages to the module's `dspi_ws_*` imports
(`crates/dspi-wasm/src/link.rs`). No `@dspi/link` JavaScript.

- **Steps**, each the core's (Link's choose_device flow), driven by the session
  model (`src/ui/session.ts`): `linkConnect` with the token kept for that hub
  and the hub's id it belongs to (the core sends it only once the hub has said
  it is that hub); if the hub wants a PIN, the session shows CG10's
  `needs_pairing` wording and a PIN field (`linkPair`; a wrong PIN keeps the
  prompt); then the DSPi a fleet backend named, or the hub's own list
  (`linkDevices`) to choose from; then `open {kind: "hub", deviceId}` binds it
  and reads it.
- **Then the core keeps it up by itself**: the Worker turns the session
  (`linkTick`) at the wait the core asks for (at most 250 ms) and whenever the
  socket has something, and pushes each new phase. The banner is the core's
  phases in CG10's wording: reconnecting (with Disconnect, the way out), no
  DSPi, closed, refused. A lost link reconnects with the core's backoff and
  binds the DSPi again; a write made while the link is down fails at once and is
  never sent later (the owner's rule); 30 s of silence ends a session (the
  core's rule since LK2).
- **No threads**: on the web `dspi-link` and the core's supervisor are
  caller-driven (`dspi_link::client::caller_driven`): whoever waits for the hub
  reads the socket meanwhile (JSPI suspends the call in `dspi_ws_wait`).
- **Several at once**: every hub session is a Worker of its own. A hub's entry
  opens a new session each time, so one hub can have several DSPis (or the same
  DSPi twice) open, and several hubs as well.
- **Tokens**: standalone, per hub in this page's storage
  (`src/fleet/hub-tokens.ts`; a page served by a hub is that hub's own origin).
  A token the hub refuses is forgotten. With `hubCredentials` (below) the
  fleet's token is used, and a token paired for in this visit is kept in memory
  only.
- **What a page cannot do**, and what stands in: it cannot send a WebSocket
  ping, so the core's ping is a request every hub answers (`unknown_type`), as
  the upstream JavaScript client does; it may close only with 1000 or
  3000-4999; and it cannot see why an upgrade failed, so a refused `Origin`, a
  full hub and an unreachable one read alike: "Cannot connect to the hub at …
  It may be off or busy, or it refuses this page: a hub accepts only pages it
  serves itself."

**The `Origin` rule** (Link spec 4, WEB-STUDY section 5): a hub accepts a
browser's WebSocket only from its own pages. So the page that reaches a hub is
the one that hub serves (`<meta name="dspi-hub" content="ws://…">` names its
Link address; the standalone page lists it first), or a fleet server's page
whose server carries Link to its hubs through its own origin. A standalone page
can list any hub, but only hubs without the check answer it.

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
| `hubCredentials(url)` | absent: the user pairs with each hub once; the token is kept in this page's storage | `{token, hubId?}` the server holds for the fleet; no PIN is asked. A refused one is not offered again this visit, and the user may pair instead (kept in memory only) |
| `hubDevices(url)` | absent: the console asks the hub (choose_device) | the hub's DSPis the server knows, `{deviceId: serial, name}`, each its own entry that opens without asking the hub; `null` for a hub it leaves to the console |

The console never signs anyone in, never stores a server credential and never
decides access. Styles are scoped under `.dspi-web` (`dist/dspi-web.css`); light
and dark follow the system unless the host sets `data-theme` on `.dspi-web`. A
host page's own element styles can still reach the console's buttons and inputs.
`hubPage` (a URL) adds a link to the serving hub's own page in the header; the
standalone page sets it from `<meta name="dspi-hub-page">`, which a hub fills in (WB7).
How to serve the build, the headers and the policy a host needs: [`DEPLOY.md`](DEPLOY.md).

Reaching hubs from a server-hosted page is bounded by the Link spec's `Origin`
rule: a hub accepts only its own pages (WEB-STUDY section 5). A fleet product
therefore carries Link to its hubs through its own server (a hub `url` on the
server's origin, as `tools/hub-host.mjs` does in the tests; behind TLS that is a
`wss://` address, which the core accepts), or serves the console from each hub
(`dspi hub --web`, WB7).

## Build and test

Everything runs in the official Playwright image, pinned by digest in
`ci/container/images.conf` (Node, Chromium, Firefox). Nothing is installed on the
host.

```bash
ci/check.sh --step web                 # the core and dspi-link's fake hub (cross
                                       # container), npm ci, typecheck, unit (the
                                       # core's too, under JSPI), e2e
apps/web/tools/ci.sh --no-e2e          # without the browsers
```

Outputs and caches go to `~/.cache/dspi-build/work/web/<branch>/` (`dist/` is there,
not in the tree). `node tools/serve.mjs <dist>` serves a build on loopback for a look.
`node tools/hub-host.mjs <dist> <port> <dspi-link-fakehub>` serves it as a hub
would, with the `Origin` rule, over fake hubs it starts on request (the Link
e2e's host; `DSPI_FAKEHUB` names the binary, which `tools/ci.sh` builds).

`dist/` holds `index.html`, `app.js` and `page.css` (the standalone page), `dspi-web.js` (the
library), `worker.js`, `dspi_wasm.wasm` (the core: 1 086 KB, 362 KB gzip with WebUSB and Link),
`dspi-web.css` and `THIRD-PARTY-NOTICES.txt`. A host serves `dspi_wasm.wasm` beside `worker.js`. The page needs
JSPI (Chrome 137, Firefox 153, Safari 27); without it every call says so. Preact (MIT) is the one runtime
dependency; `ci/licence-inventory.py` records it (section 6b), and the notices file carries its
licence and the core's Rust crates (`notices/rust-crates.txt`, written by
`tools/gen_rust_notices.py` in the cross container; the web step fails when it is stale).

-- WP WB7 -- Build settings: `DSPI_WEB_SOURCE_URL` is the GPL source link About shows
(an embedder's `sourceUrl` overrides it); `DSPI_WEB_RELEASE=1` refuses a build without
it. `tools/dist.sh <out>` is the release build, an artefact only ([`DEPLOY.md`](DEPLOY.md)).
The web step also runs the hosting routes on the real `dspi` (`test/e2e/hosting.spec.ts`:
`dspi hub --web` and `dspi web`, built in the cross container as `DSPI_TERM`).

## Files

| Path | What |
|---|---|
| `src/core/protocol.ts` | the page ↔ Worker contract, the shape WB3 implements |
| `src/core/serial.ts` | one call at a time, with a `busy` guard |
| `src/core/worker.ts` | the Worker: loads the core (`dspi_wasm.wasm`) |
| `src/core/wasm.ts` | the core's glue: imports, JSON calls, the Worker's timers |
| `src/core/client.ts` | the page's side: calls, replies, events |
| `src/core/stub.ts` | the tests' fake core |
| `src/core/usb.ts`, `usb-ids.ts` | WebUSB in the Worker: claim, transfers, notifications, unplug (WB4a) |
| `src/core/ws.ts` | the browser's WebSocket as the core's socket imports (WB4b) |
| `src/fleet/hub-tokens.ts` | hub tokens: this page's storage, or the fleet's credentials (WB4b) |
| `src/fleet/` | installations, sources, access; `LocalBackend`; what this page can reach |
| `src/ui/` | Preact components and the session model |
| `src/index.tsx` | `mount()` and the public types |
| `src/standalone.ts`, `index.html` | the standalone page (`?readonly` for a display page) |
| `test/unit/` | `node --test` |
| `test/e2e/hosting.spec.ts` | WB7: the console through each route, on the real `dspi` (hub, `dspi web`) and a static host |
| `DEPLOY.md`, `tools/dist.sh` | WB7: serving the build; the release artefact |
| `notices/rust-crates.txt`, `tools/gen_rust_notices.py` | WB7: the core's crates for `THIRD-PARTY-NOTICES.txt` |
| `test/e2e/`, `test/pages/` | Playwright; `embed.html` mounts two consoles with a server-like backend (`?core=stub`: on the fake core, `stub-worker.ts`); `usb.html` runs USB sessions over a WebUSB mock (`webusb-mock.ts`) whose devices are the core's simulated DSPi in the test module (Chromium); `fleet-hub.html` a fleet page with two hubs, `hubDevices` and `hubCredentials`, served by `tools/hub-host.mjs` (`link.spec.ts`, WB4b) |
