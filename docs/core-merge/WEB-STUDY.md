# Web study (WP WB0)

*Written 2026-11-06 at `9a157a9`. For the owner, who decides. Nothing here was published or contacted.*

Evidence is marked: **Ran** means I ran it in headless Chromium 156 or Firefox 159 in a container. **Read** means I read
the code. **Source** means a published document, listed at the end. No device was attached, so **nothing here has been
tried against real USB, MIDI or a real hub.** WB4 covers that.

## 1. Recommendation

**Do not build a new web console now.** Do the 1.5 units in row 1 of the table, and decide row 2 after you have talked
to Weeb Labs.

| | Route | What the user gets | New work (units) |
|---|---|---|---|
| **1 (recommended now)** | What exists, plus keeping the door open | The hub-served Link page (HB5, already planned) and sigman78's hosted console for WebUSB, which is live today. The core stays buildable for wasm, checked by CI | **about 1.5** |
| 2 (decide later) | Rust core in a Worker under JSPI, with a web UI | One web client from the same base, with the same logic as every other client | **39–62**, or **20–30** if sigman78's Svelte UI is ported onto the core |
| 3 (optional) | Extend sigman78's console: Link, extension channel, Web MIDI | His console gains our transports. It stays a separate codebase | **3–5**, and it needs contact with him, which is your act |
| Not recommended | Qt for WebAssembly | The demo works today, at 12 MB. **Device access is unproven** | Demo only: 1. With a device: **15–25, with risk** |

**The tension you need to weigh.** Your main goal is one project that builds every version from one base. Route 1 does
not meet that for the web: sigman78's console is a second implementation of the protocol, about 39,000 lines of TypeScript
and Svelte, and it has no extension channel. Only route 2 meets it. If the web must be built from that one base, choose
route 2. The spike removed its technical risk, and what remains is the cost of the UI.

**The main finding** is that the async problem turned out cheap to solve. With JSPI, the existing blocking `dspi-core` connected, wrote and read back in both browsers over a
transport where every call is a JavaScript Promise. `dspi-session`'s API did not change. Nearly all of what remains in
route 2 is the UI.

## 2. Rust core in wasm (spike: worked)

**What ran.** `dspi-session` and `dspi-core` compiled for `wasm32-unknown-unknown`. They drove a JavaScript "bus" with the
same shape as a WebUSB transport, where every transfer is a Promise resolved on a later task. The device behind the bus is
the core's own `FakeDevice`, in a second wasm module, so the session code under test is real. There is no wasm-bindgen; the
imports are hand-written.

Scenario, the same in every mode: full connect (39 transfers), `Session::write("vol.user")` with read-back and a bulk
re-read, an out-of-range write refused, a knob change at the device arriving as notification packets decoded by the pure
Rust half, a sleep, and 500 meter reads.

### Which option leaves `dspi-session` unchanged?

