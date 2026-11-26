# uConsole for DSPi: the web console

The web console is uConsole for DSPi in a browser. It runs the same Rust core as the desktop app, Android and the
Terminal, so the rules that protect your speakers, the words and the presets are the same. It is served by a hub
(`uconsole hub --web`), by `uconsole web` on your own computer, or by any static web server
([`DEPLOY.md`](DEPLOY.md)). It reaches the demo DSPi, a DSPi on this computer's USB (Chromium-based browsers) and the
DSPis behind DSPi Link hubs.

The browser needs WebAssembly JavaScript Promise Integration (JSPI; Chrome 137, Firefox 153, Safari 27); without it
the page says so. USB needs a Chromium-based browser and a secure page (see [USB (WebUSB)](#usb-webusb)).

## What you can do with it

The web console has the Modern layout, its only one, laid out as the desktop app's Modern pages:

- **The top bar**: the open devices as tabs (the name, then how the device is reached), the preset popover, **A | B**,
  the search, Undo, Redo, **Save to Device** and the bell with the notifications.
- **The rail of sections**: Overview, Equaliser, Routing, Effects, Measure, Analyse, Library, System, and at the foot
  Devices and Settings. Each section's pages are its tabs. **Ctrl+K** (Cmd+K on a Mac) opens the command palette; a
  right-click opens a menu on what you clicked.
- **The dock** along the bottom: the input source, the User or Master volume, Mute, Bypass EQ and the meters.
- **Modes**: Simple, Advanced (the web's default) and Installer, switched in the status bar at the bottom.

What is new since 0.14.0 (0.15.0 was not published, so this is everything since the last release):

- **The same pages as the desktop's Modern layout**: the Equaliser (PEQ, Crossover, Delay & Gain, Inputs, with the
  inspector), the matrix mixer redrawn, Effects with their graphs, Analyse › Spectrum, Measure and Auto-tune, Library
  (Presets, Snapshots, History, Files, AutoEQ), System (Units, Clock and Links, Installation, Events), Devices and
  Settings, in the desktop's words. Settings › Speakers says which output drives what.
- **Fleet on a hub's page**: a page served by a hub starts on **Devices › Fleet**: every DSPi of the hub with its
  state, alarms and firmware, with Identify and Open. The hub's page (Devices › Hub) has parts: Devices, Alarms,
  Updates, Groups and Hub, each where the hub has it.
- **Snapshots are kept in this browser**, per DSPi, so a reload or the next evening finds them. In a private window,
  or where the browser keeps no site data, they last until the page closes, and the page says so. **Export…** in a
  snapshot's menu still saves it as a file.
- **The EQ graph shows the phase** once Settings › Graphing › **Show Phase Response** is on.
- **Crossfeed's output-pair picker**: which two outputs each crossfeed pair processes, on firmware that has the pair
  map.
- **Polarity from the Routing inspector**: an output's Polarity, Normal or Inverted, on every route that feeds it.
- **Solo** in the Routing inspector and in an output's menu plays only that output; the others are muted on the device
  until **End Solo** in the strip. A solo is never saved. A page closed or reloaded during a solo asks first and ends
  the solo as it goes, as far as the browser lets it finish.
- **A | B as saved against now**: from your first edit, A is the preset as saved on the device and B is what you hear
  now. Click A, or press B. While you listen to A, your edits on B still count as unsaved. Library › Snapshots still
  compares any two snapshots.
- **Home, a remote**: in Simple mode the device opens on **Overview › Home**: one big volume with Mute, the source, the
  presets as a Recall list and one health line. Simple's dock has no Master volume and no Bypass EQ.
- **The installer's lock**: a DSPi locked by its installer (through the hub that serves it: the web keeps no installer
  PIN of its own) opens on Home, and Home is its only page (the rail keeps Overview and Devices). A banner says "Locked by the installer: only volume, mute,
  source and presets can be changed.", with **Unlock…**. The locked user may change the volume, mute, the source and
  presets; nothing else is offered. The speaker layout cannot be changed under the lock.
- **Recovery that says what happened**: while a device is away the page is fenced (nothing on it can change) and the
  strip says why; a hub that now answers as a different hub at the same address is named. The notice goes when the
  device is back.
- **Reset Demo** (in the demo's strip and the command palette) puts the demo back to its tuned two-way speaker.
- **Add to Home Screen** (or the browser's Install) opens the console as an app in a window of its own. It still
  needs the server it came from.
- The Crossover says which outputs are off; the filters say Bell, Q, Low pass and High pass. Skip links, and a rail
  with one Tab stop.
- When an AI agent works on a hub's device through a desktop app, the console shows it in a strip and on the device's
  tab. Agents themselves run only in the desktop app.

## Not on the web yet

- **Crossovers and routes from your speakers**: no **From Speaker Layout…** on the web (desktop only).
- **The word "trial"**: the web's bar for a snapshot or auto-tune applied for a while (Keep, Revert) is still named
  "Safe apply" for screen readers, and A/B's note says "Finish the safe apply first: Keep or Revert."
- **Settings**: no Updates, Networking, AI Agents or This Computer pages (nor Inputs, Outputs, I2S Configuration or
  Control Interfaces).
- **Devices**: no Hubs tab (a hub is added on Devices › This Computer). No firmware is written from the web:
  Devices › Device only checks whether a chosen image fits the DSPi.
- **Settings › Speakers** shows a speaker detection's result, but the web has no **Detect…**.
- **Under the lock** the web shows Home alone; the desktop's Modern layout shows the other pages read-only with
  **Go to Home**.
- **Device tabs** show no "N unsaved" (only their tooltip says "unsaved changes"; the preset button carries the
  mark), and there is no Save All.
- **Web MIDI**, and AutoEQ's Rebuild from GitHub.

Nothing new in 0.16.0 has run on a real DSPi: the web console is tested in Chromium and Firefox against simulated
devices, a fake hub and a WebUSB mock.

---

The rest of this page is for developers.

## The core in a Worker

The same Rust core as every other client runs in a dedicated Worker under JSPI
(`docs/core-merge/WEB-STUDY.md`): `crates/dspi-wasm`, loaded by `src/core/worker.ts` from `dspi_wasm.wasm` beside
it. A small stub core (`src/core/stub.ts`) remains as the tests' fake only. The screens live in `src/screens/`, one
folder each, and `src/ui/screens.tsx` lists a session's screens in the order and modes it shows them; the rail's
sections are `src/ui/shell/sections.ts`. On a hub with its services a session also has the **Hub** page
(`src/screens/hub/`: the hub's devices, the health board, alarms, history, groups, group writes and bulk apply, the
firmware rollout), **Hub Settings** (`src/screens/hub-settings/`: alarm rules, schedules, notifications out, MQTT
and Home Assistant, API keys) and the **Fleet** (`src/screens/hub/FleetPage.tsx`). Measure (`src/screens/measure/`)
has the Measure, Results, Live, Auto-tune, Documents and Several units tabs, with the microphone captured by an
AudioWorklet (`src/core/capture-worklet.ts`). Device (`src/screens/device/`) shows the firmware generation, raw
requests by id and the update check. Hubs are reached over TLS
([`docs/integrator/security.md`](../../docs/integrator/security.md), The browser).

## What it is for

The main use of a web console is **a hub page that gathers several DSPi installations**; single-device use is
covered by the native clients as well. So the console:

- runs **standalone** (a static page, or served by a hub), and
- drops into a **server-based fleet product** with sign-in and access control,
  without changes to the console itself.

## Shape

```
page ── mount(el, options) ── App (Preact)
          │                     ├─ DevicePicker   installations → sources (demo, USB, hubs)
          │                     ├─ FleetPage      a hub-served page's DSPis
          │                     ├─ DeviceTabs     the open sessions
          │                     ├─ SessionView    one per open device: the shell's frame and its pages
          │                     └─ About          assets/about.json
          │
          ├─ FleetBackend      who the user is, which installations, what they may do
          │    ├─ LocalBackend      standalone: hub list in this page's storage, full control
          │    ├─ HubBackend        served by a hub: the hub's inventory and roles
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
- **The core is caller-driven** on the web: the Worker drains notifications
  every 100 ms (`poll`) and pushes a `state` event when something changed, and
  turns the analyser (`rtaTick`) at the wait the core asks for. A panic in the
  core ends the module: the session reports `lost`, later calls `dead`.
- **Wording** is the core's: the Link phases come from `link_phase_texts()`
  (generated to `ci/generated/web/link-words.json` by `ci/drift.sh` until the
  binding answers at run time); About is `assets/about.json`.

## USB (WebUSB)

**Chromium-based browsers only** (Chrome, Edge, Opera; Firefox and
Safari have no WebUSB), and only on a **secure page**: `https://`, or
`http://localhost` / `http://127.0.0.1`. Elsewhere the picker says which of the
two is missing. USB MIDI cannot go through WebUSB (Chromium will not let a page
claim an audio-class interface); the console uses the vendor interface 2, which
every DSPi firmware since 1.1.5 has, stock and fork alike.

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
installed. It is the one rule of every console, `xtask/70-dspi.rules`
(`MODE="0666"`; installed as `70-dspi.rules` by `sudo uconsole --install-udev`,
the desktop packages, the desktop app's dialog, or by hand into
`/etc/udev/rules.d/`), then `sudo udevadm control --reload-rules` and replug. A Flatpak or Snap Chromium may also need its own USB
permission. ChromeOS and Android Chrome have not been checked.

**Windows: no Zadig.** The firmware's MS OS 2.0 descriptors bind WinUSB to
interface 2 by themselves since 1.1.5 (WEB-STUDY section 6). Zadig is only the
blunt fix for a machine that cached another driver for that device (it saw
older firmware, or someone bound it by hand): then pick "WinUSB" for interface 2
in Zadig, or remove the device in Device Manager with its driver and replug.
That this needs no step on a clean machine is reasoned, not yet tested on real hardware.

**macOS:** no step expected; quit any other program holding the DSPi. Reasoned
from WEB-STUDY section 6, not yet run on real hardware.

## Link (hubs)

The client is the core's own, the same as every other console's:
`dspi-link`'s Rust client under the core's Link session and supervisor, built
into `dspi_wasm.wasm`. Only the socket is the browser's: `src/core/ws.ts` hands
whole WebSocket messages to the module's `dspi_ws_*` imports
(`crates/dspi-wasm/src/link.rs`). No `@dspi/link` JavaScript.

- **Steps**, each the core's (Link's choose_device flow), driven by the session
  model (`src/ui/session.ts`): `linkConnect` with the token kept for that hub
  and the hub's id it belongs to (the core sends it only once the hub has said
  it is that hub); if the hub wants a PIN, the session shows the core's
  `needs_pairing` wording and a PIN field (`linkPair`; a wrong PIN keeps the
  prompt); then the DSPi a fleet backend named, or the hub's own list
  (`linkDevices`) to choose from; then `open {kind: "hub", deviceId}` binds it
  and reads it.
- **Then the core keeps it up by itself**: the Worker turns the session
  (`linkTick`) at the wait the core asks for (at most 250 ms) and whenever the
  socket has something, and pushes each new phase. The banner is the core's
  phases in the core's wording: reconnecting (with Disconnect, the way out), no
  DSPi, closed, refused. A lost link reconnects with the core's backoff and
  binds the DSPi again; while it is away the page is fenced (`link-fence`), a
  write made while the link is down fails at once and is never sent later, and
  30 s of silence ends a session (the core's rule). A hub that answers as a
  different hub at the address is named, with Connect Again and Close.
- **No threads**: on the web `dspi-link` and the core's supervisor are
  caller-driven (`dspi_link::client::caller_driven`): whoever waits for the hub
  reads the socket meanwhile (JSPI suspends the call in `dspi_ws_wait`).
- **Several at once**: every hub session is a Worker of its own. A DSPi is open
  once: a second way to the same unit (USB and a hub, or two hubs) shows the tab
  that is already open (`src/ui/shell/open-rule.ts`). Several hubs can be open
  at once.
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
the one that hub serves (`<meta name="dspi-hub" content="wss://…">` names its
Link address; the standalone page lists it first), the page of `uconsole web`
(which relays to other hubs over TLS pinned to their keys), or a fleet server's
page whose server carries Link to its hubs through its own origin. A standalone
page can list any hub, but only hubs without the check answer it.

## The shell (the Modern layout)

`src/ui/shell/`: the desktop app's Modern layout (rail and page tabs), the web's only one.

- **Sections** (`sections.ts`): the rail's sections, with the desktop's ids and names, and the
  screens of `src/ui/screens.tsx` that are their pages. A mode's screens decide which sections
  show. A new screen is one row in `SCREENS` and its id in one section. Devices also has the
  page's own Fleet (served by a hub) and This Computer.
- **Device tabs** (`DeviceTabs.tsx`): the open sessions. Only the one on show is mounted; where each
  was left is kept in `places.ts`. **One session per DSPi** (`open-rule.ts`): a unit already open
  over USB or a hub is not opened again. Demo devices and files are not units.
- **Frame** (`Frame.tsx`): the top bar (`TopBar.tsx`, `SessionBar.tsx`: preset popover, A | B in
  `AbSegment.tsx`, Undo and Redo, Save to Device, the bell), the rail (`Rail.tsx`), the status strip
  (what limits the device now: the lock, a solo, the link away), the page, the dock (`Dock.tsx`:
  source, volume, meters) and the status bar (`StatusRail.tsx`: connection, mode, alarms).
- **Locked user**: `lockedScreens()` in `src/ui/screens.tsx` keeps a locked session to Home (the
  mode manifest's `lockedUser.screens`, `assets/modes/modes.json`).
- **Arrangement**: `?nav=tabs` (or `MountOptions.navigation`, or this viewer's
  `dspi-web.arrangement` in storage) puts the sections across the top instead.
- The layout follows the console's own width (container queries), as a host page may show several.
- The e2e tests go around through `test/nav.ts`.

## Overview › Dashboard

`src/screens/main/`, one per session, inside its view. The tiles (the device, its effects and its connection),
the channel cards and the speaker layout's note. The presets are the top bar's popover, the volume is the dock's,
and the levels are Equaliser › Inputs and Delay & Gain's. Home (`src/screens/home/`) is Simple's first page and the
locked user's only one.

The dock's volume is User or Master Volume (kept for this viewer in the page's storage), on the tapers of Weeb Labs' DSPi Console
(the user volume's square root over -60..0 dB; the firmware's master steps down to -128, shown as −∞), with Mute and
Bypass EQ. While a slider moves, each new position is written once the one before has been read back (the newest
wins); let go, it shows the device's value. Before a preset switch, a Copy to or a Disconnect with unsaved changes,
the prompt (Save, Discard, Cancel) lists the core's own sentences (`presetChanges`); leaving the page then asks the
browser's own question.

Every change goes through the core (`write` by registry path, or a preset call)
and the page shows what the device kept. A read-only session (a viewer, or
firmware the core only reads) shows everything with the controls off. At 375 px
the pages stack in one column with no sideways scroll.

## The Equaliser and the matrix

`src/screens/eq/`: the Equaliser's PEQ, Crossover and Delay & Gain tabs share one frame: the channels with their
curve pills, the response graph on a canvas with on-graph editing after the `PeqGraphEditor` of Weeb Labs' DSPi Console for macOS (keys and
gestures listed atop `EqGraph.tsx`; touch: drag, double tap, pinch for Q, long press for menus), the band list and
the inspector (`EqInspector.tsx`). The curves are the core's (`curves`, `crates/dspi-wasm/src/curves.rs`), the phase
too when Settings › Graphing asks for it; the canvas draws only when what it shows changes. Edits are `eq.*` writes
with the read-back, a drag's coalesced (newest wins, nothing replayed after a failure). The Crossover tab
(`CrossoverScreen.tsx`, `XoverCentre.tsx`) draws every enabled output's response on one graph over one table of
high and low passes. An output's settings card (`src/screens/outputs/OutputCard.tsx`: routing, gain, delay, mute,
the limiter) and the Linkwitz Transform (`Linkwitz.tsx`) are on the PEQ tab.

