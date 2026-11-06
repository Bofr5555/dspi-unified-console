# Every client on the one core: plan

*Branch `plan/clients`, cut from `core-merge` at `6771e16`. Written 2026-11-06.*

`PLAN.md` ends when the core is merged and the Qt app runs on it (Q3, D1). This
plan does the rest: every client moves onto the core, and every client closes
its feature gaps.

**Start:** when `core-merge` reaches `main`. Owner's standing instruction: start
the first wave then, without asking again. **Every WP here depends on that
merge.** WP branches are cut from `main`, not `core-merge`. Hardware, flash,
release and password steps keep their own approvals (section 6).

**Checked against:** `wp/F1` `ccf38e0`, `wp/L1` `af1240d`, `wp/R1` `637e1d9`,
`wp/Q2a` `808ceb6`, `wp/Q2b` `2c4b6bf`, and `wp/C15` `6faf847`. C15 was still
running, so re-check its API before dispatching a WP that uses it.

Sizes are in agent-days, as in `PLAN.md`.

| Track | WPs | Size | Depends on |
|---|---|---|---|
| CG: core gaps | 9 | 6.5 | `main` |
| HB: Rust Link hub | 8 | 12 | `main` |
| QD: Qt desktop | 27 | 23 | `main`, CG, HB for the hub page |
| TM: Terminal | 13 | 7.25 | `main`, CG, HB for `dspi hub` |
| AN: Android | 21 | 24.75–28.75 | `main` (AN1), CG |
| IO: iOS | 22 | 28.5–37 | `main`, CG3, CG4 |
| MC: macOS retirement | 4 | 1.75 | QD gates |
| WB: web | 1 + stubs | 2 (study) | `main` |
| XC: cross-client tests | 5 | 2.75 | hub hardware |
| **Total** | **110** | **about 109–121**, web build not counted | |

The largest unknowns are iOS screen replacement, the Android control-surface
editor and the web build (section 9).

---

## 1. Rules for every WP

The brief and rules are `WP-BRIEF.md`'s, with these changes.

- **Branch:** `wp/<id>` from `main`, in a worktree under `~/prog/dspi-ucwt/`
  (Android: a branch of `dspi-android` in its own worktree; iOS: section 5.5).
- **Process:** Opus agents do WPs in parallel. Fable reviewers check batches, run
  the full CI and do the adversarial checks. Fixups go back to the WP agent. The
  coordinator merges. Agents run scoped checks only. WP branches are deleted
  after merge and QA.
- **Toolchains** in podman or VMs. **No tmpfs.** Scratch goes in
  `~/.cache/dspi-build/work/wp-<id>/`.
- **The macOS app is the feature floor** (owner, 2026-11-06). Section 3 assigns
  every macOS-only feature. Put logic in the core and let the client add only UI.
  A gap found in the core gets its own CG or HB WP.