| Option | Result | API change for other clients | Needs | Cost per transfer (Ran, Chromium) |
|---|---|---|---|---|
| (a) Async twin of the trait | Not spiked. Size measured | **Yes, or a second copy.** 78 functions and about 166 call sites in `dspi-session`, 17 files; with `dspi-core`, 238 functions, about 433 sites, 30 files (regex call graph, an estimate; the plan's 170 holds up) | Nothing special | n/a |
| (b) Request and reply split | Not spiked | **Yes.** The same 78 functions become state machines. The worst of the four | Nothing special | n/a |
| (c) Worker blocking in `Atomics.wait` | **Ran, passed** | No | COOP and COEP headers and a secure context. Without the headers there is no `SharedArrayBuffer` and it cannot start (Ran). As far as I know GitHub Pages cannot set those headers; I did not check | 0.027 ms |
| (d) JSPI | **Ran, passed** in Chromium 156 and Firefox 159 | No | Nothing: no headers, and it also works on plain `http://` (Ran, Chromium), so a hub-served page can use it | 0.002 ms on the main thread |

**Option (d) in a dedicated Worker is the route to take.** Ran in both browsers, with no COOP or COEP. JSPI is Baseline
since 2026-09-14: Chrome 137, Firefox 153, Safari 27 (Source). Safari was not tested. Firefox and Safari matter only for
Link, since neither has WebUSB.

### JSPI's two sharp edges (both Ran)

Both come from other code entering the module while a call is suspended, and both are removed by one rule: **one call at a
time, through a queue.** A Worker with serial RPC gives that naturally, which is why the Worker is part of the
recommendation.

1. **A read during a suspended call traps.** `get_state()` during an in-flight `set_user_volume` gives
   `RuntimeError: unreachable` in both browsers, because `Inner`'s mutex is held across the suspension. The core survived
   and the next call worked, but the UI cannot call the core freely.
2. **Two suspended calls that finish out of order corrupt memory.** Rust's stack lives in linear memory, one per module.
   With a 5 ms and a 30 ms call overlapping, **all 4096 bytes of the second call's stack buffer were overwritten**, with no
   error, in both browsers. Run one after the other, both were intact. This one is silent, so it needs a guard in the
   binding, not only a convention.

### What changed in the core (spike copy only)

Native tests on the patched copy pass: `cargo test -p dspi-transport -p dspi-session -p dspi-core`, 10 test binaries, 0
failures.

| Change | Size in the spike | Why |
|---|---|---|
| `dspi_transport::clock`: `Instant` and `sleep`. Re-exports `std` natively | 30 lines new; about 15 lines changed in session and transport | `std::time::Instant::now` and `thread::sleep` panic on this target |
| `nusb` only on non-wasm targets, and the USB module gated | A few lines. `dspi-core` reuses its existing Android `cfg` paths | See below |
| **Whole diff of core, session and transport** | 75 lines added, 35 removed, of which about 20 are the Qt spike's wrapper | Mostly `use` lines and `cfg` conditions |

**Still to do, not done in the spike:**
- **Threads.** `std::thread::spawn` fails on this target. `open_demo` over wasm fails cleanly: "cannot start the notification
  reader" (Ran). A transport with no notification source connects fine. Needed: a way to push packets into `dspi-core`
  (`Notifications` is thread-only; the spike fed `dspi-session`'s `DeviceState` instead), and timers from JavaScript in place
  of the core's spawn sites: `notify.rs`, `rta.rs`, `link.rs` (two).
- **`SystemTime::now`** also panics (`monitor.rs:91`, `files.rs:174`, `preset_file.rs:2295`). The spike did not reach these.
- **Link inside the core:** `dspi-link` uses threads and `TcpStream`. The simple route is a transport that calls MIT
  `@dspi/link` in JavaScript. Read only, not tried.

**`nusb`: the plan's lead does not help.** The pinned 0.2.5 has **no** WebUSB backend, which is why the crates fail to build
for wasm today (Ran). 0.2.9 has one, but it needs `--cfg=web_sys_unstable_apis` and wasm-bindgen, and `wait()` does not
exist there (Read), so it fits only option (a). Under JSPI, about 30 lines of JavaScript replace it.

### Size

| Module (`opt-level=s`, LTO, no `wasm-opt`) | Raw | gzip |
|---|---|---|
| The whole C ABI (`dspi-capi`), so all of the core | 753 KB | **288 KB** |
| The spike around `dspi-session` | 181 KB | 73 KB (brotli 61 KB) |
| The spike around `dspi-core` | 239 KB | 95 KB |

For scale, connect with 1 ms per transfer took 152 ms (Ran). Real USB timing is unknown.

## 3. Qt for WebAssembly (spike: partly worked)

**The demo works; device access does not.** Qt 6.11.3 `wasm_singlethread`, Emscripten 4.0.7 (the version Qt's documentation
gives), Rust's `wasm32-unknown-emscripten`.

| Question | Answer |
|---|---|
| Does the C ABI core build? | **Yes** |
| Does the Qt app build? | **Yes, with few changes:** `qt_add_executable` with `qt_import_qml_plugins` and finalize; two `QProcess` sites guarded (udev rule, udisks) |
| Does it run? | **Yes, on the demo device (Ran).** Onboarding, then the main view with graph, channels, presets and volume, driven by real clicks. Rendered with SwiftShader |
| Download | **30.1 MB wasm: 12.0 MB gzip, 9.9 MB brotli**, plus 260 KB JavaScript. 6.4 MB of source data is AutoEq and firmware in the `.qrc`, which could be left out. Time to first use was not measured on a real network |
| Same async problem? | **Yes, and worse.** See below |

The three ways of suspending, each tried with a transport that suspends before every transfer:

| Attempt | Result | What stopped it |
|---|---|---|
| JSPI, `-sJSPI` on Qt's binaries | Links, then **hangs at the Qt logo**: `SuspendError: trying to suspend without WebAssembly.promising` | Qt's documentation says JSPI needs Qt built from source, with `-feature-wasm-jspi -feature-wasm-exceptions`. **I did not build Qt from source.** |
| Asyncify, `-sASYNCIFY` | **The build fails:** Binaryen's Asyncify pass aborts, `unexpected expression type` | Probably the wasm exception handling in Rust's prebuilt std (`panic="unwind"`). Not confirmed |
| Threads, `wasm_multithread` | **Link fails:** `--shared-memory is disallowed ... not compiled with 'atomics'` | Rust's prebuilt std has no atomics. It needs `-Zbuild-std`, and then COOP and COEP. Not tried |

**So this spike is not finished for device access.** Even with those solved, one problem remains. The Qt app makes about 200 calls to
about 140 distinct C ABI functions, all synchronous from the UI thread, with no `QThread` in `src/`. A suspended call lets Qt's
timers and events run, and they re-enter the core: that is the trap from section 2, across the whole app. So the bridge
needs an async rewrite or a global guard, which is why I put device access at 15–25 units rather than the plan's 8–15. That
figure is an estimate.

It also fails your "native feel" rule: a canvas with desktop window controls inside a browser tab, and 12 MB against about 0.5 MB for the other routes.

## 4. Reusing what exists

| | sigman78's console | DSPi-Link web UI |
|---|---|---|
| Built and tested (Ran) | Builds. Tests: **1991 of 1991 pass**; a first run under load had 1 failure that did not repeat. JavaScript 692 KB (206 KB gzip). `?mock` runs | Builds, 167 of 167 pass. **69 KB of JavaScript** (25 KB gzip) |
| What it is | Full console: room, tune, program, setup, control. About 39,000 lines of code, 25,000 of tests | Small: connect, device list, one device with volume and meters, bridge settings. **Not a console** |
| Licence | MIT, © SiGMan | MIT, © Weeb Labs |
| Extension channel | **None** (Read) | None |
| Link | **None.** `DspTransport` is a small interface, so a Link transport is simple, but `DspDevice` reads bulk with chunked `0xA2`, which hubs refuse (Link spec 8.2), so bulk needs a path through `snapshot` | Is the Link client |
| Web MIDI | None. It would be a transport | n/a |

**What they need:**
- **sigman78's console, for route 3:** Link transport with a bulk path 1.5, extension channel 0.5–1, Web MIDI 0.5–1, hardware
  0.5. **3–5 units.** This does not include reaching the macOS floor: **I did not audit his console against `PARITY.md`
  section 5**, and your "macOS features are the floor" rule would apply to it. A full audit is about 0.5.
- **DSPi-Link web UI:** nothing for HB5. It is too small to be the web console; it is a remote for a hub.

**Licence fit (technical inventory, not legal advice).**
- MIT code can go into our GPL project with its notice kept. The combination is then GPL, so **sigman78 could not take route 2's
  result back into his MIT project** unless the parts are licensed for that.
- Route 3 stays MIT only if it uses no code from our GPL core, so the transports would be written fresh against the
  MIT Link spec and fixtures. The extension channel's protocol is ours, so that part would need your permission or a written spec to work from.
- Serving a GPL wasm build means offering its source. A link on the page is the usual way. **OD-Lic is unaffected**, as none of this reaches the iOS binary.

## 5. Link in the browser

**The plan's question assumes the wrong blocker.** Mixed content is not universal, and it is not the first wall. Tested with
an `https://` page and a raw socket as the hub, recording which upgrades actually arrived (Ran):

| Target from an `https://` page | Chromium 156 | Firefox 159 |
|---|---|---|
| `ws://` private IP | **Reaches the hub** | Blocked |
| `ws://name.local` | **Reaches the hub** | Blocked |
| `ws://public-name` that resolves to a LAN address | Blocked (`SecurityError`) | Not tested |
| `ws://localhost`, `ws://127.0.0.1` | Reaches the hub | **Reaches the hub** |

Chromium's exemption comes with Local Network Access: with that feature off, the private IP and `.local` were blocked (Ran).
Normally the user gets a permission prompt. Headless did not show me one, so **the prompt is untested.** Safari was not tested.

**The real wall is the hub.** Link spec section 4 says a hub refuses an upgrade whose `Origin` names any site but itself. The
browser sends the page's origin (Ran: the raw hub saw `https://dspi-test.example:8443`). The ESP32 bridge enforces this
(`http_server/origin.c`, Read).

**So: is a hub-served page the only route?**
- **With hubs as specified, yes**, and because of `Origin`, not TLS.
- **A hosted page** would need all of: a spec change that lets a hub allow a named origin, which is Weeb Labs' call and weakens
  a deliberate defence against web pages driving a hub; Chromium, or a hub on loopback; and the permission prompt.
- **`http://localhost`** served by a local `dspi hub` is a secure context, so it could offer WebUSB and Link together.

A hub-served page is `http://` on the LAN, so it is not a secure context: no WebUSB, no Web MIDI, no `SharedArrayBuffer` (Ran).
It does not need WebUSB. **JSPI works there (Ran)**, so the Rust route can serve from a Rust hub.

Two things this turned up:
- **HB1's entry does not mention the `Origin` check.** A Rust hub without it would let any web page reach it. `dspi-link`'s fake hub
  has none either. Please add it to HB1.
- **The local copy of the Mac hub shows no check** (`LinkServer.swift` has only the `/info` CORS header, Read; that copy is dated 2026-11-05).
  Worth raising with Weeb Labs if you wish; a spec-conformance matter on their side, and it may have changed since.

**Size limits:** the ESP32 has a 1920 KB app slot and keeps its small bundle. sigman78's `dist` is 4.6 MB, mostly `autoeq-db.json` and
images, so it fits a PC hub only.

## 6. WebUSB

**Chromium only:** Firefox and Safari oppose it (Source), and Firefox 159 has no `navigator.usb` (Ran). It needs HTTPS or
`localhost`: on `http://` to an IP it is absent (Ran). It is available in a dedicated Worker (Ran), so the transport can live beside the core.

| OS | Stock (1.1.5+) | bofr-main | Note |
|---|---|---|---|
| Windows | Should work with no driver step. **Not run** | Same | See below |
| Linux | Needs a udev rule | Same | sigman78 ships one, and so does our package |
| macOS | Should work. **Not run** | Same | Same path as the Mac Console's interface 2 |
| Android | **Not checked** | | sigman78's console shows a mobile splash |
| iOS | No WebUSB | | |

All **Read, not run**. Stock and bofr-main expose the same vendor interface: bofr-main's descriptor diff adds MIDI after it and asserts `ITF_NUM_VENDOR == 2`,
leaving the BOS and MS OS 2.0 parts unchanged. Chromium refuses to let a page claim audio-class interfaces (a known Chromium rule, not run here), which is why USB MIDI cannot go through WebUSB; the vendor interface is unaffected.

**Plan's correction 19 is right:** no WebUSB-specific firmware descriptor is needed. The BOS holds only the MS OS 2.0 capability, and a WebUSB descriptor would only add a landing-page hint.

**Why does sigman78's README still mention Zadig?** Mostly it is out of date, with one real case behind it.
- `DSPi` firmware has MS OS 2.0 at interface 2 for WinUSB. Commit `2541932` (2026-05-01, "Fixed USB descriptors - WCID, IAD signalling triplet and MS OS 2.0")
  repaired it, and that commit is first in **v1.1.5, his minimum**. Earlier tags have the descriptors in some form, which is what the older line reflects.
- His own `docs/HW-NOTIFICATIONS.md:68` already says "likely **no Zadig** for new devices". The README line and two UI texts did not follow.
- **The real case:** Windows caches the driver choice per VID, PID and `bcdDevice`, so a machine that saw older firmware or was bound by hand can keep a stale choice. His wiki's Windows page gives that as the common failure, with Zadig as the blunt fix (Source).

So on a clean machine, 1.1.5+ should need no Zadig. **That is reasoned, not tested**; WB4 has to confirm it on a clean Windows.

## 7. Web MIDI

Present in Chromium and Firefox (Ran), not in Safari (Source, old; not tested). Both require a permission (Source, not exercised). **In Chromium it is not available in a Worker (Ran)**, so a core in a Worker has to proxy it from the page. Absent on `http://` to an IP (Ran).

- **Firmware's MIDI build only**, and about 2 s per 6 KB bulk read (from the repo's own comment).
- The core side fits: `MidiClient` is pure. Under JSPI, skip the blocking, thread-using `MidiTransport`.
- **It is the only route that could reach Firefox without a hub.**
- **Your "no SysEx ID for now" decision limits public MIDI releases**, and this would be one.

Size: 0.5–1 within route 2 or 3. **Worth doing later, not first.**

## 8. The stubs, filled in

Sizes are the plan's units, and are estimates. These fill the plan's stubs for whichever route you choose; **I did not edit `PLAN-CLIENTS.md`, since WB0 owns only this file.**

| Stub | Route 1 (recommended now) | Route 2: Rust core | Route 3: extend sigman78's |
|---|---|---|---|
| **WB1** transport and session | **0.75:** clock seam, `nusb` gating, `SystemTime`, CI `cargo check` for wasm32 | **7–10:** WB1 of route 1; push notifications and JS timers 1.5–2; typed binding and serial-queue guard over the C ABI 2–3; WebUSB, Link and MIDI transports in the Worker 2–3; thread sites and `files` 1–1.5 | **2.5–3.5** |
| **WB2** UI | 0 | **30–50** new UI, or **12–18** porting sigman78's Svelte components and his `state` layer (about 17,000 lines) to the core's records | 0, plus any floor gaps found by the audit |
| **WB3** hosting and origin | **0.25:** a pointer in the README to his console, once you agree. HB1 `Origin` check goes under HB1 | **0.5:** an HTTPS static host, no special headers needed under JSPI; source link for GPL | 0: his site |
| **WB4** hardware test | **0.5:** Windows clean-machine Zadig check | **1:** Windows, Linux, macOS, stock and bofr-main | **0.5–1** |
| **Total** | **about 1.5** | **39–62; 20–30 reusing his UI** | **3–5** |

Rough check on route 2's UI: Qt's track is 22.5 units on an app that already exists, and Android is 26–30.

## 9. Decisions for you

1. **Must the web be built from the one base?** Yes means route 2. No or "not yet" means route 1. My advice is to ask them first, since they may have a view on sigman78's console, and to do only route 1 until then.
2. **May a pointer to sigman78's console go in the README?** It is his product. Any contact with him is yours.
3. **Should the Rust hub ever allow a named origin?** I advise no, unless Weeb Labs change the spec.

Corrections for the plan's coordinator, none applied:
- **§5.5 "Browsers block `ws://` from `https://`":** not true of Chromium for private IPs and `.local`. The wall is the hub's `Origin` check.
- **§5.5 "`nusb` has a wasm32 WebUSB backend":** not the pinned 0.2.5, and it is async-only anyway.
- **"Qt for WebAssembly 8–15":** I put device access at 15–25.
- **HB1:** add the `Origin` check.

## 10. Limits of this study

- **No real device, hub or MIDI port.** Every transfer went to `FakeDevice`. WebUSB's own glue (about 30 lines, in the spike) never ran against hardware, and its timing, stalls and disconnects are untested.
- **Safari was not run.** Its support comes from published tables only.
- **Headless browsers:** permission prompts were not exercised, and Qt rendered on SwiftShader.
- **Route 2's RTA, Link and hotplug on wasm** were not run; only the connect, write and notify paths were.
- **Option (a) and (b)** were sized by a regex call graph, not built.
- **The patch set is spike quality**, in a scratch copy: for example `clock` has no tests of its own on wasm beyond the scenario.

**Reproducing.** Spikes are in `~/.cache/dspi-build/work/wp-wb0/`, not committed: `rust/` (patched copy, with `crates/spike-*` and a git history of the patches), `web/` (the harness), `run-browser.sh`, `mixed.mjs`, `mixedff.mjs`, `qtbuild.sh`, `shot.sh`, `shots/`, `images/` (Containerfiles for `localhost/dspi-wb0-{browser,ff,qtwasm}:1`). Downloads came from Fedora's repositories, npm through the lockfiles, `download.qt.io` through aqtinstall 3.3.0, emsdk at the 4.0.7 tag's commit, and rustup.

## Sources

- [JSPI support table, web-features](https://web-platform-dx.github.io/web-features-explorer/features/wasm-jspi); [Firefox intent to ship JSPI](https://groups.google.com/a/mozilla.org/g/dev-platform/c/8cOxM-dWaNs)
- [Intent to Ship: Local network access restrictions for WebSockets](https://groups.google.com/a/chromium.org/g/blink-dev/c/O6GMKt44Ups); [Chrome: Local Network Access](https://developer.chrome.com/docs/modern-web-guidance/guides/security/local-network-access)
- [Qt 6.11 for WebAssembly](https://doc.qt.io/qt-6.11/wasm.html); [Qt 6 for WebAssembly](https://doc.qt.io/qt-6/wasm.html)
- [WebUSB support and vendor positions, web-features](https://web-platform-dx.github.io/web-features-explorer/features/webusb)
- [Chrome: Web MIDI permission prompt](https://developer.chrome.com/blog/web-midi-permission-prompt); [WEBMIDI.js on Safari](https://webmidijs.org/docs/archives/v2)
- [sigman78's wiki: Windows troubleshooting](https://github.com/sigman78/dspi-web-console/wiki/Windows-troubleshooting) (seen through a search summary; the page itself would not load for me)