`src/screens/matrix/`: the matrix reads "from inputs ↓, to outputs →": a connected crosspoint is a
fixed tile in the input's colour with its gain inside and ø when inverted, an empty one a
dot; each row ends with the outputs it feeds, each column with its gain, delay and the
inputs that feed it; enable (the PDM / Core 1 questions) and mute sit in the column's head,
an input's trim in its row's. The keys are the desktop's (arrows, Home/End, PgUp/PgDn, Space, Enter
or a digit to type, + and − with Ctrl for 3 dB, I, M, E, Delete, Shift+F10); the inspector
follows the cell, column or row chosen, and an output's inspector has Mute, Solo and Polarity
(`src/screens/outputs/polarity.ts`; the solo is the core's, `src/ui/solo.ts`). The page tools: Link L/R pairs (an
edit of a stereo pair's crosspoint mirrored to `(i ^ 1, o ^ 1)` as one undo step; off by default, kept per
device), Direct 1:1, Clear (asks past four crosspoints), Dense. The rules without I/O are
`matrix/model.ts`.

The delay follows one rule (`outputs/Level.tsx`): a drag sends
whole ms live and on release, never -0, and the thumb snaps to what was
committed; typed values are sent as typed and values from the device are shown
as they are (to 0.0001 ms); the wheel and the keys go to the next whole ms;
a key or an assistive technology's change commits at once. The longest delay is
the device's (`outputLimits`). Three binding calls cover what `write` by path
does not (`crates/dspi-wasm/src/outputs.rs`): `crossoverType` (the registry's
`eq.type` takes PEQ types only), `crosspoint` (the registry's `mix`, its packet
built in the binding) and `outputLimits`.

