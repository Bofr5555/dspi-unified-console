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
served by a real hub, by `uconsole web` on this machine and by any static host (WB7,
[`DEPLOY.md`](DEPLOY.md)). The screens of the other clients arrive in WB6 slices;
the main window (WB6a), the EQ (WB6b) and the outputs and matrix (WB6c) are in. Nothing here is published.

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
`xtask/70-dspi.rules` (installed by `sudo uconsole --install-udev`) or
`apps/desktop/packaging/linux/70-uconsole-for-dspi.rules`, then `sudo udevadm control
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

## The main window (WB6a)

`src/screens/main/`, one per session, inside its view (`src/ui/screens.tsx`
lists a session's screens; the tab row appears once a slice adds a second).
The macOS Console's main window is the floor:

- **Volume**: User or Master Volume (the Console's sidebar choice, kept for this
  viewer in the page's storage), on the Console's tapers (the user volume's
  square root over -60..0 dB; the firmware's master steps down to -128, shown as
  −∞); Mute and Bypass. While a slider moves, each new position is written once
  the one before has been read back (the newest wins); let go, it shows the
  device's value. A double or secondary click puts it back at 0 dB.
- **Presets**: the slot picker ("3: Living room", "4: Empty"), the Console's
  slot menu (Save, Rename…, Set as Default, Copy to…, Clear, Clear All Slots…)
  and its Tools entries (Commit Parameters…, Revert to Saved…, Factory
  Reset…), each asked first as on the Mac. Before a switch, a Copy to or a
  Disconnect with unsaved changes, the Console's prompt (Save, Discard, Cancel)
  lists the core's own sentences (`presetChanges`); leaving the page then asks
  the browser's own question.
- **Inputs and outputs**: the live inputs as the Console counts them (by source;
  on USB what the device streams) and the enabled outputs; each with its name
  (click to rename), its trim (`pre` or `out.gain`, ranges from the core's
  registry) and, for an output, its mute.

Every change goes through the core (`write` by registry path, or a preset call)
and the page shows what the device kept. A read-only session (a viewer, or
firmware the core only reads) shows everything with the controls off; only
which volume is shown stays the viewer's. The parts redraw only when what they
show changes (the meters and the rest of the device's state do not reach them),
rows keep one height whatever their state, and at 375 px the cards stack in one
column with the slot and tools entries in one menu button.

## The EQ (WB6b)

`src/screens/eq/`: the channels with their curve pills, the response graph on a
canvas with on-graph editing after the Console's `PeqGraphEditor` (keys and
gestures listed atop `EqGraph.tsx`; touch: drag, double tap, pinch for Q, long
press for menus), the band chip and the band list. The curves are the core's
(`curves`, `crates/dspi-wasm/src/curves.rs`); the canvas draws only when what
it shows changes. Edits are `eq.*` writes with the read-back, a drag's
coalesced (newest wins, nothing replayed after a failure).

## Outputs, crossover and matrix (WB6c)

`src/screens/outputs/`, `src/screens/matrix/`. An output chosen on the EQ
screen is the Console's output page: its settings card on top (the first two
inputs' routing with level and INV, GAIN, DELAY, mute, the limiter with its
settings and batch actions), the graph, then the PEQ or XO bands (the
crossover: family, pass, slope, frequency, bypass; Bypass All asks with the
Console's warning). Outputs also offer the Linkwitz Transform, edited in its
panel (f0, Q0, fp, Qp, Apply). The Matrix screen is the Console's Matrix Mixer:
crosspoints, and per output ENABLE (the PDM / Core 1 questions), GAIN, DELAY,
MUTE.

The delay follows the owner's rule (SL1, `outputs/Level.tsx`): a drag sends
whole ms live and on release, never -0, and the thumb snaps to what was
committed; typed values are sent as typed and values from the device are shown
as they are (to 0.0001 ms); the wheel and the keys go to the next whole ms;
a key or an assistive technology's change commits at once. The longest delay is
the device's (`outputLimits`). Three binding calls cover what `write` by path
does not (`crates/dspi-wasm/src/outputs.rs`): `crossoverType` (the registry's
`eq.type` takes PEQ types only), `crosspoint` (the registry's `mix`, its packet
built in the binding) and `outputLimits`.

EQ follow-ups: the shape card asks a shape's slope as the Console's chooser; a
host's `data-theme` change redraws the graph; the curves hook shows every
answer as it comes, so a drag over a slow queue never freezes the curve.

## The System view, scene bar and status rail (WP SW3)

`src/screens/system/`: the web is the first client of the Advanced layer
(`docs/UI-MODES.md`). One feed per session (`feed.ts`) asks the hub's audio-link
system model over the session's own Link connection (`hubSystem` and `hubRole`,
`crates/dspi-wasm/src/system.rs`, the hub's `system.*` of
[`docs/integrator/audio-link.md`](../../docs/integrator/audio-link.md)) and polls
`system.status` every 3 s while the page is visible. The hub judges every request by
the session's role; the page only leaves out what it would refuse.

- **System view** (WB6d, `SystemScreen.tsx`): rooms and zones with their units, each
  unit connected or not, its audio link's lock and its alarms; the topology (sender,
  fan-outs, units, the output each hangs on, hop delays, each unit's link slots). On
  a hub with an installation it is the first screen; a device the console holds
  itself (USB, the demo), or a hub without an installation, is a single unit, read
  only while the view is shown.
- **Clock and links** (Installer mode, `ClockLinks.tsx`): per unit `d_local` against
  its limits, `hops_offset` (with what the unit has, when it differs), the path and
  latencies, a measured latency and each room's lip sync; the link delay D; the slot
  matrix (link channels to each unit's 8 inputs); Compute and Align
  (`system.align`, asked first: a re-anchor is 2 to 6 ms of silence) with each
  unit's result; link stats and the trim scan. Edits are staged and saved as one
  `system.set`. Model and alignment are Admin only: other roles see it read only.
- **Scene bar** (WB6e, `SceneBar.tsx`): the installation's scenes, numbered;
  a recall is `system.scene.apply` (asked first when the device has unsaved
  changes), then the frame, the path and each unit's result (queued, applied, late,
  refused). A scene the role may not run is refused whole and the op named.
- **Status rail** (WB6f, `StatusRail.tsx`), below the screen: connection, the last
  scene, the mode (Simple, Advanced, Installer; remembered per device or for every
  device; no PIN on the web, so nothing is locked), the system mute (`system.mute`)
  and the alarms with Acknowledge.

The e2e (`test/e2e/system.spec.ts`) runs against `uconsole --demo hub --web
--audio-link`: the fake device's audio-link roles (a sender, a fan-out, four units)
with a system model, open (Admin) and with a PIN (Control).

## Undo and Redo, the event log, diagnostics (WP WB8)

- **Undo and Redo** (`src/ui/undo.tsx`) are the core's journal (WP P0W: `undo`,
  `redo`, `canUndo`), as on the other clients: two buttons beside the screen tabs,
  and the keys Ctrl+Z (Cmd+Z on a Mac), Ctrl+Shift+Z and Ctrl+Y. A text field with
  focus keeps the keys for its own undo; on a page with several consoles they act
  on the one the focus is in (or the first, with the focus nowhere). A slider drag
  and an EQ graph drag or pinch are marked (`setDragging`), so a drag is one step;
  the mark always ends (the pointer let go anywhere, cancelled, its capture lost,
  the control gone), once the drag's last write has gone. A graph drag sends each
  PEQ band whole (`filter`, the core's `set_filter`, as the desktop's graph does),
  so the core journals one change per tick. An undo another writer stepped in on
  says who, in the core's words (`note`); changes that cannot be undone are named
  when stepped over. A viewer's session and firmware the core only reads have both
  off. The buttons are in every mode.
- **Events** (`src/screens/events/`, Advanced and Installer): the core's event log
  (WP P0M: clips, the audio link's lock and loss, resets, connections, temperature,
  counters; `eventLogRead` per device, only what is new, every second while shown and
  the page is visible) and, on a hub's installation, its alarms; time, unit and text,
  newest first; a unit filter; Copy as Text (one tab-separated line per event).
- **Save Diagnostics…** (on the Events screen): the core's bundle, what `uconsole
  diag` and the desktop's Help > Save Diagnostics… write (`diagnostics`,
  `crates/dspi-wasm/src/diag.rs`: `DspiCore::diagnostics_zip` as base64), handed to the
  viewer as a download, with the browser's user agent as `client/browser.txt`.
  Nothing is sent anywhere.
- **A refused scene** says it in the core's sentence (`sceneOutcome`, the core's
  `scene_recall_outcome`, the one every client shows): the held frame only when the
  sender refused; a timeout, a lock and any other answer each have their own.
- The demo's `demo {action: "clip", mask}` makes the demo device clip, for tests.

## Embedding in a fleet server

```js
import { mount } from "/static/dspi-web/dspi-web.js";   // serve dist/ as a directory