- **Lost Link connection, every client** (owner's rule):
  - show "connection lost, reconnecting" with an abort/disconnect button;
  - the mirror stays readable;
  - writes fail fast and are never replayed.

  The core provides `LinkPhase::Reconnecting`, `reconnect_attempt`,
  `reconnect_in_ms`, `link_retry_now`, and a `link_close` usable from any thread
  (C15). A client must not add its own retry loop or queue a write.
- **`LinkPhase` wording is one table for all clients**, written by QD10 and
  reused: `NeedsPairing` (by `pin_source`), `NoDevice` ("Connected to hub, no
  DSPi found"), `ChooseDevice`, `Reconnecting`, `Refused`, `Closed`.
- **After a reconnect** the client calls `rta_start` again if its analyser was
  running. A listener callback must not call a `set_*_listener` method: it
  self-deadlocks (review 3-A, on Android's engine).
- **Blocking calls stay off the UI thread**, `poll_hotplug` included (F1 note).
- **Shared client files:** `DSPiBridge.{h,cpp}`, `main.cpp`, `main.qml` and the
  Terminal's `apps/terminal/src/main.rs` command table take additive edits under
  a `// -- WP <id> --` marker. New features go in their own controller class and
  QML file, as `CsController` does.

Mark used below: **[HW]** needs a device, **[OWNER]** needs the owner,
**[COORD]** the coordinator runs it, not an agent.

---

## 2. Corrections to the inputs

These were checked against the code. The plan follows the corrected version.

| # | Input said | Code says | Effect |
|---|---|---|---|
| 1 | `_ref/DSPi-Console` holds the macOS reference | It is stale Weeb Labs `main` `fc686ab`: 20 Swift files, no iOS, no Link. The current reference is `~/.cache/dspi-build/work/parity-ref` (an export of `ext-channel-116`, 110 Swift files, 72 k lines), with history in `work/ext116/src` | Section 3 used parity-ref |
| 2 | Mac is the only hub with `snapshot`, `lock`, `rename`, `web` | The ESP32 bridge advertises all of those too (`hub.c:375`). Only `fw_install` is Mac-only (spec line 388) | Hub comparison, 5.4 |
| 3 | Core gives Qt the Link client | The C ABI has no `dspi_link_*` and no `dspi_open_midi` (Q2a, Q2b, C15 all checked). The only C-side trace is a `link` feature with nothing behind it | **CG1** |
| 4 | Parity 3.0: Link adapter plus mDNS | `PLAN.md` §3 put browsing out of scope and no crate browses. Android (NSD) and iOS (Bonjour) have their own; Qt and Terminal have none | **CG2** |
| 5 | F1 leaves iOS only needing a build | `nusb` is a non-optional dependency of `dspi-core` and `dspi-transport`, and nusb 0.2.9 has no iOS backend (`platform/mod.rs`: Linux, Android, Windows, macOS, wasm32). `firmware.rs` also uses it. The VM's Rust has only `*-apple-darwin` targets. **It will not compile for iOS today** | **CG3**, IO1 |
| 6 | Android Link engine needs `token_for`, `retry_advice`, backoff | C15's supervisor moved these into the core. Android's `LinkSessionEngine` (421 lines, own backoff) would duplicate it, so the WP removes code | AN3 |
| 7 | Android's Kotlin API needs small fixes | All 154 FFI methods exist in the core and all 37 records match, except `LinkHello` and `LinkStatus` gaining fields. But `tools/gen_kotlin_contract.py` parses `rust/dspi-ffi/src/{records,core,lib}.rs`, and the core spreads its methods over many modules behind `cfg_attr`. The generator needs rewriting; it is not a path change. Review 3-A adds: `FfiDspiCore.kt` breaks at `:57` (`initLogging` package), `:393` (exhaustive `when`), `:65-77` and `:383-387` (Link); bindgen must run once per namespace (`ffi/build.gradle.kts:74-77`) | AN1 is 2.5, not 1; **CG9** |
| 8 | Parity B2: Android has input-source names | No such code anywhere in Android, FFI or Kotlin | AN6 |
| 9 | Parity B23: Android latency "not checked" | Absent in FFI, bridge and UI | AN6 |
| 10 | Parity 3.2: 0x82 clock diagnostics is a Qt gap against the reference | The Mac reference has no `CLOCK_DIAG` at all. It is above the floor; the Terminal has it. Still in QD5 as asked | Sizing only |
| 11 | Parity: custom `.uf2` as a gap | The Mac has none either: it reboots to the bootloader and leaves the UF2 to you. Android does better. Above the floor; still in QD11 as asked | Sizing only |
| 12 | Parity 3.0: PICOBOOT writer in core | C11 ships inspection and drive copy only. That meets the floor; Android's PICOBOOT stays in Kotlin | No WP |
| 13 | Merge notes: preset diff keeps the old core's wording, "same as before" | Swift's diff includes preamps and the pair map (`PresetSnapshot.swift:203,260`), which the old core's excludes. Below the floor | **CG7** |
| 14 | Parity: "never a hub" for Android per D1/D5 | D1 is a question and D5 says "client only for v1; hub on demand". Overtaken: owner decided (section 8) | None |
| 15 | Parity 3.3 lists auto-reconnect, Link, MIDI, demo and ext probe as Terminal gaps | R1 does all of them (unmerged until wave 3 merges) | Not in TM |
| 16 | Terminal has full preset parity | `dspi_session::preset_file` has no pair map, while `dspi_core::files` does (`pairMap`). A Terminal `.dspipreset` loses it. There is also no CS JSON in the CLI | TM10 |
| 17 | C15's Link client meets the floor | `dspi-link` has no `fw.install` client, only the capability string | HB4 |
| 18 | Qt bundles "beta4" | Those are untagged `beta3-16-g557bce7` builds (handover) | QD11 |
| 19 | The bridge is pending on the owner | `br0` is up and `macr`'s `vm.toml` has the NIC. Only the patched QEMU and a restart remain | XC1 |
| 20 | Memory: WebUSB needs firmware descriptors | sigman78's MIT console drives the vendor interface over WebUSB on stock firmware (`WebUsbTransport.ts`, read, not run). The firmware's MS OS 2.0 descriptor binds WinUSB on Windows | WB0 |
| 21 | Studies: "macOS and iOS stay Swift" | Superseded by the owner's decisions | MC, IO |

**Safety gap found:** a bundled stock image runs at 1.15 V. Flashing one to a K340
through any updater breaks the "never 1.15 V on the K340" rule, and no updater
checks for it. **CG8** adds the check.

---

## 3. The macOS feature floor

Reviewed from the app itself (menus, settings pages, the iOS target's exclusion
list, `#if os(macOS)` blocks), not only from the audit. **"have"** means present
and checked in that client's code.

Metal renderers and AppKit windows are implementation, not features, so they have
no rows. The Advanced page's onboarding debug section is developer-only.

**Link hub and gateway** (hub on mobile: owner decided, section 8)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M1 | Share devices: server, open or pairing, port | HB1–3, HB5, QD14 | TM11 | no (Play build); sideload later | no |
| M2 | Paired clients: roles, revoke, pairing window | HB1, QD14 | TM11 (CLI) | no | no |
| M3 | Install firmware through a hub (hub side) | HB4 | TM11 | no | no |
| M4 | Web UI served by the hub | HB5 | TM11 | no | no |
| M5 | Bonjour advertisement | HB5 | TM11 | no | no |
| M6 | Menu-bar gateway, hide from Dock | QD12, MC1 | n.a.: no GUI; runs as a service | n.a.: has a service notification | n.a.: no such surface |
| M7 | Open at login, start minimised | QD13 | n.a.: a system service unit | n.a. | n.a. |
| M8 | Keep the machine awake while sharing | QD13 | TM11 | n.a.: no hub | n.a.: no hub |

**Link client**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M9 | Find hubs, add by address | CG2, QD9 | CG2, TM9 | have; AN4 re-resolves | have; kept in IO6 |
| M10 | One merged device list | QD9 | have (R1) | AN7 | have |
| M11 | Non-blocking PIN prompt, clear errors | QD9 | have (R1) | AN5 | have; re-pointed in IO6 |
| M12 | Lost-connection banner (owner's rule) | QD10 | TM8 | AN5 | IO7 |
| M13 | Analyser "following another console" | CG5, QD19 | CG5, TM5 | CG5, AN17 | have; re-pointed in IO14 |
| M14 | Install firmware through a hub (client side) | HB4, QD11 | HB4, TM3 | later | later: the only firmware route iOS could have |

**App and firmware**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M15 | App self-update | QD16 | n.a.: package manager | n.a.: Play | n.a.: App Store |
| M16 | Firmware wizard: bundled final image, verify, downgrade | QD11 | TM3 (non-graphical) | have; AN18 refreshes the image | n.a.: no USB bootloader (owner); IO18 hides the dead window |

**Onboarding** (Terminal: owner decided, skips graphical onboarding)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M17 | First-run wizard | have | skip | AN19 | have; IO18 drops the firmware step |
| M18 | Basics tour with coach marks, replay | QD15 | skip | AN19 | IO21 |
| M19 | Just-in-time hints | QD15 | skip | AN19 | IO21 |
| M20 | What's New | have | skip | AN19 | have |

**Graph and analysis** (Terminal: owner decided, skips on-graph editing; the `;` bar is the equivalent)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M21 | HUD chip, marquee, pull from curve, shape menu | have | skip | have | IO19 |
| M22 | Analyser bar strip | have | TM6 | AN17 | have |
| M23 | Analyser overlay on the graph | have | TM5 | have | have |
| M24 | About 30 shortcuts, Space for the generator, graph keys | QD17 | have (own mnemonics) | AN18 (hardware keyboard) | IO20 (iPad keyboard) |
| M25 | Host audio format (idle channel count) | QD18 | have (`INPUT_FORMAT`) | AN18 | IO20 |
| M26 | C0 and C1 CPU bars | QD5 | have | have | have |
| M27 | Pop-out tool windows | have | have (graph pop-out) | n.a.: one window; routes | n.a.: one sheet at a time |

**Files**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M28 | Filter import with channel pickers, export | have | have | have | IO17 |
| M29 | `.dspipreset` with apply options and report | have | have; TM10 adds the pair map | have | IO17 |
| M30 | Save Master Volume, Save Output Configuration | have | TM10 checks and adds | have | have |
| M31 | AutoEQ favourites | have | have | have | IO18 |
| M32 | AutoEQ update, rebuild from GitHub, import | have | CG6, TM12 | CG6, AN18 | CG6, IO18 |

**Device settings**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M33 | Input-source names | QD4 | have | AN6 | have |
| M34 | Device name | QD4 | have | AN6 | have |
| M35 | Output latency | QD5 | have | AN6 | have |
| M36 | Core-voltage status | QD3 | TM2 | AN6 | have |
| M37 | Pair-map editor, K340 preset, in files and diff | QD2, CG7 | TM1, TM10 | have; CG7 | have |
| M38 | Tube Basic and Advanced | have | TM7 | have | have |
| M39 | Demo device (dev flag on Mac) | QD7 | have (R1) | have | have |
| M40 | USB MIDI (dev flag on Mac) | CG1, as a dev option | have (R1) | n.a.: vendor USB works | have; re-pointed in IO5 |

Every row is checked against one more source at its WP: the Mac file it comes from
is that WP's reference.

---

## 4. Core gaps

### 4.1 CG: small core WPs

**CG1: C ABI for Link and MIDI.** Size 1.
- **Owns:** `crates/dspi-capi/src/fns_link.rs`.
- **Implements:** every `link_*` function, the listener and the free functions
  over C, as Q2b does; `dspi_open_midi` behind `midir`.
- **Done when:** `ci/check.sh --crate dspi-capi` passes; `link_close` works from a
  listener thread; the null and short-buffer rules of §1.9 hold.
- **Deps:** `main`.

**CG2: Find hubs in the core.** Size 1.
- **Owns:** `crates/dspi-link/src/discovery.rs`, `crates/dspi-core/src/link_browse.rs`,
  `crates/dspi-capi/src/fns_browse.rs`.
- **Implements:** browse `_dspi._tcp` through `mdns-sd` behind a feature, off for
  mobile; `link_browse_start`, `link_browse_stop`, results listener; resolve by
  hub id for re-resolving (finding F5). Its responder half is what HB5 reuses.
- **Done when:** tests pass against a fake advertiser; the mobile libraries do not
  link it; the dependency reason is stated.
- **Deps:** `main`.

**CG3: The core without USB (iOS).** Size 1.
- **Owns:** `usb` feature (default on) in `crates/dspi-transport/Cargo.toml` and
  `crates/dspi-core/Cargo.toml`; its gates in `device.rs`, `firmware.rs` and
  `crates/dspi-transport/src/usb.rs`; a new `ios-check` step in `ci/check.sh`.
- **Done when:** `dspi-ffi` passes `cargo check` for `aarch64-apple-ios`, with the
  `usb` feature off; default builds are unchanged.
- **Deps:** `main`.

**CG4: Raw commands for bindings.** Size 0.5.
- **Owns:** `crates/dspi-core/src/raw.rs`.
- **Implements:** `raw_get` and `raw_set` on `DspiCore` (exported to UniFFI), so
  Swift code that has not moved yet shares the core's one link (IO4). `raw_set`
  goes through `Writable`, and a read-only connection refuses it. Adds a §1.2 line.
- **Done when:** tests cover read-only refusal and no use while a bulk session runs.
- **Deps:** `main`.

**CG5: Analyser following state.** Size 0.5.
- **Owns:** `crates/dspi-core/src/rta.rs`.
- **Implements:** report whether the analyser is driving or following another
  session, with the origin, from `RTA_CONFIG`, like `case following(origin:)` in
  `SpectrumAnalyser.swift:84`.
- **Done when:** the fake hub test shows a config from another session switches
  state, and our own does not.
- **Deps:** `main`.

**CG6: AutoEQ update with a host-supplied fetch.** Size 1.
- **Owns:** `crates/dspi-session/src/autoeq_update.rs`,
  `crates/dspi-core/src/autoeq.rs`.
- **Implements:** update, rebuild from GitHub and import, using a fetch callback
  (reference: `AutoEQManager.swift:248,331,500`). No HTTP client in the core.
- **Done when:** tests pass with a scripted fetch, including rate-limit and partial
  failures; exported to C and UniFFI.
- **Deps:** `main`.

**CG7: Preset diff at the floor.** Size 0.5.
- **Owns:** the diff section of `crates/dspi-core/src/presets.rs`.
- **Implements:** Swift's table, `PresetSnapshot.swift`: preamps and pair map added;
  user volume and DAC mute as Swift treats them.
- **Done when:** a test covers each row against Swift's wording.
- **Deps:** `main`.

**CG8: Firmware guard.** Size 0.5.
- **Owns:** `crates/dspi-core/src/firmware_guard.rs`.
- **Implements:** before any flash, compare the connected board's
  `EXT_CORE_VREG` state with the image, and return a warning that needs explicit
  confirmation when a raised-voltage board is about to get an image not known to
  keep it. Every updater calls it (QD11, TM3, AN18).
- **Done when:** tests cover K340-like, stock and no-extension boards.
- **Deps:** `main`.

**CG9: Compile the generated bindings in CI.** Size 0.5.
- **Owns:** new `kotlin` and `swift` steps in `ci/check.sh`, the CI image's
  Containerfile, `crates/dspi-ffi/tests/bindgen_wp_f1.rs`.
- **Implements:** compile F1's generated Kotlin with `kotlinc` in the container. Compile
  the Swift with an official Swift toolchain in the container if it builds there,
  otherwise as an opt-in step in the VM. Downloads pinned and verified. Neither is in the
  gate today, and a Kotlin compile would have caught the breaks in AN1.
- **Done when:** both steps pass, and a deliberately broken namespace fails them.
- **Deps:** `main`.

### 4.2 HB: the Rust Link hub

The Qt app needs it regardless, under the floor rule and for retiring Swift. The
crate has no OS or UI code, so a later Android sideload build needs only a binding.
No work is added for that now.

**Reference:** Swift hub files, 3 984 lines (`LinkServer`, `LinkHub`,
`LinkSessionHandler`, `CommandRouter`, `AuthStore`, `PollScheduler`, `NotifyRelay`,
`SnapshotCache`, `LinkPolicy`, `LinkService`, `DeviceRegistry`, `HubTransport`,
`USBHubDevice`, `HubDevice`, `PairingPrompt`). Also `bridge/esp32/components/hub`,
`hub_auth`, and the spec. `dspi-link`'s `fakehub.rs` (1 251 lines) already has
hello, tokens, roles, snapshot, polls, locks and CMD ordering at test grade.
Earlier studies said 10–15 days.

**The hard part is device ownership.** Today one `DspiCore` owns one connection. A
hub owns all local devices and serves many sessions, and the app itself must be one
of them. Design: the hub owns the transports, and the app opens an in-process
transport through `open_transport`, with its own session id, so the echo rule works
unchanged. HB0 proves this first.

**HB0: Design and skeleton.** Size 1.
- **Owns:** `crates/dspi-hub/` (new), `docs/core-merge/HUB.md`.
- **Implements:** threads, ownership, the in-process transport, interfaces for
  HB1–HB6, `todo!` markers as in `PLAN.md`.
- **Done when:** the app-through-hub path passes a demo-device test; a **Fable
  review of the design passes before HB1–HB3 start**.
- **Deps:** `main`.

**HB1: Server, handshake, auth, roles.** Size 2.
- **Owns:** `crates/dspi-hub/src/{server,auth,store}.rs`.
- **Implements:** the WebSocket server with hello, `/dspi/v1/info`, session limits,
  open and PIN modes, window PIN, tokens, rate limits, revoke; close codes 4001,
  4002, 4003; ping every 10 s; 30 s dead-peer limit.
- **Done when:** our L1 client and the fixtures pass, including wrong PIN answered
  with `unauthenticated`, as real hubs do.
- **Deps:** HB0.

**HB2: Commands and notifications.** Size 2.5.
- **Owns:** `crates/dspi-hub/src/{router,policy,lock,relay}.rs`.
- **Implements:** per-device ordered CMD with `max_inflight`; `policy/commands.json`
  by role; device locks; NOTIFY with `origin` per session; resync. Decide and
  document how `0x00` is classed (the upstream policy lacks it, so it is admin-only;
  Weeb Labs has been asked).
- **Done when:** fake-device tests cover two clients seeing each other's writes, an
  own echo dropped, and a lock held across a vanished client released by timeout.
- **Deps:** HB0.

**HB3: Registry, snapshot, polls, rename.** Size 2.
- **Owns:** `crates/dspi-hub/src/{registry,snapshot,polls}.rs`.
- **Implements:** device list with hot-plug; `device.snapshot`; `poll.subscribe`
  budget; `device.rename`.
- **Done when:** C15's meter feed works against it; unplug and replug tests pass.
- **Deps:** HB0.

**HB4: Firmware install through a hub.** Size 1.
- **Owns:** `crates/dspi-hub/src/fw.rs`, and `crates/dspi-link/src/fw_client.rs`
  (client side, which does not exist yet).
- **Implements:** `fw.install`, binary upload, progress, done, using C11's copy and
  CG8's guard, both directions. The client API goes into `dspi-core`.
- **Done when:** demo-device and fake-bootloader tests pass, including a bad hash and
  a lost client. **No real flash in this WP.**
- **Deps:** HB1, HB2, CG8.

**HB5: Advertising and web UI.** Size 1.
- **Owns:** `crates/dspi-hub/src/{advertise,web}.rs`, bundled assets.
- **Implements:** Bonjour through CG2's library; serving the MIT DSPi-Link web bundle
  at a pinned tag; the `web` capability.
- **Done when:** the bundle's version and hash are pinned; a browser test connects to
  the demo device through it.
- **Deps:** HB1, CG2.

**HB6: C ABI for the hub.** Size 1.
- **Owns:** `crates/dspi-capi/src/fns_hub.rs`.
- **Implements:** start and stop, settings, client list, role, revoke, pairing
  window, status listener.
- **Done when:** Qt offscreen test starts a demo hub and the Terminal connects to it.
- **Deps:** HB1–HB3.

**HB7: Conformance.** Size 1.5.
- **Owns:** `crates/dspi-hub/tests/conformance/`.
- **Implements:** our client, the TypeScript client and the Swift client (VM) against
  it; faults from L1's fake-hub suite; the fixtures.
- **Done when:** all three pass, with differences to the Mac hub recorded. **[HW]**
  Swift run needs `macr` (XC1).
- **Deps:** HB1–HB6.

---

## 5. Client tracks

### 5.1 Qt desktop (`apps/desktop/qt`)

Q3 makes it run on the core. Each WP below owns a new controller and QML file where
possible, and takes additive edits to the shared files.

**QD0: C ABI pitfalls found in review 3-A.** Size 0.25. Do it inside Q3 if Q3 has not
closed. **Owns:** `src/ConfigFiles.cpp`, the block at `src/DSPiBridge.cpp:1409-1433`.
**Implements:** `ConfigFiles.cpp:17-21` retries an import after a short buffer, which applies
it twice: a reply too long is now a failure. `DSPiBridge.cpp:1409-1433` holds `state()` across
writes: re-read it after each. **Done when:** a test shows one import applies once; the
`state()` pointer is not used across a write. **Deps:** `main`.

**QD1: `CsController` surfaces `error`.** Size 0.25. **Owns:** `src/CsController.cpp`.
**Done when:** a refused `cs_apply` shows its `error` text; the stricter number
parsing is reported, not silent. **Deps:** `main`.

**QD2: Pair-map editor and dimmed rows.** Size 1.
- **Owns:** `qml/CrossfeedWindow.qml`, `src/ExtController.{h,cpp}`.
- **Implements:** Default, K340 and Custom pairs (reference
  `CrossfeedPairMapView.swift`), hidden on a stall. The dimmed rows show the preset's
  own values from the table the window already holds, not the custom ones.
- **Done when:** demo-device test edits a map and reads it back; an invalid pair is
  refused. **Deps:** `main`.

**QD3: Core-voltage status.** Size 0.5. **Owns:** `qml/settings/pages/` overview
status file. **Implements:** status only, as on the Mac (OD7). **Done when:**
it shows live mV and hang count on the demo device, and is hidden on stock firmware.
**Deps:** QD2 (`ExtController`).

**QD4: Input-source and device names.** Size 0.5. **Owns:** the inputs settings page's
name rows, the device-name row. **Done when:** edits round-trip on the demo device,
and another client's `NOTIFY_NAMES` updates them. **Deps:** `main`.

**QD5: Output latency, clock diagnostics, C0/C1 bars.** Size 0.5.
**Owns:** the latency settings row, `src/StatsController.cpp`, `qml/StatsWindow.qml`,
`components/CpuSection.qml`. **Done when:** the option list comes from the device, a
stall hides the section, and both CPU bars show. **Deps:** `main`.

**QD6: Read-only mode.** Size 1.
- **Owns:** a new `qml/components/ReadOnlyBanner.qml`, a `writable` property on the
  bridge, and the compat branch of `main.qml`.
- **Implements:** older and newer firmware show all settings, disable every control,
  keep scrolling and reading, and offer the update. Wording from
  `WIRE-FORMAT-COMPAT.md` §4.4.
- **Done when:** V17, V26, V31 and V33 fixtures run offscreen; no setter is called in
  read-only mode (test); clip clear and bootloader still work. **Deps:** `main`.

**QD7: Demo mode.** Size 0.25. **Owns:** the demo entry in the device list and Q3's
flag. **Done when:** demo is selectable with no hardware and is clearly labelled.
**Deps:** `main`.

**QD8: Link controller and token store.** Size 1.
- **Owns:** `src/LinkController.{h,cpp}`, `src/TokenStore.{h,cpp}`.
- **Implements:** wraps CG1, off the UI thread. Tokens in Keychain, Credential Manager
  or Secret Service, falling back to a 0600 file with a warning. Tokens are deleted
  only on `token_refused`.
- **Done when:** fake-hub test covers pairing, a token reused after restart, and
  `rate_limited` keeping the token. **Deps:** CG1.

**QD9: Device list, discovery, PIN prompt.** Size 1.5.
- **Owns:** `qml/DevicesPopup.qml` (new), `qml/PairingDialog.qml` (new).
- **Implements:** one merged list of USB, demo and hubs (browsed and by address);
  non-blocking PIN prompt, worded by `pin_source`; every `LinkPhase`, including "no
  DSPi found".
- **Done when:** runs offscreen against the fake hub. Also needs the macOS plist from
  MC1. **Deps:** QD8, CG2.

**QD10: Lost-connection banner.** Size 0.5. **Owns:** `qml/components/LinkBanner.qml`,
and the `LinkPhase` wording table for other tracks. **Done when:** the owner's rule
holds in a fake-hub fault test: banner appears at once, the mirror stays readable, a
write fails and is not sent later, abort works during a dial. **Deps:** QD8.

**QD11: Firmware updater.** Size 1.
- **Owns:** `src/FirmwareUpdater.{h,cpp}`, `qml/FirmwareUpdateWindow.qml`, `firmware/`.
- **Implements:** images per OD4; custom `.uf2` with `uf2_inspect`; CG8's guard;
  `firmware_version_matches` as the confirm step; install through a hub when offered.
- **Done when:** offscreen tests cover a wrong-chip file, the guard and a fake
  bootloader drive. **No real flash here; QD24–26 do that. Deps:** CG8, HB4 for the
  hub path (that part may land later).

**QD12: Tray and menu-bar mode.** Size 1. **Owns:** `src/TrayController.{h,cpp}`.
**Implements:** `QSystemTrayIcon`; close to tray; start minimised; quick volume;
status; the hub keeps serving (reference `MenuBarMode.swift`). **Done when:** Linux,
Windows and macOS each behave, with fallback when a desktop has no tray. **Deps:** `main`.

**QD13: Open at login and keep awake.** Size 0.5. **Owns:** `src/SystemIntegration.{h,cpp}`.
**Implements:** XDG autostart, Windows Run key, macOS login item; inhibit sleep per
OS while sharing. **Done when:** each OS's mechanism is checked in its VM. **Deps:** QD12.

**QD14: Networking page.** Size 1.5. **Owns:** `qml/settings/pages/` networking file,
`src/HubController.{h,cpp}`. **Implements:** `NetworkingSettingsView.swift`: share
toggle, access, name, port, paired clients, roles, revoke, pairing code with expiry,
other hubs, background options. Warns that a PIN crosses the network in plain text
without TLS, as the spec says. **Done when:** offscreen on a demo hub, and a real
client pairs through it. **Deps:** HB6, QD8.

**QD15: Coach marks and hints.** Size 1.5. **Owns:** `qml/onboarding/` (new),
`assets/onboarding/catalogue.json` (shared with AN19). **Implements:** the catalogue,
basics tour, replay, just-in-time hints; ordering rules from
`OnboardingCoordinator.swift`. **Done when:** a test tour completes offscreen, and it
does not reappear. **Deps:** `main`.

**QD16: App self-update.** Size 1. **Owns:** `src/AppUpdater.{h,cpp}`. **Implements:**
check our releases, compare versions, show notes, open the page. Installing in place
only after OD8. **Done when:** tested on a local feed; off by default in distro
packages. **Deps:** `main`.

**QD17: Shortcuts.** Size 0.5. **Owns:** shortcut definitions in `qml/main.qml`
under its marker, and `qml/SignalGeneratorWindow.qml`. **Done when:** every Mac
shortcut has an equivalent, Ctrl on Linux and Windows, Cmd on macOS. **Deps:** `main`.

**QD18: Host audio format.** Size 1. **Owns:** `src/HostAudioFormat*.{h,cpp,mm}`.
**Implements:** the idle channel count per OS: CoreAudio, WASAPI, PipeWire or ALSA.
Reference `HostAudioFormatMonitor.swift`. **Done when:** a fallback to the device's
report exists; each OS is checked. **Deps:** `main`.

**QD19: Analyser over Link.** Size 0.25. **Owns:** `qml/SpectrumAnalyserWindow.qml`.
**Done when:** a following state shows, driven by CG5. **Deps:** CG5, QD8.

**QD20: Check before Windows C# retires.** Size 0.5. **Owns:** a report in
`docs/core-merge/WINDOWS-RETIREMENT.md`. Audit's items:
- undo and redo: Qt has none. C# has IO undo, which is OD5;
- 0xFC/0xFD pair mask (`DspDevice.cs:288`): Qt has `crossfeedOutputPairMask`;
  compare behaviour;
- settings depth, the bulk-endpoint debug window, and 12 PEQ bands against Qt's 10.

**Done when:** each item has an outcome and any gap has a WP. **Deps:** `main`.

**QD21: Packaging.** Size 1. **Owns:** `apps/desktop/packaging/**`. **Implements:**
Linux packages on Qt 6 (raises the glibc floor), AUR source URL, QML precompile, an
installer check that the app is not running. **Done when:** artefacts build; Windows
and macOS still pass. **Deps:** Q3.

**QD22: Accessibility.** Size 2. **Runs alone in the last wave**: it touches most of
the 131 QML files. **Owns:** all of `apps/desktop/qt/qml`. **Done when:** `Accessible.*`
on all custom controls in `components/` and the windows; a keyboard-only and screen
reader pass on Linux. **Deps:** every other QD UI WP.

**QD23: Localisation.** Size 2. **Runs after QD22**, for the same reason. **Owns:**
all QML, `translations/`. **Implements:** `qsTr` throughout; the build step for `.ts`;
languages per OD6. **Done when:** a pseudo-locale run shows no untranslated string
and no clipped layout in the main windows. **Deps:** QD22.

**QD24: Hardware pass, Linux.** Size 0.5. **[HW][COORD]**. **QD25: Windows.** Size 1.
**[HW][COORD]**: stock USB pass-through per `~/vm/windows/USB-PASSTHROUGH.md`, with the
RP2 bootloader device attached too for firmware. **QD26: macOS.** Size 0.5. **[HW][COORD]**.
- **Owns:** a checklist in `docs/core-merge/HW-QT.md`.
- **Done when:** connect, read, write, notifications, replug, firmware path and Link
  (Mac hub or bridge) pass, on the K340 under section 6's rules, with backups first.
- **Deps:** QD0–QD14; QD26 also XC1.

### 5.2 Terminal (`apps/terminal`, `crates/dspi-tui`, `crates/dspi-cmd`)

Owner decided: it skips what needs a graphical UI (on-graph editing, onboarding) and
covers the non-graphical equivalents, including JSON import and export.

**TM1: Pair-map row.** Size 0.5. **Owns:** `screens/crossfeed.rs`. **Done when:** a
Default, K340 and Custom row works on the demo device; the row is hidden on stock
firmware. Do not confuse it with the existing `Pairs` mask row. **Deps:** `main`.

**TM2: Core-voltage screen.** Size 0.5. **Owns:** `settings/overview.rs`. **Implements:**
status in the TUI; writes stay in the CLI (OD7). **Done when:** demo-device render test.
**Deps:** `main`.

**TM3: Firmware flashing.** Size 1. **Owns:** a new `apps/terminal/src/flash.rs`, and
`dspi-core` as a dependency of the Terminal for `firmware.rs` only.
**Implements:** `dspi flash [FILE]` with `uf2_inspect`, CG8's guard, and confirmation.
**Done when:** fake bootloader drive test passes; refuses without `--force` in a script.
**Deps:** CG8.

**TM4: Read-only fallback.** Size 1. **Owns:** connect path in `shell/connect.rs`,
`apps/terminal/src/main.rs`. **Implements:** `probe_any`, the display in all screens,
and writes refused in one place; `--json` reports `read_only`. Replaces the refusal.
**Done when:** V17, V31 and V33 fixtures render; no write leaves (mock transport).
**Deps:** `main`.

**TM5: Analyser overlay.** Size 1. **Owns:** `graph.rs`, `screens/spectrum.rs`.
**Done when:** curves on the response graph via the engine's subscription (the file says
it allows this), and the following state shows. **Deps:** CG5.

**TM6: Bar strip.** Size 0.5. **Owns:** a new `widgets/bar_strip.rs`. **Done when:** third-octave
strip renders in both ASCII and braille modes. **Deps:** TM5.

**TM7: Tube Basic/Advanced.** Size 0.25. **Owns:** `screens/tube.rs`. **Done when:** a mode
toggle with Swift's row set. **Deps:** `main`.

**TM8: Lost-link status line.** Size 0.25. **Owns:** the status line in `shell/mod.rs`.
**Done when:** the owner's rule holds in a fake-hub test: status line, abort key, a
failed write is not replayed. **Deps:** `main`.

**TM9: Find hubs.** Size 0.5. **Owns:** `dspi link scan` in `main.rs`, picker in
`shell/connect.rs`. **Done when:** a test with a fake advertiser; `--json` output. **Deps:** CG2.

**TM10: JSON import and export gaps.** Size 0.5.
- **Owns:** `crates/dspi-session/src/preset_file.rs`, a `cs` command in `main.rs`.
- **Implements:** what exists is `.dspipreset`, filter files, `dump --json`, `--dry-run`.
  Missing and added: the pair map in `.dspipreset`, matching `dspi_core::files`;
  `dspi cs export|import` as JSON; Save Master Volume and Save Output Configuration, if
  the check finds them absent from the CLI.
- **Done when:** round-trip with Qt's and the Mac's document in fixtures. **Deps:** `main`.

**TM11: `dspi hub`.** Size 0.5. **Owns:** `apps/terminal/src/hub.rs`. **Implements:** a headless
hub with pairing, clients and roles as CLI commands, and a systemd unit example.
**Done when:** a Link client connects. It is not required by the owner's Qt decision, but
a headless gateway is what a Terminal hub is for. **Deps:** HB6.

**TM12: AutoEQ update.** Size 0.25. **Owns:** `screens/autoeq.rs`. **Implements:** CG6,
removing the `curl` call. **Deps:** CG6.

**TM13: K340 hardware pass.** Size 0.5. **[HW][COORD]**. **Owns:** a checklist,
`docs/core-merge/HW-TERMINAL.md`. **Done when:** USB replug and real MIDI ports end to end
(R1's open item) pass. **Deps:** TM1, TM2, TM4.

### 5.3 Android (`dspi-android`, branch `feature/new-core`)

The swap is well contained: UI talks to the `core-api` interface, and only `ffi` sees
UniFFI. Android stays unreleased until AN14, by owner decision.

**AN1: New crates in place of `dspi-ffi` and the patched core.** Size 2.5.
- **Owns:** `rust/`, `third_party/`, `tools/{stage-core.sh,gen_kotlin_contract.py,opcode_coverage.py,REFERENCE.toml}`,
  `ffi/**`, `core-api/**`, `NOTICE.md`, generated licences.
- **Implements:**
  - depend on `dspi-ffi` from the unified repo (OD1 sets how);
  - rewrite the generator to read the core's modules (`DspiConstants.kt`, `CoreTypes.kt`,
    `DspiCoreApi.kt`, `FfiDspiCore.kt`), keeping its `--check`;
  - bindgen once per namespace (`ffi/build.gradle.kts:74-77`);
  - `FfiDspiCore.kt:57`: `init_logging` from package `se.bofr.dspi.ffi.uniffi.logging`
    (also `DspiApplication.kt`);
  - `FfiDspiCore.kt:393`: `ReadOnly` and `Io` in the exhaustive `when`, and in `core-api`'s
    `DspiException.kt`;
  - `FfiDspiCore.kt:65-77, 383-387`: Link methods as C15 leaves them; `LinkHello` and
    `LinkStatus` fields; `on_link_status_changed` and `on_link_token_issued` present
    (behaviour in AN3);
  - `NOTIFY_STALE` in `DspiConstants`; `with_foreign` listeners;
    `MidiInputHandle.close_port`; `MidiSender` unused here;
  - drop patches 0000–0004 and the `wip/wire-compat` branch.
- **Done when:** `tools/ci.sh all` passes and all Kotlin module tests pass unchanged, on 3
  ABIs. Records the new library's size. **No phone yet. Deps:** `main`.

**AN2: Read-only on older firmware.** Size 0.5. **Owns:** `feature-home/.../HomeModel.kt`,
`README.md`. **Implements:** V17–V31 now decode (free with the core). `WriteGuardedCore`
stays. Fixes the README claim (handover). **Done when:** fixture tests; banner text
per `WIRE-FORMAT-COMPAT.md` §4.4. **Deps:** AN1.

**AN3: Link engine on the core's supervisor.** Size 1.
- **Owns:** `link/.../LinkSessionEngine.kt`, `link/.../LinkCoreOps.kt`.
- **Implements:**
  - delete the engine's own reconnect, backoff and token deletion: they would fight C15's
    supervisor;
  - follow `link_status`, and implement `on_link_status_changed` and `on_link_token_issued`;
  - delete a token only when `token_refused` is set;
  - remove `setListener` from inside listener callbacks (`LinkSessionEngine.kt:164, 178,
    214`): it self-deadlocks;
  - call `rta_start` again after a reconnect;
  - dial off the serial FFI queue. The token for a known hub is looked up after the hello
    (re-check how C15 exposes this when it lands).
- **Done when:** `link` tests rewritten against the fake hub with L1's fault cases;
  `rate_limited` keeps the token. **Deps:** AN1.

**AN4: Network handling.** Size 1. **Owns:** `link/.../{LinkNetworkMonitor,NsdHubBrowser,LinkSettings}.kt`.
**Implements:** `onLost` and network change call `link_retry_now`; browse while
reconnecting and re-resolve by hub id via `link_set_url` (F5); test the F10 items: `WifiLock`,
binding sockets to Wi-Fi. **Done when:** unit tests with a fake monitor; F10 items
recorded as used or rejected. **Deps:** AN3.

**AN5: Link UI.** Size 1. **Owns:** `ui-common/.../LinkBanner.kt` (new), `app/.../{LinkDialogs,AppNetworkingController,ConnectionStrip}.kt`.
**Implements:** owner's banner on every screen; wrong-PIN text by `pin_source`; "no DSPi
found" instead of "Connecting" (F9), with QD10's table. **Done when:** Compose tests per
`LinkPhase`. **Deps:** AN3.

**AN6: Settings rows.** Size 1. **Owns:** `feature-settings/.../{GeneralSettingsTab,PinOverviewTab}.kt`,
`feature-home/.../` source picker. **Implements:** core voltage status, device name (also
`device.rename` on a hub), input-source names, output latency. **Done when:** demo tests
round-trip; hidden when the firmware lacks them. **Deps:** AN1.

**AN7: One device list.** Size 1. **Owns:** `app/.../{AppMode,EntryNavigator,UsbStartGate}.kt`.
**Done when:** USB, Link and demo appear in one list, as on the Mac. Demo stays
reachable with a DSPi attached. **Deps:** AN3.

**AN8: Control-surface spike.** Size 0.5. **Owns:** a report. **Implements:** decide the phone
layout for the editor against Swift's 5 270-line tab and Qt's `settings/cs` (about 4 300
lines of QML and the core's `cs_model`). Sets AN9–AN12's sizes and splits. **Done when:** a
reviewed screen map exists. **Deps:** AN1.

**AN9: CS bindings.** Size 2–4. **AN10: IR learn.** Size 1. **AN11: Display pages.** Size 1–2.
**AN12: Groups, macros, aux.** Size 1.5–2.5.
- **Owns:** one new file set each under `feature-settings/.../cs/`.
- **Implements:** the core's `cs_snapshot`, `cs_apply` and `cs_pin_uses`.
- **Done when:** runs on the demo device (`FakeDevice`'s CS support looks minimal: check, and extend it);
  IR learn and display need **[HW]**; an `error` reply is shown.
- **Deps:** AN8.

**AN13: Phone test.** Size 1. **[HW][OWNER][COORD]**. **Owns:** a checklist,
`docs/HIL-NEW-CORE.md`. **Requires** the owner's adb port. **Done when:** the earlier HIL set
passes on the nusb `from_fd` path (unproven on hardware): USB, notifications, old firmware,
Link with a Mac hub, 45 s background, no secrets in logcat. Side by side with
`applicationIdSuffix ".hil"`: never uninstall the Play build. **Deps:** AN1–AN6.

**AN14: First new-core update.** Size 0.5. **[OWNER]**. **Owns:** version, release notes,
store text. **Implements:** the unreleased crossfeed pair fix ships here; the README;
versionCode bump; Link and Data safety text re-checked. **Done when:** an AAB is ready for
the owner to upload. **Deps:** AN13, gate R-And.

**AN15: R8.** Size 1. **Owns:** `app/build.gradle.kts`, keep rules. **Implements:** JNA and UniFFI
keep rules, `isMinifyEnabled`. **Done when:** a release build passes a phone test
**[HW]**. Play deadline February 2027. Own release. **Deps:** AN14.

**AN16: Edge-to-edge.** Size 1. **Owns:** `app/.../MainActivity.kt`, inset handling.
**Done when:** every route is checked on the phone **[HW]**. Own release. **Deps:** AN14.

**AN17: Analyser bar strip and following.** Size 0.75. **Owns:** `feature-analysis/` new
bar-strip file, overlay state. **Deps:** AN1, CG5.

**AN18: Small floor items.** Size 1. **Owns:** `feature-firmware/` images, AutoEQ update UI,
a host-format reader, `feature-tools/` generator lifetime, graph keyboard map. **Implements:**
images per OD4 with CG8's guard; CG6's update with a Kotlin fetch; `AudioManager` channel count;
signal generator keeps running off-screen, as on the Mac; hardware-keyboard shortcuts. **Deps:**
AN1, CG6, CG8.

**AN19: Onboarding.** Size 1.5. **Owns:** a new `feature-onboarding/` module. **Implements:**
first-run, coach marks, What's New, from QD15's catalogue. **Deps:** QD15.

**AN20: Accessibility.** Size 2. **Owns:** semantics across `feature-*`. TalkBack for
graph, matrix and meters. **AN21: Localisation.** Size 2. **Owns:** `strings.xml` across modules.
Both run alone in the last wave. **Deps:** all other AN UI WPs; OD6.

### 5.4 macOS Swift Console: retire or generate

Owner's order of preference: (A) retire it, moving users to Qt on macOS; otherwise (B)
keep a Swift UI on parts generated from the core. **It is Weeb Labs' product; we control
only our fork. "Retire" means our line, unless Weeb Labs agrees.** Their app stays
available to their users whatever we do.

**What Qt must reach:** section 3's Qt column. All of it is in scope already, because
the owner made the Mac the floor for every client, and the hub on Qt is in scope
regardless. So those items count for neither route.

| | A: retire, Qt on Mac | B: Swift UI on generated parts |
|---|---|---|
| Work that exists only for this route | MC1–MC4: **1.75** | B1 UniFFI slices for macOS 0.5; B2 USB through `nusb` on the Mac path 0.5–1; B3 Swift hub on CG4 raw commands 1–2; B4 tests 1: **3–4.5**, *after* iOS (IO4, IO8–IO14) |
| If iOS has not moved yet | no change | adds **13.5–22** of iOS's replacement work |
| Cheaper variant | none | Generated tables only (`Constants.swift` and layouts from `dspi-proto`): **2–3**. Removes hand-copied numbers, not the second protocol stack |
| Free from the iOS track | nothing needed | UniFFI build, the `DeviceTransport` seam, every moved model group, tests, because the sources are shared |
| What remains to maintain | Qt only on Mac | Mac-only Swift: hub, menu bar, updater, Metal renderers, HUD, on top of what iOS keeps |
| What the user loses | SwiftUI and Metal fidelity, native feel | Nothing |
| Hub | Rust (HB, in scope) | Swift, as today; but HB is still in scope for Qt, so two hubs stay in play |
| Blocks | Developer ID and an Apple silicon run (owner) | None new |

**Honest reading:** iOS keeps most of the shared Swift alive either way, so retirement saves
only the Mac-only part: about 4 k lines of hub plus menu-bar, updater and Metal code. The floor
rule still decides it, since it means Qt reaches the Mac's features anyway. After that, A costs
less than B and drops a codebase. **Recommendation: A** (OD2). If the owner chooses A, the iOS
branch need not keep the macOS target building after MC4, which removes some of iOS's
replacement risk.

**MC1: macOS integration.** Size 0.5. **Owns:** `apps/desktop/packaging/macos/`,
`qt/CMakeLists.txt` bundle part. **Implements:** `NSLocalNetworkUsageDescription`,
`NSBonjourServices`, "Settings…" naming (`LINUX-CONSOLE-PORTABILITY.md` §9), accessory mode
for QD12. **Done when:** Local Network prompt appears in `macr`, and browsing works. **Deps:** QD9.

**MC2: Signing and notarisation.** Size 0.5. **[OWNER]**: Apple Developer ID, password. **Owns:**
the signing step in the build script, using hardened runtime; no App Store. **Done when:**
a notarised `.dmg` opens with no warning on a clean Mac account. **Deps:** QD21, OD2.

**MC3: Apple silicon run.** Size 0.5. **[HW][OWNER]**: the universal app's arm64 half has never
run, and the VM and known spare Macs are Intel. **Done when:** QD26's checklist passes. **Deps:** MC2.

**MC4: Retirement.** Size 0.25. **Owns:** `docs/core-merge/MAC-RETIREMENT.md`, our README lines.
**Implements:** migration notes, saved `.dspipreset` compatibility check, our Mac build no longer
published. Freeze `ext-channel-116` as the reference tag; hub test harness stays until HB7 passes.
**Done when:** the owner confirms at gate R-Mac. **Deps:** R-Qt, HB7, MC3.

### 5.5 iOS (Console fork, new branch `rust-core-116`)

Cut from `ext-channel-116`; the Swift-only `-116` branches stay as they are (owner's
rule). USB MIDI is the only USB transport on iPhone and iPad: no DriverKit. Builds run in
`macvm`, one heavy job at a time, GUI session only (`ssh macvm who`, `launchctl asuser`),
host unlocked. The VM has no GitHub credentials: use git bundles. **Pushing needs an instruction.**

**The seams, and the risk.**

| Layer | Size today | Coupling |
|---|---|---|
| Views | most of 72 k lines | call `DSPViewModel`; 35 files hold an observed object |
| `DSPViewModel` | 3 367 lines, 124 `@Published` | Stay the views' API |
| `Commands.swift` | 4 929 lines, 321 functions, 289 wire references | **The replacement** |
| Wire use elsewhere | `StatsView` 41, `DSPi_ConsoleApp` 29, `SpectrumAnalyser` 19, `TestSignalsView` 18, `ContentView` 14, others under 8 | Moved with their domain |
| `DeviceTransport` | one protocol | **The seam for coexistence** |

**Approach: strangler behind `DeviceTransport`.** First a `CoreBackedTransport` puts the
core under all existing Swift code, using CG4 for raw commands: one link, one request engine.
That is shippable on its own. Then domains move one at a time, each keeping the view model's
API and its `@Published` properties.

**Risks to resolve in IO3 and IO8:**
- **Two mirrors.** `@Published` and the core's `DeviceState` must not disagree. Swift writes
  optimistically; the core's mirror follows confirmed writes. The slider fast path
  (`Commands.swift:257`) is the sharpest case.
- **Threads.** Swift may call its transport on the main thread where a core call must not run.
- **22 k lines of tests** assert bytes through `FakeTransport`. Each move must port or retire
  them explicitly, not lose coverage quietly.
- **Shared sources.** Until MC4 the macOS target must keep building unless OD2 says otherwise.
- **Going back.** `.dspipreset`, `PresetSnapshot` and the crossfeed extension have Swift
  codecs that must match the core's byte for byte: fixtures from `dspi-core` go in both.

**IO1: XCFramework.** Size 1. **Owns:** `apps/ios/build-xcframework.sh` in the unified repo.
**Implements:** add `aarch64-apple-ios`, `aarch64-apple-ios-sim` and `x86_64-apple-ios` in the VM
(the VM is Intel, so its Simulator needs x86_64). `staticlib`, usb off. Records library size.
**Done when:** the framework links in an empty app on the Simulator. **Deps:** CG3.

**IO2: Swift package and compile test.** Size 1. **Owns:** `DSPi Console/Core/Generated/` and the
Xcode project lines. **Implements:** the first real compile of the bindings in the app (F1 only
generates them; CG9 gates the compile). `init_logging`, one run per namespace, as AN1. **Done when:**
a test calls `core_info()` and opens the demo device. **Deps:** IO1.

**IO3: Seam map.** Size 0.5. **Owns:** a report, `docs/IOS-SEAMS.md` in `dspi-ios`. **Implements:**
every wire-using function mapped to a core method; those with none become core WPs; threading
and mirror decisions stated. Re-sizes IO8–IO14. **Done when:** Fable has reviewed it. **Deps:** IO2.

**IO4: `CoreBackedTransport`.** Size 1.5. **Owns:** `DSPi Console/Core/CoreBackedTransport.swift`.
**Implements:** `DeviceTransport` over CG4 and the core's notifications; the core's `FakeDevice`
under the demo mode. **Done when:** existing `FakeTransportTests` and wire tests pass with it behind
a launch flag. **Deps:** IO3, CG4.

**IO5: CoreMIDI through the core.** Size 1. **Owns:** `MIDI/CoreMidiPipe.swift`, reducing
`MIDI/MIDITransport.swift` to CoreMIDI I/O. **Implements:** `MidiSender`, `MidiInputHandle` with
`close_port`, the Simulator relay port. **Done when:** vectors from `midi_vectors.json` pass;
relay run **[HW]** on the K340 in the Simulator. **Deps:** IO4.

**IO6: Link through the core.** Size 1.5. **Owns:** `Link/LinkClient*.swift` replaced by
`Core/CoreLink.swift`; keeps `HubBrowser.swift`, `LinkTokenStore.swift` and the pairing sheet.
**Implements:** Bonjour stays in Swift (the entitlement belongs to it); keychain tokens;
`link_retry_now` on foreground. iOS client patches (`console-116` 0014–0020) are the
behaviour reference. Local Network prompt behaviour with Rust's BSD sockets is checked here.
**Done when:** the Mac hub run from `A7-LINK-CLIENT-REPORT.md` passes **[HW]**. **Deps:** IO4, XC1.

**IO7: Banner and errors.** Size 0.5. **Owns:** `iOS/LinkBannerView.swift`. **Done when:** the owner's
rule holds in a fake-hub run in the Simulator; QD10's wording; errors shown. **Deps:** IO6.

**IO8: Reads and the mirror.** Size 3–5. **Go/no-go gate after it (OD3).** **Owns:**
`Commands.swift` "Bulk Parameter Transfer" and "Notification → State Mirroring" sections
(about 840 lines). **Implements:** view-model state filled from `DspiState`, notifications from the
core; older firmware goes read-only (free). **Done when:** every view shows correct values on the demo
device and against fixtures; own echoes do not flicker. **Deps:** IO4.

Then one WP per domain, each with: **Owns** its `Commands.swift` sections and the wire use in its
views; **Done when:** wire tests are ported, the domain has no `transport.` calls left, and the app
works on the demo device. **Size ranges are rough; IO3 re-sizes them.**

| WP | Domain | Size |
|---|---|---|
| IO9 | Volume, EQ, crossover, outputs, matrix, slider fast path | 1.5–2.5 |
| IO10 | Effects (loudness to upmixer) | 1.5–2.5 |
| IO11 | Pins, I2S, ADAT, inputs, UART/I2C, names | 1.5–2 |
| IO12 | Control surfaces | 1.5–3 |
| IO13 | Presets, `PresetDocument`, `PresetSnapshot` | 1.5–3 |
| IO14 | Analyser (following state), Stats, signal generator, monitor | 1.5–3 |

**IO15: Simulator and MIDI relay test.** Size 1. **[HW][COORD]**. **IO16: Real device test.** Size 0.5.
**[HW][OWNER]**: no iPhone or iPad has ever run this app. Device availability and team signing are the
owner's. **Owns:** `docs/IOS-HW.md`. **Done when:** USB-C MIDI, Link, background and return all pass.
**Deps:** IO5, IO6, IO14.

**View-layer gaps** (parity 3.1 and section 3). These can run alongside the domains, as
they touch views only:

**IO17: File import and export.** Size 1. **Owns:** `iOS/DocumentPicker.swift`, the `notYet` block in
`DSPi_ConsoleApp.swift`. **Done when:** filter files and `.dspipreset` both round-trip. **Deps:** IO2.

**IO18: AutoEQ and firmware window.** Size 0.75. **Owns:** iOS AutoEQ menu, firmware wizard step.
**Implements:** Favourites, CG6's update; hide the dead firmware window and tell the user to use
a computer. **Deps:** CG6, IO2.

**IO19: Touch graph.** Size 1.5. **Owns:** `iOS/PeqTouchEditor.swift`, a touch HUD. Marquee, pull
from curve, shape chooser, value chip. **Deps:** IO2.

**IO20: Hardware keyboard and host format.** Size 0.75. **Owns:** `iOS/IOSApp.swift` commands,
`iOS/AudioRouteMonitor.swift`. **Deps:** IO2.

**IO21: Coach marks.** Size 1. **Owns:** `Onboarding/` iOS panel (the macOS panel is excluded from
this target). **Deps:** IO2.

**IO22: Accessibility and localisation.** Size 3. **Owns:** all views. Runs alone, last. **Deps:**
every other IO WP; OD6.

Store items (registered SysEx ID, review video, TestFlight) are later, per the owner:
`APPSTORE-MIDI-RISK.md` applies at that point. The firmware and app's manufacturer ID `0x7D` must
not reach a public release (OD9).

### 5.6 Web

**WB0: Study.** Size 2. **Owns:** `docs/core-merge/WEB-STUDY.md`, throwaway spikes under
`~/.cache/dspi-build/work/wp-wb0/`. Must judge, with a working spike where marked:

| Option | Questions to answer |
|---|---|
| Rust core in wasm + a web UI **(spike)** | The session's 17 transport-using files block on `&mut dyn Transport`, about 170 call sites and 45 uses of threads or `Instant`. Compare: (a) an async twin of the trait; (b) a request and reply split (`PLAN.md` §1.3); (c) blocking in a Worker with `Atomics.wait`, needing COOP and COEP; (d) JavaScript Promise Integration. Which leaves `dspi-session` unchanged for other clients? `nusb` has a wasm32 WebUSB backend, but needs async. |
| Qt for WebAssembly **(spike)** | Does the Qt app and the C ABI core build? Download size? Same async problem, via Asyncify or threads. |
| Reuse what exists | sigman78's `dspi-web-console` (MIT, Svelte, WebUSB, demo mode, firmware 1.1.5+) and the DSPi-Link web UI (MIT, which HB5 already serves). What would they need for ext channel and Link? Licence fit with GPL-3.0? |
| Link in the browser | Browsers block `ws://` from an `https://` page, and no hub has TLS. So is a hub-served page the only Link route? |
| WebUSB | Chromium only. Check on stock and bofr-main firmware, per OS. |
| Web MIDI | Chromium, Firefox with permission; SysEx permission. Firmware's MIDI build only. |

- **Done when:** a recommendation, with sizes, fills in the stubs below.
- **Deps:** `main`; no hardware needed beyond an optional WebUSB check **[HW][COORD]**.

**Stubs, unsized until WB0:** WB1 transport and session route; WB2 UI; WB3 hosting and
origin; WB4 hardware test. Rough range by route: reuse 3–8, Qt for WebAssembly 8–15, Rust and a
new UI 40–70.

### 5.7 Cross-client tests

**XC1: `macr` restart.** Size 0.5. **[COORD]** with the owner's go-ahead for the restart.
**Owns:** nothing in a repo. **Implements:** patched QEMU and libusb per
`qemu-usb/REPORT.md` "How to apply"; start `macr`; check `br0` address and Bonjour from the phone;
re-attach the DSPi with QMP; rule: host unlocked. **Done when:** Mac hub is reachable from the LAN and
the 6 KB read works. **Prerequisite for every Mac test. Deps:** none; the owner's bridge part is done.

**XC2: Mac hub ping test.** Size 0.25. **[HW][COORD]**. **Owns:** a report. **Done when:** the Mac hub
answers pings, so L1's 5 s probe does not kill healthy sessions; idle past 35 s survives; rate-limit keeps the
token. **Do this before Android and iOS Link WPs merge.** **Deps:** XC1.

**XC3: Real-hub test harness.** Size 0.5. **Owns:** `crates/dspi-link/tests/real_hub/`, opt-in by
environment. **Implements:** the scripted parts of `LINK-HUB-FINDINGS.md` section 5. **Deps:** `main`.

**XC4: ESP32 bridge suite.** Size 1. **[HW][OWNER][COORD]**: **is an ESP32 on hand and wired to a
DSPi?** **Owns:** a report. **Implements:** all 9 items of section 5: idle, wrong PIN ×5, Wi-Fi
pulls at 5, 20, 32 and 60 s, all slots full, DHCP change, board off, GETs over 132 B, cut after
`auth.pair`, Doze. **Done when:** each is pass or fail with the fix filed. **Deps:** XC3, AN5 for phone cases.

**XC5: Meters over a bridge.** Size 0.5. **[HW][OWNER][COORD]**. **Owns:** a report. **Done when:** CPU, frame
time and audio are measured with meters on, using `poll.subscribe`, against kingsnake's report that they
had to turn meters off. **Deps:** XC4.

---

## 6. Hardware and owner tasks

**Rules for every hardware step:**
- back up settings and flash before any flash;
- **never a 1.15 V build on the K340**: it needs 1.20 V;
- at most 80 % volume, and test audio only through the guarded player;
- **the coordinator runs flashes.** Subagents have refused to flash on relayed approval, and
  they are right to; they prepare, and the coordinator acts;
- run `scripts/check_usb_config.py` on any firmware with a descriptor change;
- the Mac VM needs a GUI login and an unlocked host;
- no agent types a password.

| Task | Needs | WPs |
|---|---|---|
| Phone adb port (changes each time) | Owner tells the port | AN13, AN15, AN16 |
| Play upload, forms, closed-test updates | Owner | AN14–AN16 |
| ESP32 bridge wired to a DSPi | Owner: is one available? | XC4, XC5 |
| Real iPhone or iPad, signing team | Owner | IO16 |
| Apple Developer ID, notarisation password | Owner | MC2 |
| Apple silicon Mac | Owner: is one available? | MC3 |
| `macr` restart, DSPi moved between VMs | Coordinator; owner agrees to the restart | XC1, QD25, QD26, IO5, IO6, IO15, HB7 |
| Real flash on the K340 | Coordinator; backups first | QD24–QD26, TM13 |
| Registered SysEx ID | Owner, $240 | Before any public MIDI release |

---

## 7. Waves and gates

Every wave needs `core-merge` on `main`. Within a wave, no two WPs own the same file. A
batch of 4–8 WPs goes to one Fable reviewer.

| Wave | WPs (parallel) | Needs |
|---|---|---|
| 0 | XC1, XC2, XC3 | `main` |
| 1 | CG1–CG9, HB0, WB0; QD0, QD1, QD2, QD4, QD5, QD6, QD7, QD20; TM1, TM2, TM4, TM7, TM8, TM10; AN1 | `main` |
| 2 | HB1, HB2, HB3; QD3, QD8, QD11, QD12, QD15, QD17, QD18; TM3, TM5, TM9, TM12; AN2, AN3, AN6, AN8; IO1 | wave 1 |
| 3 | HB4, HB5, HB6; QD9, QD10, QD13, QD16, QD19; TM6; AN4, AN5, AN7, AN17, AN18; IO2, IO3 | wave 2; HB0's design review |
| 4 | HB7; QD14, QD21; TM11, TM13; AN9–AN12, AN13, AN14; IO4, IO17–IO21 | wave 3; IO3's review |
| 5 | MC1, MC2, MC3; QD24–QD26; AN15, AN16, AN19, IO5, IO6, IO7, IO8 **(then OD3)**; XC4, XC5 | wave 4; XC1 |
| 6 | IO9–IO14 | go at OD3 |
| 7 | QD22 then QD23; AN20 and AN21; IO15, IO16, IO22; MC4 | everything else in that client |

**Review gates:**
1. **This plan** gets a Fable review before wave 0 (owner).
2. **After each wave:** the full `ci/check.sh --all` on the merged result; each other repo's own
   CI (`tools/ci.sh all`; `xcodebuild` iPhone and iPad test in the VM); a code-quality review; an
   adversarial review, especially the owner's Link rule and "no write in read-only".
3. **HB0** and **IO3** get a design review before work builds on them.
4. **Before each release:**

| Gate | Must pass | Then |
|---|---|---|
| **R-Qt** | Section 3's Qt column; QD24–26; no setter on read-only; Link rule on a real hub; OD4 images and the guard | Owner decides to publish |
| **R-Term** | TM13; TM10's round-trip with Qt's and Mac's documents | Owner decides |
| **R-And** | AN13 on the phone, `tools/ci.sh release`; no secrets in logs; Data safety matches the app | Owner uploads each test update (AN14, AN15, AN16 separately) |
| **R-iOS** | IO15, IO16; a registered SysEx ID; review video | Owner; store is later |
| **R-Mac** | R-Qt, MC3, HB7, OD2 | MC4 |
| **R-Web** | Set by WB0 | |

---

## 8. Decisions

### Decided by the owner (recorded, not open)
- **Link hub on mobile:** no hub on iOS. No hub in Android's Play build. A sideload-only
  Android build with a hub is a possible later item (section 9).
- **Terminal** skips features that need a graphical UI (on-graph editing, onboarding) and covers
  the non-graphical equivalents, JSON import and export included (TM10).
- **Rust Link hub** is in scope for Qt.
- **The macOS app is the feature floor** for all clients.
- **No DriverKit;** firmware updates over USB from iOS stay impossible.
- **Earlier owner decisions kept:** no separate Android Link patch; R8 and edge-to-edge go in
  separate releases.

### For the owner

| # | Decision | Recommendation |
|---|---|---|
| OD1 | `dspi-unified-console` is private, but the public Android repo and the public fork will build from it. Publish it before AN14? | **Yes, at the core merge.** Otherwise the public apps cannot be built from source, which GPL distribution needs. |
| OD2 | **macOS Swift Console:** (A) retire our line for Qt on Mac, or (B) Swift on generated parts (5.4). This replaces the earlier "macOS stays" assumption. | **A.** Extra cost 1.75 against 3–4.5 after iOS, or 16.5+ before; it drops Mac-only Swift. It costs native feel. Needs a Developer ID and an Apple silicon run. Tell Weeb Labs; their app is unaffected. |
| OD3 | iOS go/no-go after IO8: continue replacing domains (IO9–IO14, 9–15.5 more), or stop with the core under the transports only (IO4–IO7) | **Decide on IO8's evidence.** Stopping there is a usable result: one link code path, Swift model stays. |
| OD4 | Which firmware do Qt and Android bundle: stock 1.1.6 from Weeb Labs' tag, or a bofr-main build? | **Stock, built from a tag,** while extensions wait on Weeb Labs. K340 owners use a local `.uf2` with CG8's guard. |
| OD5 | Qt undo and redo: Windows C# has IO undo; the Mac has none. It is above the floor. | **No,** unless QD20 finds Windows users depend on it. Cost 1–2 if wanted; the Terminal's journal is a reference. |
| OD6 | Localisation: no client has it, the Mac included. Which languages? | **Do the plumbing (QD23, AN21, IO22) with English plus Swedish.** Dropping these three saves about 4. |
| OD7 | Core-voltage writes in GUIs: the Mac shows status only, and the Terminal CLI has set, trial and auto. | **Status only in GUIs.** A wrong value can hang a board. |
| OD8 | Qt self-update: check-and-link, or in-place install, which needs a signing key and a feed? | **Check-and-link now.** In-place later, adding 1–2 per OS. |
| OD9 | Register a SysEx ID ($240) to replace `0x7D`? | **Yes, before any public firmware or iOS release using MIDI.** `0x7D` is not allowed in public products. |
| OD10 | Confirm **Windows C# is retired** (assumption from the audit). | **Yes, after QD20.** It is frozen at V26. Weeb Labs' choice for their own repo. |
| OD11 | `dspi hub` on the Terminal (TM11) | **Yes,** 0.5: it costs little and gives a headless gateway. |

---

## 9. Unknowns and later work

**Unknowns, with what pins them down:**

| Item | Rough size | Pinned by |
|---|---|---|
| iOS screen replacement (IO8–IO14) | 12–20.5, could be 30 | IO3 (map), IO8 (hardest part first). Mirror and thread problems could change the approach |
| Android control-surface editor (AN9–AN12) | 5.5–9.5 | AN8. Phone layout for a 5 k-line desktop tab is a UX question; IR and display parts need hardware. `FakeDevice`'s CS support looks minimal, so allow about 0.5 more |
| Web build | 3–70 by route | WB0 |
| Rust hub (HB) | 12, could be 18 | HB0. Multi-device ownership in the core is the risk |
| nusb `from_fd` on Android | 0–2 | AN13: first run on a phone. No rusb fallback remains |
| Accessibility work | ±50 % | The first screen of each |
| Real ESP32 behaviour | unknown | XC4. Fixes file into `dspi-link` as new WPs |
| Qt native feel on Mac | not sizable | Owner's judgement at R-Mac |

**Not in any wave:**
- **Android sideload build with a hub**, for a custom player: size 3–5. A UniFFI wrapper for
  `dspi-hub`, NSD registration, a foreground-service hub, settings; separate application id and
  no Play distribution. Needs nothing new now: HB has no OS code.
- Hub with TLS: 2–4. No hub has it; would also open browser access from `https://`.
- Firmware install on iOS through a hub (M14): 0.5–1, after HB4.
- Android `MediaSession` and volume keys (optional in the audit): 1.
- `P2` in `PLAN.md` (shared protocol tables).
- App Store and code-signing work beyond MC2.
- For the owner to raise with Weeb Labs, not from here: H1–H4 in `LINK-HUB-FINDINGS.md`, the
  `0x00` entry in `policy/commands.json`.