## The design language

The Modern language of `docs/DESIGN-LANGUAGE.md` on the web: a layer at the end of
`dspi-web.css` (the Qt tokens: control, selection, separator, stripe, accent fill and text,
radii, the 760 px content width), no outlines on cards, buttons and fields (the focus ring
stays), the accent only for on, chosen and the one primary action (options as chips and
segments in the selection tone), notices with a tint and an edge. `ui/design.tsx` holds
`EmptyState`, `NoticeRow`, `ChipGroup` and `Glyph`; the empty states use the desktop app's texts.
Typed numbers go through `ui/number.ts` (`readNumber`, `plainNumber`):
a decimal comma, U+2212 and a unit after the number, as Qt's `BorderedField.number()`.

## The System section, scene bar and status bar

`src/screens/system/`: the web is the first client of the Advanced layer
(`docs/UI-MODES.md`). One feed per session (`feed.ts`) asks the hub's audio-link
system model over the session's own Link connection (`hubSystem` and `hubRole`,
`crates/dspi-wasm/src/system.rs`, the hub's `system.*` of
[`docs/integrator/audio-link.md`](../../docs/integrator/audio-link.md)) and polls
`system.status` every 3 s while the page is visible. The hub judges every request by
the session's role; the page only leaves out what it would refuse.