const console = mount(document.getElementById("uconsole"), {
  backend: myFleetBackend,          // see src/fleet/types.ts
  title: "Acme venues",
  sourceUrl: "https://…/source",    // GPL: link to this build's source
});
// console.unmount();
```

What is unsaved (WB6b): `console.unsaved()` and `console.onUnsavedChange(fn)`.
By default the console asks the browser's leave-page question while something
is; a host page that asks its own way mounts with `leavePrompt: false`.

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
standalone page sets it from `<meta name="dspi-hub-page">`, which a hub fills in with
`/hub/`, upstream's DSPi Link page (WB7; `/` on a hub is a landing page linking both).
How to serve the build, the headers and the policy a host needs: [`DEPLOY.md`](DEPLOY.md).

Reaching hubs from a server-hosted page is bounded by the Link spec's `Origin`
rule: a hub accepts only its own pages (WEB-STUDY section 5). A fleet product
therefore carries Link to its hubs through its own server (a hub `url` on the
server's origin, as `tools/hub-host.mjs` does in the tests; behind TLS that is a
`wss://` address, which the core accepts), or serves the console from each hub
(`uconsole hub --web`, WB7).

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

`dist/` holds `index.html`, `app.js`, `page.css` and the icons `favicon.ico`, `favicon.svg`,
`apple-touch-icon.png` (the standalone page), `dspi-web.js` (the
library), `worker.js`, `dspi_wasm.wasm` (the core: 1 086 KB, 362 KB gzip with WebUSB and Link),
`dspi-web.css` and `THIRD-PARTY-NOTICES.txt`. A host serves `dspi_wasm.wasm` beside `worker.js`.
(With WB6a's preset calls the core is 1 126 KB, 373 KB gzip; with WB6b's curves 1 154 KB, 381 KB gzip;
on main before WB6c 1 168 KB; with WB6c's output calls 1 178 KB, 389 KB gzip.) The page needs
JSPI (Chrome 137, Firefox 153, Safari 27); without it every call says so. Preact (MIT) is the one runtime
dependency; `ci/licence-inventory.py` records it (section 6b), and the notices file carries its
licence and the core's Rust crates (`notices/rust-crates.txt`, written by
`tools/gen_rust_notices.py` in the cross container; the web step fails when it is stale).