- **System › Units** (`SystemScreen.tsx`): rooms and zones with their units, each
  unit connected or not, its audio link's lock and its alarms; the topology (sender,
  fan-outs, units, the output each hangs on, hop delays, each unit's audio link slots). On
  a hub with an installation the System section comes first; a device the console holds
  itself (USB, the demo), or a hub without an installation, gets an empty state with the
  way to the hubs.
- **System › Clock and Links** (`AlinkClock.tsx`): per unit `d_local` against
  its limits, `hops_offset` (with what the unit has, when it differs), the path and
  latencies, a measured latency and each room's lip sync; the audio link delay D; the slot
  matrix (audio link channels to each unit's 8 inputs); Compute and Align
  (`system.align`, asked first: a re-anchor is 2 to 6 ms of silence) with each
  unit's result; link stats and the trim scan. Edits are staged and saved as one
  `system.set`. Model and alignment are Admin only: other roles see it read only.
- **System › Installation** (`installation/`): the units the hub has not placed yet, rooms, zones,
  groups, senders and fan-outs, saved with the hub's Admin role; a room's or zone's volume is
  `ZoneVolume.tsx`. The web's own **Signal Chain** is the section's last tab.
- **Scene bar** (`SceneBar.tsx`, on the Overview and System in Advanced and Installer): the
  installation's scenes, numbered; a recall is `system.scene.apply` (asked first when the device
  has unsaved changes), then the frame, the path and each unit's result (queued, applied, late,
  refused). A scene the role may not run is refused whole and the op named.
- **Status bar** (`StatusRail.tsx`), below the page: connection (red with "(unsecured)" for a
  hub link that is not encrypted), the last scene, the mode (Simple, Advanced, Installer;
  remembered per device or for every device; a hub can pin it, and a hub's installer lock shows
  as Locked), the system mute (`system.mute`) and the alarms with Acknowledge.

The e2e (`test/e2e/system.spec.ts`) runs against `uconsole --demo hub --web
--alink`: the fake device's audio-link roles (a sender, a fan-out, four units)
with a system model, open (Admin) and with a PIN (Control).

## Undo and Redo, the event log, diagnostics

- **Undo and Redo** (`src/ui/undo.tsx`) are the core's journal (`undo`,
  `redo`, `canUndo`), as on the other clients: two buttons in the top bar,
  and the keys Ctrl+Z (Cmd+Z on a Mac), Ctrl+Shift+Z and Ctrl+Y. A text field with
  focus keeps the keys for its own undo; on a page with several consoles they act
  on the one the focus is in (or the first, with the focus nowhere). A slider drag
  and an EQ graph drag or pinch are marked (`setDragging`), so a drag is one step;
  the mark always ends (the pointer let go anywhere, cancelled, its capture lost,
  the control gone), once the drag's last write has gone; and, as a safety net, when the
  window loses focus or the page is hidden mid-drag. A graph drag sends each
  PEQ band whole (`filter`, the core's `set_filter`, as the desktop's graph does),
  so the core journals one change per tick. An undo another writer stepped in on
  says who, in the core's words (`note`); changes that cannot be undone are named
  when stepped over. A viewer's session, firmware the core only reads and a locked
  user have both off.