-- WP WB7 -- Build settings: `DSPI_WEB_SOURCE_URL` is the GPL source link About shows
(an embedder's `sourceUrl` overrides it); `DSPI_WEB_RELEASE=1` refuses a build without
it. `tools/dist.sh <out>` is the release build, an artefact only ([`DEPLOY.md`](DEPLOY.md)).
The web step also runs the hosting routes on the real `uconsole` (`test/e2e/hosting.spec.ts`:
`uconsole hub --web` and `uconsole web`, built in the cross container as `DSPI_TERM`).

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
| `src/ui/screens.tsx` | a session's screens (WB6 slices) |
| `src/screens/main/` | the main window (WB6a): volume, presets, inputs and outputs; `model.ts` the tapers and layout |
| `src/screens/eq/` | the EQ (WB6b): graph, chip, band list; `model.ts` its logic, `draw.ts` the canvas; `Linkwitz.tsx` the Transform's panel (WB6c) |
| `src/screens/outputs/` | WB6c: an output's settings card (`OutputCard.tsx`), the level and delay control (`Level.tsx`, SL1), the limiter (`Limiter.tsx`), the crossover (`Crossover.tsx`); `model.ts` their logic |
| `src/screens/matrix/` | WB6c: the matrix mixer |
| `src/screens/events/` | WB8: the event log and Save Diagnostics…; `model.ts` its rows, filter and text |
| `src/ui/undo.tsx` | WB8: Undo and Redo, the keys, what a step left alone |
| `src/screens/system/` | SW3: the System view, Clock and links, the scene bar, the status rail; `model.ts` their logic, `feed.ts` the hub's `system.*` |
| `src/index.tsx` | `mount()` and the public types |
| `src/standalone.ts`, `index.html` | the standalone page (`?readonly` for a display page) |
| `test/unit/` | `node --test` |
| `test/e2e/hosting.spec.ts` | WB7: the console through each route, on the real `uconsole` (hub, `uconsole web`) and a static host |
| `DEPLOY.md`, `tools/dist.sh` | WB7: serving the build; the release artefact |
| `notices/rust-crates.txt`, `tools/gen_rust_notices.py` | WB7: the core's crates for `THIRD-PARTY-NOTICES.txt` |
| `test/e2e/`, `test/pages/` | Playwright; `embed.html` mounts two consoles with a server-like backend (`?core=stub`: on the fake core, `stub-worker.ts`); `usb.html` runs USB sessions over a WebUSB mock (`webusb-mock.ts`) whose devices are the core's simulated DSPi in the test module (Chromium); `fleet-hub.html` a fleet page with two hubs, `hubDevices` and `hubCredentials`, served by `tools/hub-host.mjs` (`link.spec.ts`, WB4b) |