- **Events** (`src/screens/events/`, Advanced and Installer): the core's event log
  (clips, the audio link's lock and loss, resets, connections, temperature,
  counters; `eventLogRead` per device, only what is new, every second while shown and
  the page is visible) and, on a hub's installation, its alarms; time, unit and text,
  newest first; a unit filter; Copy as Text (one tab-separated line per event; on a page
  without the Clipboard API, such as a hub's page over plain http, the older copy command,
  and failing that the text shown selected to copy by hand).
- **Save Diagnostics…** (on the Events page, so Advanced and up, as the desktop's Settings › Advanced): the core's bundle, what `uconsole
  diag` and the desktop's Help › Save Diagnostics… write (`diagnostics`,
  `crates/dspi-wasm/src/diag.rs`: `DspiCore::diagnostics_zip` as base64), handed to the
  viewer as a download, with the browser's user agent as `client/browser.txt`.
  Nothing is sent anywhere.
- **A refused scene** says it in the core's sentence (`sceneOutcome`, the core's
  `scene_recall_outcome`, the one every client shows): the held frame only when the
  sender refused; a timeout, a lock and any other answer each have their own.
- **A taken scene and a written alignment** are the core's sentences too
  (`sceneOutcome`, `alignOutcome`): a unit whose firmware has no audio link takes a
  scene at once ("applied now on K (no audio link); at audio link frame N … on the others"),
  "on time on every unit" only when it is true, and an alignment says what it wrote,
  what it skipped and why, or that it wrote nothing. Such a unit shows "No audio link
  (firmware)", never "Unknown"; a save that names one shows the hub's warning.
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

What is unsaved: `console.unsaved()` and `console.onUnsavedChange(fn)`.
By default the console asks the browser's leave-page question while something
is; a host page that asks its own way mounts with `leavePrompt: false`. Other
options: `workerUrl`, `hubPage`, `readOnly`, `navigation` (`"rail"` or `"tabs"`).

`FleetBackend` (`src/fleet/types.ts`):

| Member | Standalone (`LocalBackend`) | A fleet server |
|---|---|---|
| `kind` | `local` | the server's own name, for logs and About |
| `installations()` | one, "This console": demo, USB, saved hubs | the user's sites from its API |
| `addHub`, `removeSource` | present | omit them when the server manages the list |
| `access(installation, source, device)` | always `control` | `control` or `view` per role; `view` makes the session read only |
| `user()` | `null` | the signed-in user, shown in the header |
| `hubCredentials(url)` | absent: the user pairs with each hub once; the token is kept in this page's storage | `{token, hubId?}` the server holds for the fleet; no PIN is asked. A refused one is not offered again this visit, and the user may pair instead (kept in memory only) |
| `hubDevices(url)` | absent: the console asks the hub (choose_device) | the hub's DSPis the server knows, `{deviceId: serial, name}`, each its own entry that opens without asking the hub; `null` for a hub it leaves to the console |
| `pinnedMode(installation, source, device)` | absent: the user chooses (a hub's own pin still applies) | `simple`, `advanced` or `installer` per role; it overrides the user's choice, and the mode switch says so |

A page served by a hub uses `HubBackend` (`src/fleet/hub.ts`): the serving hub is an installation of its own, its
DSPis from the hub's inventory, what a user may do from the hub's role for the page's own Link session.

The console never signs anyone in, never stores a server credential and never
decides access. Styles are scoped under `.dspi-web` (`dist/dspi-web.css`); light
and dark follow the system unless the host sets `data-theme` on `.dspi-web`. A
host page's own element styles can still reach the console's buttons and inputs.
`hubPage` (a URL) adds a link to the serving hub's own page in the header; the
standalone page sets it from `<meta name="dspi-hub-page">`, which a hub fills in with
`/hub/`, upstream's DSPi Link page (`/` on a hub is a landing page linking both).
How to serve the build, the headers and the policy a host needs: [`DEPLOY.md`](DEPLOY.md).

Reaching hubs from a server-hosted page is bounded by the Link spec's `Origin`
rule: a hub accepts only its own pages (WEB-STUDY section 5). A fleet product
therefore carries Link to its hubs through its own server (a hub `url` on the
server's origin, as `tools/hub-host.mjs` does in the tests; behind TLS that is a
`wss://` address, which the core accepts), or serves the console from each hub
(`uconsole hub --web`).

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

Outputs and caches go to `${DSPI_BUILD_CACHE:-~/.cache/dspi-build}/work/web/<branch>/` (`dist/` is there,
not in the tree). `node tools/serve.mjs <dist>` serves a build on loopback for a look.
`node tools/hub-host.mjs <dist> <port> <dspi-link-fakehub>` serves it as a hub
would, with the `Origin` rule, over fake hubs it starts on request (the Link
e2e's host; `DSPI_FAKEHUB` names the binary, which `tools/ci.sh` builds).

`dist/` holds `index.html`, `app.js`, `page.css`, `manifest.json` and the icons (`favicon.ico`, `favicon.svg`,
`apple-touch-icon.png`, `icon-192.png`, `icon-512.png`) (the standalone page), `dspi-web.js` and `dspi-web.css` (the
library), `worker.js`, `capture-worklet.js` (the microphone), `dspi_wasm.wasm` (the core: about 6.9 MB, 1.9 MB gzip
in November 2026), `autoeq_database.json` (the built-in AutoEQ database), `SOURCE.txt` and
`THIRD-PARTY-NOTICES.txt`. A host serves `dspi_wasm.wasm` beside `worker.js`. Preact (MIT) is the one runtime
dependency; `ci/licence-inventory.py` records it (section 6b), and the notices file carries its
licence and the core's Rust crates (`notices/rust-crates.txt`, written by
`tools/gen_rust_notices.py` in the cross container; the web step fails when it is stale).

Build settings: `DSPI_WEB_SOURCE_URL` is the GPL source link About shows
(an embedder's `sourceUrl` overrides it); `DSPI_WEB_RELEASE=1` refuses a build without
it. `tools/dist.sh <out>` is the release build, an artefact only ([`DEPLOY.md`](DEPLOY.md)).
The web step also runs the hosting routes on the real `uconsole` (`test/e2e/hosting.spec.ts`:
`uconsole hub --web` and `uconsole web`, built in the cross container as `DSPI_TERM`).

## Files

| Path | What |
|---|---|
| `src/core/protocol.ts` | the page ↔ Worker contract, the shape `crates/dspi-wasm` implements |
| `src/core/serial.ts` | one call at a time, with a `busy` guard |
| `src/core/worker.ts` | the Worker: loads the core (`dspi_wasm.wasm`) |
| `src/core/wasm.ts` | the core's glue: imports, JSON calls, the Worker's timers |
| `src/core/client.ts` | the page's side: calls, replies, events |
| `src/core/stub.ts` | the tests' fake core |
| `src/core/usb.ts`, `usb-ids.ts` | WebUSB in the Worker: claim, transfers, notifications, unplug |
| `src/core/ws.ts` | the browser's WebSocket as the core's socket imports |
| `src/core/capture.ts`, `capture-worklet.ts` | the measurement microphone, through an AudioWorklet |
| `src/fleet/hub-tokens.ts` | hub tokens: this page's storage, or the fleet's credentials |
| `src/fleet/` | installations, sources, access; `LocalBackend`, `HubBackend`; what this page can reach |
| `src/ui/` | Preact components and the session model |
| `src/ui/screens.tsx` | a session's screens, by mode; the locked user's |
| `src/ui/shell/` | the Modern shell: sections, device tabs, top bar, frame, dock, A \| B, preset popover |
| `src/ui/solo.ts`, `src/ui/SoloStrip.tsx` | Solo and End Solo, the strip while an output is soloed |
| `src/ui/palette/`, `src/ui/menus/` | the command palette; the menus, from `assets/actions.json` |
| `src/screens/home/` | Home: the remote of Simple mode and the locked user |
| `src/screens/main/` | Overview › Dashboard: the tiles and channel cards; `model.ts` the tapers |
| `src/screens/eq/` | the Equaliser: graph, band list, inspector, Crossover and Delay & Gain tabs; `model.ts` its logic, `draw.ts` the canvas; `Linkwitz.tsx` the Transform's editor |
| `src/screens/outputs/` | an output's settings card (`OutputCard.tsx`), the level and delay control (`Level.tsx`), the limiter (`Limiter.tsx`), the crossover (`Crossover.tsx`), polarity (`polarity.ts`); `model.ts` their logic |
| `src/screens/matrix/` | the matrix mixer and the Routing inspector; `model.ts` its rules |
| `src/screens/effects/` | the effects and their graphs; `pairmap.tsx` Crossfeed's output-pair picker |
| `src/screens/snapshots/` | snapshots and A/B; `store.ts` the snapshots this browser keeps per DSPi |
| `src/screens/settings/` | Settings' pages (`pages.ts`), Speakers (`speakers/`) |
| `src/screens/hub/` | the hub's page (Devices, Alarms, Updates, Groups, Hub) and the Fleet (`FleetPage.tsx`) |
| `src/ui/design.tsx`, `src/ui/number.ts` | EmptyState, NoticeRow, ChipGroup, Glyph; typed numbers as Qt reads them |
| `src/screens/events/` | the event log and Save Diagnostics…; `model.ts` its rows, filter and text |
| `src/ui/undo.tsx` | Undo and Redo, the keys, what a step left alone |
| `src/screens/system/` | the System section's pages, the scene bar, the status bar; `model.ts` their logic, `feed.ts` the hub's `system.*` |
| `src/index.tsx` | `mount()` and the public types |
| `src/standalone.ts`, `index.html`, `manifest.json` | the standalone page (`?readonly` for a display page, `?nav=tabs`); the web app manifest |
| `test/unit/` | `node --test` |
| `test/e2e/hosting.spec.ts` | the console through each route, on the real `uconsole` (hub, `uconsole web`) and a static host |
| `DEPLOY.md`, `tools/dist.sh` | serving the build; the release artefact |
| `notices/rust-crates.txt`, `tools/gen_rust_notices.py` | the core's crates for `THIRD-PARTY-NOTICES.txt` |
| `test/e2e/`, `test/pages/` | Playwright; `embed.html` mounts two consoles with a server-like backend (`?core=stub`: on the fake core, `stub-worker.ts`); `usb.html` runs USB sessions over a WebUSB mock (`webusb-mock.ts`) whose devices are the core's simulated DSPi in the test module (Chromium); `fleet-hub.html` a fleet page with two hubs, `hubDevices` and `hubCredentials`, served by `tools/hub-host.mjs` (`link.spec.ts`) |
