# One unified console: plan for every client

*Branch `plan/clients`. Written 2026-11-06, revised after Fable review. Checked
against `core-merge` up to `d190535`: `wp/F1`, `wp/L1`, `wp/R1`, `wp/Q2a`,
`wp/Q2b`, `wp/C15` at `2cb89c1`.*

**Purpose.** One project, built from one base, for every client. In the owner's
words: "one single unified thing that can build all the versions from the same
base." `PLAN.md` ends when the core is
merged and the Qt app runs on it (Q3, D1). This plan does the rest.

**The target:**

| Platform | Frontend | Why |
|---|---|---|
| macOS, iOS, iPadOS | **SwiftUI over the Rust core.** Swift keeps UI only, one Xcode project | Owner: "the only people that might care about look and feel matching everything else are mac and ios users" |
| Linux, Windows | Qt on the Rust core | "Close enough" (owner). Replaces the Linux and Windows C# consoles |
| Android | Kotlin/Compose on the Rust core | Exists; switches core |
| Terminal | The session crates directly | Exists |
| Web | The Rust core in wasm with a minimal client in this repo, no external project. Scheduled after the plan's WPs (owner, 2026-11-07) | Section 5.5 |

**Start:** when `core-merge` reaches `main`. Owner's standing instruction: start
the first wave then, without asking again. **Every WP depends on that merge**, and
WP branches are cut from `main`. Hardware, flash, release and password steps keep
their own approvals (section 6). **Contacting Weeb Labs or making anything public is
only ever the owner's act. No agent plans or does either.**

Sizes are relative units, for comparing WPs. Measured in the core merge: a size-1 WP took 10–35 minutes of agent time, about an hour with review and fixups. They are estimates, and elapsed time depends on parallelism and on hardware and owner steps.

| Track | WPs | Size (units) | Depends on |
|---|---|---|---|
| MR: one repo, one build | 3 | 2.5 | `main` |
| LC, HO: licence inventory, handover | 2 | 1.75 | `main`; HO1 late |
| CG: core gaps | 28 | 16.75 | `main` |
| HB: Rust Link hub | 7 | 12 | `main` |
| QD: Qt (Linux, Windows) | 33 | 24.25 | `main`, CG, HB for the hub page |
| TM: Terminal | 23 | 11.75 | `main`, CG, HB for `dspi hub` |
| AN: Android | 29 | 28–32 | `main`, MR1, CG9 |
| SW: Swift (macOS and iOS) | 29 | 35.25–39.75 (SW8–SW14 re-sized by SW3) | `main`, MR1, CG3, CG4 |
| WB: web | 2 + stubs | 2.75 (study, WB1) | `main` |
| XC: audit, cross-client tests | 7 | 3.75 | hub hardware, K340 |
| **Total** | **168** | **about 129–138**, not counting the web build | |

*These are the counts as first planned. The WPs added since (review follow-ups such as CG23-CG31, AN32-AN39, QD36-QD38,
TM25, TM26, SWF1-SWF3) and the phases after the plan (web, optimisation, polish: sections 5.5, 5.7, 5.8) are counted in the
status section below.*

**M0, the first milestone, is about 12–14 days after `main`** (section 0.3). The
largest unknowns are Swift domain replacement, the Android control-surface editor
and the web build (section 9).

---

## Status (2026-11-19)

*WP DOCSYNC, 2026-11-19, synced with `git log main` at `11625ba5`: the rows below that changed since 2026-11-15 are
marked again (SW7, SW-M2, SW-M3, SW-M4, WB6 and its slices WB6d to WB6l); the paragraph "Where we are" of 2026-11-15
is kept below it.*

**Where we are (2026-11-19).** Open in this plan, besides the hardware and owner rows ([HW], [OWNER]): the Swift tail
(SW19, SW20, SW22 to SW24, SW-V1, and SWS13, the retirement of the opt-out paths), Android's AN21 (localisation), the
web's WB4c and WB9, the handover HO1, and the optimisation and polish waves (OPT0 to OPT9, PL-*), which have not
started. The Swift app runs on the core by default for USB, USB MIDI, DSPi Link, firmware updates and the Mac's hub (SW-M1, SW5, SW6, SW-M2,
SW-M3). The web's UI floor is complete (WB6a to WB6l). Beyond this plan, on `main`: PRO phase 2 (`PLAN-P2.md`), the
security waves SEC-A to SEC-D (`PLAN-SEC.md`), PRO phase 3 (`PLAN-P3.md`), PRO phase 5 (`PLAN-P5.md`), the multi-unit
measurement on the alink time base (`PLAN-AT.md`) and both firmware generations (`PLAN-PG.md`); each of those files
marks its own WPs. Their hardware runs (P3HIL, P5HIL, ATHIL, PGSMOKE, PGHIL) are owed.

*Status of 2026-11-15, from the status markers in this plan, synced with `git log main` at `f60845ef` (WP DOCSYNC). A hash is the merge that
brought the WP into `main`. **DONE**: merged. **PARTIAL**: merged or partly done; the rest is named. **IN PROGRESS**: on its
branch, not finished. **TODO**: not started. **MOVED**: moved to later work, folded elsewhere or replaced. **[HW]**: a device
step is still owed. **[OWNER]**: waits on the owner.*

**Where we are (2026-11-15).** PRO phase 1 (section 5.9) is complete: all 26 WPs are merged. The Swift app runs on the core for USB,
CoreMIDI and DSPi Link (SW-M1, SW5, SW6), and its domains moved to the core (SW8-SW14). The web client has its transports,
hosting and hardware pass (WB2-WB8) and the first three UI slices (WB6a-WB6c). Qt and Android finished their last-wave WPs
(QD21a-QD23, AN15, AN20). **The owner's order** still holds: the plan's WPs, then the web, then optimisation (5.7), then
polish (5.8), per resource.

### At a glance

| State | WPs |
|---|---|
| **In progress** (2026-11-19) | No WP of this plan. `wp/PGSENDER` (PLAN-PG: the dual-stack alink sender on its paged stack) was reviewed with WP DOCSYNC (`review/pgsdoc`). On 2026-11-15 SW7, SW-M2, SW-M3 and PLANP2 were in progress; all four are merged |
| **Next** (2026-11-19) | Swift: SWS13 (retire the opt-out paths), PGSWIFT (PLAN-PG), SW19, SW20, SW22, SW23, SW24, SW-V1 (VM). Web: WB4c (later, OD9), then WB9. Android and handover: AN21, HO1. Then OPT0-OPT9, then PL-*. The hardware runs of the later plans (P3HIL, P5HIL, ATHIL, PGSMOKE, PGHIL) with the owner |
| **Blocked** | **Need hardware or the owner:** SW15 (simulator and relay run), SW16 (iPhone or iPad), SW-M5 (Apple silicon Mac); XC4, XC5 (an ESP32 bridge), XC6 (go for a VREG write); QD35 hand checks; AN16 and AN22 (AN15's release upload, phone checks) |
| **Done since 2026-11-15** | **Swift:** SW7 and SW-M2 (review batch 2, on `main` from `dc06920a`), SW-M3 (review batch 6, on `main` from `ef0e4095`), SW-M4 (`6d337b34`), SWS4 and SWS7 (review batch 6), P2SW and SECSW (`49cd5370`). **Web:** WB6d to WB6l (`6d337b34`, `5778bae0`, `8634bf02`), so WB6 is done. **Plans beyond this one:** PLANP2 and PRO phase 2, PLAN-SEC, PRO phase 3, PRO phase 5, PLAN-AT, PLAN-PG (their own files) |
| **Done since the last status (2026-11-08)** | **PRO phase 0 and 1:** P0W, P0W2, P0M, P0U, P0E; all of P1-A to P1-G (P1UG ... P1TZ). **Swift on the core:** SW-M1, SW5, SW6, SW8-SW14, SW17, SW18, SWMAC1. **Web:** WB2-WB5, WB7, WB8, WB6a-WB6c, UIM1. **Qt and Android:** QD21a, QD22, QD23, the Qt swap (POSTSWAP), AN15, AN20, AN42, ANLAST. **Core and hub:** ROMODE, ECHO1, CORENITS, ANDFU, CLEAN1, BULKFIX, HUBADMIN, TMCORE, PAIRBAND; the alink and USB-direct work (ALINK1, AUSB1-AUSB3) |

**Open checks found by the dispatch (not WPs yet):**
- **Firmware image labels:** Android's firmware page says "Official release 1.1.6 beta 5 / Included with this app" while every
  client bundles `DSPi-RP2*-v1.1.6.uf2` and CG27 made the demo final 1.1.6. Check which build the images are, then fix the
  label (one line in `feature-firmware`) or the image.
- **HB7's [HW] half:** hw-mac ran the hubs on `macvm` with the K340 (pass). Check that this covers our client and the TS client
  against the Swift hub, or run the rest.
- **N1 from hw-mac-2:** `SpdifInputWireTests`' live test always skips (offline view model). Folded into the next Swift WP.
- **XC2:** its report is not in the repo and 5 scenarios were skipped; record that as the result or re-run them.
- **Owed outside this repo:** the firmware stamp in `bofr-main` for CG8's guard (coordinator, a firmware item).

**MR: one repo, one build**: 3 WPs: 3 DONE.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| MR1 | Import with history | DONE | ef2c48c (Android half), bdd4a61 (Swift half, review c4s) |  |
| MR2 | One entry point | DONE | 6b3e2c8 |  |
| MR3 | Container user | DONE | 5c86b7f |  |

**LC, HO, DOC: licence inventory, handover, docs**: 4 WPs: 3 DONE, 1 TODO.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| DOC1 | README describes the project as it is now | DONE | 8e92bcee |  |
| HO1a | Stale docs (from HO1) | DONE | 2df60015 (review c5a) |  |
| HO1 | Handover pack (the rest) | TODO |  | After R-Qt (QD24, QD25) and SW8; before R-Mac and R-iOS |
| LC1 | Licence inventory, iOS first | DONE | c274193 | Re-run after each SW wave |

**CG: core gaps**: 32 WPs: 32 DONE.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| CG1 | C ABI for Link and MIDI | DONE | 4255432 |  |
| CG2 | Find hubs in the core | DONE | 23cf570, fixups 980b101 | Real LAN untested |
| CG3 | USB as a feature | DONE | 43e6713 | Its iOS check was done in MR2 (6b3e2c8) |
| CG4 | Raw commands for bindings | DONE | 5022678 |  |
| CG5 | Analyser following state | DONE | cef77e2, fixups 1823624 |  |
| CG6 | AutoEQ update with a host-supplied fetch | DONE | 962db6a, fixups a8422d5 |  |
| CG7 | Preset diff at the floor | DONE | c54ed0e | Input source names were done in CG13 (56c1a68) |
| CG8 | Firmware guard | DONE | 1a0019d | Firmware stamp in bofr-main still open (coordinator) |
| CG9 | Compile the generated bindings in CI | DONE | cdeb8ec |  |
| CG10 | Shared wording and onboarding data | DONE | 7b10d71 |  |
| CG11 | Core-voltage calibration | DONE | ca059da |  |
| CG12 | Output ring health | DONE | 3296862 |  |
| CG13 | Input source names in the core | DONE | 56c1a68 |  |
| CG14 | Confirmation word for raising the voltage | DONE | ca059da (in wp/CG11) |  |
| CG15 | Creator info for every About screen | DONE | 4be11e7 | Wording is a draft until the owner approves it |
| CG16 | Control surfaces on the demo device | DONE | 83d02e5 |  |
| CG17 | One pair-map validity | DONE | 8ef62f8 |  |
| CG18 | Core config export and import must not invent values | DONE | 5de87b6 |  |
| CG19 | One home for the analyser's sharing rules | DONE | 176fddf |  |
| CG20 | Control-surface editing rules in the core | DONE | a4136af, fixups 6744c45 |  |
| CG21 | One read per Stats tick | DONE | 3fa04b9 |  |
| CG22 | Analyser and signal generator in the Rust fake device | DONE | c1332e7, fixups 716d798 |  |
| CG23 | Raw notification tap and connection generation | DONE | a9002593 (review c4i) |  |
| CG24 | A device lock held across a flow | DONE | a9002593 (review c4i) | Broke the Terminal's flash in read-only; fixed by TM25 |
| CG25 | Stream-state events in `NotifyResult` | DONE | a9002593 (review c4i) |  |
| CG26 | Host-supplied transport | DONE | a9002593 (review c4i) | Retires at SW-M3 |
| CG27 | Final firmware version on the demo device | DONE | f6e74fa9 (review c4j) |  |
| CG28 | Re-read the control-surface status after a display write | DONE | 0cf648c5 (review c4k) |  |
| CG29 | About: the Links and support section is Weeb Labs' | DONE | 2f1e95f3 (release branch, review c4l) |  |
| CG29b | About support paragraph says "I (Weeb Labs)" | DONE | 2f1e95f3 (release branch) |  |
| CG30 | Analyser watching with nothing chosen | DONE | 0f163dfa (review c5d) |  |
| CG31 | External writes reach every client's state; hub task XML | DONE | 0f163dfa (review c5d) |  |

**HB: Rust Link hub**: 9 WPs: 8 DONE, 1 MOVED.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| HB0 | Design and skeleton | DONE | 79caa34 |  |
| HB1 | Server, handshake, auth, roles | DONE | eabb407 |  |
| HB2 | Commands and notifications | DONE | eabb407 (in wp/HB1) | Network voltage refusal provisional: owner |
| HB3 | Registry, snapshot, polls, rename | DONE | eabb407 (in wp/HB1) |  |
| HB4 |  | MOVED |  | Later work |
| HB5 | Advertising and web UI | DONE | a871687 | Owner (2026-11-08): keep the upstream DSPi-Link page (section 5.5); TXT key `rev`: owner |
| HB6 | C ABI for the hub | DONE | 9b6abcf | [HW] HUB_OPT_USB, attach_app |
| HB7 | Conformance | DONE | 144f7057 (review c4m) | [HW] Swift hub on `macr` half: see the open checks above |
| HB8 | HB7's CI nits | DONE | 2df60015 (review c5a) |  |

**QD: Qt (Linux, Windows)**: 41 WPs: 34 DONE, 2 PARTIAL, 1 TODO, 3 MOVED, 1 DROPPED.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| QD0 | Seam and C ABI pitfalls | DONE | a6f2f15 |  |
| QD1 | `CsController` surfaces `error` | DONE | db1b672 |  |
| QD2 | Pair-map editor and dimmed rows | DONE | 31884c4 |  |
| QD3 | Core-voltage status | DONE | 119aa08 |  |
| QD4 | Input-source and device names | DONE | 31561ec (in wp/QD5) |  |
| QD5 | Output latency, clock diagnostics, C0/C1 bars | DONE | 1af6d8f |  |
| QD6 | Read-only mode | DONE | 3c32027 |  |
| QD7 | Demo mode | DONE | f6788a5 |  |
| QD8 | Link controller and token store | DONE | 4feb207 (in wp/QD6); flake fix QD8b a281366 | Keyring checks on a real desktop, winvm, macOS: QD35 |
| QD9 | Device list, discovery, PIN prompt | DONE | fc0f2678 (review c4q) |  |
| QD10 | Lost-connection banner | DONE | 874bd1ef (review c4h) |  |
| QD11 | Firmware updater | DONE | c079e03 | [HW] run list in its report |
| QD12 | Tray | DONE | 912d034 | [HW] tray hand check: QD35 |
| QD13 | Open at login and keep awake | DONE | fc0f2678 (review c4q) | OS checks: QD35 |
| QD13b | QD13 fixups: another copy's login entry, six-digit PIN | DONE | b89d1733 (review c4s2) |  |
| QD14 | Networking page | DONE | 874bd1ef (review c4h) |  |
| QD15 |  | MOVED |  | Later work: coach marks |
| QD16 | App self-update | DONE | fc0f2678 (review c4q) |  |
| QD17 | Shortcuts | DONE | 874bd1ef (review c4h) |  |
| QD18 | Host audio format | DONE | c23f450 | [HW] PipeWire 8 ch, Windows sound settings: QD35 |
| QD19 | Analyser over Link | DONE | 874bd1ef (review c4h) |  |
| QD20 |  | MOVED |  | Folded into `docs/PARITY.md` rows (XC0, HO1) |
| QD21 |  | MOVED |  | Split into QD21a and QD21b |
| QD21a | Packaging, Linux and Windows | DONE | bbc5abae |  |
| QD21b | Qt `.dmg` and its matrix row | DROPPED |  | The swap (2026-11-13): the rebuilt Qt app has no macOS build; the Swift app is the Mac release (the qtport plan's Q8) |
| QD22 | Accessibility | DONE | 709ded30 | Alone in `qml/` |
| QD23 | Localisation plumbing | DONE | 5a5c89a8 | After QD22 |
| QD24 | Hardware pass, Linux | PARTIAL |  | Record `docs/core-merge/HW-QT.md` (hw-1): connect, read, write, reselect pass. Owed: replug [OWNER], Link from Qt, notifications from another client, the firmware path (coordinator and owner's go), RTA with a signal |
| QD25 | Windows | PARTIAL |  | Record `docs/core-merge/HW-QT.md` (hw-win): connect, read, write, detach and re-attach pass; notifications failed (F1, fixed in CG31, not re-run). Owed: notifications re-run, Link from Qt, the firmware path, F3 (no smoke on Windows) |
| QD27 | AutoEQ on the core | DONE | c1dcbb4 |  |
| QD28 | Advanced core voltage | DONE | 2e0cc79, fixup QD28b 8a0675c |  |
| QD29 | Output ring health | DONE | cf09e5c |  |
| QD30 | Popover cursor on the graph gear | DONE | c71da36 | Owner retest on KDE Wayland and X11: QD35 |
| QD31 | Save bar reports setter failures | DONE | 8beebb1 |  |
| QD32 | Qt on the shared wording | DONE | 3e00d97 |  |
| QD33 | Qt About page (in CG15) | DONE | 8073408 |  |
| QD34 | Polling off the UI thread | DONE | 9ed8574 |  |
| QD35 | Qt hand checks | TODO |  | [HW][OWNER] hand checks, now with QD13's OS checks |
| QD36 | Qt follow-ups from review c4h | DONE | e6db6e70 (review c4o) |  |
| QD37 | Qt follow-ups from reviews c4o, c4q, c4s2 | DONE | 4cc9ff5a (review c5c) |  |
| QD38 | Qt smoke time budget (gate-2 F1) | DONE | eaa11829 (review c5f) |  |

**TM: Terminal**: 26 WPs: 25 DONE, 1 PARTIAL.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| TM1 | Pair-map row | DONE | cf20fcc |  |
| TM2 | Core-voltage screen | DONE | 448dd56 |  |
| TM3 | Firmware flashing | DONE | e0f0e52 | [HW] first real flash: coordinator, owner's go |
| TM4 | Read-only fallback | DONE | fb103a0 (with wp/TM21) |  |
| TM5 | Analyser overlay and following | DONE | f1c0446 |  |
| TM6 | Bar strip | DONE | a521fc1 |  |
| TM7 | Tube Basic/Advanced | DONE | 0d65340 |  |
| TM8 | Lost-link status line | DONE | 24575bf |  |
| TM9 | Find hubs | DONE | 8c274f0 |  |
| TM10 | JSON import and export | DONE | bf35c3a, fixup 3072918 | Real Mac export as fixture: coordinator |
| TM11 | `dspi hub` | DONE | f19a837, fixup TM11b 21ebe2d |  |
| TM12 | AutoEQ update | DONE | 8730758 |  |
| TM13 | K340 hardware pass | PARTIAL |  | Record `docs/core-merge/HW-TERMINAL.md` (hw-1, hw-win): real MIDI ports end to end pass (Linux, Windows); replug passed only as a VM detach on Windows. Owed: a physical replug on Linux [OWNER], or the coordinator accepting the VM detach |
| TM14 | No tmpfs in tests | DONE | fcfdb58 (earlier commit; no WP merge) |  |
| TM15 | Closed pipes | DONE | 0ee1112 |  |
| TM16 | Output ring health | DONE | 6d33b8b |  |
| TM17 | Device names and latency in the TUI | DONE | 5e3a386 |  |
| TM18 | Volume mode, Identify, tube view | DONE | 6c179d6 |  |
| TM19 | Multi-line Save | DONE | 6b970ef |  |
| TM20 | Tokenizer and latency confirm | DONE | a5fd081 |  |
| TM21 | Preset capture must not invent values | DONE | fb103a0 |  |
| TM22 | Terminal About page (in CG15) | DONE | ea3d660 |  |
| TM23 | Settings page drawing | DONE | 3eb7665, fixup a76539f |  |
| TM24 | Terminal control surfaces on `cs_rows` | DONE | 06c7af9 |  |
| TM25 | Flash after CG24's device lock | DONE | 2df60015 (review c5a) |  |
| TM26 | Fixes from the K340 run hw-1 | DONE | 2df60015 (review c5a) |  |

**AN: Android**: 39 WPs: 34 DONE, 2 PARTIAL, 3 TODO.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| AN1 | New crates in place of `dspi-ffi` and the patched core | DONE | 684e9f1 |  |
| AN2 | Read-only on older firmware | PARTIAL | 40463af | Phone proof moved to AN13 |
| AN3 | Link engine on the core's supervisor | DONE | 4901541 |  |
| AN4 | Network handling | DONE | 10fd095 | Shipped in 0.12.0 |
| AN5 | Link UI | DONE | 4147f5e | Shipped in 0.12.0 |
| AN6 | Settings rows | DONE | 9940f64 | Shipped in 0.12.0 |
| AN7 | One device list | DONE | 0d33076 | Settings drafts on a device switch: small follow-up (review c4f) |
| AN8 | Control-surface spike | DONE | a85c29c |  |
| AN9 | CS bindings | DONE | 012a0f3 |  |
| AN10 | IR learn | DONE | 2026c83 | [HW] IR learn on the K340 with a receiver and a remote |
| AN11 | Display pages | DONE | 0a6125c | [HW] real panels |
| AN12 | Groups, macros, aux | DONE | 0863194, fixup 0e9db67 |  |
| AN13 | Phone test | PARTIAL |  | Record `docs/HIL-NEW-CORE.md`: AN29, AN30 and USB basics (owner), the demo set (phone-test-2), About (release-check-2). Owed: Link half, older firmware, 45 s background, notifications and no secrets in logcat on USB; LK2, AN15, AN20 phone checks once shipped [HW][OWNER] |
| AN14 | First new-core update | DONE | ca6d52f3 (tag `android-v0.12.0`) | 0.12.0 uploaded to Play by the owner |
| AN15 | R8 | DONE | b34ec774 | Phone run [HW][OWNER]; ships as its own release (AN22) |
| AN16 | Edge-to-edge | TODO |  | After AN15's release is uploaded; phone check [HW][OWNER] |
| AN17 | Analyser bar strip and following | DONE | 35339d2 |  |
| AN18 | Small floor items | DONE | 44d9768, fixups 575475d | Its moved items were done in AN35 (0cf648c5) |
| AN19 | Getting Started and What's New | DONE | a116312 |  |
| AN20 | Accessibility | DONE | 3e3583e7 | TalkBack pass afterwards [HW][OWNER] |
| AN21 | Localisation plumbing | TODO |  | After AN20; likely 2.5, not 1.75 |
| AN22 | The AN15 release | TODO |  | Re-scoped: its old content (AN4-AN6) shipped in 0.12.0. [OWNER] upload |
| AN23 | Advanced core voltage | DONE | 9d0df07 |  |
| AN25 | Output ring health | DONE | ced3bd8 |  |
| AN26 | Android About page (in CG15) | DONE | ef5c56f |  |
| AN27 | No title bar | DONE | 0ec2801 |  |
| AN28 | Demo mode on the real core | DONE | f7cbc97, merge fix 0185343 |  |
| AN29 | USB prompt once, not at every plug-in | DONE | 9fa266c, nit 96ec833 | Owner passed it on the phone at 0.12.0 |
| AN30 | Graph spectrum overlay draws the selected channel | DONE | 9522048 | Owner passed it on the phone at 0.12.0 |
| AN31 | A visible way into Tools | DONE | e323750 |  |
| AN32 | Release fixes for Android 0.12.0 | DONE | 2f1e95f3 (release branch merged back) |  |
| AN33 | No host-device nouns in the Android texts | DONE | 2f1e95f3 (release branch merged back) |  |
| AN34 | The analyser bar strip on screen | DONE | f6e74fa9 (review c4j) |  |
| AN35 | AN18's remaining items | DONE | 0cf648c5 (review c4k) |  |
| AN36 | AN19 and AN5 fixups | DONE | fc0f2678 (review c4q); second part 2f1e95f3 |  |
| AN37 | A channel page's bar strip shares the gear's selection | DONE | 0cf648c5 (review c4k) |  |
| AN38 | Analyser wake: one bindings copy per test JVM | DONE | 172aa93d (review c5b) |  |
| AN39 | Android small follow-ups | DONE | 172aa93d (review c5b) | Its overlay item was done in CG30 |
| AN42 | Undo and redo in the Android app (PRO1 W2) | DONE | 5e355a18 | Before P1AN1 (both touch the app bar) |

**SW: Swift (macOS and iOS)**: 35 WPs: 25 DONE, 9 TODO, 1 MOVED (2026-11-19; it was 21 DONE, 3 IN PROGRESS, 10 TODO on 2026-11-15).

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| SW1 | XCFramework | DONE | fff4d2f (review c4e) |  |
| SW2 | Swift package and compile test | DONE | fff4d2f (review c4e) |  |
| SW3 | Seam map | DONE | 170c528 |  |
| SW4 | `CoreBackedTransport` | DONE | 23aea616 (review c4n) |  |
| SW5 | CoreMIDI through the core | DONE | 4e655d0e | VM; [HW] relay run on the K340 |
| SW6 | Link through the core | DONE | f60845ef | VM; [HW] |
| SW7 | Banner and errors | DONE | review batch 2 (`eecdab08`), on `main` from `dc06920a` |  |
| SW8 | Reads and the mirror | DONE | 0755bba3 | Merged in review c6b |
| SW9 | Volume, EQ, crossover, outputs, matrix | DONE | d1b1918a (review sw9 + fixup) | Typed setters and the write lane; Undo works on this domain through the core (PARITY section 4). Cut from wp/SWMAC1 |
| SW10 | Effects | DONE | 8a33ea09 (review sw10 + fixup) | Loudness to upmixer on the typed setters and SW9's lane; Undo works on each effect through the core (PARITY section 4). Cut from wp/SW9 |
| SW11 | Pins, I2S, ADAT, inputs, names | DONE | 44196ca0 (review sw11) | Typed API through a core (status setters wait for the device's answer: `writeAndWait`); stream and name events on the core's flags; Undo follows the registry (PARITY section 4). Cut from wp/SW10, main merged at 1c87fcee |
| SW12 | Control surfaces | DONE | a95b9cdf (review sw12 + fixup) | `csFetch`/`csSnapshot`/`csApply` through a core, the snapshot into the Swift models; IR learn holds CG24's lock in the core; CS edits recorded, Undo steps over them (PARITY section 4). Review sw11's N1-N3 fixed. Cut from main 44196ca0 |
| SW13 | Presets | DONE | a4d4e1ef (review sw13) | Presets, the directory, both persistence modes, the flash saves and the factory reset on the core's typed calls (the core takes the device lock and waits); `.dspipreset` export is `configExport`, import `configImport` with the file's text; preset edits recorded, Undo steps over them, a load, an import and a reset end the history (PARITY section 4). Cut from main a95b9cdf |
| SW14 | Analyser, Stats, signal generator | DONE | de56e719 | The analyser on the core's worker (`rtaSetRequest`, its snapshot through `RtaListener`, CG5's following state), meters, Stats, the signal generator and the monitor log on the core's typed calls; no request of the domain raw; signal-generator edits recorded, Undo steps over them (PARITY section 4). Review sw13's N2, N3 and F1 fixed. Cut from main a4d4e1ef |
| SW15 | Simulator and relay test | TODO |  | [HW] after SW5, SW6, SW14 |
| SW16 | Real iPhone or iPad | TODO |  | [HW][OWNER]: no device yet |
| SW17 | File import and export | DONE | 84220ac7 (review c6n) |  |
| SW18 | AutoEQ and firmware window | DONE | 54bde172 | iOS Tools > AutoEQ (Favorites, CG6 update: rebuild, import, reset); no firmware window, Tools entry or `.uf2` on iOS, banner and wizard point to a computer. Review sw18 accepted with nits; fixups F1-F3 in wp/SW18F |
| SW19 | Touch graph | TODO |  | VM |
| SW20 | Hardware keyboard and host format | TODO |  | VM |
| SW21 |  | MOVED |  | Later work: coach marks |
| SW22 | Accessibility and localisation plumbing | TODO |  | Last, alone |
| SW23 | Advanced core voltage | TODO |  | VM |
| SW24 | Mac and iOS About (in CG15) | TODO |  | VM |
| SW25 | Generated Swift constants | DONE | 12667465 (review c4p) |  |
| SWF1 | Hardware tests never leave a device changed | DONE | bc1f9ff4 (review c5e) | Confirmed on the K340 (hw-mac-2) |
| SWF2 | Mac USB beside a held MIDI port, refused writes revert, hub voltage rule | DONE | bc1f9ff4 (review c5e) | Confirmed on the K340 (hw-mac-2) |
| SWF3 | iOS layout and version | DONE | bc1f9ff4 (review c5e) | Confirmed (hw-mac-2) |
| SW-M1 | USB through the core | DONE | 30e9b0fe | Core default on the Mac; `-CoreTransport NO` opts out. Hardware check on `macr`: see the WP report |
| SW-M2 | Firmware through the core | DONE | review batch 2 (`eecdab08`), on `main` from `dc06920a` |  |
| SW-M3 | Hub over `dspi-hub` | DONE | review batch 6 (`2f7a811e`), on `main` from `ef0e4095` | The Swift hub and the opt-out paths still ship; their retirement is SWS13 |
| SW-M4 | What stays Swift | DONE | `6d337b34` (review batch 3) | `docs/SWIFT-STAYS.md` |
| SW-M5 | Real Mac hardware pass | TODO |  | [HW][OWNER]: Apple silicon Mac |
| SW-U1 | Upstream tracking rule | DONE | fc0f2678 (review c4q) |  |
| SW-V1 | Visual and feel evidence for OD3 | TODO |  | No longer gates OD3 (decided 2026-11-13) |

**WB: web** (section 5.5, after the plan's WPs): 12 WPs (WB6 in slices): 10 DONE, 2 TODO (2026-11-19; WB6 was in progress on 2026-11-15). The old stubs WB2-WB4
(UI, hosting, hardware) are replaced by the WPs below.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| WB0 | Study | DONE | 5cd58c7 |  |
| WB1 | Keep the core portable to the web | DONE | 0e29dfb |  |
| WB2 | Runtime without threads | DONE | 88dc7342 (review c6a) |  |
| WB3 | The binding (`crates/dspi-wasm`) | DONE | 77d9c013 (review c6e) |  |
| WB4a | WebUSB transport | DONE | cd897dd2 | After WB3 |
| WB4b | Link transport | DONE | 120a78a3 | After WB3 |
| WB4c | Web MIDI | TODO |  | Later; public use waits on OD9 |
| WB5 | Web client skeleton | DONE | 88dc7342 (review c6a) |  |
| WB6 | UI floor, in slices | DONE | WB6a to WB6c (c7d99131 and before), WB6d, WB6f, WB6g (`6d337b34`), WB6e, WB6h, WB6i (`5778bae0`), WB6j, WB6k, WB6l (`8634bf02`) | The slice table in section 5.5 |
| WB7 | Hosting | DONE | d51a1c94 | After WB4b |
| WB8 | Hardware | DONE | 5b108bea | Last; parts [HW][OWNER] |
| WB9 | Gate (R-Web) | TODO |  | After WB8 |

**OPT: core and transfer optimisations** (section 5.7, after the web): 10 WPs: 10 TODO. OPT0 first; every later one
measures against it.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| OPT0 | Bench harness | TODO |  |  |
| OPT1 | Connect cost | TODO |  | After OPT0 |
| OPT2 | Bulk transfer size | TODO |  | After OPT0; K340 |
| OPT3 | Polling cadence | TODO |  | After OPT0 |
| OPT4 | Notification batching | TODO |  | After OPT0 |
| OPT5 | Allocation in hot paths | TODO |  | After OPT0 |
| OPT6 | Slider write coalescing | TODO |  | After OPT0 |
| OPT7 | Link framing and pipelining | TODO |  | After OPT0 |
| OPT8 | MIDI SysEx chunking | TODO |  | After OPT0; K340, then `macvm` |
| OPT9 | Sizes | TODO |  | After WB3 (the wasm module) |

**PL: UI polish** (section 5.8, last): 5 audits plus their fix WPs: all TODO.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| PL-QT0 | Qt audit | TODO |  | After QD22, QD23 |
| PL-AN0 | Android audit | TODO |  | After AN20, AN21 |
| PL-TM0 | Terminal audit | TODO |  |  |
| PL-SW0 | Swift audit | TODO |  | After SW22; VM |
| PL-WB0 | Web audit | TODO |  | After WB6 |
| PL-*n* | Fixes cut from each audit | TODO |  | Per audit |

**XC: audit, cross-client tests**: 7 WPs: 4 DONE, 3 TODO.

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| XC0 | Finish the floor audit | DONE | c69fe82 |  |
| XC1 | `macr` restart | DONE | no commit (coordinator) | Result in work/clients-wave0.md |
| XC2 | Mac hub ping test | DONE | no commit (coordinator) | Report not in the repo; 5 scenarios skipped (open checks above) |
| XC3 | Real-hub test harness | DONE | 902dcce |  |
| XC4 | ESP32 bridge suite | TODO |  | [HW][OWNER] needs an ESP32 bridge |
| XC5 | Meters over a bridge | TODO |  | [HW][OWNER] after XC4 |
| XC6 | Core voltage on the K340 | TODO |  | [HW] K340, backups first; owner's go for the VREG write |

**P1: PRO phase 1** (section 5.9, "a single device you can trust"): 26 WPs, 52 units: all DONE. Android's undo is AN42
(DONE, AN table). Undo in Qt, the web, Swift and the Terminal, Qt's notices, shortcuts, read-only floor, power-on group and
"Protect all outputs" are already on `main` (section 5.9's table).

| ID | Title | Status | Merged in / branch | Next or blocker |
|---|---|---|---|---|
| P1UG | Undo groups for compound edits | DONE | 5e355a18 | Wave P1-A |
| P1XF | Crossfeed named presets in the mirror | DONE | 5e355a18 | Wave P1-A |
| P1DF | Document diff and partial apply | DONE | 5e355a18 | Wave P1-A |
| P1NT | One error-key table for notices | DONE | 5e355a18 | Wave P1-A |
| P1SN | Snapshots | DONE | 4bc97606 | Wave P1-B; after P1DF |
| P1OF | Offline editing and templates | DONE | 0c83f07e | Wave P1-B; after P1DF |
| P1TK | Confirmation tokens for dangerous operations | DONE | d63ea832 | Wave P1-B; after P1NT |
| P1PR | Protect all outputs in the core; core compound writes as groups | DONE | a63e3b3e | Wave P1-B; after P1UG |
| P1HA | Guarded apply on the hub | DONE | 27cc37bf | Wave P1-B |
| P1AB | A/B compare | DONE | 0c34b4aa | Wave P1-C; after P1SN, P1DF |
| P1SA | Safe apply with rollback | DONE | 35cf785a | Wave P1-C; after P1SN, P1DF, P1HA |
| P1LM | Limits profile | DONE | ce3a5892 | Wave P1-C; after P1UG, P1DF, P1SN |
| P1LK | Lockout, PIN and the mode manifest | DONE | ae43e568 | Wave P1-D; after P1LM |
| P1AQ | AutoEQ preview | DONE | a42759c2 | Wave P1-D; after P1SN, P1LM |
| P1HL | Limits and lock on the hub | DONE | 7750a7b9 | Wave P1-D; after P1LM, P1HA |
| P1TM | Terminal commands | DONE | ce44e4d0 | Wave P1-E; after P1SN, P1AB, P1OF, P1SA, P1LM, P1LK, P1TK |
| P1QT1 | Qt: snapshots, A/B, import preview, safe apply, offline files | DONE | 594a3e36 | Wave P1-D; after P1-C |
| P1WB1 | Web: snapshots, A/B, import preview, safe apply, offline files | DONE | 594a3e36 | Wave P1-D; after P1-C and WB6k |
| P1AN1 | Android: snapshots, A/B, import preview, safe apply, offline files | DONE | 594a3e36 | Wave P1-D; after P1-C and AN42 |
| P1QT2 | Qt: Installer page, lock, tokens, AutoEQ preview, notices, groups | DONE | 53d28d1e | Wave P1-E; after P1QT1, P1-D |
| P1WB2 | Web: Installer, lock, power-on, stale, notices, tokens, AutoEQ preview, groups | DONE | 53d28d1e | Wave P1-E; after P1WB1, P1-D |
| P1AN2 | Android: Installer, lock, mode switch, power-on, notices, tokens, AutoEQ preview, groups | DONE | 53d28d1e | Wave P1-E; after P1AN1, P1-D |
| P1SW1 | Swift: snapshots, A/B, import preview, safe apply, offline files | DONE | ce44e4d0 | Wave P1-E; VM; after SW11–SW14 |
| P1SW2 | Swift: Installer, lock, mode switch, power-on, tokens, AutoEQ preview, groups | DONE | d6b7bb22 | Wave P1-F; VM; after P1SW1, SW18 |
| P1TZ | Tokens required by default | DONE | 39b7811d | Wave P1-G; after every client's second P1 WP and P1TM |
| P1SW3 | Swift: notices and the read-only floor | DONE | db3a9ad5 | Wave P1-G; VM; after P1SW2 |

---

## 0. One base

### 0.1 One repo

Target layout: `crates/`, `apps/desktop`, `apps/terminal`, `apps/android`,
`apps/apple`. **Owner decided: monorepo** ("yes"). This replaces the earlier "separate branch of the
Console fork" rule.

**MR1: Import with history.** [DONE ef2c48c] Size 0.75. Two halves.
- **Android half (wave 0):** `dspi-android` to `apps/android` by a subdirectory rewrite
  and an unrelated-histories merge. History is small (155 commits, 64 MB `.git`). Done
  before AN1, which then uses the crates by path and removes `rust/`, `third_party/`
  and patches 0000–0004. **The old question of how a public repo consumes a private
  one disappears.**
- **Swift half (wave 2, with SW1 and SW2):** `ext-channel-116` of the fork to
  `apps/apple` (486 commits, 19 MB `.git`). One Xcode project, both targets.
- **The Swift-only `-116` branches stay in the fork**, which becomes the mirror that
  tracks upstream. The public `dspi-android` repo stays as a pointer or mirror, since
  Play and the GPL source link point at it.
- **Owns:** `apps/android/**`, `apps/apple/**` at import; root `.gitignore`.
- **Done when:** `git log --follow` works on a sample file; a scan of the imported history
  finds no key, `upload.properties`, keystore or `.aab` (`keystore/` is gitignored today;
  only the `tools/debug-keystore.sh` script is tracked); each app's own CI still passes
  from its new path. No push.
- **Deps:** `main`.

**Upstream tracking** (SW-U1 writes `docs/UPSTREAM-TRACKING.md`): the diff against upstream
Swift is confined to `Core/`, removal of `Commands.swift` and view call sites. Each upstream
release gets a merge WP, sized 0.5–1 each. A change outside those places needs a reason in the
review.

### 0.2 One build

**MR2: One entry point.** [DONE 6b3e2c8] Size 1.5.
- **Owns:** `ci/check.sh --all-clients` (or `xtask`, which is an empty stub today), `ci/matrix.sh`, `ci/run.sh`, `ci/container/`.
- **Implements:** from one commit, build and test everything, then print a matrix:

| Client | Where |
|---|---|
| Linux Qt, Terminal | container |
| Windows Qt, Terminal | `xwin` in a container; run in `winvm` when scheduled |
| Android AAB | Gradle container |
| wasm check | container |
| macOS Swift app, iOS Simulator | `macvm` through `launchctl asuser`: GUI login, host unlocked, one heavy job at a time. **"Not run" until SW2 adds these rows**, and later whenever the VM is down |

- **Reproducible images:** `ci/run.sh`'s base images `localhost/dspi-qtport:4` and `localhost/dspi-termport-xwin:1` have no Containerfile in the repo (`ci/container/Containerfile` only builds on top of the first), so a clean machine cannot recreate them. **Commit a Containerfile for every image**, with pinned bases and verified downloads.
- **CG3's iOS check:** Linux containers have no Apple SDK, so the container step is
  `cargo check --target aarch64-apple-ios` only, with no link. If a dependency's build
  script needs the SDK, it moves to the VM. Real linking is always `macvm` (SW1).
- **Drift step:** regenerate `dspi_core.h`, the Kotlin contract and bindings, and the Swift
  bindings, then fail on any diff. **Generated code is never hand-edited.**
- **Done when (MR2 passed, 23 rows, 25.6 min):** a run from a clean checkout gives the matrix with every non-Apple row, and the macOS and iOS
  rows reading "not run"; an edited generated file fails it; **every image builds from its committed Containerfile with an empty podman and download cache.** **The full matrix is M0's gate.**
- **Deps:** MR1, CG9.

**MR3: Container user.** [DONE 5c86b7f] Size 0.25. From MR2. **Owns:** `ci/run.sh`, the images' `USER`. Files written under `--userns=keep-id` belong to uid 524288 on the host, so worktree cleanup needs `podman unshare`. **Needs** a chown of the shared caches while nothing is building, so it runs between waves. Also adds `gen_kotlin_contract.py --check` to the root gate; an `android-tests` row, as no Kotlin module test runs in the matrix, so an unclassified write would pass it; `ci/matrix.sh --only` refuses unknown ids; `dev.sh`'s Gradle journal lock collisions across containers.

**What stays on the host:** the Arm firmware toolchain only (existing exception). Everything
else runs in podman or a VM, with verified downloads.

### 0.3 Milestone M0: "one base"

One commit, one command. Every client builds and runs the demo device on the same core:
- Linux and Windows Qt (Q3);
- Terminal;
- an Android AAB on `dspi-ffi` (AN1, AN2);
- the Swift app on macOS and the iOS Simulator with the core under its transports (SW1–SW4);
- the parity table (`docs/PARITY.md`, XC0) and the build matrix (MR2).

**WPs:** MR1, MR2, LC1, XC0, CG1, CG3, CG4, CG9, AN1, AN2, SW1–SW4, plus Q3 from `PLAN.md`. **About
12–14 days after `main`**, before the floor work, reached at the end of wave 3. No UI is rewritten, so
a Swift user sees no change; that is the point of the demonstration.

**Reached (2026-11-08):** gate-3 ran `ci/check.sh --all` on `bc1f9ff4` and passed all 26 steps (test 3673 passed, 0 failed),
with SW1-SW4 done. M0 is ready for the owner's review. The Qt `.dmg` row is not part of M0's list (QD21b was dropped at the swap).

**Gate:** the owner reviews M0. It is the natural point for OD-WL. It does not block CG, HB or QD work
in flight. SW8's evidence (a Swift view model fed from the core) is the stronger second proof if the
owner wants it first.

### 0.4 Licensing (OD-Lic: deferred by the owner)

**Owner's direction (decided):** dual-license only the parts needed for the App Store build. Those from Weeb
Labs and those from the owner each get an iOS-friendly licence; the rest stays GPL. Exact terms are for the owner
and Weeb Labs to agree, and no agent drafts legal text as final. **R-iOS waits for the owner to confirm the
terms.** The owner has deferred those terms; LC1's inventory still runs. This section is a technical inventory,
**not legal advice.**

| Source | What the repo actually says | Notes |
|---|---|---|
| macOS Console | **`LICENSE` is GPL-3.0, but its README says MIT** (`README.md:5` badge, `:2066`; upstream too) | Weeb Labs told the owner MIT. Contributors: Weeb Labs, Jonathan Rosser (2 commits) |
| Firmware | GPL-3.0 (`LICENSE`, README) | Contributors: Weeb Labs, bofr, Jonathan Rosser (17), two others |
| DSPi-Terminal, base of the core | GPL-3.0-or-later (`Cargo.toml`, README); **no `LICENSE` file** | Sole author Weeb Labs |
| DSPi-Console-Linux, ported into `dspi-core` | **No licence anywhere** in the repo. Android's `NOTICE.md` records "MIT per Weeb Labs, file pending" | Sole author Weeb Labs. Our workspace relabels it GPL on that word alone |
| DSPi-Console-Windows | **No `LICENSE` file; README says GPL-3.0** | Not imported; referenced only |
| DSPi-Link (spec, policy, fixtures, web bundle) | MIT | Fine in any route |
| sigman78 web console, dspictl | MIT | |
| DSPi-ESP32-Front-Panel, DSPiCliRemote | No licence | Not imported; do not copy |
| AutoEq data | MIT per Android's `NOTICE.md` | Upstream terms to be read in LC1 |
| Android port | GPL-3.0 + §7 terms (attribution, marked versions, no trademark) by Bofr5555. UI and texts ported from the GPL macOS app | |
| `dspi-unified-console` | Root `LICENSE` GPL-3.0; `GPL-3.0-or-later` workspace field | Working assumption, owner, 2026-11-06 |
| Qt | LGPL-3 | Dynamic today in all our packages. Any static Qt (QD21) needs the relinking provision |
| Rust dependencies (254 crates at `d190535`) | All MIT, Apache, BSD, Zlib or Unicode, plus MPL-2.0 (`uniffi*`, `cbindgen`, `option-ext`); `r-efi` offers a non-LGPL choice | Nothing here blocks any route |
| SwiftNIO | Apache-2.0 | |

**The conflict.** GPL-3.0's no-further-restrictions terms and the App Store's usage rules are a known
conflict. Weeb Labs' macOS app ships outside the store, so it has not met it. iOS cannot avoid it: **a GPL
core in an iOS app is the problem.** Play has no such conflict, and neither do direct macOS, Windows or
Linux downloads. **GPL firmware ships in the iOS bundle today:** the iOS target's exclusions (`project.pbxproj:56-79`) list no `.uf2` file (only Swift files, `Info.plist` and the entitlements), so
`DSPi Console/Firmware/DSPi-RP2040-v1.1.6.uf2` and `DSPi-RP2350-v1.1.6.uf2` are included. SW18 removes them from the iOS target.

**Who can grant what.** Weeb Labs for their code, subject to outside contributors; the owner for his. Known outside
contributors so far: Jonathan Rosser (2 commits on the Mac Console, 17 on the firmware). LC1 finds out whether any of
that reaches the iOS binary.

**Keep the licensed surface small.** The iOS build's dependency set is listed and kept small: `usb` and `midir` off,
no CLI or Terminal code, no `cli` feature, no Qt, no firmware images. `crates/dspi-tui` and `dspi-cmd` stay out (F1's
trim). CI fails if the iOS dependency list grows without a change to the inventory.

**LC1: Licence inventory, iOS first.** [DONE c274193] Size 0.75. **Owns:** `docs/licensing/IOS-INVENTORY.md`, a check script in `ci/`, `NOTICE.md`,
per-crate `license` fields, SPDX headers in new code, `deny.toml`.
- **Implements:** the concrete list that makes dual licensing simple: **exactly which crates and files go into the iOS
  binary** (core crates, Swift sources, assets), each with its copyright holders (Weeb Labs, owner, third party, from
  `git blame` and the import history), its current licence, and which permission it needs.
- **Flags:** any third-party part with an incompatible licence; **any contributor beyond Weeb Labs and the owner**, with
  their files. Also records the other imported trees and the disagreements in the table above.
- **Metadata fixes:** the workspace `repository` field points at WeebLabs/DSPi-Terminal; `crates/dspi-link` and `xtask` have no licence field, despite a comment saying dspi-link sets its own. Correct both to what the repo actually is, with dspi-link's MIT origin recorded.
- **Limits:** it changes no licence and drafts no legal text as final. It provides facts for the owner and Weeb Labs.
- **Done when:** the inventory is generated from the iOS build's real dependency graph (`cargo tree` for the iOS target,
  the Xcode target's membership), and re-running it detects a new file or crate; `cargo-deny` runs in CI. It is re-run
  after each SW wave, since SW adds and removes Swift. **Deps:** `main`.

### 0.5 Handover quality for Weeb Labs

**Rules from now on, for every WP:**
- **Follow their conventions:** `Documentation/` per app; `CLAUDE.md` build notes; releases tagged
  `v<version>` and named `DSPi Console v<version>`; no em-dashes (their `networking_plan.md`); from their Qt
  guide: "Never redraw anything unnecessarily", and QML files listed in both `resources.qrc` and `qmldir`.
- **Shared fixtures:** `.dspipreset` and filter files live in one fixture set that every client tests.
- **Keep settings keys:** QSettings, UserDefaults and Keychain or Keystore keys stay unchanged wherever an app
  replaces itself.
- **Stock firmware degrades cleanly:** clients gate extension features on `EXT_LIST`. A stall means the feature is
  hidden, never an error.
- **No new fork leftovers:** no new `se.bofr` or Bofr5555 names in shared code.

**HO1: Handover pack.** [TODO; its stale-docs part done in HO1a] Size 1. Late: before R-Mac and R-iOS. **Owns:** `README.md` top-level map, `Documentation/`
per app, `docs/HANDOVER.md`.
- **Parity against each console replaced:** macOS, Windows C#, Linux, Terminal, as rows of `docs/PARITY.md`. Windows C#
  rows to settle, from the audit: undo and redo (Qt has none; C# has IO undo; owner: no for now), the 0xFC/0xFD pair mask
  (`DspDevice.cs:288` against Qt's `crossfeedOutputPairMask`), settings depth, the bulk-endpoint debug window, 12 PEQ bands
  against 10.
- **Stale docs:** done in HO1a, except `apps/desktop/packaging/arch/PKGBUILD` (QD21a's) and `apps/apple/README.md` (upstream's file; every edit widens the upstream diff, SW-U1: the owner's call).
- **Migration:** settings, tokens, favourites and files per console, including what does not carry over.
- **Leftovers to list and resolve:**
  - Kotlin packages naming `se.bofr` (325 files; a rename is a mechanical choice for later). **Not leftovers:** the Play
    listing, the upload key and `applicationId se.bofr.dspi_console_android`, which the owner keeps;
  - `NOTICE.md` §7 terms naming Bofr5555;
  - Apple side: bundle IDs, team, signing and store submission go through Weeb Labs' developer account (the
    `WeebLabs.DSPi-Console-iOS` ID is theirs; the team is empty in the project);
  - untagged bundled "beta4" images (really `beta3-16-g557bce7`);
  - the `0x7D` SysEx ID, which stays for now (owner); it limits public MIDI releases (section 8).
- **Firmware alignment:** what `bofr-main` adds to `release/v1.1.6` (core voltage, pair map, USB MIDI, tagged bulk
  invalidations, size trims); the `0x00` reservation is not granted yet. It is offered as a PR series **through the owner**.
- **Done when:** a reader who knows only their current consoles can build every client from the README. **Deps:** R-Qt, SW8.

**DOC1: The README as the project is now.** [DONE 8e92bcee] Size 0.25. **Owns:** the top-level `README.md`. **Implemented:** the README describes the unified console as it is (one core, the clients, how to build), with part of HO1's top-level map. Left for the owner: `apps/android/README.md`, `apps/desktop/README.md` and `apps/apple/README.md` (taken by HO1a, except the Apple one).

**HO1a: Stale docs now.** [DONE 2df60015] Size 0.5. HO1's stale-docs bullet, split off so it need not wait for R-Qt. **Owns:** `apps/terminal/README.md`, `docs/terminal/platforms.md`, `apps/desktop/Documentation/parity-plan.md`, `apps/android/README.md`, `apps/desktop/README.md`. **Implemented:** the Terminal, desktop and Android docs describe this repository; the Qt parity plan is retired into `docs/PARITY.md`. Public-facing wording is the owner's to approve.

---

## 1. Rules for every WP

The brief and rules are `WP-BRIEF.md`'s, with these changes.

- **Branch:** `wp/<id>` from `main`, in a worktree under `~/prog/dspi-ucwt/`.
- **Process:** Opus agents do WPs in parallel. Fable reviewers check batches, run the full CI and do
  the adversarial checks. Fixups go back to the WP agent. The coordinator merges. Agents run scoped checks
  only. After merge and QA, WP branches are deleted and caches of merged WPs removed.
- **Never push to Weeb Labs' repos.** Commit identity and trailer as in `WP-BRIEF.md`.
- **Toolchains** in podman or VMs. **No tmpfs.** Scratch goes in `~/.cache/dspi-build/work/wp-<id>/`.
- **The macOS app is the feature floor** (owner). Section 3 assigns each macOS feature. Put logic in the core,
  and let a client add only UI. A gap found in the core gets its own CG or HB WP.
- **Lost Link connection, every client** (owner's rule):
  - show "connection lost, reconnecting" with an abort/disconnect button;
  - the mirror stays readable;
  - writes fail fast and are never replayed.

  The core provides `LinkPhase::Reconnecting`, `reconnect_attempt`, `reconnect_in_ms`, `link_retry_now`,
  and a `link_close` usable from any thread (C15). **A client must not add its own retry loop or queue a
  write** (the Terminal's `Reconnector` is the one exception, as `dspi-tui` doesn't link the core; it queues nothing). Wording comes from CG10.
- **After a reconnect** the client calls `rta_start` again if its analyser was running. A listener callback must
  not call a `set_*_listener` method: it self-deadlocks (review 3-A).
- **C15's surface to use:** `link_connect`'s fifth argument `expected_hub_id`, error code `wrong_hub`, and
  `SystemStatus.stale` and `age_ms` for stale meters.
- **Blocking calls stay off the UI thread**, `poll_hotplug` included (F1 note).
- **Same-file ownership:**
  - Qt: **one WP per wave may edit `DSPiBridge.{h,cpp}` and `main.qml`** (W1 QD0, W2 QD6, W3 QD17). All others add a
    controller and QML file through QD0's registrar and banner host.
  - Terminal: `apps/terminal/src/main.rs`'s command table takes additive edits under a `// -- WP <id> --` marker.
- **Do not make the web harder** (owner). New code in the session and core takes time through the clock seam once WB1 lands, keeps device I/O behind `Transport`, adds no OS call outside a gated module, and adds no thread where a poll by the caller will do. Client code is unaffected.
- **Platform wording** (owner: "put placeholders and switch in the right words depending on which version was compiled"). Shared text uses CG10's placeholders, resolved by compile target in the core. A client never substitutes. **Don't name the host device** (owner: "Having words like phone or computer makes the text wordy"): say it plainly with no device noun; users know what they're holding.
- **Do not foreclose undo** (owner: "no for now, but dont make the plumbing deliberately difficult to add later"). No undo WP.
  Every Qt setter stays routable through one place: the bridge or a registered controller's single apply path, never a direct
  `dspi_*` write from QML or a scattered call.
- **Stranded logic does not stay in a client:** the `LinkPhase` wording table and the onboarding data live in CG10,
  AutoEQ in CG6 (QD27 re-points Qt), and the firmware guard in CG8.

Marks: **[HW]** needs a device, **[OWNER]** needs the owner, **[COORD]** the coordinator runs it, not an agent.

---

## 2. Corrections to the inputs

Checked against the code. The plan follows the corrected version.

| # | Input said | Code says | Effect |
|---|---|---|---|
| 1 | `_ref/DSPi-Console` holds the macOS reference | Stale Weeb Labs `main` `fc686ab`: 20 Swift files, no iOS, no Link. The current reference is `~/.cache/dspi-build/work/parity-ref` (export of `ext-channel-116`, 110 Swift files, 72 k lines), history in `work/ext116/src` | Section 3 used parity-ref |
| 2 | Mac hub has `fw_install`; bridge lacks it | **No hub implements `fw.install`**: `LinkSessionHandler.swift:96,98,411-416` omits it, and `crates/dspi-link/tests/fixtures.rs:101` agrees. The other capabilities are on both hubs (`hub.c:375`). The spec only reserves it | M3, M14 deleted; HB4 later |
| 3 | Core gives Qt the Link client | The C ABI has no `dspi_link_*` and no `dspi_open_midi`. Only an empty `link` feature | **CG1** |
| 4 | Parity 3.0: Link adapter plus mDNS | `PLAN.md` §3 put browsing out of scope and no crate browses. Android and iOS have their own; Qt and Terminal have none | **CG2** |
| 5 | F1 leaves iOS needing only a build | `nusb` (locked at 0.2.5) is non-optional in `dspi-core` and `dspi-transport`, and has no iOS backend. `firmware.rs` also uses it. The VM's Rust has only `*-apple-darwin` targets. **It will not compile for iOS today** | **CG3**, SW1 |
| 6 | Android Link engine needs `token_for`, `retry_advice`, backoff | C15's supervisor moved these into the core, so Android's own loop (`LinkSessionEngine`, 421 lines) must go | AN3 |
| 7 | Android's Kotlin API needs small fixes | All 154 FFI methods exist, and all 37 records match bar added fields. But `gen_kotlin_contract.py` parses `rust/dspi-ffi/src/*`, and the core spreads methods over modules behind `cfg_attr`. Review 3-A: `FfiDspiCore.kt` breaks at `:57`, `:393`, `:65-77`, `:383-387`; bindgen must run per namespace (`ffi/build.gradle.kts:74-77`) | AN1 is 2.5; **CG9** |
| 8 | Parity B2: Android has input-source names | No such code in FFI or Kotlin | AN6 |
| 9 | Parity B23: Android latency "not checked" | Absent | AN6 |
| 10 | Parity 3.2: 0x82 clock diagnostics is a Qt gap | The Mac reference has no `CLOCK_DIAG`. Above floor; still in QD5 as asked | Sizing |
| 11 | Parity: custom `.uf2` as a gap | The Mac reboots to the bootloader and leaves the UF2 to you. Above floor; still in QD11 | Sizing |
| 12 | Parity 3.0: PICOBOOT in core | C11 ships inspection and drive copy, which meets the floor; Android's PICOBOOT stays in Kotlin | No WP |
| 13 | Merge notes: preset diff "same as before" | Swift's diff includes preamps and the pair map (`PresetSnapshot.swift:203,260`). Below floor | **CG7** |
| 14 | Parity: "never a hub" per D1/D5 | D1 is a question; D5 says "client only for v1; hub on demand". Overtaken: owner decided (section 8) | None |
| 15 | Parity 3.3: reconnect, Link, MIDI, demo, ext probe are Terminal gaps | R1 does them all | Not in TM |
| 16 | Parity D5: Terminal has config import and export at parity | See TM10: format chosen by extension only, no stdin or stdout, `--json` ignored, no JSON filter bank, no CS block or pair map in the document | TM10 |
| 17 | Qt bundles "beta4" | Untagged `beta3-16-g557bce7` builds (handover) | QD11, HO1 |
| 18 | The bridge is pending on the owner | `br0` is up and `macr`'s `vm.toml` has the NIC. Patched QEMU and a restart remain | XC1 |
| 19 | Memory: WebUSB needs firmware descriptors | sigman78's MIT console uses WebUSB on stock firmware (`WebUsbTransport.ts`, read, not run); MS OS 2.0 binds WinUSB | WB0 |
| 20 | Studies: "macOS and iOS stay Swift" and the early "retire Swift" idea | Owner: SwiftUI on Mac and iOS over the core | SW track |
| 21 | Parity: coach marks are a Mac feature, gap elsewhere | **Off in the shipped Mac build** (`OnboardingCoordinator.swift:35,144,163`; `DSPi_ConsoleApp.swift:12061-12065`). Only the wizard and What's New ship | Later (owner: "not for now at least") |
| 22 | Licences (`NOTICE`, studies) | Section 0.4: the Mac's `LICENSE` and README disagree; Windows has only a README line; Console-Linux has nothing | OD-Lic |

**Safety gap:** a stock image has no `core_vreg` code and runs at 1.15 V. Flashing one to a K340 through any updater breaks the
"never 1.15 V" rule, and no updater checks. **CG8** adds the check.

---

## 3. The macOS feature floor

The Mac has every row: **"have (Swift, stays)"**, so it has no column. **XC0 finished this audit**: it lives, with
FEATURE-PARITY's matrix, in `docs/PARITY.md` section 5, with a file reference for each cell it read; this table is a view of it.

**"have"** = checked in that client's code. **"partial"** = read, and the cell says what differs. A WP id = that WP adds it. iOS shares the Mac's sources, so its
"have" is the audit's or the target's membership list. Metal renderers and AppKit windows are implementation, not rows.

**Link hub** (mobile: owner decided)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M1 | Share devices: server, hub name, open or pairing, port | HB1–3, HB5, QD14 | TM11 | no (Play); sideload later | no |
| M2 | Paired clients: roles, revoke, pairing window | HB1, QD14 | TM11 (CLI) | no | no |
| M4 | Web UI served by the hub | HB5 | TM11 | no | no |
| M5 | Bonjour advertisement | HB5 | TM11 | no | no |
| M6 | Menu-bar gateway | QD12 (tray) | n.a.: no GUI; a service | n.a.: service notification | n.a.: no such surface |
| M7 | Open at login, start minimised | QD13 | n.a.: a service unit | n.a. | n.a. |
| M8 | Keep the machine awake while sharing | QD13 | TM11 | n.a.: no hub | n.a.: no hub |

M3 and M14 (`fw.install`) are deleted: no hub implements them (section 9).

**Link client**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M9 | Find hubs, add by address | CG2, QD9 | CG2, TM9 | have; AN4 re-resolves | have; kept in SW6 |
| M10 | One merged device list | QD9 | have (R1) | AN7 | have |
| M11 | Non-blocking PIN prompt, clear errors | QD9 | have (R1) | AN5 | have; re-pointed in SW6 |
| M12 | Lost-connection banner (owner's rule) | QD10 | TM8 | AN5 | SW7 |
| M13 | Analyser "following another console", "Take control" (`SpectrumAnalyserView.swift:1046`) | CG5, QD19 | CG5, TM5 | CG5, AN17 | have; re-pointed in SW14 |

**App and firmware**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M15 | App self-update | QD16 | n.a.: package manager | n.a.: Play | n.a.: App Store |
| M16 | Firmware wizard: bundled image, verify, downgrade, enter bootloader without installing, export configuration before flashing (`FirmwareUpdateView.swift:384-385`); hidden on STM32H723 (`DSPi_ConsoleApp.swift:11974`) | QD11 | TM3 (non-graphical) | have most; AN18 | n.a.: no USB bootloader (owner); SW18 hides the dead window |
| M41 | Firmware mismatch banner: any version other than the expected one (`FirmwareUpdateView.swift:481-526`, `DSPViewModel.swift:2099-2106`) | have | TM4 | partial: incompatible wire format only; AN18 | partial: Update opens the dead window; SW18 |

**Onboarding** (Terminal: decided, skips)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M17 | Getting Started wizard (`DSPi_ConsoleApp.swift:12054`) | have | skip | AN19 | have; SW18 drops the firmware step |
| M20 | What's New | have | skip | AN19 | have |
| M18, M19 | Coach-mark tour, hints: **off in the shipped Mac build** | later | skip | later | later |

**Main window and presets**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M42 | Unsaved-changes guard (`DSPi_ConsoleApp.swift:11604,11644`, `PresetSnapshot.swift:701`) | have | have | partial: no device switch yet; AN7 | have |
| M43 | Preset slot menu (`ContentView.swift:1546-1566`) | have | have | have | have |
| M44 | Commit, Revert, Factory Reset (`DSPi_ConsoleApp.swift:11959-11969`) | have | have | have | have |
| M45 | Channel clipboard (`DSPi_ConsoleApp.swift:11706-11731`) | have | have | partial: menu only; keys in AN18 | partial: menu only; keys in SW20 |
| M46 | Identify output (`ContentView.swift:518`, `Commands.swift:3688-3704`) | have | partial: other level and passes; TM18 | have | have |
| M47 | Sidebar volume mode, remembered (`Components.swift:579`) | have | partial: not saved; TM18 | have | have |
| M48 | L/R link of an input pair, with which side to keep (`InputChannelHeader`, `Components.swift:1201-1228,1284,1441-1481`) | have | have | have | have |
| M49 | Linkwitz Transform (`Components.swift:2357`) | have | have | have | have |
| M50 | Bypass Crossovers, Clear All Bands (`Components.swift:1839,1870`) | have | have (`;` bar) | have | have |
| M51 | Slider-reset and rename gestures (`ContentView.swift:1411,463`) | have | n.a.: keys, no gestures | have (double-tap resets) | have (long-press resets; rename from the context menu) |
| M52 | Limiter batch actions (`OutputLimiterView.swift:277-285`) | have | have | have | have |
| M57 | Force a reconnect (`Components.swift:538`) | have (rescan: `CpuSection.qml:127`; QD work confirms it re-opens the current device) | have | have | have |
| M58 | Curve visibility pills (`ContentView.swift:461`, `GraphView.swift:635`) | have | skip: depends on the graph | have | have |
| M59 | Dashboard cards per row (`DashboardView.swift:511-538`) | have | have | have | have |
| M60 | Quick-strip toggles; secondary click opens the tool (`ContentView.swift:575-602`) | have | have | have | have |
| M61 | Help links, Reset Channel Names (`DSPi_ConsoleApp.swift:12080-12086,2391`) | have | have | have | have |
| M62 | Show Debug Information (`DSPi_ConsoleApp.swift:2409`). Only reveals the onboarding developer section, and coach marks are later, so no WP | missing | have | have | have |

**Graph and analysis** (Terminal: decided, skips on-graph editing; the `;` bar is the equivalent)

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M21 | HUD chip, marquee, pull from curve, shape menu | have | skip | have | SW19 |
| M22 | Analyser bar strip | have | TM6 | AN17 | have |
| M23 | Analyser overlay on the graph | have | TM5 | have | have |
| M24 | About 30 shortcuts, Space for the generator, graph keys | QD17 | have (own mnemonics) | AN18 | SW20 |
| M25 | Host audio format (idle channel count, `DSPViewModel.swift:1947-1951`) | QD18 | skip: the device report collapses when idle, and it has no host API | AN18 | SW20 |
| M26 | C0 and C1 CPU bars | QD5 | have | have | have |
| M27 | Pop-out tool windows | have | have (graph) | n.a.: routes | n.a.: sheets |
| M53 | Stats for nerbs, Reset Watermarks, Output Ring Health (`StatsView.swift:206-223,478-504,918-1187`) | partial: no ring health; CG12, QD29 | partial: no ring health; CG12, TM16 | partial: no ring health; CG12, AN25 | have |
| M54 | Interrupt Monitor (`InterruptMonitor.swift:1001-1010`) | have | have | have | have |

**Files**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M28 | Filter import with channel pickers, export | have | have; TM10 adds JSON | have | SW17 |
| M29 | `.dspipreset` with apply options and report | have | TM10 | have | SW17 |
| M30 | Save Master Volume, Save Output Configuration | have | have | have | have |
| M31 | AutoEQ favourites | have | have | have | SW18 |
| M32 | AutoEQ update, rebuild from GitHub, import, Reset to Built-in | have; QD27 re-points | CG6, TM12 | CG6, AN18 | CG6, SW18 |

**Device settings**

| # | Feature | Qt | Terminal | Android | iOS |
|---|---|---|---|---|---|
| M33 | Input-source names | QD4 | partial: command only; TM17 | AN6 | have |
| M34 | Device name | QD4 | partial: command only; TM17 | AN6 | have |
| M35 | Output latency | QD5 | partial: command only; TM17 | AN6 | have |
| M36 | Core-voltage status | QD3 | TM2 | AN6 | have |
| M37 | Pair-map editor, K340 preset, in files and diff | QD2, CG7 | TM1, TM10 | have; CG7 | have |
| M38 | Tube Basic and Advanced | have | TM7 | have | have |
| M39 | Demo device (dev flag on Mac) | QD7 | have (R1) | have | have |
| M40 | USB MIDI (dev flag on Mac) | CG1, as a dev option | have (R1) | n.a.: vendor USB works | have; SW5 |
| M56 | Core-voltage writes and calibration under Advanced. **Above the floor:** owner's addition; the Mac has status only today | QD28 | have (CLI: set, trial, auto, clear) | AN23 | SW23 (both targets) |
| M55 | Settings: Spectrum, Graphing, Global Parameters (`DSPi_ConsoleApp.swift:1707-2381`) | partial: Global lacks Device Name (QD4), Output Latency (QD5) | partial: same; TM17 | partial: same; AN6 | have |

---

## 4. Core gaps

### 4.1 CG: small core WPs

**CG1: C ABI for Link and MIDI.** [DONE 4255432] Size 1.
- **Owns:** `crates/dspi-capi/src/fns_link.rs`.
- **Implements:** every `link_*` function (with `expected_hub_id`), the listener and the free functions; `dspi_open_midi` behind `midir`.
- **Done when:** `ci/check.sh --crate dspi-capi` passes; `link_close` works from a listener thread; Q1's C ABI rules hold (failure values, null and short buffers). **Deps:** `main`.

**CG2: Find hubs in the core.** [DONE 23cf570] Size 1.
- **Owns:** `crates/dspi-link/src/discovery.rs`, `crates/dspi-core/src/link_browse.rs`, `crates/dspi-capi/src/fns_browse.rs`.
- **Implements:** browse `_dspi._tcp` through `mdns-sd` behind a feature, off for mobile; start, stop, results listener; resolve by hub id (F5). HB5 reuses the responder half.
- **Done when:** tests pass against a fake advertiser; mobile libraries do not link it; the dependency reason is stated. **Deps:** `main`.

**CG3: USB as a feature.** [DONE 43e6713; its iOS check in MR2] Size 1.
- **Owns:** `usb` feature in `crates/dspi-transport/Cargo.toml` and `crates/dspi-core/Cargo.toml`; its gates in `device.rs`, `firmware.rs`, `crates/dspi-transport/src/usb.rs`; an `ios-check` step in `ci/check.sh`.
- **Implements:** on for desktop, Android and macOS (`nusb` has `macos_iokit`); off for iOS.
- **Done when:** `dspi-ffi` passes `cargo check` for `aarch64-apple-ios` with `usb` off, as in section 0.2; default builds are unchanged. **Deps:** `main`.

**CG4: Raw commands for bindings.** [DONE 5022678] Size 0.5.
- **Owns:** `crates/dspi-core/src/raw.rs`.
- **Implements:** `raw_get` and `raw_set` on `DspiCore` (exported), so Swift that has not moved yet shares the core's one link (SW4). `raw_set` goes through `Writable`. Adds a §1.2 line in `PLAN.md`.
- **Done when:** tests cover read-only refusal and bulk-session exclusion. **Deps:** `main`.

**CG5: Analyser following state.** [DONE cef77e2] Size 0.5.
- **Owns:** `crates/dspi-core/src/rta.rs`.
- **Implements:** driving or following another session, with the origin, from `RTA_CONFIG`, like `case following(origin:)` at `SpectrumAnalyser.swift:84`; `take_control`.
- **Done when:** a fake-hub test shows another session's config switches state, our own does not, and Take control works. **Deps:** `main`.

**CG6: AutoEQ update with a host-supplied fetch.** [DONE 962db6a] Size 1.
- **Owns:** `crates/dspi-session/src/autoeq.rs` (extended, **no new file**), `crates/dspi-core/src/autoeq.rs` (new binding).
- **Implements:** update, rebuild from GitHub, import and Reset to Built-in, through a fetch callback (reference `AutoEQManager.swift:248,331,500`, `DSPi_ConsoleApp.swift:11391`). No HTTP client in the core.
- **Done when:** tests pass with a scripted fetch, including rate limit and partial failure; exported to C and UniFFI. **Deps:** `main`.

**CG7: Preset diff at the floor.** [DONE c54ed0e; source names in CG13] Size 0.5.
- **Owns:** the diff section of `crates/dspi-core/src/presets.rs`.
- **Implements:** Swift's table, `PresetSnapshot.swift`: preamps and pair map added; user volume and DAC mute as Swift treats them.
- **Done when:** a test covers each row against Swift's wording. **Deps:** `main`.

**CG8: Firmware guard.** [DONE 1a0019d] Size 0.75.
- **Owns:** `crates/dspi-core/src/firmware_guard.rs`.
- **How an image's voltage is read:** firmware with `core_vreg` boots at max(stored, build default) (`core_vreg.c:59-62`), so a stored or built 1.20 V holds. A stock image has no `core_vreg` code at all, so a stored value does nothing and it runs at 1.15 V. The guard scans the UF2 for the build's marker: its `binary_info` stamp, or a `core_vreg` marker. **Absent means unknown, treated as 1.15 V.** Today's images carry no such stamp, so all count as unknown; adding one to `bofr-main` is a small firmware item for the coordinator, and nothing depends on it.
- **Which boards are at risk:** the connected board reports a build default or live voltage above 1.15 V on `EXT_CORE_VREG`. A board already in the bootloader is unknown, so ask.
- **Rule:** at-risk board plus unknown or lower image: **blocked without typed confirmation.** Every updater calls it (QD11, TM3, AN18, SW-M2).
- **Done when:** tests cover K340-like, stock, no-extension and in-bootloader cases; exported to C and UniFFI. **Deps:** `main`.

**CG9: Compile the generated bindings in CI.** [DONE cdeb8ec] Size 0.5. **Wave 0**, since AN1 depends on it.
- **Owns:** `kotlin` and `swift` steps in `ci/check.sh`, the CI image's Containerfile, `crates/dspi-ffi/tests/bindgen_wp_f1.rs`.
- **Implements:** compile F1's Kotlin with `kotlinc` in the container. Compile the Swift with an official Swift toolchain in the container if it works there, otherwise as a VM step. Downloads pinned and verified. A Kotlin compile would have caught review 3-A's breaks.
- **Done when:** both steps pass, and a deliberately broken namespace fails them. **Deps:** `main`.

**CG10: Shared wording and onboarding data.** [DONE 7b10d71] Size 0.5. **Owns:** `crates/dspi-core/src/link_text.rs`, `assets/onboarding/{whatsnew,getting_started}.json`.
- **Implements:** one `LinkPhase` wording table as stable keys plus English: `NeedsPairing` (by `pin_source`), `NoDevice` ("Connected to hub, no DSPi found"), `ChooseDevice`, `Reconnecting`, `Refused`, `Closed`. Clients localise by key.
- **Done when:** every phase has text; exported to C and UniFFI. **Deps:** `main`.

**CG11: Core-voltage calibration.** [DONE ca059da] Size 1.5. Owner: writes go "under an advanced menu (we could put a 'calibrate core voltage' there as well)".
- **Owns:** `crates/dspi-session/src/vreg_calibrate.rs` (pure state machine; the caller passes status and the clock), `crates/dspi-core/src/vreg.rs`, the hang simulation in `crates/dspi-transport/src/fake.rs`.
- **Firmware facts it rests on** (`core_vreg.h`, `core_vreg.c`, extension channel spec §2.3):
  - `TRIAL` is RAM only. A hang (the 8 s watchdog) or a reboot returns to the persisted voltage, and the next boot reports `failed_trial_vreg` and `failed_trial_uptime_s`;
  - a trial's hang does not count toward automatic raising (`core_vreg.c:99-104`);
  - a power cycle clears that record, so the result is then unknown;
  - `SET` persists, and is refused below the build default;
  - RP2350 datasheet, as `core_vreg.h` records it: recommended to 1.16 V, absolute maximum 1.21 V, regulator ±3 %. **1.20 V is already outside the recommended range, and 1.25 V and above exceed the absolute maximum at the upper tolerance.**
  - K340 history: 5 hangs in about 9 min at 1.15 V, none in 41 min at 1.20 V. So a short soak proves little.
- **The flow, never a default path:**
  1. Reachable only from Advanced, only when `EXT_CORE_VREG` is present and the connection is writable. Never starts by itself, and never during first run.
  2. Explicit warning with the datasheet limits above and "audio may cut out and the device may drop off USB; lower the volume". **Typed confirmation for anything above 1.20 V.**
  3. Shows existing `hang_count` and failure, then `CLEAR`, so the result is unambiguous.
  4. **Range:** the firmware allows 1.15–1.30 V. The flow offers only the build default up to 1.25 V (`CORE_VREG_AUTO_CAP`). It never goes below the build default, and **1.30 V is not offered in any GUI** (Terminal CLI only). So on a K340 (default 1.20 V) it never tries 1.15 V.
  5. `TRIAL`, then a soak the user chooses: default 30 min, minimum 10, audio playing recommended. Abort at any time sends `TRIAL_END`.
  6. **Result, stated plainly:** passed (trial still active, uptime at least the soak); failed (`failed_trial_vreg`, how long it ran; offer the next step up, stopping at 1.25 V with "this looks like a hardware problem"); unknown (power cycled, or the record is gone: run again).
  7. **Never commits by itself.** After a pass the user chooses `SET` or `TRIAL_END`. `SET` is re-read and verified: `stored_sel` and `live_vreg`. Reset is `SET 0`.
- **Survives the app going away:** the host keeps no authority. On any connect it reads `trial_active` and offers end or keep, including after a Link drop or a phone in the background. **Link rule applies:** no write is replayed. Admin-only over a hub, so `Denied` hides it.
- **Also exposes** manual `SET`, `TRIAL`, `TRIAL_END`, `AUTO` and `CLEAR` for the Advanced page, with the same limits and confirmation. Exported to C and UniFFI.
- **Done when:** on `FakeDevice` with simulated hangs: pass, fail, step up, stop at the cap, unknown, abort, reconnect mid-trial, read-only and absent. Tests prove the flow **never sends `SET` without an explicit call, never trials below the build default or above 1.25 V**, and never panics on any status (`proptest_*`). **No hardware in this WP.**
- **Deps:** `main`.

**CG12: Output ring health.** [DONE 3296862] Size 0.25. From XC0 (M53).


- **Owns:** new `crates/dspi-session/src/ring_health.rs`, a binding in `crates/dspi-core/src/stats.rs`.
- **Implements:** Fill Target, Underrun Episodes, Refused Blocks, Lowest Fill, Worst Input Gap, Worst USB Gap and Over-Budget Steps, from `REQ_OUTPUT_LATENCY` index 2 with `REQ_GET_STATUS` 26–31 (reference `StatsView.swift:206-223,478-504`), plus the ring-scaled fill thresholds (`:1222`). In the session because the Terminal does not link `dspi-core`. Exported to C and UniFFI. SW14 uses it.
- **Done when:** tests cover a device without the status (no section), a `UInt32.max` lowest fill (no value yet) and a short reply. **Deps:** `main`.

**CG13: Input source names in the core.** [DONE 56c1a68] Size 0.25. From CG7. **Owns:** a source-name cache in `crates/dspi-core/` (connect, `NOTIFY_EVT_INPUT_SOURCE_NAME`, set), `source_title` in `presets.rs`. **Implements:** the user's name first in the preset diff's Input source row, as Swift's `inputSourceTitle`. **Done when:** a renamed source shows its name in `preset_changes`, updated by another client's notify. **Deps:** CG7.

**CG14: Confirmation word for raising the voltage.** [DONE ca059da] Size 0.5. Owner: "no per serial record, it will have to check the current settings in the unit, any upwards change to above 1.2v should require a confirmation word. So of the unit is already 1.25 and the user changes to 1.3 then they will have to type the word". **Owns:** one rule in `dspi-session`, applied in CG11's flow and manual ops, raw `core_vreg_op` in the core and C ABI, `dspi ext vreg`, and CG8's guard. **Rule:** target above 1200 mV and above the unit's current voltage, read at that moment, needs `OVERVOLT`. An unreadable current counts as needing it. No host-side record. **Accepted limit:** a K340 already on stock reports 1150 mV and is treated as that. **Done when:** the owner's example is tested at the transport on every path. **Deps:** CG8, CG11.

**CG15: Creator info for every About screen.** [DONE 4be11e7] Size 0.25. Owner: "Make sure to properly update the about screen with creator info." **Owns:** `assets/about.json`, an accessor beside CG10's in `crates/dspi-core/src/link_text.rs`, with the same placeholders. **Implements:** one table of who made what, the links and the credit line, so no client carries its own copy: Weeb Labs for DSPi, the firmware and the Consoles; the owner for the unified console, the core merge and the Android app; credits from `NOTICE.md` and `docs/licensing/`. The wording is the owner's to approve and stays a draft until they do; no agent writes legal text here. The Terminal doesn't link `dspi-core`, so TM22 takes the data another way without a second copy. **Done when:** exported to C and UniFFI, every client's page shows it. **Clients, 0.25 each, wave 2 or their About WP:** QD33 (`AboutPage.qml`), TM22 (`settings/about.rs`), AN26 (`GeneralSettingsTab.kt`), SW24 (the Mac's and iOS's About). **Deps:** CG10.

**CG16: Control surfaces on the demo device.** [DONE 83d02e5] Size 0.75. From review w1b. **Owns:** a control-surface model in `crates/dspi-transport/src/fake/`, moved from `apps/terminal/src/cs_demo.rs`, which goes. **Implements:** the record store and refusal rules (`NO_SUCH_SLOT`, `NO_SUCH_TYPE`, pin clash) checked against the firmware, so every client's demo has surfaces and QD1's, TM's and AN9's pages can be exercised. Existing tests that assert a stall (T4) change with it. **Done when:** `dspi cs` works without the wrapper; the Qt control-surface pages render on the demo device in the offscreen pass; the unexercised items in QD1's report are covered. **Deps:** TM10.

**CG17: One pair-map validity.** [DONE 8ef62f8] Size 0.25. From TM1 and QD2. **Owns:** `PairMap::is_valid` and K340 constants in `crates/dspi-proto/src/ext.rs`, `dspi_crossfeed_pair_map_valid`. **Implements:** replaces the copies in `screens/crossfeed.rs`, the core, the fake and `ExtController.cpp`. **Deps:** QD2.

**CG18: Core config export and import must not invent values.** [DONE 5de87b6] Size 0.75. From TM21. **Owns:** `crates/dspi-core/src/files.rs`, `config_import`. **Implements:** `config_export` writes zeros for sections an old format lacks, because the mirror comes from `decode_bulk`, and exports `default_state` while disconnected. Gate per key on `format_version`; refuse when disconnected. Import leaves omitted keys alone rather than defaulting them, and an absent `eq`, `crossover` or filter bank is not flattened (`channel_write`). Documents from the Mac and Terminal keep their meaning. **Done when:** round-trip tests on V17 and V32 fixtures, and the importer table in `docs/terminal/preset-format.md` loses its Qt row's exceptions. **Deps:** TM21.

**CG19: One home for the analyser's sharing rules.** [DONE 176fddf] Size 0.25. From TM5. **Owns:** `crates/dspi-core/src/rta.rs`. **Implements:** the core uses `dspi_session::rta_sharing`, keeping its epoch check. **Deps:** TM5, CG5.

**CG20: Control-surface editing rules in the core.** [DONE a4136af] Size 1.5. From AN8. **Owns:** an export of `dspi_session::cs_model`'s rules. **Implements:** the rules live in `cs_model`, `CsHelper.qml` and the Swift tab and have drifted: Qt offers Hold on a long or double press, which the firmware refuses (`control_surfaces.c:1736-1741`). Clients ask the core. **Before** AN9, SW12. AN sizes from AN8's spike: AN9 2-2.5, AN10 1, AN11 1, AN12 1.5-2, plus `cs_demo_learn` 0.25.

**CG21: One read per Stats tick.** [DONE 3fa04b9] Size 0.25. From review w2e. **Owns:** `crates/dspi-session/src/ring_health.rs`, the fill export. **Implements:** a tick reads `REQ_GET_BUFFER_STATS` once and derives fills from a cached ring target, invalidated on device change and on a latency write, as the Mac's one transfer per 60 ms. Android and Qt call it. **Deps:** AN25, QD29.

**CG22: Analyser and signal generator in the Rust fake device.** [DONE c1332e7] Size 0.5. From AN28: the Android demo now runs on the real core's demo device and lost the spectrum analyser and signal generator the Kotlin fake had. **Owns:** `crates/dspi-transport/src/fake*` (analyser frames and signal-generator state), their tests. **Implements:** the fake device answers the RTA and signal-generator requests like firmware V3 (bass bank 10-200 Hz plus FFT bands, packed frames), deterministic under the `clock` seam; the signal generator never starts by itself. **Done when:** core tests read RTA frames and drive the signal generator against the fake; the Android demo shows both again. Before the first Play update on the new core. **Deps:** AN28, CG20.

**CG23: Raw notification tap and connection generation.** [DONE a9002593] Size 0.5. From SW3 (`docs/SWIFT-SEAMS.md` section 6). **Owns:** `crates/dspi-core/src/raw.rs`, extended; one recording line in `notify.rs` beside `inner.monitor.record`. **Implements:** `notify_tap_set(enabled)` and `notify_tap_drain()`, giving each packet verbatim with `origin`, `own` (the echo filter's verdict) and `received_ms`, recorded where `process_notifications` drains so the mirror and the tap see one order; a bounded ring (512) that sets `lost` on overflow, off by default; `connection_generation()`. `CoreBackedTransport` needs it for `InterruptMonitor` and the `onX` callbacks; `NotifyResult` carries only flags. Retires when SW14 moves `InterruptMonitor` to `monitorRead` (SW-M4 removes it). **Done when:** on `FakeDevice` our own write's echo has `own = true` and a GPIO change `own = false`; a fake-hub test gives another session's origin; overflow sets `lost`; the generation changes on reopen and Link re-bind and never repeats. **Deps:** CG4.

**CG24: A device lock held across a flow.** [DONE a9002593] Size 0.5. From SW3. **Owns:** the lock section of `crates/dspi-core/src/presets.rs`; `learnArm`, `learnRead`, `learnCancel` in `cs.rs`; `enter_bootloader` (`hardware.rs`). **Implements:** `device_lock_acquire(reason, timeout_ms)` and `device_lock_release()` with Swift's semantics, held in `Inner` and checked against the connection generation; the core's own locked flows nest inside it; USB, MIDI and the demo device succeed at once. `learnArm` holds the lock for the 15 s learn window until a final `learnRead`, `learnCancel` or the timeout; `enter_bootloader` takes it. Covers the four Swift functions with no core path (IR learn arm and end, bootloader for install and its release); the same gap affects QD11 and AN18. **Done when:** in a fake-hub test another session's write gets `Locked` during a learn window and the bootloader restart; the lock is released on each exit and on disconnect; a nested `save_preset` does not deadlock; it is never held after its connection changes. **Deps:** CG20.

**CG25: Stream-state events in `NotifyResult`.** [DONE a9002593] Size 0.25. From SW3. **Owns:** `crates/dspi-core/src/notify.rs`, one flag in `consts.rs`. **Implements:** `NOTIFY_STREAMS` (bit 11) for `AdatState`, `I2sSlaveState` and `AdatInputState`, which the session decodes and the core drops today; the mirror follows `ADAT_STATE`'s `enabled` and `pin`, as the Mac does; a client re-reads ADAT out status or input lock on the flag. **Done when:** each event on `FakeDevice` sets the flag and the ADAT fields follow; Qt and Android are unchanged (a new bit they ignore). **Deps:** `main`.

**CG26: Host-supplied transport.** [DONE a9002593] Size 0.75. From SW3. **Owns:** a new `crates/dspi-core/src/host_transport.rs`. **Implements:** a UniFFI foreign trait `HostTransport` (`control_in`, `control_out`, `kind`) adapted to `dspi_transport::Transport`; `open_host(transport, serial)` through `open_transport`; `host_notify(packet, origin)` feeding that connection's notification reader; trait calls come on `coreQueue`'s thread and must not block for long. It lets the Mac's core reach the local device through the Swift hub's session 1 until SW-M3 (option A, section 5.4), and lets SW4's tests run with Swift's `FakeTransport` underneath. Retires at SW-M3, once the domain WPs have ported their byte tests. **Done when:** a scripted host transport gives the same `fetch_all` mirror as `FakeDevice`; writes go out in order; notifications reach the mirror; a transport that errors or hangs ends in `Disconnected` or `Transfer`, never a panic. **Deps:** CG23.

**CG27: Final firmware version on the demo device.** [DONE f6e74fa9] Size 0.25. From review c4c. **Owns:** `DEMO_FW_BETA` in `crates/dspi-transport/src/fake.rs`, the tests and fixtures that expect beta 4. **Implements:** the demo device reports final 1.1.6 (`DEMO_FW_BETA = 0`), not 1.1.6-beta4, which is older than the bundled images, so no client shows a stale-firmware banner on the demo. Android hides the banner in demo mode today; Qt and the Terminal do not. **Done when:** a core test reads the final version from the demo device; Qt's and the Terminal's demo runs show no firmware banner. **Deps:** CG22.

**CG28: Re-read the control-surface status after a display write.** [DONE 0cf648c5] Size 0.25. From review c4f (AN11's finding). **Owns:** the `displayConfig` and `displayPage` arms of `write_op` in `crates/dspi-core/src/cs.rs`. **Implements:** both arms re-read the CS status (`read_status_into`) as well as the display, so `status.dirty` and every client's save bar follow at once; AN11's extra `refreshStatus` can then go. **Done when:** a core test on `FakeDevice` shows `status.dirty` set straight after a display write, with no other read. **Deps:** CG20.

**CG29: About: the Links and support section is Weeb Labs'.** [DONE 2f1e95f3, via the release branch, review c4l] Size 0.25. **Owns:** `assets/about.json` and the About pages that show it. **Implemented:** the "Links and support" section names Weeb Labs' links, as theirs. Shipped in Android 0.12.0.

**CG29b: The support paragraph says "I (Weeb Labs)".** [DONE 2f1e95f3, via the release branch] Size 0.1. A wording fixup to CG29 in `assets/about.json`; AN26's test follows it.

**CG30: Analyser watching with nothing chosen.** [DONE 0f163dfa, review c5d] Size 0.5. From review c5b (c4j N2, AN39's overlay item). **Owns:** `RtaRequest` in `crates/dspi-core`, `dspi_rta_set_request_watching` in `dspi-capi`, the Android and Qt request senders. **Implemented:** a view on screen with an empty own selection keeps the core polling the analyser of a console it follows, and draws it, without asking for channels of its own (Swift's `isWatching`). The C record keeps its layout. Tests on the fake device and the fake hub.

**CG31: External writes reach every client's state; hub task XML.** [DONE 0f163dfa, review c5d] Size 0.5. From hw-win F1, F2. **Owns:** `dspi_proto::wire::sync_twins`, the patch paths in `dspi-session` and `dspi-core`, `dspi-hub-task.xml`. **Implemented:** the image holds an output's delay twice; a notified write patched one copy and every output page read the other, so a delay set by another client did not show. A patched value now updates its twin. The Windows hub task XML declares no encoding, so `Register-ScheduledTask -Xml` accepts it. Tests in proto, session, core and a Qt smoke check on the fake hub.

### 4.2 HB: the Rust Link hub

In scope on Qt regardless (owner), and used by Terminal and, later, the Mac. No OS or UI code in the crate, so a sideload Android build needs only a binding; no work is added for that now.

**Reference:** Swift hub files, 3 984 lines (`LinkServer`, `LinkHub`, `LinkSessionHandler`, `CommandRouter`, `AuthStore`, `PollScheduler`, `NotifyRelay`, `SnapshotCache`, `LinkPolicy`, `LinkService`, `DeviceRegistry`, `HubTransport`, `USBHubDevice`, `HubDevice`, `PairingPrompt`); the ESP32 hub; the spec; `fakehub.rs` (1 251 lines, test grade). Earlier studies said 10–15.

**The hard part is device ownership.** One `DspiCore` owns one connection today. A hub owns all local devices and serves many sessions, and the app must be one of them. Design: the hub owns the transports and the app opens an in-process transport through `open_transport`, with its own session id, so the echo rule is unchanged.

**HB0: Design and skeleton.** [DONE 79caa34] Size 2.
- **Owns:** `crates/dspi-hub/` (new), `docs/core-merge/HUB.md`, and the core changes it needs in `crates/dspi-core/src/{device.rs,hotplug.rs}`.
- **Implements:** threads, ownership, the in-process transport, interfaces for later HB WPs, `todo!` markers as in `PLAN.md`.
- **Done when:** the app-through-hub path passes a demo-device test. **A Fable review of the design passes before wave 2's HB1–HB3.** **Deps:** `main`.

**HB1: Server, handshake, auth, roles.** [DONE eabb407] Size 2. **Owns:** `crates/dspi-hub/src/{server,auth,store}.rs`.
- **Implements:** WebSocket server, hello, `/dspi/v1/info`, session limits. **PIN is the default; open access is opt-in.** Window PIN, tokens, rate limits, revoke; close codes 4001, 4002, 4003; ping every 10 s; 30 s dead-peer limit.
- **Origin check (WB0):** refuse a WebSocket whose `Origin` is not the hub's own page, per Link spec section 4. Without it any web page could reach the hub.
- **Done when:** L1's client and fixtures pass, including wrong PIN answered with `unauthenticated`, as real hubs do, and a foreign `Origin` is refused. **Deps:** HB0.

**HB2: Commands and notifications.** [DONE eabb407] Size 2.5. **Owns:** `crates/dspi-hub/src/{router,policy,lock,relay}.rs`.
- **Implements:** per-device ordered CMD with `max_inflight`; `policy/commands.json` by role; locks; NOTIFY with `origin` per session; resync. Documents how `0x00` is classed (upstream's policy lacks it, so admin-only).
- **Done when:** tests cover two clients seeing each other's writes, an own echo dropped, and a vanished client's lock released by timeout. **Deps:** HB0.

**HB3: Registry, snapshot, polls, rename.** [DONE eabb407] Size 2. **Owns:** `crates/dspi-hub/src/{registry,snapshot,polls}.rs`. **Done when:** C15's meter feed works against it; unplug and replug tests pass. **Deps:** HB0.

**HB4:** [MOVED] moved to later work: `fw.install`, which no hub implements.

**HB5: Advertising and web UI.** [DONE a871687] Size 1. **Owns:** `crates/dspi-hub/src/{advertise,web}.rs`, bundled assets. **Implements:** Bonjour through CG2's library; the MIT DSPi-Link web bundle at a pinned tag, **on only while sharing and bound to the hub's port.** **Done when:** the bundle's version and hash are pinned, and a browser test reaches the demo device. **Deps:** HB1, CG2.

**HB6: C ABI for the hub.** [DONE 9b6abcf] Size 1. **Owns:** `crates/dspi-capi/src/fns_hub.rs`. **Implements:** start, stop, settings, client list, role, revoke, pairing window, status listener. **Done when:** a Qt offscreen test starts a demo hub and the Terminal connects. **Deps:** HB1–HB3.

**HB7: Conformance.** [DONE 144f7057; [HW] `macr` half: see the status] Size 1.5. **Owns:** `crates/dspi-hub/tests/conformance/`. **Implements:** our client, the TypeScript client and the Swift client (VM) against it; L1's faults; fixtures. **Done when:** all three pass, and differences from the Swift hub are recorded. **[HW]**: `macr` (XC1). **Deps:** HB1–HB6.

**HB8: HB7's CI nits.** [DONE 2df60015, review c5a] Size 0.25. **Owns:** a `conformance-ts` step in `ci/check.sh`, `ci/container/images.conf`, `crates/dspi-hub/tests/conformance/run-clients.sh`. **Implemented:** the TypeScript conformance run is a gate step; its Node image digest is read from `images.conf`; `run-clients.sh swift` exits 3 without a GUI login, as `ci/macvm.sh` does.

---

## 5. Client tracks

### 5.1 Qt: Linux and Windows

Q3 makes it run on the core. macOS leaves this track, with Swift as the working assumption: **the Qt `.dmg` is to be a buildable, unpublished CI artefact until the owner's final call at OD3.** *(The swap, 2026-11-13: the rebuilt Qt app replaced the one this track built and has no macOS build; QD21b is dropped. The QD entries below are the old app's record; the rebuilt app was built by the QP work packages on the `qtport` branch; its design is `apps/desktop/README.md` and `apps/desktop/Documentation/behaviour.md`.)* Paths are under `apps/desktop/qt/`.

**QD0: Seam and C ABI pitfalls.** [DONE a6f2f15] Size 0.25. Do the pitfalls inside Q3 if it is still open. **Owns:** `src/ConfigFiles.cpp`, `src/DSPiBridge.cpp:1409-1433`, new `src/Registrar.h`, new `qml/BannerHost.qml`, `main.qml`.
- **Implements:** `ConfigFiles.cpp:17-21` retries after a short buffer, so an import applies twice: a reply too long is now a failure. `DSPiBridge.cpp:1409-1433` holds `state()` across writes: re-read it. The registrar and banner host let later WPs add without editing the bridge.
- **Done when:** a test shows one import applies once. **Deps:** `main`.

**QD1: `CsController` surfaces `error`.** [DONE db1b672] Size 0.25. **Owns:** `src/CsController.cpp`. **Done when:** a refused `cs_apply` shows its `error`. **Deps:** `main`.

**QD2: Pair-map editor and dimmed rows.** [DONE 31884c4] Size 1. **Owns:** `qml/CrossfeedWindow.qml`, `src/ExtController.{h,cpp}`. **Implements:** Default, K340 and Custom (reference `CrossfeedPairMapView.swift`), hidden on a stall. Dimmed rows show the preset's own values from the table the window holds. **Done when:** a demo test edits and reads back a map; an invalid pair is refused. **Deps:** QD0.

**QD3: Core-voltage status.** [DONE 119aa08] Size 0.5. **Owns:** a status block in `qml/settings/pages/OverviewPage.qml`. **Implements:** status; writes are QD28. **Done when:** live mV and hang count show on the demo device; hidden on stock firmware. **Deps:** QD2.

**QD4: Input-source and device names.** [DONE 31561ec] Size 0.5. **Owns:** source-name rows in `qml/settings/pages/InputsPage.qml`; a Device Name row in `qml/settings/pages/GlobalParametersPage.qml`, as on the Mac. **Done when:** edits round-trip on the demo device; another client's `NOTIFY_NAMES` updates them. **Deps:** QD0.

**QD5: Output latency, clock diagnostics, C0/C1 bars.** [DONE 1af6d8f] Size 0.5. **Owns:** an Output Latency row in `qml/settings/pages/GlobalParametersPage.qml`, as on the Mac (`DSPi_ConsoleApp.swift:1912`), `src/StatsController.cpp`, `qml/StatsWindow.qml`, `qml/components/CpuSection.qml`. **Done when:** options come from the device; a stall hides the section; both CPU bars show. **After QD4**, which adds to the same page. **Deps:** QD0, QD4.

**QD6: Read-only mode.** [DONE 3c32027] Size 1. **Owns:** `qml/components/ReadOnlyBanner.qml`, a `writable` bridge property, compat branch of `main.qml`. **Implements:** older and newer firmware show all settings, disable every control, keep reading, offer the update (`WIRE-FORMAT-COMPAT.md` §4.4). **Done when:** V17, V26, V31, V33 fixtures run offscreen; no setter is called (test); clip clear and bootloader still work. **Deps:** QD0.

**QD7: Demo mode.** [DONE f6788a5] Size 0.25. **Owns:** the demo entry through the registrar. **Done when:** selectable with no hardware; clearly labelled. **Deps:** QD0.

**QD8: Link controller and token store.** [DONE 4feb207] Size 1. **Owns:** `src/LinkController.{h,cpp}`, `src/TokenStore.{h,cpp}`. **Implements:** wraps CG1, off the UI thread, with `expected_hub_id`. Tokens in Credential Manager or Secret Service, falling back to a 0600 file with a warning; deleted only on `token_refused`. **Done when:** a fake-hub test covers pairing, reuse after restart, `wrong_hub`, and `rate_limited` keeping the token. **Deps:** CG1.

**QD9: Device list, discovery, PIN prompt.** [DONE fc0f2678] Size 1.5. **Owns:** `qml/DevicesPopup.qml`, `qml/PairingDialog.qml`. **Implements:** one list of USB, demo and hubs; non-blocking PIN prompt; every `LinkPhase`, CG10's text. **Done when:** runs offscreen against the fake hub. **Deps:** QD8, CG2, CG10.

**QD10: Lost-connection banner.** [DONE 874bd1ef] Size 0.5. **Owns:** `qml/components/LinkBanner.qml`. **Done when:** in a fake-hub fault test: banner at once, mirror readable, a write fails and is not sent later, abort works during a dial, analyser restarts. **Deps:** QD8, CG10.

**QD11: Firmware updater.** [DONE c079e03] Size 1. **Owns:** `src/FirmwareUpdater.{h,cpp}`, `qml/FirmwareUpdateWindow.qml`, `firmware/`. **Implements:** images per OD4; custom `.uf2` with `uf2_inspect`; CG8's guard; `firmware_version_matches` replacing `FwVersion` compare; export configuration first; hidden on STM32H723. **Done when:** offscreen tests cover wrong-chip file, guard and fake bootloader drive. **No real flash here. Deps:** CG8.

**QD12: Tray.** [DONE 912d034] Size 0.75. **Owns:** `src/TrayController.{h,cpp}`. **Implements:** Linux and Windows `QSystemTrayIcon`; close to tray; start minimised; the Mac's menu items (`MenuBarMode.swift:104-126`), including Share on This Network and Allow a New Client (`:117-120`); hub keeps serving. Those two stay hidden until QD14 registers its controller. **Done when:** an offscreen test covers the state machine, including no-tray fallback; checked once in `winvm` and on the host desktop. **Deps:** QD0.

**QD13: Open at login and keep awake.** [DONE fc0f2678] Size 0.5. **Owns:** `src/SystemIntegration.{h,cpp}`. **Implements:** XDG autostart and Windows Run key; sleep inhibit while sharing. **Done when:** unit tests check the written entry and the inhibit call; each is verified on its OS. **Deps:** QD12.

**QD13b: QD13 fixups.** [DONE b89d1733, review c4s2] Size 0.25. From review c4q (F1, N1). **Owns:** `src/SystemIntegration.cpp`, the pairing PIN. **Implemented:** the login entry of another copy of the program is left alone, not overwritten; the PIN is six digits.

**QD14: Networking page.** [DONE 874bd1ef] Size 1.5. **Owns:** `qml/settings/pages/NetworkingPage.qml`, `src/HubController.{h,cpp}`. **Implements:** `NetworkingSettingsView.swift`: share toggle, hub name, access (PIN by default), port, paired clients, roles, revoke, pairing code with expiry, other hubs, background options. Connects QD12's two hub menu items. Says a PIN crosses the network in plain text without TLS. **Done when:** offscreen on a demo hub, and a real client pairs. **Deps:** HB6, QD8.

**QD31: Save bar reports setter failures.** [DONE 8beebb1] Size 0.25. From QD1. **Owns:** `SettingsContext.save`'s bridge setters. A failed setter above the control-surface step must not read as saved. **Deps:** QD1.

**QD32: Qt on the shared wording.** [DONE 3e00d97] Size 0.25. From CG10. **Owns:** `apps/desktop/qt/whatsnew.json`, strings in `GettingStartedView.qml`. Re-points to CG10's data, and checks its menu paths against Qt's. **Deps:** CG10.

**QD34: Polling off the UI thread.** [DONE 9ed8574] Size 0.5. From QD6. **Owns:** `StatsController::pollSlow` and `pollFast`, `DSPiBridge::pollStatus`. **Implements:** blocking reads move to a worker; the smoke tests that rely on synchronous results change with them. **Deps:** QD6.

**QD35: Qt hand checks.** [TODO] Size 0.5. **[HW][OWNER][COORD]**. From the wave-2 log ("QD35 hand items"). **Owns:** a hand-check section in `docs/core-merge/HW-QT.md`. **Implements:** the checks the Qt WPs left for a person at a real desktop: QD12's tray (host desktop, then `winvm`); QD18's channel count (PipeWire at 8 channels, Windows sound settings, no audio played); QD30's popover cursor on KDE Wayland and X11; QD8's keyring on a real desktop, `winvm` and macOS; QD13's OS checks (GNOME and KDE autostart, `systemd-inhibit --list`; Windows Startup apps, `powercfg /requests`). QD11's hardware run list stays with QD24 and QD25. **Done when:** each item is recorded pass or fail with the OS and date, and each fail has a WP. **Deps:** QD8, QD12, QD13, QD18, QD30.

**QD36: Qt follow-ups from review c4h.** [DONE e6db6e70, review c4o] Size 0.5. **Owns:** the device list, the window shortcuts, the Networking page. **Implemented:** rescan and reconnect do nothing during a hub session, so no local device opens under a bound Link controller (F2); a rescan keeps the open demo and reads it again (F6); hidden windows' Escape and Return shortcuts work only while their window has the focus (F5); the Networking page says the hub shares a demo of its own (F4).

**QD37: Qt follow-ups from reviews c4o, c4q, c4s2.** [DONE 4cc9ff5a, review c5c] Size 0.5. **Owns:** `HubController`, a `hubSession` property in `DSPiBridge`, `CpuSection.qml`, `DevicesPopup.qml`, `AppUpdater.cpp`, `SystemIntegration.cpp`. **Implemented:** the hub watch state is a counter (closing Settings and the device list in either order); the CPU tooltip during a hub session; one unsaved-changes ask on a USB pick during a hub session; the built-in update feed is https only, a loopback http override is allowed; a symlinked or unquoted login entry for this program counts as this copy.

**QD38: Qt smoke time budget.** [DONE eaa11829, review c5f] Size 0.25. From gate-2 F1 (a 300 s timeout that passed on a rerun). **Owns:** the smoke driver and `run.sh`. **Implemented:** a watchdog per section (`DSPI_SMOKE_SECTION_LIMIT`, 120 s by default, exit 4 naming the section), section timings in the log, the total limit 300 s to 600 s. A planted hang is caught.

**QD30: Popover cursor on the graph gear.** [DONE c71da36] Size 0.25. **Owns:** `qml/components/PopoverWindow.qml`. **Implements:** owner's report on KDE Wayland: with a `PopoverWindow` open (graph gear; check the spectrum strip's too), the pointer over the gear doesn't return to the hand until wiggled, because the popup holds a grab. **Done when:** the cursor stays right without a wiggle and a click on the gear closes the popover once, on Wayland and X11; the owner retests. **Deps:** `main`.

**QD29: Output ring health.** [DONE cf09e5c] Size 0.25. **Owns:** the section in `qml/StatsWindow.qml`, `src/StatsController.cpp`, after QD5, which owns both in wave 1. **Done when:** the section and ring-scaled thresholds show on the demo device; hidden when the status is absent. **Deps:** CG12, QD5.

**QD15:** [MOVED] moved to later work: coach marks (owner: "not for now at least").

**QD16: App self-update.** [DONE fc0f2678] Size 1. **Owns:** `src/AppUpdater.{h,cpp}`. **Implements:** check our releases, compare, show notes, open the page. **The check is opt-in, with a privacy line saying what is sent.** In-place install only after OD8. **Done when:** tested on a local feed; off in distro packages. **Deps:** QD0.

**QD17: Shortcuts.** [DONE 874bd1ef] Size 0.5. **Owns:** shortcuts in `qml/main.qml`, `qml/SignalGeneratorWindow.qml`. **Reference:** the list in `DSPi_ConsoleApp.swift` around lines 11700–12100. **Done when:** every Mac shortcut has a Ctrl equivalent, in a test table. **Deps:** QD0.

**QD18: Host audio format.** [DONE c23f450] Size 1. **Owns:** `src/HostAudioFormat*.{h,cpp}`. **Implements:** idle channel count by WASAPI and by PipeWire or ALSA, falling back to the device's report (reference `HostAudioFormatMonitor.swift`). **Done when:** a fake backend tests the fallback; each OS is checked once. **Deps:** QD0.

**QD19: Analyser over Link.** [DONE 874bd1ef] Size 0.25. **Owns:** `qml/SpectrumAnalyserWindow.qml`. **Done when:** following state and Take control work on the fake hub. **Deps:** CG5, QD8.

**QD20:** [MOVED] folded into `docs/PARITY.md` rows (XC0, HO1).

**QD21: Packaging.** [MOVED] Size 1.5. Split by where it builds into QD21a and QD21b; the content is unchanged.

**QD21a: Packaging, Linux and Windows.** [DONE bbc5abae] Size 1. **Owns:** `apps/desktop/packaging/{arch,linux,windows}/**`, the QML-precompile lines in `apps/desktop/qt/CMakeLists.txt`. **Implements:** Linux on Qt 6 (raises the glibc floor), AUR source URL (`PKGBUILD`), QML precompile, installer check that the app is not running, keep Qt dynamic or add the LGPL relinking provision. **Done when:** the artefacts build. **Deps:** Q3.

**QD21b: Qt `.dmg`.** [DROPPED at the swap: no Qt macOS build] Size 0.5. VM. **Owns:** `apps/desktop/packaging/macos/**`, the `qt-dmg` row in `ci/matrix.sh`, `ci/macvm.sh`. **Implements:** the `.dmg` as a buildable, unpublished CI artefact. **Done when:** the `qt-dmg` row runs and passes. **SW-V1 needs it. Deps:** QD21a.

**QD22: Accessibility.** [DONE 709ded30] Size 2. **Alone in the last wave**: touches most of 131 QML files. **Owns:** all of `qml/`. **Done when:** `Accessible.*` on custom controls in `components/` and the windows; keyboard-only and screen-reader pass on Linux. **Deps:** other QD UI WPs.

**QD23: Localisation plumbing.** [DONE 5a5c89a8] Size 1.75. **After QD22. Owns:** all QML, `translations/`. **Implements:** `qsTr` and the `.ts` build, **English only for now** (owner). **Done when:** a pseudo-locale run shows no untranslated string or clipped layout in main windows. **Deps:** QD22.

**QD24: Hardware pass, Linux.** [PARTIAL] Size 0.5. **QD25: Windows.** [PARTIAL] Size 1. Both **[HW][COORD]**. Stock USB pass-through per `~/vm/windows/USB-PASSTHROUGH.md`, with the bootloader device attached too. **Owns:** `docs/core-merge/HW-QT.md`. **Done when:** connect, read, write, notifications, replug, firmware path and Link pass on the K340 under section 6's rules. **Deps:** QD0–QD14.
- **Record (2026-11-08):** `docs/core-merge/HW-QT.md`, from hw-1 (Linux) and hw-win (Windows). Pass: connect, read, write, reselect (Linux), detach and re-attach (Windows). **Still owed:** QD24's replug [OWNER] and both platforms' Link from the Qt client and firmware path (flash: coordinator and owner's go, backups first); notifications from another client (Linux not checked; Windows failed as F1, fixed in CG31, not re-run); the RTA items with a real signal; F3 (no smoke driver on Windows).

**QD27: AutoEQ on the core.** [DONE c1dcbb4] Size 0.5. **Owns:** `src/AutoEqLibrary.{h,cpp}` (453 lines), `qml/AutoEqRebuildWindow.qml`. **Implements:** re-point at CG6, with Qt supplying the fetch; remove the duplicated logic. **Done when:** existing behaviour passes against a scripted fetch. **Deps:** CG6.

**QD28: Advanced core voltage.** [DONE 2e0cc79] Size 1. **Owns:** `qml/settings/pages/CoreVoltagePage.qml`, reachable from `AdvancedPage.qml`, a `VregController`. **Implements:** thin UI over CG11: status, manual ops, the calibration flow, CG11's warnings. **Done when:** offscreen on the demo device with simulated hangs, every state shows; no path reaches a write without the confirmation; not reachable outside Advanced. **Deps:** CG11, QD3.

### 5.2 Terminal (`apps/terminal`, `crates/dspi-tui`, `crates/dspi-cmd`)

Owner decided: it skips what needs a graphical UI (on-graph editing, onboarding) and covers the non-graphical equivalents, including JSON import and export.

**TM1: Pair-map row.** [DONE cf20fcc] Size 0.5. **Owns:** `screens/crossfeed.rs`. **Done when:** Default, K340, Custom work on the demo device; hidden on stock; not confused with the `Pairs` mask row. **Deps:** `main`.

**TM2: Core-voltage screen.** [DONE 448dd56] Size 0.5. **Owns:** `settings/overview.rs`. **Implements:** TUI status; writes stay in the CLI, as they are. **Done when:** demo render test. **Deps:** `main`.

**TM3: Firmware flashing.** [DONE e0f0e52] Size 1. **Owns:** `apps/terminal/src/flash.rs`, `dspi-core` as a dependency for `firmware.rs` only. **Implements:** `dspi flash [FILE]` with `uf2_inspect` and CG8's guard. **`--force` does not override the guard**; only a separate `--accept-core-voltage-risk` flag plus the typed confirmation does. Suggests `dspi export` first. **Done when:** a fake bootloader drive test passes; `--force` alone is refused on an at-risk board. **Deps:** CG8.

**TM4: Read-only fallback.** [DONE fb103a0] Size 1. **Owns:** connect path in `crates/dspi-tui/src/shell/connect.rs`, `apps/terminal/src/main.rs`. **Implements:** `probe_any`, display in all screens, a firmware banner, writes refused in one place; `--json` reports `read_only`. **Done when:** V17, V31, V33 fixtures render; no write leaves (mock transport). **Deps:** `main`.

**TM5: Analyser overlay and following.** [DONE f1c0446] Size 1. **Owns:** `graph.rs`, `screens/spectrum.rs`. **Done when:** curves on the graph; following state and a take-control key work on the fake hub. **Deps:** CG5.

**TM6: Bar strip.** [DONE a521fc1] Size 0.5. **Owns:** `widgets/bar_strip.rs`. **Done when:** third-octave strip renders in ASCII and braille. **Deps:** TM5.

**TM7: Tube Basic/Advanced.** [DONE 0d65340] Size 0.25. **Owns:** `screens/tube.rs`. **Done when:** a toggle with Swift's row set. **Deps:** `main`.

**TM8: Lost-link status line.** [DONE 24575bf] Size 0.25. **Owns:** status line in `shell/mod.rs`. **Done when:** in a fake-hub test: status line, abort key, failed write not replayed. **Deps:** CG10. Wave 2, since CG10 lands in wave 1.

**TM9: Find hubs.** [DONE 8c274f0] Size 0.5. **Owns:** `dspi link scan`, picker in `shell/connect.rs`. **Done when:** a fake-advertiser test; `--json` output. **Deps:** CG2.

**TM10: JSON import and export.** [DONE bf35c3a] Size 1.
- **Owns:** `crates/dspi-session/src/preset_file.rs`, `cmd_export` and `cmd_import` in `apps/terminal/src/main.rs`, a `cs` command.
- **What exists:** `.dspipreset`, a filter text format, `dump --json`, `--dry-run`.
- **Gaps (R1's check):** format is chosen by extension only, so `dspi export x.json` writes the filter text; no `-` for stdin or stdout; `--json` has no effect on import or export; filter banks have no JSON form; the preset document has no control-surface block and appears to lack the pair map (compare `dspi_core::files` and `config_import`); `import_preset`'s doc is stale; Save Master Volume and Save Output Configuration to be checked.
- **Done when:** round-trip with Qt's and the Mac's documents in the shared fixtures; `dspi cs export|import` work on the demo device. **Deps:** `main`.

**TM11: `dspi hub`.** [DONE f19a837] Size 1. **Owns:** `apps/terminal/src/hub.rs`, `apps/terminal/packaging/service/`. **Implements:** a headless hub with pairing, clients and roles as CLI commands, and sleep inhibit. Owner: "include some placeholder systemd service / windows background / mac eq". So three documented placeholders with install notes: a systemd user unit, a Windows scheduled task (with a note on a service wrapper), and a launchd plist. **PIN pairing stays the default in all three.** **Done when:** a Link client connects; `systemd-analyze verify` passes in a container, `plutil -lint` in `macvm`, and the task imports in `winvm`. **Deps:** HB6.

**TM12: AutoEQ update.** [DONE 8730758] Size 0.25. **Owns:** `screens/autoeq.rs`, `crates/dspi-cmd` command. **Implements:** CG6, removing the `curl` call. **Done when:** update, rebuild and reset work on a scripted fetch, with no external process. **Deps:** CG6.

**TM13: K340 hardware pass.** [PARTIAL] Size 0.5. **[HW][COORD]**. **Owns:** `docs/core-merge/HW-TERMINAL.md`. **Done when:** USB replug and real MIDI ports end to end pass (R1's open item). **Deps:** TM1, TM2, TM4. **State:** hw-1 ran these checks on the K340 (2026-11-08, `work/hw-1/REPORT.md`): pass, bar the RTA items, which need audio. Its two defects were fixed in TM26. Record: `docs/core-merge/HW-TERMINAL.md` (2026-11-08). Owed: USB replug with a physical unplug (hw-win replugged only by VM detach).

**TM15: Closed pipes.** [DONE 0ee1112] Size 0.25. **Owns:** output handling in `apps/terminal/src/main.rs`, under its marker. **Implements:** `dspi --demo dump | head -5` panics with "failed printing to stdout: Broken pipe". Handle a closed pipe across all CLI output: stop quietly, with the usual exit status. **Done when:** a test pipes every command that prints into a closed reader, with no panic. **Deps:** `main`.

**TM19: Multi-line Save.** [DONE 6b970ef] Size 0.5. From TM17, confirmed by review. **Owns:** `set` and `run_commands` in `crates/dspi-tui/src/live.rs`, the save path in `settings/mod.rs`. **Implements:** a Save with several Flash-hazard lines dropped all but the last, silently. One confirmation naming all; all written in order; unwritten edits stay in the draft. `cs_dirty` keys on the request, not the page. **Deps:** `main`.

**TM20: Tokenizer and latency confirm.** [DONE a5fd081] Size 0.5. From TM17, confirmed by review. **Owns:** `tokenize` in `crates/dspi-cmd/src/lib.rs`, `out.latency` confirm in `crates/dspi-session/src/write.rs`. **Implements:** empty quoted token and escaped quote; confirm polls `pending` as the Mac does. Hardware check in TM13. **Deps:** `main`.

**TM21: Preset capture must not invent values.** [DONE fb103a0] Size 0.5. From TM4. **Owns:** `capture` in `crates/dspi-session/src/preset_file.rs`. **Implements:** `capture` turns any refused or failed read into 0, false or a default, and an import then writes them: on old firmware a -12 dB master exported as 0 dB. A failed read fails the export; a refused read on a read-only device uses the mapped image or omits the key; import leaves omitted keys alone and warns on an old `meta.wire_format_version`. **Deps:** TM4, TM10.

**TM23: Settings page drawing.** [DONE 3eb7665] Size 0.25. From TM22, confirmed by review. **Owns:** `draw_page` and `Row::Pair` in `settings/mod.rs`. A multi-line row straddling the top redraws from its first line; `Row::Pair` aligns by characters, not columns. **Deps:** `main`.

**TM24: Terminal control surfaces on `cs_rows`.** [DONE 06c7af9] Size 0.5. From CG20. **Owns:** the Terminal's control-surface page. **Implements:** the Terminal asks the core's `cs_rows` editing rules instead of its own copy. **Done when:** tests show the Terminal offers exactly what `cs_rows` allows (no Hold where the firmware refuses it). **Deps:** CG20.

**TM25: Flash after CG24's device lock.** [DONE 2df60015, review c5a] Size 0.5. From the dispatch: `main`'s test step was red since c4i. **Owns:** `apps/terminal/src/flash.rs`, `flash/tests_wp_tm3.rs`. **Implemented:** the flash reboots a read-only board into the bootloader again (CG24 had made `enter_bootloader` take the device lock, and the Terminal dropped the error); a refused reboot is said at once instead of a 60 s wait for a drive.

**TM26: Fixes from the K340 run hw-1.** [DONE 2df60015, review c5a] Size 0.5. **Owns:** the Terminal's argument parsing, `WriteError`, the `cs.aux` reads, `dspi hub`'s start order and logger. **Implemented:** a boolean switch before the command (`dspi --dry-run set ...`) runs the command instead of opening the interface, and the interface refuses cleanly without a terminal (D1); reading an action says it is write-only (D2); a stalled `cs.aux` read names why (M1); `dspi hub` binds, attaches, then accepts, so no early client hears "No DSPi found" (M2); the hub logger caps tungstenite's level instead of raising it (M3).

**TM14: No tmpfs in tests.** [DONE fcfdb58] Done: fcfdb58 had already done it. Size 0.25. **Owns:** tests in `crates/dspi-tui/src/{actions.rs,settings/config.rs,settings/mod.rs}`. **Implements:** five `temp_dir()` uses move to a target-dir path. **Done when:** none remain, and tests pass. **Deps:** `main`.

**TM16: Output ring health.** [DONE 6d33b8b] Size 0.25. **Owns:** `screens/stats.rs`. **Done when:** the section and ring-scaled thresholds render on the demo device; absent status hides it. **Deps:** CG12.

**TM17: Device names and latency in the TUI.** [DONE 5e3a386] Size 0.5. **Owns:** `settings/global.rs`, `settings/inputs.rs`. **Implements:** Device Name and Output Latency on Global Parameters and source names on Inputs, over the existing `dev.name`, `out.latency` and `in.name`. **Done when:** edits round-trip on the demo device; hidden when absent. **Deps:** `main`.

**TM18: Volume mode, Identify, tube view.** [DONE 6c179d6] Size 0.25. Also remembers TM7's tube Basic/Advanced choice (`live.rs:139` rebuilds the panel on open). **Owns:** the `VolumeModeToggle` arm in `crates/dspi-tui/src/live.rs`, `settings/config.rs`, `identify_command` in `screens/mod.rs`. **Implements:** the volume mode is saved; Identify as `Commands.swift:3688-3704`: -12 dB, two passes, clamped to the valid-channel mask. **Done when:** a test restores the mode after a restart; a test checks Identify's commands. Wave 2, since TM14 owns tests in `settings/config.rs` in wave 1. **Deps:** `main`.

### 5.3 Android (`apps/android`)

The swap is contained: UI talks to the `core-api` interface, and only `ffi` sees UniFFI. Owner: the next Play update waits for this.

**Fast lane, independent of Qt:** AN1 → {AN2, AN3, AN6} → {AN4, AN5} → AN13 → AN14.
- **What happened:** the first update on the new core, 0.12.0 (AN14, tag `android-v0.12.0`), shipped AN1-AN6 and the floor
  WPs merged by then, on the owner's partial phone checks (AN13, below). The next releases are AN15's R8 (AN22) and then AN16,
  one change per release.
- XC2 must pass before Link ships against a Mac hub. **It depends on XC1, which needs the owner's go-ahead for the `macr` restart.**
- **So the lane cannot stall silently:** AN13 runs its USB half first, with no hub. Its Link half runs against XC2's Mac hub or the ESP32 bridge (XC4). If neither is available, the coordinator reports that. (0.12.0 shipped with the Link half still owed; it is the owner's, with the phone.)

**AN1: New crates in place of `dspi-ffi` and the patched core.** [DONE 684e9f1] Size 2.5.
- **Owns:** `rust/` (removed), `third_party/` (removed), `tools/{stage-core.sh,gen_kotlin_contract.py,opcode_coverage.py,REFERENCE.toml}`, `ffi/**`, `core-api/**`, `NOTICE.md`, generated licences.
- **Implements:**
  - use `crates/dspi-ffi` by path;
  - rewrite the generator to read the core's modules, keeping its `--check`;
  - bindgen once per namespace (`ffi/build.gradle.kts:74-77`);
  - `FfiDspiCore.kt:57`: `init_logging` from `se.bofr.dspi.ffi.uniffi.logging` (also `DspiApplication.kt`);
  - `FfiDspiCore.kt:393`: `ReadOnly` and `Io` in the `when`, and in `core-api`'s `DspiException.kt`;
  - `FfiDspiCore.kt:65-77, 383-387`: Link methods as C15 leaves them, including `expected_hub_id`; `on_link_status_changed` and `on_link_token_issued` present (behaviour in AN3);
  - `NOTIFY_STALE` in `DspiConstants`; `SystemStatus.stale` and `age_ms`; `MidiInputHandle.close_port`; `MidiSender` unused here;
  - drop the `wip/wire-compat` branch.
- **Done when:** `tools/ci.sh all` and all Kotlin module tests pass, on 3 ABIs; library size recorded. **No phone yet. Deps:** MR1, CG9.

**AN27: No title bar.** [DONE 0ec2801] Size 0.25. Owner: the "big fat dspi banner on top of the Android app" is wordy, and "while a title bar makes sense on a computer it is less useful on a mobile device". **Owns:** `AppScaffold.kt:76`. **Implements:** remove the `TopAppBar`, which holds only the title; demo state stays visible through `ConnectionStripBar`; edge-to-edge insets still correct. Screens with their own back and title rows are unchanged. **Done when:** Home starts at the connection strip on the emulator or a Compose test, and demo mode still says so. Goes out with the first update after the new core. **Deps:** AN1. SW checks the same on iOS when SW15 reaches it.

**AN28: Demo mode on the real core.** [DONE f7cbc97] Size 0.5. From AN8. The app's demo runs on the Kotlin `FakeDspiCore`, so CG16's surfaces and the core's `fill_band` never reach it. Route the demo to the real core's demo device. **Deps:** AN1.

**AN29: USB prompt once, not at every plug-in.** [DONE 9fa266c] Size 0.5. From a user report (`work/bug-android-1/REPORT.md`). Android keeps a USB grant only until the device is unplugged; only the "Always open" tick in the system dialog makes it stick, and USB access never shows in the permission manager. The app makes it worse: 1.5 s after an attach broadcast it opens its own `requestPermission` dialog over the system's "Open with" dialog (`UsbSessionEngine.kt` `GraceExpired`), so the tick is never seen. **Owns:** `usb/.../UsbSessionEngine.kt`, `usb/.../AndroidUsbSession.kt`, the permission hint string (added to `ConnectionStrip.kt` under a `// -- WP AN29 --` marker, since AN5 has not started). **Implements:** an attach broadcast while the UI is visible never opens the app's own dialog after the grace period; it settles on `PermissionRequired` (the existing "Allow USB access" button). Cold start keeps one request. The pending or required strip says that ticking "Always open" in Android's dialog stops the prompt; no host-device nouns. **Done when:** engine tests with the fake `UsbHost`: attach broadcast, UI visible, no activity intent within the grace gives zero `requestPermission` calls and `PermissionRequired`; attach activity intent with permission connects with no dialog; cold start with a device attached makes exactly one request. [HW] tick "Always open" once, re-plug 3 times: the app opens connected with no dialog. In the first update on the new core. **Deps:** AN1.

**AN30: Graph spectrum overlay draws the selected channel.** [DONE 9522048] Size 0.25. From the same user report ("the spectrum doesn't go below 200 Hz"). `GraphSpectrumOverlay.kt` `buildTrace` tests the band mask `populated` with a channel number and indexes the core's packed `frames` by channel number, so any single channel but the first draws its 10-200 Hz bass-bank bands from an empty frame at the floor. **Owns:** `feature-graph/.../GraphSpectrumOverlay.kt` and its tests. **Implements:** find the channel's frame among `frames[0 until count]` by `frame.channel`, as `feature-analysis` `rtaFrame` does; `bandPoints` leaves out unpopulated bands like the Mac's `RtaRenderCache.visibleBands`. **Done when:** tests on a packed snapshot: single channel 2 draws channel 2's levels with the columns below 200 Hz off the floor; selection [1,3] draws two curves from the right frames; order 8 at 48 kHz has no band point at 251, 316 or 501 Hz. In the first update on the new core. **Deps:** none.

**AN31: A visible way into Tools.** [DONE e323750] Size 0.5. From the same user report: the effects not on the strip (Tube Modeller, Subharmonic, Upmixer) are reachable only by an unexplained long-press on four strip icons. The Windows console's top-left menu is the Mac's Tools menu (`DSPi_ConsoleApp.swift` `CommandMenu("Tools")`). **Owns:** `feature-home/.../ContentView.kt` (`SidebarBottomPanel` and the Home top row), the entry in `feature-tools/.../ToolsScreen.kt`. **Implements:** a menu button on Home (no title bar: AN27) opening the Tools hub with the Mac Tools menu's items; a long-press on a strip icon opens that icon's own tool page, as the Mac's right-click does, not the hub. **Done when:** a Compose test reaches Tube Modeller, Subharmonic and Upmixer from Home with taps only; a long-press on crossfeed opens the crossfeed page; `docs/PARITY.md` M60 notes it. In the first update on the new core. **Deps:** AN27.

**AN32: Release fixes for Android 0.12.0.** [DONE dd89b4c] Size 0.5. From the 0.12.0 release check. On `main` since 2f1e95f3 (the release branch merged back). **Owns:** version and Play notes, `DspiRoutes.kt` (`ANALYSIS_PAGE`), graph axis labels, the analyser's pickers. **Implements:** version 0.12.0 (versionCode 12); each Tools analysis item opens its own page; overlapping axis labels left out; analyser pickers on two rows below 600 dp. **Done when:** `ToolsAnalysisEntryAn32Test` (renamed from `AnalysisEntryAn32Test` in abada542, to dodge the test isolation fault AN38 fixed), `SpectrumSegmentsAn32Test`, `GraphAxisLabelsAn32Test` pass. **Deps:** AN31.

**AN33: No host-device nouns in the Android texts.** [DONE af2d269] Size 0.25. From review c4a (O1). On `main` since 2f1e95f3. **Owns:** the Link prompts, the Networking page ("This Phone" becomes "Local"), the remote-firmware notice, the firmware updater, the Play listing. **Implements:** the owner's rule that user-visible text never names the host device. **Done when:** `NoHostNounsAn33Test` scans every string literal, `res/values*` and the Play texts. **Deps:** AN5.

**AN34: The analyser bar strip on screen.** [DONE f6e74fa9] Size 0.5. From review c4c: AN17's strip is not reachable in the app (M22). **Owns:** `feature-home/.../ContentView.kt`, `ChannelScreen.kt`, `feature-home/build.gradle.kts` (depends on `:feature-analysis`), an "RTA Bars" toggle in `feature-graph/.../GraphOptions.kt`, the graph overlay. **Implements:** AN17's strip placed at the call sites its report lists; the overlay draws `drawn(own, followed)`. **Done when:** a Compose test on the demo device shows the strip on Home and the channel screen after the toggle; a followed selection is drawn without taking control (fake hub); M22 in `docs/PARITY.md` is user-visible. **Deps:** AN17, AN7.

**AN35: AN18's remaining items.** [DONE 0cf648c5] Size 0.75. From review c4c (AN18 partial by scope). **Owns:** key handling in `AppScaffold.kt`, the strip layout in `feature-home` (`ContentView.kt`, `InputChannelHeader.kt`, `ChannelSettings.kt`). **Implements:** the hardware shortcuts (M24, about 30, Space, M45's channel clipboard keys) beside AN18's `PeqKeyMap.kt`; `HostAudioFormat`'s channel count wired into the strip layout and refreshed on `AudioDeviceCallback` (M25). **Done when:** a test table covers every shortcut; a test with a fake `HostAudioFormat` shows the strip follow the count. **Deps:** AN18, AN34 (same files).

**AN36: AN19 and AN5 fixups.** [DONE fc0f2678, second part 2f1e95f3] Size 0.25. From review c4a (F1, N1). **Owns:** the seed in `ui-common/.../DataStoreAppSettingsStore.kt`; the hint in `app/.../LinkDialogs.kt`; an error code on `NeedsPin` in `link/.../LinkSessionEngine.kt`. **Implements:** the onboarding seed computed after the one-time import of the legacy settings files, so a user with only those files skips Getting Started; the wrong-PIN hint shown only for `wrong_pin`, not for `rate_limited` or other errors. **Done when:** `wpAn19_someoneWhoUsedTheAppBeforeSkipsSetup` covers the legacy-only case; a test shows the hint for `wrong_pin` and not for `rate_limited`. **Deps:** AN5, AN19; after AN33 reaches `main` (same dialog texts).

**AN37: A channel page's bar strip shares the gear's selection.** [DONE 0cf648c5, review c4k] Size 0.25. From review c4j. **Owns:** the channel page's strip in `feature-home`, the graph gear's spectrum selection. **Implemented:** the bar strip on a channel page and the graph gear use one spectrum selection, so they cannot disagree.

**AN38: Analyser wake: one bindings copy per test JVM.** [DONE 172aa93d, review c5b] Size 0.5. Test-only. **Owns:** `DspiRobolectricTestRunner`, `app/src/test/**`. **Implemented:** the Rust library keeps one callback table per listener interface for the process; a plain-JVM test loading a second copy of the UniFFI bindings took it over, and every later Robolectric demo core lost its analyser wakes. The runner keeps the bindings out of the sandbox. On a phone the app has one class loader, so it was never affected.

**AN39: Android small follow-ups.** [DONE 172aa93d, review c5b] Size 0.5. **Owns:** `feature-graph`, `feature-settings/.../cs/`. **Implemented:** AN11's extra `REFRESH_STATUS` dropped (CG28 made it redundant, c4k N1); `lintDebug` clean in `feature-graph` (c3b). Its overlay item (draw a followed spectrum with an empty own selection) needed the core and was done in CG30.

**AN2: Read-only on older firmware.** [PARTIAL 40463af] Size 0.5. **Owns:** `feature-home/.../HomeModel.kt`, `README.md`. **Implements:** V17–V31 now decode; `WriteGuardedCore` stays; fixes the README claim. The banner still fires on the wire format only: **the first Play update must show no version banner on a K340 or on stock final firmware** (the comparison is AN18's). **Done when:** fixture tests; banner per `WIRE-FORMAT-COMPAT.md` §4.4. **Deps:** AN1.

**AN3: Link engine on the core's supervisor.** [DONE 4901541] Size 1. **Owns:** `link/.../{LinkSessionEngine,LinkCoreOps}.kt`.
- **Implements:** delete Android's own reconnect, backoff and token deletion, which would fight C15; follow `link_status`; implement `on_link_status_changed` and `on_link_token_issued`; delete a token only on `token_refused`; pass `expected_hub_id`; handle `wrong_hub`; remove `setListener` from inside callbacks (`LinkSessionEngine.kt:164,178,214`); `rta_start` again after a reconnect; dial off the serial FFI queue.
- **Done when:** `link` tests are rewritten against the fake hub with L1's faults; `rate_limited` keeps the token. **Deps:** AN1.

**AN4: Network handling.** [DONE 10fd095] Size 1. **Owns:** `link/.../{LinkNetworkMonitor,NsdHubBrowser,LinkSettings}.kt`. **Implements:** `onLost` and network change call `link_retry_now`; browse while reconnecting and re-resolve via `link_set_url` (F5); test F10's items. `WifiLock` needs no new permission; **re-check Data safety if sockets get bound to Wi-Fi.** **Done when:** unit tests with a fake monitor; F10 items recorded as used or rejected. **Deps:** AN3.

**AN5: Link UI.** [DONE 4147f5e] Size 1. **Owns:** `ui-common/.../LinkBanner.kt`, `app/.../{LinkDialogs,AppNetworkingController,ConnectionStrip}.kt`. **Implements:** owner's banner on every screen; wrong-PIN by `pin_source`; "no DSPi found" (F9), CG10's text. **Done when:** Compose tests per `LinkPhase`. **Deps:** AN3, CG10.

**AN6: Settings rows.** [DONE 9940f64] Size 1. **Owns:** `feature-settings/.../{GeneralSettingsTab,PinOverviewTab}.kt`, source picker in `feature-home`. **Implements:** core voltage status, device name, input-source names, output latency. **Done when:** demo tests round-trip; hidden when absent. **Deps:** AN1.

**AN7: One device list.** [DONE 0d33076] Size 1. **Owns:** `app/.../{AppMode,EntryNavigator,UsbStartGate}.kt`. **Implements:** a user's device switch calls `withUnsavedChanges` first (M42). **Done when:** USB, Link and demo in one list; demo reachable with a DSPi attached; a test shows a switch with a dirty preset asks first. **Deps:** AN3.

**AN8: Control-surface spike.** [DONE a85c29c] Size 0.5. **Owns:** a report. **Implements:** a phone layout against Swift's 5 270-line tab and Qt's `settings/cs` and pages. Sets AN9–AN12's sizes. **Done when:** a reviewed screen map exists. **Deps:** AN1.

**AN9: CS bindings.** [DONE 012a0f3] Size 2–4. **AN10: IR learn.** [DONE 2026c83] Size 1. **AN11: Display pages.** [DONE 0a6125c] Size 1–2. **AN12: Groups, macros, aux.** [DONE 0863194] Size 1.5–2.5. **Owns:** one file set each under `feature-settings/.../cs/`. **Implements:** `cs_snapshot`, `cs_apply`, `cs_pin_uses`. **Done when:** runs on the demo device (`FakeDevice`'s CS support looks minimal: check, and extend); IR and display need **[HW]**; an `error` reply is shown. **Deps:** AN8.

**AN13: Phone test.** [PARTIAL] Size 1. **[HW][OWNER][COORD]**. **Owns:** `docs/HIL-NEW-CORE.md`. **Requires** the owner's adb port. **Done when:** the earlier HIL set passes on the nusb `from_fd` path (unproven on hardware): USB, notifications, old firmware, Link with a Mac hub, 45 s background, no secrets in logcat. Side by side with `applicationIdSuffix ".hil"`: never uninstall the Play build. **Deps:** AN1, AN2, AN3; USB half needs nothing more; Link half needs XC2 or XC4.
- **Record (2026-11-08):** 0.12.0 shipped on the owner's partial checks: the owner passed AN29, AN30 and USB basics on the phone, and phone-test-2 passed the demo set (the `macr` hub was visible from the phone). Record: `docs/HIL-NEW-CORE.md`. **Still owed:** the Link half with the phone, older firmware, 45 s in the background, no secrets in logcat on USB, and notes on the nusb `from_fd` path [HW][OWNER].

**AN14: First new-core update.** [DONE ca6d52f3, tag `android-v0.12.0`; 0.12.0 uploaded to Play] Size 0.5. **[OWNER]**. **Owns:** version, notes, store text. **Implements:** the unreleased crossfeed pair fix ships here; README; versionCode bump; source tarball on the release (OD1). **Done when:** an AAB is ready for the owner to upload. **Deps:** AN13, R-And.

**AN15: R8.** [DONE b34ec774] Size 1. **Owns:** `app/build.gradle.kts`, `app/proguard-rules.pro`, consumer rules in `ffi/` for UniFFI and JNA. **Done when:** a release build passes a phone test **[HW]**. Play deadline February 2027; own release. **Deps:** AN14.

**AN16: Edge-to-edge.** [TODO] Size 1. **Owns:** `app/.../MainActivity.kt`, insets. **Done when:** every route checked on the phone **[HW]**. Own release, after AN15's is uploaded. **Deps:** AN14, AN22.

**AN17: Analyser bar strip and following.** [DONE 35339d2] Size 0.75. **Owns:** a bar-strip file and overlay state in `feature-analysis/`. **Done when:** Compose tests on the demo device; following and Take control work on the fake hub. **Deps:** AN1, CG5.

**AN18: Small floor items.** [DONE 44d9768; its moved items in AN35] Size 1.25. **Owns:** `feature-firmware/`, the version comparison in `feature-home/.../{HomeModel,FirmwareMismatchBanner}.kt`, AutoEQ update UI, a host-format reader, `feature-tools/` generator lifetime, graph keyboard map. **Implements:** images per OD4; **CG8's guard called on Android's Kotlin flashing path, PICOBOOT included**; export first; CG6's update with a Kotlin fetch; `AudioManager` channel count; generator keeps running off-screen; hardware shortcuts; the mismatch banner compares firmware versions like the Mac (`DSPViewModel.swift:2099-2106`), arriving with the updated images (M41). **Firmware built with our extension channel counts as acceptable:** the K340's `bofr-main` build reports a matching version plus a build stamp, so it must not get a false "update" prompt. **Done when:** a unit test proves the flash path cannot skip the guard; a test shows the banner for another version on the same wire format, and none for a matching version with a build stamp; each item has a test. **Deps:** AN1, CG6, CG8.

**AN19: Getting Started and What's New.** [DONE a116312] Size 1. **Owns:** `feature-onboarding/`. **Done when:** first run shows once; replay from Help; data from CG10. **Deps:** CG10.

**AN20: Accessibility.** [DONE 3e3583e7] Size 2. **Owns:** semantics across `feature-*`. **Done when:** a TalkBack pass on graph, matrix and meters **[HW]**. **AN21: Localisation plumbing.** [TODO] Size 1.75, likely 2.5: only 2 `strings.xml` exist, so the Compose literals move first. **Owns:** `strings.xml` across modules, **English only for now** (owner). **Done when:** a pseudo-locale run shows no hard-coded string. Both alone in the last wave. **Deps:** other AN UI WPs.

**AN22: The AN15 release.** [TODO] Size 0.25. **[OWNER]**. Re-scoped: its old content (AN4–AN6) shipped in 0.12.0. **Owns:** version, notes. **Implements:** the release that carries AN15's R8 on its own, and AN13's Link half if it is done by then. **Done when:** a phone re-test of the release build passes **[HW]**, and an AAB is ready for the owner to upload. **Deps:** AN14, AN15.

**AN23: Advanced core voltage.** [DONE 9d0df07] Size 1. **Owns:** `feature-settings/.../CoreVoltageScreen.kt`, reached from `AdvancedSettingsTab.kt`. **Implements:** thin UI over CG11. The soak survives the screen going off or the process dying, by re-reading `trial_active`. **Done when:** Compose tests on the demo device with simulated hangs; not reachable outside Advanced. **Deps:** CG11, AN6.

**AN25: Output ring health.** [DONE ced3bd8] Size 0.25. **Owns:** the section in `feature-analysis/.../StatsView.kt`, the stats controller in `bridge/`. **Done when:** a Compose test shows the section and ring-scaled thresholds on the demo device; hidden when absent. **Deps:** CG12, AN1.

### 5.4 Swift: macOS and iOS (`apps/apple`)

Owner: **SwiftUI on macOS and iOS over the Rust core; Swift keeps UI only**, with Swift as the working assumption and a final call after visual evidence (SW-V1, OD3). One line, one Xcode project. **Every WP is done only when both targets build and test.** SW numbers match the earlier IO numbers. USB MIDI is the only USB transport on iPhone and iPad: no DriverKit. Builds run in `macvm`, one heavy job at a time, GUI session only (`ssh macvm who`, `launchctl asuser`), host unlocked. The VM has no GitHub credentials: use git bundles.

**The seams, and the risk.**

| Layer | Size today | Coupling |
|---|---|---|
| Views | most of 72 k lines | call `DSPViewModel`; 35 files hold an observed object |
| `DSPViewModel` | 3 367 lines, 124 `@Published` | Stays the views' API |
| `Commands.swift` | 4 929 lines, 321 functions, 289 wire references | **Removed** |
| Wire use elsewhere | `StatsView` 41, `DSPi_ConsoleApp` 29, `SpectrumAnalyser` 19, `TestSignalsView` 18, `ContentView` 14, others under 8 | Moves with its domain |
| `DeviceTransport` | one protocol | **The seam for coexistence** |

**Approach: strangler behind `DeviceTransport`.** First `CoreBackedTransport` puts the core under all existing Swift, using CG4. That alone gives M0. Then domains move one at a time, each keeping the view model's API and `@Published` properties.

**Risks to resolve in SW3 and SW8:**
- **Two mirrors.** `@Published` and `DeviceState` must not disagree. Swift writes optimistically; the core's mirror follows confirmed writes. The slider fast path (`Commands.swift:257`) is the sharpest case.
- **Threads.** Swift may call its transport on the main thread, where a core call must not run.
- **22 k lines of tests** assert bytes through `FakeTransport`. Each move must port or retire them explicitly.
- **File formats.** `.dspipreset`, `PresetSnapshot` and the crossfeed extension have Swift codecs that must match the core's: shared fixtures.
- **Upstream merges** get harder as `Commands.swift` goes: SW-U1.

**SW1: XCFramework.** [DONE fff4d2f] Size 1. **Owns:** `apps/apple/build-xcframework.sh`. **Implements:** in the VM, add `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios` (the VM is Intel) and the macOS slices. `usb` off for iOS, on for macOS. Library size recorded. **Done when:** it links in the Simulator and on macOS. **Deps:** CG3, MR1.

**SW2: Swift package and compile test.** [DONE fff4d2f] Size 1. **Owns:** `DSPi Console/Core/Generated/`, Xcode project lines. **Implements:** bindings into the app; `init_logging`; one run per namespace. Adds the macOS and iOS rows to MR2's matrix. **Done when:** a test calls `core_info()` and opens the demo device on both targets, and both rows run in the matrix. **Deps:** SW1, CG9, MR2.

**SW3: Seam map.** [DONE 170c528] Size 0.5. **Owns:** `docs/SWIFT-SEAMS.md`. **Implements:** each wire-using function mapped to a core method; those with none become core WPs; thread and mirror decisions; re-sizes SW8–SW14. **Done when:** Fable has reviewed it. **Deps:** SW2.

**SW4: `CoreBackedTransport`.** [DONE 23aea616] Size 1.5. **Owns:** `DSPi Console/Core/CoreBackedTransport.swift`. **Implements:** `DeviceTransport` over CG4 and the core's notifications; the core's `FakeDevice` under demo mode. **Done when:** existing `FakeTransportTests` and wire tests pass behind a launch flag on both targets. **Deps:** SW3, CG4, CG23, CG26.

**SW5: CoreMIDI through the core.** [DONE 4e655d0e, wp/SW5: `MIDI/CoreMidiPipe.swift` under `CoreBackedTransport(link: .midi)`; the core is the default for iOS's USB MIDI and the Mac's MIDI paths, `-CoreTransport NO` opts out to `LegacyMIDITransport`] Size 1. **Owns:** `MIDI/CoreMidiPipe.swift`, reducing `MIDI/MIDITransport.swift` to I/O. **Implements:** `MidiSender`, `MidiInputHandle` with `close_port`, the relay port. **Done when:** `midi_vectors.json` passes; relay run **[HW]** on the K340. **Deps:** SW4.

**SW6: Link through the core.** [DONE f60845ef, wp/SW6: `Core/CoreLink.swift` drives the core's Link session under `CoreBackedTransport(link: .link)`; the core is the default for DSPi Link on Mac and iOS, `-CoreTransport NO` opts out to Swift's `LinkClient`; checked against the Swift hub in process and the Rust demo hub (see the WP report)] Size 1.5. **Owns:** `Link/LinkClient*.swift` replaced by `Core/CoreLink.swift`; keeps `HubBrowser.swift`, `LinkTokenStore.swift`, the pairing sheet. **Implements:** Bonjour stays in Swift (the entitlement belongs to it); `expected_hub_id`; `link_retry_now` on foreground; `rta_start` after a reconnect. Checks the Local Network prompt with Rust's BSD sockets. **Done when:** the run in `A7-LINK-CLIENT-REPORT.md` passes **[HW]**. **Deps:** SW4, XC1.

**SW7: Banner and errors.** [DONE 2026-11-16: review batch 2, on `main` from `dc06920a`] Size 0.5. **Owns:** `Core/LinkBannerView.swift`. **Done when:** the owner's rule holds in a fake-hub run; CG10's wording. **Deps:** SW6, CG10.

**SW8: Reads and the mirror.** [DONE 0755bba3, merged in review c6b] Size 3–3.5 (SW3). **SW-V1 and OD3 follow it.** **Owns:** `Commands.swift` "Bulk Parameter Transfer" and "Notification → State Mirroring" (about 840 lines). **Implements:** view-model state from `DspState`, notifications from the core, older firmware read-only. **Done when:** every view shows correct values on the demo device and fixtures; no echo flicker. **Deps:** SW4.

Then one WP per domain. Each **owns** its `Commands.swift` sections and wire use in its views; **done when** wire tests are ported, the domain has no `transport.` calls, and both targets work on the demo device. Sizes and deps from SW3's re-sizing (`docs/SWIFT-SEAMS.md` section 7); CG23–CG26 add 2.0.

| WP | Domain | Size |
|---|---|---|
| SW9 [DONE d1b1918a] | Volume, EQ, crossover, outputs, matrix, slider fast path | 2–2.5 |
| SW10 [DONE 8a33ea09] | Effects (loudness to upmixer) | 1.5–2 |
| SW11 [DONE 44196ca0] | Pins, I2S, ADAT, inputs, UART/I2C, names. Also needs CG25 | 2 |
| SW12 [DONE a95b9cdf] | Control surfaces. Also needs CG24 | 2–2.5 |
| SW13 [DONE a4d4e1ef] | Presets, `PresetDocument`, `PresetSnapshot` | 2–2.5 |
| SW14 [DONE de56e719] | Analyser (following state), Stats, signal generator, monitor | 2–2.5 |

**SW15: Simulator and relay test.** [TODO] Size 1. **[HW][COORD]**. **SW16: Real iPhone or iPad.** [TODO] Size 0.5. **[HW][OWNER]**: no device has ever run this app. **Owns:** `docs/SWIFT-HW.md`. **Done when:** USB-C MIDI, Link, background and return pass. **Deps:** SW5, SW6, SW14.

**iOS view-layer gaps** (parity 3.1, section 3). Views only; alongside the domains:

**SW17: File import and export.** [DONE 84220ac7] Size 1. **Owns:** `iOS/DocumentPicker.swift`, the `notYet` block in `DSPi_ConsoleApp.swift`. **Done when:** filter files and `.dspipreset` round-trip against shared fixtures. **Deps:** SW2.

**SW18: AutoEQ and firmware window.** [DONE 54bde172] Size 0.75. **Owns:** iOS AutoEQ menu, firmware wizard step. **Implements:** Favourites, CG6's update; hide the dead firmware window and point to a computer, including the mismatch banner's Update button (`ContentView.swift:1075-1077`) and the Tools entry (`iOS/IOSApp.swift:218-220`); no `.uf2` images in the iOS target. **Done when:** UI tests cover the menu, show no firmware entry on iOS, and show the banner cannot open the window; the iOS bundle has no `.uf2`; the Mac is unchanged. **Deps:** CG6, SW2.

**SW19: Touch graph.** [TODO] Size 1.5. **Owns:** `iOS/PeqTouchEditor.swift`, a touch HUD. **Implements:** marquee, pull from curve, shape chooser, value chip. **Done when:** UI tests on iPhone and iPad cover each gesture. **Deps:** SW2.

**SW20: Hardware keyboard and host format.** [TODO] Size 0.75. **Owns:** `iOS/IOSApp.swift` commands, `iOS/AudioRouteMonitor.swift`. **Done when:** a UI test covers the iPad shortcut table; the route count falls back to the device report. **Deps:** SW2.

**SW21:** [MOVED] moved to later work: coach marks.

**SW23: Advanced core voltage.** [TODO] Size 1. **Owns:** `Core/CoreVoltageView.swift`, reached from the Advanced settings page. **Implements:** thin UI over CG11, both targets; extends today's status view (`CoreVoltageStatusView.swift`). iOS needs no background mode: on return it re-reads `trial_active`. **Done when:** UI tests on both targets with simulated hangs; not reachable outside Advanced. **Deps:** CG11, SW2.

**SW22: Accessibility and localisation plumbing.** [TODO] Size 2.75, English only for now. **Owns:** all views. Alone, last. **Done when:** a VoiceOver pass in the Simulator; a pseudo-locale run. **Deps:** other SW WPs.

**Mac-only additions:** about 3.5–5.5.

**SW-M1: USB through the core.** [DONE 30e9b0fe, wp/SWM1: the core's `UsbPipe` under `Core/CoreUsb.swift`; USBNotificationReader removed, USBDevice.swift keeps the device model; the core is the Mac's default, `-CoreTransport NO` opts out] Size 1–1.5. **Owns:** `Core/CoreUsb.swift`; reduces `USBDevice.swift` and `USBNotificationReader.swift`. **Implements:** `nusb`'s `macos_iokit` through CG3's `usb` feature, including hot-plug. **Done when:** a demo run and a mock test pass; the macOS target has no IOKit vendor path left outside the hub. **Deps:** SW4.

**SW-M2: Firmware through the core.** [DONE 2026-11-16: review batch 2, on `main` from `dc06920a`] Size 0.5–1. **Owns:** `FirmwareInstaller.swift`, `BootloaderLocator.swift`, reduced to UI over C11's `firmware.rs` and CG8's guard. **Done when:** `FirmwareInstallerTests` pass; a fake drive test; guard cannot be skipped. **No real flash here. Deps:** SW-M1, CG8, CG24.

**SW-M3: Hub over `dspi-hub`.** [DONE 2026-11-16: `2f7a811e` into review batch 6, on `main` from `ef0e4095`; the Swift hub files are not removed yet: SWS13, below] Size 1–2. **Owns:** a UniFFI wrapper for the hub, `NetworkingSettingsView.swift` re-pointed; Swift hub files removed at the end. **Owner decided: "once it comes into the other versions it should come in here as well".** It does not block R-Mac. The Swift hub stays as the reference harness until HB7 passes, then retires. **Done when:** HB7's Swift client passes against the wrapper from the Mac app. **Deps:** HB7, SW-M1.
- <!-- WP SECSW --> **Retiring the Swift hub (SWS13) is blocked by seven items** (review batch 6, 2026-11-16):
  1. `HardwareTestSupport` still runs on `CoreUsb`: port it to the Rust hub's path.
  2. `Link.usb` (the Rust hub's core USB) has never run on a real DSPi: a K340 pass on the Mac VM (owner's standing go to move it; no flash).
  3. Pairings do not carry over: the Rust hub has a new hub id and store. A release note, or import the id from `link-auth.json`.
  4. TLS: the Mac's Rust hub serves plaintext until P2SW's `HubStartOptions.tls` (default `require`) lands. *(Resolved: P2SW merged, `49cd5370`; WP DOCSYNC, 2026-11-19.)*
  5. `CoreHub.swift` still depends on Swift hub types: `LinkAuthStore`, `LinkDiscovery`, `LinkAuthMode`, `LinkRole`, `LinkService`.
  6. HB7's conformance client is the opt-out `LinkClient` (`-CoreTransport NO`), which stays plaintext only: it needs the hub's `tls off` loopback mode.
  7. `ownHubID` is unknown before the hub's first start (the browser cannot leave this Mac's own hub out until then).

**SW-M4: What stays Swift.** [DONE 2026-11-15: `6d337b34`, review batch 3] Size 0.5. **Owns:** `docs/SWIFT-STAYS.md`. **Implements:** an audit that each item is UI or OS integration only, with no protocol: menu-bar mode, login item, keep-awake, `AppUpdateChecker` and installer, `HostAudioFormatMonitor`, Metal renderers. **Done when:** Fable has reviewed it, and a grep for opcodes outside `Core/` is empty. **Deps:** SW14.

**SW-M5: Real Mac hardware pass.** [TODO] Size 0.5. **[HW][OWNER]**: an Apple silicon run has never happened, and the VM and known spare Macs are Intel. **Owns:** `docs/SWIFT-HW.md`. **Done when:** the checklist passes under section 6's rules. Developer ID only if we distribute a binary (OD-DevID). **Deps:** SW-M1, SW-M2.

**SW-V1: Visual and feel evidence for OD3.** [TODO] Size 0.75. **Owns:** `docs/SWIFT-VS-QT-MAC.md`, screenshots under it. Owner: "come back with visual checks and feel to make a final decision, i assume swift will be needed for now".
- **Implements:** side-by-side screenshots of the main window, graph editing, settings, dialogs and menus, Qt `.dmg` against the Swift app on the Mac VM, on the demo device (the Qt half lapsed with the swap: no Qt macOS build). Notes on frame rate and responsiveness, with the VM's limits stated. Also a current iOS screenshot set. A short **section for the owner to try on real hardware [OWNER]**: a checklist of feel items (drag, scroll, native menus, text, sliders).
- **Done when:** the report goes to the owner at OD3's point, with no recommendation hiding the evidence. **Deps:** SW8 (QD21b dropped at the swap).

**SW-U1: Upstream tracking rule.** [DONE fc0f2678] Size 0.25. **Owns:** `docs/UPSTREAM-TRACKING.md` (section 0.1). **Done when:** a dry-run merge of a later upstream commit is recorded. **Deps:** MR1.

**SW25: Generated Swift constants.** [DONE 12667465, review c4p] Size 0.5. **Owns:** a generated `dspi_constants` module for Swift. **Implemented:** the core's constants generated for Swift, as for Kotlin; SW4's `CoreBackedTransport` uses `DspiConstants.NOTIFY_DISCONNECTED`. The domain WPs (SW9-SW14) move their hard-coded constants to it.

**SWF1: Hardware tests never leave a device changed.** [DONE bc1f9ff4, review c5e] Size 0.5. From hw-mac F1, O1. **Owns:** `HardwareTestSupport` and the Swift hardware tests. **Implemented:** a hardware test takes the device's raw 0xA0 image after the app's full read and fails if the image differs afterwards (`DeviceImageGuard`); tests of the view model's own logic use an offline demo view model, not the shared USB one; benchmarks skip on hardware; the signal-generator diagnostics need an explicit opt-in. Confirmed on the K340 in hw-mac-2.

**SWF2: Mac USB beside a held MIDI port, refused writes revert, hub voltage rule.** [DONE bc1f9ff4, review c5e] Size 0.75. From hw-mac F2, F3, F4. **Owns:** `USBDevice`, the Mac's local transport set, `NetworkTransport`, `HubTransport`, `CoreBackedTransport`, the Swift hub's policy. **Implemented:** USB works while CoreMIDI holds the DSPi's MIDI port (vendor requests without the exclusive device open), with USB MIDI as the fallback; a SET a hub refuses re-reads, so the field and the dirty preset revert and the status row says why; the Swift hub refuses core-voltage raises from network sessions, as HB2. Confirmed on the K340 in hw-mac-2.

**SWF3: iOS layout and version.** [DONE bc1f9ff4, review c5e] Size 0.5. From hw-mac F5-F8. **Owns:** iOS tool sheets, `ToolColumns`, the output strip, the iOS target's version. **Implemented:** tool sheets lay out at the sheet's visible size; tool windows stack their columns at compact width; on a narrow console the output strip keeps delay and mute on screen; the iOS `MARKETING_VERSION` is 1.1.6 like the Mac's (it was 1.1.6-beta5, which showed a false "newer firmware" banner). Confirmed in hw-mac-2.

Store items (review video, TestFlight, the SysEx ID question) are later: `APPSTORE-MIDI-RISK.md` applies then.

### 5.5 Web

**WB0: Study.** [DONE 5cd58c7] Size 2. **Owns:** `docs/core-merge/WEB-STUDY.md`, throwaway spikes under `~/.cache/dspi-build/work/wp-wb0/`. Must judge, with a working spike where marked:

| Option | Questions |
|---|---|
| Rust core in wasm + a web UI **(spike)** | The session's 17 transport-using files block on `&mut dyn Transport`: about 170 call sites and 45 uses of threads or `Instant`. Compare (a) an async twin of the trait; (b) a request and reply split (`PLAN.md` §1.3); (c) blocking in a Worker with `Atomics.wait`, needing COOP and COEP; (d) JavaScript Promise Integration. Which leaves `dspi-session` unchanged for other clients? `nusb` has a wasm32 WebUSB backend. |
| Qt for WebAssembly **(spike)** | Do the Qt app and the C ABI core build? Download size? Same async problem. |
| Reuse what exists | sigman78's console (MIT, Svelte, WebUSB, demo mode, 1.1.5+) and the DSPi-Link web UI (MIT, which HB5 serves). What do they need for extensions and Link? Licence fit (OD-Lic)? |
| Link in the browser | Browsers block `ws://` from `https://`, and no hub has TLS. Is a hub-served page the only route? |
| WebUSB | Chromium only. Check on stock and bofr-main, per OS. **Why does sigman78's README still mention Zadig on Windows, given the firmware's MS OS 2.0 descriptor?** |
| Web MIDI | Chromium, Firefox with permission. Firmware's MIDI build only. |

**Done when:** a recommendation with sizes fills the stubs. **Deps:** `main`.

**Owner's decision after WB0:** "We can defer the web version for later, but don't make the code more difficult to port to the web than necessary. I would rather see one single base than including an external project." So the study's route of pointing at sigman78's console is not taken, and no README pointer goes in. `WEB-STUDY.md` keeps the evidence.

**Owner's order (2026-11-07):** the web comes after the plan's WPs, before optimisation and polish. It is scheduled below.

**WB1: Keep the core portable to the web.** [DONE 0e29dfb] Size 0.75. Wave 2. **Owns:** a clock seam in `dspi-transport` (`Cargo.toml`, `src/lib.rs`, new `src/clock.rs`, `fake.rs`), the six `SystemTime::now` sites (including `autoeq.rs`, `files.rs`, `monitor.rs`), `nusb` gating in `dspi-core/Cargo.toml` (`device.rs`, `firmware.rs`, `hotplug.rs`, `connect.rs`) on top of CG3, a `wasm-core` step in `ci/check.sh`. Compile-only by design: the 10 `thread::spawn` sites still need work when the web is scheduled. **Implements:** the non-UI part of the study's spike patch, properly tested. **Done when:** `dspi-session` and `dspi-core` pass `cargo check --target wasm32-unknown-unknown` in the gate, native behaviour is unchanged, and no client work is added. **Deps:** CG3, MR2.

**Corrections from WB0, kept:** browsers can reach `ws://` private addresses from `https://` in Chromium (Firefox loopback
only); the wall is the hub's `Origin` check, so a hub-served page is the only route with hubs as specified. `nusb` 0.2.5 has
no WebUSB backend. Qt for WebAssembly with device access is 15-25, not 8-15, and unproven. The old stubs WB2 (UI), WB3
(hosting and origin) and WB4 (hardware test) are replaced by the WPs below.

**The web WPs.** Built on WB0's spike and WB1's gate: the existing blocking `dspi-core` under **JSPI in a dedicated Worker,
one call at a time** (WEB-STUDY section 2), so no other client's API changes. No external project: no sigman78 code and no
`@dspi/link` JavaScript; Link stays our Rust `dspi-link` over a thin browser-WebSocket shim. Route 2 in WEB-STUDY sized it at
39-62 units, nearly all UI.

**Owner's web decisions (2026-11-08):**
1. **Hub web page:** keep the upstream DSPi-Link page in `dspi hub` (HB5). Replace it with ours only if it clashes or lacks
   what is needed. So WB7's hub route serves our client beside the upstream page, not in its place, unless that case arises.
2. **Framework and model:** a natural fit with the Rust core in wasm, not a fashionable choice for its own sake. **The main use
   is a web hub that collects several DSPi installations.** The client runs **standalone** and is **embeddable in a
   server-based fleet version** with sign-in and access control. So: a multi-device and multi-hub model throughout (the
   binding, the Worker RPC and the UI hold many devices and hubs at once), auth as a pluggable seam (the hub's PIN and tokens
   standalone, the fleet server's sign-in when embedded), and no single-device assumption anywhere. WB5 records its framework
   choice against this; it replaces the earlier "none or Preact" recommendation.

| WP | Size | Owns | Implements | Done when |
|---|---|---|---|---|
| **WB2** Runtime without threads [DONE 88dc7342] | 2-3 | `crates/dspi-core/src/{notify,rta}.rs` (spawn sites), `crates/dspi-session/src/{notify,autoeq}.rs`, the wasm step in `ci/check.sh` | the runtime gap of WEB-STUDY section 11: notification packets pushed in by the caller (no reader thread); caller-driven ticks in place of the wake, RTA worker and autoeq rebuild threads, used only on wasm (native unchanged); `std::fs` users behind byte APIs; the `link` feature's `getrandom` and `dspi-link`'s own `Clock` onto the seam | the wasm gate **runs** a scenario under Node with JSPI (connect to the demo device, a write with read-back, notifications, RTA frames), not only `cargo check`; native tests unchanged |
| **WB3** The binding | 2-3 | new `crates/dspi-wasm/` | exports over the core (JSON records, as the C ABI's shapes), the three clock imports, **the serial queue and a re-entrancy guard in the module** (WEB-STUDY's silent stack corruption), panic-safe; many devices and hubs at once (decision 2) | a test proves a second call while one is suspended is refused, not run; size recorded (study: 288 KB gzip for all of the core) |
| **WB4a** WebUSB transport | 1-1.5 | `crates/dspi-wasm/src/usb.rs`, the Worker's `usb.js` | vendor interface 2, control transfers as Promises, disconnect events; Chromium only, secure context only | a run against a WebUSB mock in headless Chromium; hardware in WB8 |
| **WB4b** Link transport | 1.5-2 | `crates/dspi-wasm/src/link.rs`, `ws.js` | `dspi-link`'s client over the browser WebSocket; the core's supervisor and the owner's Link rule unchanged | the page, served by `dspi hub` (so the `Origin` check passes), pairs and reconnects against the fake hub in headless Chromium and Firefox |
| **WB4c** Web MIDI (later) | 0.5-1 | `crates/dspi-wasm/src/midi.rs`, a page-side proxy (Web MIDI is not available in a Worker in Chromium) | SysEx over the page's MIDI port | a fake MIDI port test; **public use waits on OD9 (SysEx ID)** |
| **WB5** Web client skeleton [DONE 88dc7342] | 2 | new `apps/web/` (a pinned build container) | Worker RPC, the device picker (demo, USB, hubs; several at once), the connection strip, the lost-link banner from CG10's wording, About from `assets/about.json`, read-only mode, the auth seam and an embeddable entry point (decision 2) | the demo device shown in headless Chromium in CI; a `web` row in `ci/matrix.sh` |
| **WB6** UI floor, in slices (each one WP) | 30-45 in all, plus 4-6 for WB6d-WB6f | `apps/web/src/<screen>/` per slice | section 3's floor: main window, volume and presets (M42-M44); EQ graph with editing on a canvas (M21); crossover, outputs, matrix; effects; inputs, names, latency, pins; settings and core voltage (M56, behind Advanced); analyser, bar strip and Stats (M22, M23, M53); files through browser download and upload (M28-M30); AutoEQ (CG6, fetch supplied by the page); control surfaces (CG20). Since UIM1 also the System view, the scene bar and the status rail (`docs/UI-MODES.md`: the web is the first client of the Advanced layer). Slices and order: the table below | each slice: its `docs/PARITY.md` web column cells, tests on the demo device |
| **WB7** Hosting | 0.5-1 | `crates/dspi-hub/src/web.rs` (HB5's page), `dspi web` or `dspi hub --local` in `apps/terminal` | the hub serves our client beside the upstream page (decision 1; Link, plain http, no WebUSB); `http://localhost` from a local `dspi` gives a secure context with WebUSB and Link together; a static HTTPS build; the embedded build for a fleet server. **Publishing anywhere is the owner's act** | a browser test reaches the demo through each route |
| **WB8** Hardware | 1 | `docs/WEB-HW.md` | (a) K340 on the Linux host, Chromium in a container; (b) Windows clean machine, the Zadig question: `winvm` with the K340 passed in, or the owner; (c) macOS Chromium on `macvm`; (d) Android Chrome WebUSB: the owner's phone | each row pass or fail |
| **WB9** Gate | 0.25 | plan sections 5.5 and 7 | R-Web's criteria (section 7); the web column in `docs/PARITY.md` | |

**Order:** WB2 and WB5 in parallel (WB5 against a stub); WB3 after WB2; WB4a and WB4b after WB3; WB6 slices after WB5 and
WB4a (the demo first), in the order of the slice table below; WB7 after WB4b; WB8 last; WB9 closes it. WB4c when OD9 allows public MIDI.

**WB6 slices** (owner, 2026-11-12, UIM1): the web is the first client of the Advanced layer (`docs/UI-MODES.md`), so the System
view, the scene bar and the status rail come before the remaining tool screens. They are built on what the core has today and
grow when the core gains host-side scenes, a change history and alarms (not yet planned here); each of the three names what it
waits for. One slice is one WP; letters give the order.

| Slice | Status | Scope |
|---|---|---|
| **WB6a** Main window, volume, presets | DONE | The session's main screen: volume (User and Master), bypass, the preset menu with Commit, Revert and Factory Reset, and the unsaved-changes guard (M42-M44, M47, M50 global bypass) |
| **WB6b** EQ | DONE | The EQ screen: channel pills, the response graph on a canvas with on-graph editing, the band chip and the band list (M21, M58, B5-B7) |
| **WB6c** Outputs, crossover, matrix | DONE | An output's settings card, its crossover tab and Linkwitz panel, the limiter, and the Matrix screen (M49, M52, B8, B10-B12) |
| **WB6d** System view | DONE (`6d337b34`) | A read-mostly screen of the whole device as a signal chain: inputs with meters, the routing summary, the processing blocks per channel with their on or off state (EQ, crossover, limiter, effects), and outputs with meters, gain, mute and delay; a block opens its screen. Over today's `DspState` and status, no new core calls |
| **WB6e** Scene bar | DONE (`5778bae0`) | A bar across the top of a session with the device's ten preset slots as numbered recall buttons, the active one and its unsaved state marked, Store and Recall as explicit verbs, and recall confirmed when there are unsaved changes; it moves to host-side scenes and A/B compare when the core has them |
| **WB6f** Status rail | DONE (`6d337b34`) | A persistent strip per console (connection, device, preset, mode) and a health panel that keeps problems until they are acknowledged: clipping, limiter gain reduction, the two cores' load (M26), a lost link; alarms from the hub join it when the hub has them |
| **WB6g** Inputs and channels | DONE (`6d337b34`) | The input pages: preamp trims, the L/R link of a pair with which side to keep (M48), input-source names, the 8-channel matrix's per-input trims; the channel clipboard, Copy and Paste Parameters (M45) |
| **WB6h** Effects | DONE (`5778bae0`) | Crossfeed, loudness, leveller, psychoacoustic bass, tube, subharmonic and upmixer screens, each with its on or off state in the System view |
| **WB6i** Settings | DONE (`5778bae0`) | Device name, output latency, GPIO pins, Spectrum and Graphing settings, help links, Reset Channel Names, debug information (M55, M61, M62); core voltage behind the Installer mode, or behind Advanced until the mode model exists (M56) |
| **WB6j** Analyser, generator and Stats | DONE (`8634bf02`) | The analyser overlay and bar strip (M22, M23), the signal generator and Identify (M46), Stats for nerbs and the Interrupt Monitor (M53, M54), dashboard cards per row (M59) |
| **WB6k** Files and AutoEQ | DONE (`8634bf02`) | Filter import and export, Save Master Volume and Output Configuration through browser download and upload (M28-M30); AutoEQ with the page supplying the fetch (M31, M32, CG6). `.dspipreset` import (with the apply options) and export are no longer WB6k's: P1WB1 built them (`FilesScreen.tsx`, wasm `configImport`/`configExport`; P1DCFIX's note) |
| **WB6l** Control surfaces and shortcuts | DONE (`8634bf02`) | The control-surface editor (CG20), the quick strip of effect toggles (M60), and the app shortcuts beyond the graph keys (M24) |

### 5.6 Audit and cross-client tests

**XC0: Finish the floor audit.** [DONE c69fe82] Size 0.5. **Wave 0.** **Owns:** `docs/PARITY.md`, moved into the repo from `dspi-mobile/docs/FEATURE-PARITY.md`, and the references to the old path in `docs/core-merge/PLAN.md` and `docs/PORTABILITY.md` §11. **Implements:** walk the Mac's menus and every settings page; turn every "hit" in section 3 into read code, per client; add anything missing with a WP or a reason. **Done when:** no "hit" remains, and Fable has spot-checked 10 rows. **Deps:** `main`.

**XC1: `macr` restart.** [DONE] Size 0.5. **[COORD]**, **with the owner's go-ahead.** **Implements:** patched QEMU and libusb per `qemu-usb/REPORT.md`; check `br0` and Bonjour from the phone; re-attach the DSPi over QMP; host unlocked. **Done when:** the Mac hub is reachable from the LAN and the 6 KB read works. **Prerequisite for every Mac test. Deps:** none.

**XC2: Mac hub ping test.** [DONE] Size 0.25. **[HW][COORD]**. **Owns:** a report. **Done when:** the Mac hub answers pings, so L1's 5 s probe does not kill healthy sessions; idle past 35 s survives; rate limit keeps the token. **Before AN13's Link half on a Mac hub. Deps:** XC1.

**XC3: Real-hub test harness.** [DONE 902dcce] Size 0.5. **Owns:** `crates/dspi-link/tests/real_hub/`, opt-in by environment. **Implements:** the scripted parts of `LINK-HUB-FINDINGS.md` section 5. **Done when:** it runs against the fake hub with the same assertions and skips cleanly with no hub set. **Deps:** `main`.

**XC4: ESP32 bridge suite.** [TODO] Size 1. **[HW][OWNER][COORD]**: **is an ESP32 on hand and wired to a DSPi?** **Owns:** a report. **Implements:** all 9 items: idle, wrong PIN ×5, Wi-Fi pulls at 5, 20, 32, 60 s, all slots full, DHCP change, board off, GETs over 132 B, cut after `auth.pair`, Doze. **Done when:** each is pass or fail, with fixes filed. **Deps:** XC3, AN5 for phone cases.

**XC5: Meters over a bridge.** [TODO] Size 0.5. **[HW][OWNER][COORD]**. **Owns:** a report. **Done when:** CPU, frame time and audio are measured with meters on, using `poll.subscribe`, against kingsnake's report. **Deps:** XC4.

**XC6: Core voltage on the K340.** [TODO] Size 0.5. **[HW][COORD]**, **K340 only, backups first.** **Owns:** a report. **Implements:** back up settings and flash; through Qt, then Android and Swift if available: status; a short trial at the build default (1.20 V) with abort; a pass result with `TRIAL_END`; a `SET` to the same value, verified, and re-read after power cycle. **Not in this WP without the owner's approval at that step:** a real hang test (a trial at 1.15 V) or anything above 1.20 V. The hang path is proven on `FakeDevice`. **Done when:** the report records results, and `live_vreg` and settings match the backup at the end. **Deps:** CG11, QD28.

**CLEAN1: Core and C ABI files rewritten, notices for release, the `uconsole` command.** [DONE ae693fc6, review cl1 + fixup] Size 4. **Owns:** in `crates/dspi-core` `files.rs` (and its tests), `monitor.rs`, `consts.rs`, `presets.rs`, `records.rs`, `curves.rs`; in `crates/dspi-capi` `types.rs`, `types/consts.rs`, `fns_misc.rs`, `fns_device.rs`; `xtask` (the capi constants generator); the notices (`NOTICE.md`, `assets/about.json`, `assets/licences/`, the Android and desktop notice files, `ci/release-source.sh`, `ci/release-terminal.sh`); the Terminal's command name. **Implemented:**
- The eleven core and C ABI files are written anew from `dspi-proto`, the firmware headers and the macOS console, with the same interface: the C header's declarations are unchanged and anchored by `crates/dspi-capi/tests/data/abi-anchor.txt`; UniFFI records and exports unchanged (only doc comments, hence checksums, differ). The C ABI's constants are generated: `cargo run -p xtask -- capi-consts`, checked by `drift`.
- Behaviour kept and pinned by tests: the response curves match recorded outputs (`tests/fixtures/curves/responses.jsonl`), the configuration and filter exports match the demo device's recorded documents, the preset change rows follow the macOS console's words. A factory reset now waits for the device to apply it before re-reading; channel ids from a file resolve with checked arithmetic.
- Notices (drafts, pending the owner): the program as a whole under GPL-3.0, Bofr5555's own additions GPL-3.0-or-later with the section 7 terms; each release ships with the source archive of its commit (`ci/release-source.sh`, `--with-desktop` for a Qt release), named by every notice and by the web console's `SOURCE.txt`; the bundled firmware's third-party notices; the Terminal's Rust crate licence texts (`uconsole --third-party-licences`); the Android files ported from DSPi Console for Windows named as GPL-3.0 material; Noto Sans Mono and AutoEq credited; About texts without a first-person donation paragraph.
- The Terminal's command is `uconsole`, named once in `dspi_proto::command!()` (`ci/command-name.sh` for scripts): help, messages, completions, its config, AutoEQ and hub-store directories, the hub's service files (templates rendered by `render.sh`). `dspi` and `dspi-term` are no longer installed; no compatibility, as `CHANGELOG.md` says.
**Open (owner):** where the source archive is published; the draft wording; the mirror tag `v1.1.6` in Bofr5555/DSPi; a board check of the configuration import with the output configuration kept apart from presets. **Deps:** none.

**UIM1: UI modes, first steps.** [DONE 1332b379] Size 1.5. **Owns:** `docs/UI-MODES.md` (new), the WB6 slice table in section 5.5 and the web cells of `docs/PARITY.md`, Android's `feature-home` (Home's phone layout). **Implements** the owner's decisions of 2026-11-12: one app per platform with the modes Simple, Advanced and Installer and a locked-user state, remembered per device with an app default (Simple on phones, Advanced on desktops and the web); nothing locked unless an installer sets a PIN; modes, PIN and roles fitting the hub's future fleet user management; channel colours as small tags only. **Done:**
- `docs/UI-MODES.md`: the mode model, the no-PIN rule, the fleet mapping, a proposed screen manifest for the core (no code), the channel-colour rule, the client order, the open questions.
- Android: the phone's Home starts with volume, preset and source, the output meters and a health line, then the graph, the channel list and tool icons, the dashboard; the tablet keeps the macOS sidebar; meters and card strokes neutral (tests `HomeOrderWpUim1Test`, `HealthLineWpUim1Test`).
- Web: WB6d System view, WB6e scene bar and WB6f status rail before the remaining tool screens (plan only).
**Follow-ups (not WPs yet):** the core's mode manifest; the mode switch per client; the Qt Pro track after the swap; Android's Pro subset.

### 5.7 Core and transfer optimisations (after the web)

The owner's third item. No benchmark exists today (no `benches/` in any crate), so the first WP builds the harness, and every
later one states a baseline and a target and proves the gain with it. Results go in a new `docs/PERF.md`.

| WP | Size | What to measure, and where |
|---|---|---|
| **OPT0** Harness | 1 | A counting, latency-modelled transport over `FakeDevice` (per-transfer cost for USB, Link RTT 5/20/50 ms, MIDI at a few KB/s) that records transfers, bytes, wall time and allocations (a counting `GlobalAlloc` in the bench binary); criterion or a plain runner; opt-in `DSPI_BENCH_HW=1` on the K340, read-mostly, with hw-1's backup and diff. A CI step that runs the fake benches and fails on a regression beyond a set margin |
| **OPT1** Connect cost | 0.5-1 | Transfers and time of a full connect (the spike counted 39-45): `crates/dspi-core/src/connect.rs` (`fetch_all`), `crates/dspi-session/src/probe.rs`: reads the bulk image already covers, per-channel name reads, extension probes repeated per connect |
| **OPT2** Bulk transfer size | 0.5 | `BULK_WRITE_CHUNK` 2048 (`connect.rs`), `BULK_CHUNK` 2048 (`dspi-hub/src/device.rs`), `BULK_CHUNK_SIZE` (`consts.rs`) against the 6136-byte image: 3 chunks each way. **First check the firmware's limit**; if 2048 is the protocol's, record that and close. Measured on the K340 |
| **OPT3** Polling cadence | 1 | Transfers per second when idle and when hidden, per client: Qt `StatsController`, `Poller`; Android's bridge pollers; the core's RTA `STATUS_INTERVAL` 500 ms and `MIN_BIN_INTERVAL` 50 ms (`dspi-session/src/rta.rs`); `poll_hotplug`. Target: no device polling while every window is hidden (Qt tray, Android background), the Mac's 60 ms stats tick kept while shown |
| **OPT4** Notification batching | 0.5-1 | Listener calls and UI refreshes per notification burst (a preset load, a remote client's slider drag): `crates/dspi-core/src/notify.rs`, the C ABI listener, the Kotlin bridge, Qt's handler. One `NotifyResult` per drain, one UI refresh per frame |
| **OPT5** Allocation in hot paths | 1 | Allocations per meter tick and per RTA frame: `get_state` copying the whole `DspState` across UniFFI and the C ABI each tick, RTA frame vectors, decode in `dspi-proto/src/packets.rs`. Targets: no allocation per tick on the Rust side; a smaller record for the per-tick read |
| **OPT6** Slider write coalescing | 0.5-1 | Writes per second and time to the final value during a 1 s drag, over USB and over Link at 20/50 ms RTT: `dspi-session/src/fast.rs`, `write.rs`. Latest wins while a write is in flight; never replays after a Link drop (the owner's rule) |
| **OPT7** Link framing and pipelining | 1-1.5 | Bytes on the wire per second with meters on (`poll.subscribe` against per-read CMD), CMD pipelining up to `max_inflight`, connect over Link at each RTT: `dspi-link/src/{client,socket,frames}.rs`, `dspi-hub/src/{router,polls,snapshot}.rs` (`POLL_BUDGET_BPS` 200 000, `POLL_MAX_HZ` 20). The hub serving repeated reads from its snapshot for several clients |
| **OPT8** MIDI SysEx chunking | 0.5-1 | Bulk read time over USB MIDI (about 2 s per 6 KB today): the piece size and pause in `dspi-transport/src/midi.rs`, 7-bit packing in `dspi-proto/src/frame.rs`; pacing per host. Measured on the K340 through ALSA, then CoreMIDI on `macvm` |
| **OPT9** Sizes | 0.5 | Library and bundle sizes recorded in the gate: Android `.so` per ABI, XCFramework slices, the wasm module; LTO, `opt-level`, `panic` per target, with a budget that fails CI if exceeded |

**Every OPT WP is done when** `docs/PERF.md` shows the baseline and the new figure from OPT0's harness (and the K340 figure
where marked), the gain is above the harness's run-to-run noise, and behaviour tests are unchanged. Hardware runs follow
section 6's rules.

### 5.8 UI polish (last)

The owner's fourth item. One read-only audit per client, then fix WPs cut from its findings (each 0.25-1, grouped by screen so
owned files stay disjoint). Each audit runs after that client's accessibility and localisation WPs, so the fixes are not
redone.

| WP | Size | Method |
|---|---|---|
| **PL-QT0** Qt audit | 1 | Every window and settings page against the Mac reference on the demo device (Swift screenshots from `macvm`; Qt offscreen screenshots at 3 window sizes, light and dark): clipping and layout at small sizes, keyboard focus order, wording against CG10 and the no-host-noun rule, empty, error and loading states, units and number formats, tooltips, redraws ("never redraw anything unnecessarily"). Output `docs/polish/QT.md`, ranked, each item with a size and files |
| **PL-AN0** Android audit | 1 | The same on the emulator in a container, phone and tablet widths, portrait and landscape, font scale 1.3; phone-test-2's screenshots as a start |
| **PL-TM0** Terminal audit | 0.5 | Every screen at 80x24, 40 columns and wide; ASCII and braille; colour and no colour; every `--json` output's shape |
| **PL-SW0** Swift audit | 1 | iOS against the Mac on the simulator, and the Mac app against upstream after the SW domain moves (regressions); a VM job |
| **PL-WB0** Web audit | 0.5-1 | After WB6 |
| **PL-*n*** fixes | per audit | From each report |

### 5.9 PRO phase 1: a single device you can trust

*WP PLANP1, 2026-11-13, on `main` at `1c87fcee`. The spec is PRO1's report (`work/pro1/REPORT.md`, rows W, S, U and T6
of section 2, phase 1 of section 4) with the owner's decisions below. Phase 0 (P0W, P0W2, P0M, P0U, P0E) is on `main`.*

**Goal.** An installer prepares a system at the desk, applies it on site with a way back, locks what must not change, and
hands it over. Everything is core-first: the logic lands once in `dspi-core`/`dspi-session` (and the hub where a network client
needs it), and each client adds one dialog or page for it.

**Owner decisions this phase builds on (2026-11-12 and 2026-11-13):**
- Undo in all apps (OD5's "no for now" is superseded; section 8).
- Limits are enforced when a profile is set; with no profile the device behaves as today.
- The system runs fully without an installer PIN when none is set: nothing is locked, every mode is reachable, no PIN prompt
  (`docs/UI-MODES.md` section 1). Modes, the PIN and the lock hook into the web fleet's future user management on the hub, with
  the roles **Admin, Control and Viewer** (no fourth role).
- Modes are **Simple, Advanced and Installer**. Qt's Window > Mode… entry stays hidden for now (QPI5); Qt reaches the
  Installer pages from Settings until the owner unhides it.
- Hardware settings apply at once: no Save bar on the power-on and Installer pages; a change is a write, journalled like any
  other.
- Output gain reaches +20 dB in every client (`out.gain` -60 to +20 dB, `docs/GLOSSARY.md`); a limit can only narrow that.
- "Stats for nerbs" is spelled so.

**Already done on `main` (checked in the code, 2026-11-13; status updated from the merges on `main`, 2026-11-15):**

| PRO1 item | State | Where |
|---|---|---|
| W1 one write path, undo, redo, history | DONE (P0W, P0W2, QTGAP1, SWAPFIX) | `crates/dspi-core/src/journal.rs`, `crates/dspi-session/src/history.rs`; `docs/integrator/undo.md`. A drag marked with `set_dragging` is one step. **Missing:** a grouping call for compound edits that are not drags (P1UG). **Now DONE:** P1UG (5e355a18) |
| W2 undo in the clients | Terminal, Qt (QP18: Edit menu, History window), web (WB8), Swift (SWMAC1, SW9's domains; the rest with SW10–SW14) DONE; **Android: AN42, DONE (5e355a18)** | `apps/desktop/qt/src/controllers/HistoryController.cpp`, `apps/web/src/ui/undo.tsx`, `Core/CoreUndo.swift` |
| U3 shortcuts | Qt DONE (QP18: Ctrl+1–9, commit, revert, mute, bypass, undo, redo, History, one table `MenuTable.qml`); web: undo keys only; Android: `ConsoleKeyMap.kt`; Swift: Mac's, no preset keys | The A/B key and the missing preset keys ride with each client's P1 WP below |
| U5 non-modal notices | Qt DONE (QP18: `NoticeCenter`, banners, no "Success" modals); web and Android have notice hosts; **Swift has about 70 alerts** (69 `Alert(`, 43 `.alert(` in 10 files; P1SW3 counts them). **Missing:** one error-key table in the core (P1NT). **Now DONE:** the core's table (P1NT, 5e355a18); Swift's alerts onto it (P1SW3, db3a9ad5) | |
| S2 protect all outputs | Qt DONE as a client action (`ToolsController::protectAllOutputs`); web and Android have the other batch actions, not this one. **Missing:** the core helper so all share it (P1PR). **Now DONE:** P1PR (a63e3b3e) | |
| S5 power-on page | Qt DONE (QP11: Settings > Global, "At power-on"); others DONE: web (P1WB2, 53d28d1e), Android (P1AN2, 53d28d1e), Swift (P1SW2, d6b7bb22) | `apps/desktop/qt/qml/settings/pages/GlobalPage.qml` |
| S8 read-only floor | Qt DONE (QD6, QP16); Android DONE for read-only (G1 guard) without "stale since"; web read-only DONE (WB5), stale partly; **Swift: read-only banner and feature hiding** (`CoreMirror.swift` `deviceReadOnly`), not every control, no "stale since". **Now DONE:** Swift's full floor and "stale since" (P1SW3, db3a9ad5); "stale since" on web and Android (P1WB2, P1AN2: 53d28d1e) | PARITY A10 |
| UIM1 modes | Design DONE (`docs/UI-MODES.md`); web has a mode type and list (UIM1; `screens/system/model.ts`, `DEFAULT_MODE = "advanced"`), no switch from a manifest; Qt's `ModeController` and Mode window exist, hidden (QPI5); **no mode manifest in the core, no PIN, no lock**. **Now DONE:** the mode manifest, PIN and lock (P1LK, ae43e568) | |
| Scenes on the hub | The hub's **audio-link system scenes** exist (SW2: `crates/dspi-hub/src/system/scene.rs`, frame-timed ops: mute, preset load, master volume, delay across units). They are not host-side whole-device snapshots; W3 is still to do | |
| P0U clamps | DONE: a preset or document load clamps an out-of-range value and reports it (P0U F1, CS1) | `crates/dspi-session/src/preset_file.rs` |
| W5 diff | DONE (P1DF, 5e355a18); before it, partial: `preset_changes` diffs the live state against the slot (CG7); no document-to-document or document-to-device diff | `crates/dspi-core/src/presets.rs` |
| W8 partial apply | DONE (P1DF, 5e355a18); before it, partial: three coarse switches (`ApplyOptions`: audio processing, volume levels, hardware I/O) | `preset_file.rs`, `config_import` options in `files.rs` |
| W9 tokens | DONE (P1TK, d63ea832; required by default: P1TZ, 39b7811d); before it, Android only (`bridge/DangerousOpsContract.kt`, 10 s tokens); the core has the `OVERVOLT` word (CG14) | |
| T6, W3, W4, W6, W7, S1, S4 | DONE: W3 P1SN (4bc97606); W4 P1AB (0c34b4aa); W6 P1SA (35cf785a), its hub half P1HA (27cc37bf); W7 P1OF (0c83f07e); S1 P1LM (ce3a5892); S4 P1LK (ae43e568); the hub half of S1 and S4 P1HL (7750a7b9); T6 P1AQ (a42759c2). Client dialogs: P1QT1, P1WB1, P1AN1 (594a3e36); P1QT2, P1WB2, P1AN2 (53d28d1e); P1SW1 (ce44e4d0); P1SW2 (d6b7bb22); P1SW3 (db3a9ad5) | |

**Not in this phase:** the hub's shared snapshot store, the parameter API, alarms and fleet (phase 2 and 3); firmware gain
clamps (FW2) and a full-wipe factory reset (FW11), which stay proposals.

**Naming.** Host-side whole-device states are **snapshots** in code and UI. **Scene** stays the hub's audio-link system scene
(SW2). PRO1's "scenes are snapshots with a number" is not taken: one word per concept (`docs/GLOSSARY.md`). P1SN adds the
glossary rows. (Owner decision OD-P1a, below.)

**Shared rules for every P1 WP.** Section 1 and `wave1-brief.md`. Binding wiring (the core's `lib.rs`; the C ABI in `crates/dspi-capi` (`fns_*.rs`, `types.rs`, the generated header); the UniFFI
exports in `crates/dspi-ffi`; `crates/dspi-wasm/src/lib.rs`) is shared and additive under `// -- WP <ID> --`; after an export change run
`ci/run.sh 'ci/drift.sh write'`, regenerate the ABI anchor and the Android contract, and classify each new method in
`WriteGuardedCore`. Every snapshot recall, guarded apply, A/B toggle and template apply is a bulk image: it ends the undo
history before it, as `docs/integrator/undo.md` says. The lost-Link rule holds: nothing is replayed, a revert is never queued.
Swift WPs run on the macOS VM, one at a time.

#### Core and hub WPs

**P1UG: Undo groups for compound edits.** [DONE 5e355a18] Size 1. Wave P1-A. **Owns:** `crates/dspi-core/src/journal.rs` and its
`journal/tests_wp_p1ug.rs`, `crates/dspi-session/src/history.rs`; binding wiring (shared). **Implements:** `begin_group(label)` and
`end_group()` (C `dspi_undo_group_begin/end`, UniFFI, wasm): everything written between them is one undo step with the label,
as a drag is; nesting counts; a group is always ended by a disconnect, an open, a re-read (`fetch_all`) or a bulk image, as
`set_dragging` is; an empty group records nothing; the skip-and-name rule applies per value. This WP adds the API only; the core's own compound writes adopt it in P1PR
(`params.rs` is P1XF's in this wave). From review sw9 and
`followups-2026-11-13.md`: linked-pair preamp, Clear All and direct routing are one step each. **Done when:** core tests
for a linked pair, a 10-band Clear All and a direct 1:1 routing (each one undo, one redo, every value back), nesting, an
unclosed group ended by a disconnect, and a value changed by another writer inside a group left alone and named; one test each
through C and wasm. `fetch_all` (in `connect.rs`) already ends a drag through `journal.rs`'s re-read note, so no edit outside the owned files is
expected. **Deps:** none. **Clients:** each client's second P1 WP wraps its Clear All, direct routing and other
multi-write actions in a group.

**P1XF: Crossfeed named presets in the mirror.** [DONE 5e355a18] Size 0.5. Wave P1-A. **Owns:** the crossfeed setters in
`crates/dspi-core/src/params.rs` (`set_crossfeed_preset` and neighbours), the crossfeed fields' handling where a notification or a
re-read patches the mirror (`crates/dspi-core/src/notify.rs`, `state.rs` crossfeed lines only), tests `tests_wp_p1xf.rs`.
**Implements:** while a named preset (Default, Chu Moy, Meier) is chosen, the mirror gives that preset's frequency and feed
to every client, so a graph draws the preset and not the custom pair; the custom pair is kept and comes back when Custom is
chosen; Qt's behaviour (the preset's values shown) becomes the core's. Checks the preset-file export writes what the Mac
writes; if the export must change, it is reported, not edited (P1DF owns `files.rs` in this wave). From the SW10 report.
**Done when:** core tests: choose Chu Moy, the state reads its values before and after the device's echo and after
`fetch_all`; Custom restores the custom pair; the export matches the Mac fixture. **Deps:** none. **Clients:** Swift's graph
stops flipping to custom (checked in P1SW2); no client change otherwise.

**P1DF: Document diff and partial apply.** [DONE 5e355a18] Size 2.5 (W5 1.5, W8 1). Wave P1-A. **Owns:**
`crates/dspi-session/src/preset_file.rs`, `config_import`/`config_inspect` and their options in `crates/dspi-core/src/files.rs`,
`crates/dspi-core/src/presets.rs` (the diff wording), a new `crates/dspi-core/src/doc_diff.rs`; binding wiring (shared).
**Implements:** W5: diff two documents, or a document against the device, in the words `preset_changes` already uses (one row per
changed value: section, item, before, after), with a short summary; shown before an import. W8: apply options per block (EQ per
channel, crossover, routing and matrix, outputs, limiters, effects, volume levels, hardware I/O, names), replacing the three coarse
switches with a superset (the old `options` bits keep their meaning). Also `meta.kind` in the document (`preset`, `snapshot`,
`template`, `limits`; unknown kinds read as `preset`), additive to schema 1, so P1SN, P1OF and P1LM write documents of one family
(PRO1 section 6: no second format). **Done when:** tests: doc-vs-doc and doc-vs-demo diffs against fixtures, including the Mac's
exported document; each block applied alone changes only its values (diffed before and after on the demo device); old option bits
round-trip; a document with an unknown `meta.kind` still imports; a Mac and a Windows-console document still import. **Deps:**
none. **Clients:** an import preview with the diff and block choices (each client's first P1 WP).

**P1NT: One error-key table for notices.** [DONE 5e355a18] Size 0.5. Wave P1-A. **Owns:** `crates/dspi-core/src/error.rs` (a stable key per
error), a new `assets/notices/notices.json` and `crates/dspi-core/src/notice_text.rs` (CG10's placeholder rules); binding wiring
(shared). **Implements:** U5's core half: every typed error carries a key; the table gives, per key, a title, the reason and what
to do, in shared wording with no host-device nouns; a call `notice_for(error)` in every binding. Kinds: info, warning, error;
no "success" kind (a success is shown by the change itself). **Done when:** every `DspiError` variant has a key and a row (a test
fails on a new variant without one); the wording passes the no-host-noun check; C, UniFFI and wasm return the same text for the
same error. **Deps:** none. **Clients:** each client's notice host reads the table (second P1 WP); Swift's alerts move to it
(P1SW3).

**P1SN: Snapshots.** [DONE 4bc97606] Size 3. Wave P1-B. **Owns:** new `crates/dspi-core/src/snapshot.rs` (and `snapshot/` tests), new
`crates/dspi-session/src/snapshot_store.rs`; glossary rows in `docs/GLOSSARY.md`; binding wiring (shared). **Implements:** W3.
Capture: the bulk image plus the device-global settings outside it (names, output latency, pair map, control-surface
configuration), stored as a `.dspipreset` document with `meta.kind = "snapshot"` (P1DF) and a host-side index (name, time,
serial, note) in a folder the client names, one subfolder per serial (as the event log's naming). Recall: one `0xA1` plus the
writes for the non-bulk part, under the device lock (CG24), recorded in the event log as a recall with its diff summary; a
snapshot from another platform or channel count is refused with a reason; one from an older wire version is read through
`wire_compat` and written only when a forward map exists (FW9). Calls: `snapshot_capture(name, note)`, `snapshot_list(serial)`,
`snapshot_recall(id)`, `snapshot_rename`, `snapshot_delete`, `snapshot_export(id)` and import of a document as a snapshot.
**Done when:** tests on the demo device: capture, change values, recall, every value back (bulk and non-bulk), the history ended,
an event-log record; a recall on a dropped connection sends nothing; index corruption (torn file, hand edit) never fails a list;
names with any Unicode; the store under `~/.cache/dspi-build/work/wp-P1SN/` in tests, never tmpfs. **Deps:** P1DF. **Clients:**
a Snapshots window or page in each client (first P1 WP).

**P1OF: Offline editing and templates.** [DONE 0c83f07e] Size 2.5. Wave P1-B. **Owns:** the demo device's seeding in
`crates/dspi-transport/src/fake*` (`FakeDevice::from_document`), new `crates/dspi-core/src/offline.rs`, new
`assets/templates/` (with a K340 template); binding wiring (shared). **Implements:** W7. `open_document(text)` opens a demo device
seeded from a document (platform and channel count from its `meta`), so every screen edits it; `save_document()` exports it;
"Save as template" writes `meta.kind = "template"`; templates listed from `assets/templates/` and the user's folder; a session
on a document says so (`DeviceInfo` marks it, the clients show "File: <name>" in place of a serial; as built, the mark is
`offline_document()` (its JSON) and the serial `OFFLINE-DOCUMENT`, not a `DeviceInfo` field: review p1b F7). Nothing touches USB. **Done
when:** tests: open the Mac fixture, edit EQ, save, diff the saved file against the original (P1DF) shows exactly the edit; a
K340 template opens with 4 outputs and the K340 pair map; a document for another platform opens that platform's demo; undo works
on an offline session. **Deps:** P1DF. **Clients:** "Open File…" and "Save as Template…" in each client (first P1 WP).

**P1TK: Confirmation tokens for dangerous operations.** [DONE d63ea832] Size 2. Wave P1-B. **Owns:** new
`crates/dspi-core/src/confirm.rs`; the gate lines in `presets.rs` (`save_preset`, delete, copy, `factory_reset`),
`hardware.rs` (`enter_bootloader`, saves of output configuration), `vreg.rs`, `firmware.rs`, `files.rs` (`config_import`);
a `ConfirmationRequired` error in `error.rs` with its P1NT row; binding wiring (shared). **Implements:** W9: `confirm_token(op)`
mints a single-use token for one operation, valid 10 s (Android's contract); the gated calls take an optional token and refuse
without one **when the core instance requires tokens** (`require_confirmation(true)`). The default stays off until every client
has adopted it (P1TZ), so no client breaks in between. The `OVERVOLT` word (CG14) stays as it is. **A token is a discipline,
not a boundary:** the caller that mints it also uses it, and over Link each client's core mints its own, unseen by the hub; the
hub's roles stay the boundary, and P1TZ is not a security change. Snapshot recall, the A/B toggle and a guarded apply are
deliberately not token-gated (A/B must be instant; a guarded apply has its own way back). **Done when:** tests: each gated
call with no token, an expired one, one for another op, a reused one, and a good one; a token never crosses a reconnect; C and
wasm refuse the same way. **Deps:** P1NT. **Clients:** each client's confirmation dialogs mint the token and turn the requirement
on (second P1 WP); Android's `DangerousOps` maps onto the core's tokens instead of minting its own.

**P1PR: Protect all outputs in the core; core compound writes as groups.** [DONE a63e3b3e] Size 0.5. Wave P1-B. **Owns:** a
`protect_all_outputs(ceiling_dbfs)` helper beside the limiter setters in `crates/dspi-core/src/params_fx.rs`, the linked-pair
preamp lines in `crates/dspi-core/src/params.rs`, tests `tests_wp_p1pr.rs`; binding wiring (shared).
**Implements:** S2: every output's limiter on at the ceiling (default -1 dBFS), stereo pairs linked, one undo group (P1UG);
plus `outputs_without_limiter(threshold_db)` for the warning "an output above -10 dB gain has no limiter". Where the core itself
writes both sides of a linked input pair's preamp, those writes become one P1UG group. **Done when:** tests on the demo device:
one call, every limiter on and linked, one undo restores all; the warning list; a linked-pair preamp change is one undo step. **Deps:** P1UG. **Clients:** Qt
moves `ToolsController::protectAllOutputs` onto it; web, Android and Swift add the action to their "All outputs" menus (second P1
WP).

**P1HA: Guarded apply on the hub.** [DONE 27cc37bf] Size 1. Wave P1-B. **Owns:** new `crates/dspi-hub/src/apply_guard.rs`, an additive
`apply.begin`/`apply.confirm`/`apply.cancel` entry in `router.rs` and `policy.rs` (Admin and Control), conformance tests.
**Implements:** W6's hub half: on `apply.begin` the hub takes the device's image from its snapshot (`0xA0`), lets the client apply,
and reverts by `0xA1` if `apply.confirm` does not arrive within the timeout or the client's session ends; one guard per device;
the revert is the hub's own write, attributed to the hub, and never runs on another board after a replug (`NO_DEVICE`). Additive
to Link 1.2 and versioned; the upstream page and other hubs are unaffected. **Done when:** hub tests: confirm in time keeps the
change; a timeout reverts; a dropped client reverts; a replugged different serial is not written; a second begin is refused;
a Viewer is refused. **Deps:** none. **Clients:** none directly (P1SA uses it).

**P1AB: A/B compare.** [DONE 0c34b4aa] Size 1. Wave P1-C. **Owns:** new `crates/dspi-core/src/ab.rs`; binding wiring (shared).
**Implements:** W4: two snapshots as A and B (the current state captured as A when none is given), `ab_toggle()` recalls the other
by `0xA1`, `ab_diff()` gives P1DF's rows between them, `ab_keep(which)` ends the compare. The toggle is recorded as a recall (event
log), not as an undo step. **Done when:** tests: toggle twice returns to A exactly; the diff matches P1DF's for the same pair; a
compare ends cleanly on a disconnect. **Deps:** P1SN, P1DF. **Clients:** an A/B control in the Snapshots window and a key (first
P1 WP).

**P1SA: Safe apply with rollback.** [DONE 35cf785a] Size 2.5. Wave P1-C. **Owns:** new `crates/dspi-core/src/guarded.rs`; binding wiring
(shared). **Implements:** W6's core: `apply_guarded(document | snapshot, options, timeout_s)` captures a snapshot, applies (P1DF's
blocks), and then waits for `apply_confirm()`; a timeout, `apply_revert()` or a lost connection reverts to the snapshot. Over Link
the hub runs the timer (P1HA), so a vanished client cannot leave a half-applied device; on USB the core runs it on the caller's
poll (no new thread; the web rule). The countdown and the result are a state the clients poll or are notified of. S7's part:
one test per multi-step flow (an import's hardware part, output enables, a CS apply) that a cut mid-way leaves the device either
applied or reverted. **Done when:** tests on the demo device and through the fake hub: confirm keeps; timeout reverts; a cut
connection over Link reverts on the hub and the client says so on reconnect; nothing is replayed. **Deps:** P1SN, P1DF, P1HA.
**Clients:** a countdown bar with Keep and Revert (first P1 WP).

**P1LM: Limits profile.** [DONE ce3a5892] Size 3. Wave P1-C. **Owns:** new `crates/dspi-session/src/limits.rs` and
`crates/dspi-core/src/limits.rs`; the check call in the one write path (`journal_begin` in `journal.rs`, under a marker); the
clamp-and-report step in `preset_file.rs`'s apply and in `snapshot.rs`'s recall (under markers); `docs/integrator/limits-profiles.md`
(the placeholder becomes the spec); binding wiring (shared). **Implements:** S1. A profile per device (by serial), stored host-side
as a document with `meta.kind = "limits"` and carried in an exported document's `meta`: per output max gain (never above the
registry's +20 dB), max PEQ boost, minimum high-pass frequency and slope, delay range; limiter forced on with a ceiling; master
volume ceiling; which pages are locked; the installer's note and contact. **The PIN is not in the profile document.** Its hash is
kept beside it in a host-only file, salted per device (a random salt with the serial) with a slow hash, and never leaves the
host: not in an exported document, a snapshot, a template or a diagnostics bundle (P1LK uses it). Profile and PIN files are
written atomically (a temporary file and a rename). Enforcement when a profile exists: a direct write outside a limit is refused with P1NT's notice; an import, recall,
A/B or template is shortened to the limit and reported, as delays are today. No profile: today's behaviour, nothing shown. The
UI says plainly that a limit the firmware does not know can be crossed from a control surface, the UART or another console
without the profile (FW2 closes it later).
- **Narrowing a limit** while the device is outside it clamps the device at once when the profile is applied (writes, journalled,
  reported in the import's words); hardware settings apply at once.
- **A preset-slot load** under a profile (the firmware loads the slot's stored values, which the host never clamped): by default
  the core reads the result back, clamps what is outside the profile and reports it (OD-P1j).
- **A corrupt or missing profile** gives a loud notice ("Limits profile unreadable"); by default the core falls back to the last
  good copy if one exists (fails closed), otherwise runs unlimited with the notice standing (OD-P1k).
- **Reverts are never refused:** P1SA's and P1HA's reverts restore a state captured before, past any lock or limit set since.

**Done when:** tests: each limit refuses a typed write, `write_param` and an undo that would cross it, and is clamped-and-reported
in an import and a recall; narrowing a limit clamps and reports; a slot load outside the profile is clamped and reported; no
profile changes nothing (the existing per-setter tests pass unchanged); a corrupt profile with a last good copy keeps that copy,
without one runs unlimited with the notice, never a panic; a torn write leaves the previous file; an exported document, a
snapshot, a template and a diagnostics bundle contain no PIN hash; a guarded revert passes a limit set during its countdown. **Deps:** P1UG, P1DF, P1SN. **Clients:** a Limits
page in Installer mode (second P1 WP).

**P1LK: Lockout, PIN and the mode manifest.** [DONE ae43e568] Size 2. Wave P1-D. **Owns:** new `crates/dspi-core/src/modes.rs`, new
`assets/modes/modes.json` (the manifest of `docs/UI-MODES.md` section 6), the PIN and lock calls and the lock's check in
`crates/dspi-core/src/limits.rs` (P1LM's file, done in P1-C);
`docs/UI-MODES.md` (built parts marked); binding wiring (shared). **Implements:** S4 and U7. The manifest as a typed record on
every binding (modes, screens, operations, defaults per platform class). `pin_set`, `pin_clear`, `unlock(pin)`, `lock()`,
`lock_state()` per device; a device is locked only when an installer has set a PIN and chosen what it covers (section 1 of
UI-MODES: no PIN, no lock, every mode reachable, no prompt); unlocking lasts for the session. The locked-user state allows volume,
mute and preset within the limits; the core refuses other writes while locked (a UI lock, said in one line in the unlock dialog,
not a security boundary). A hub-pinned mode (P1HL) overrides the user's choice. **Done when:** tests: no PIN, every mode and
every write available, no prompt asked for; PIN set and locked, only volume, mute and preset pass and everything else is refused
with a notice; unlock, then all pass; repeated wrong PINs open a refusal window kept on the clock seam (the next allowed
time, never a sleep; the web rule); a profile without a PIN is editable by anyone; the manifest round-trips on C,
UniFFI and wasm. **Deps:** P1LM. **Clients:** the mode switch from the manifest (web, Android, Swift; Qt's stays hidden), the
Installer PIN dialog and the locked-user view (second P1 WP).

**P1AQ: AutoEQ preview.** [DONE a42759c2] Size 0.5. Wave P1-D. **Owns:** a preview and apply-to-pair layer in
`crates/dspi-core/src/autoeq.rs` and `crates/dspi-session/src/autoeq.rs` (`plan` stays); binding wiring (shared).
**Implements:** T6: `autoeq_preview(profile, channel_pair)` returns the bands and the response curve without writing;
`autoeq_apply(profile, channel_pair)` captures an A snapshot first (P1SN), applies in one undo group (P1UG), and goes through the
limits (P1LM); any channel pair, not only inputs 1 and 2. **Done when:** tests: preview writes nothing (transfer count); apply to
outputs 3–4 changes only them; a profile boost above a limit is clamped and reported; the A snapshot recalls the previous state.
**Deps:** P1SN, P1LM, P1UG. **Clients:** the AutoEQ window shows the preview curve on the graph and a channel-pair choice before
Apply (second P1 WP).

**P1HL: Limits and lock on the hub.** [DONE 7750a7b9] Size 2. Wave P1-D. **Owns:** `crates/dspi-hub/src/policy.rs`, `store.rs` (the
profile and PIN hash per device), an additive `limits.get`/`limits.set`/`mode.pinned` entry in `router.rs`, conformance tests.
**Implements:** S1's hub half and S4's: the hub holds a device's profile, checks network clients' writes against it (P1LM's check,
shared code), and refuses with the same notice. **Bulk images:** while a device has a profile or a lock, the hub refuses
`SET_ALL_PARAMS` (0xA1) and `SET_ALL_PARAMS_CHUNK` (0xA3) from Control clients (Admin only), since an image carries any value
past every limit (decode-and-clamp is the later option); so on such a device an import, snapshot recall or guarded apply that
sends an image is Admin-only over Link, while the hub's own guarded revert (P1HA) is never refused. The PIN hash on the hub is
salted and slow as in P1LM. A locked device's Control clients get only the lock's scope (volume,
mute, preset). The hub's own system scenes (SW2: master volume, preset load across units) pass the same check (OD-P1l). An
Admin may set or clear a profile and unlock without the PIN; Control works within
the limits; Viewer reads; a hub may pin a mode per role (`docs/UI-MODES.md` section 4: the `FleetBackend` method). Local USB users
of the same machine keep their own profile copy (P1LM). **Done when:** hub tests: a Control client's write above a limit is
refused; a Control client's bulk image on a limited or on a locked device is refused, an Admin's accepted; a Control client of a
locked device outside the lock's scope is refused; a system scene above a limit is refused or clamped as P1LM says; an Admin's profile change is accepted and a Control's refused, a pinned mode is reported to a client, the store keeps no
plain PIN (diagnostics test from P0E still passes). **Deps:** P1LM, P1HA (router edits sequenced). **Clients:** web's fleet seam
reads the pinned mode (P1WB2).

**P1TM: Terminal commands.** [DONE ce44e4d0] Size 1. Wave P1-E. **Owns:** commands in `crates/dspi-cmd` and `apps/terminal/src/main.rs`'s
table (additive, below TM15's macro block). **Implements:** `uconsole snapshot {save,list,recall,diff,export}`, `uconsole ab`,
`uconsole open FILE` (offline, P1OF), `uconsole diff A B`, `uconsole import --only BLOCKS --guard SECONDS` (P1DF, P1SA),
`uconsole limits {show,set,clear}` (needs the unlock when a PIN is set), `uconsole lock {status,unlock,set-pin,clear-pin}`,
`--yes` minting a confirmation token (P1TK), with `--json` and exit codes; the TUI shows "File:" for an offline session. **Done when:**
command tests against the demo device for each, scripts in `docs/integrator/` updated. **Deps:** P1SN, P1AB, P1OF, P1SA, P1LM, P1LK, P1TK.
**Clients:** Terminal.

**P1TZ: Tokens required by default.** [DONE 39b7811d] Size 0.25. Wave P1-G. **Owns:** the default in `confirm.rs`. **Implements:**
`require_confirmation` defaults on and the switch is removed, once every client's second P1 WP and P1TM have adopted tokens.
**Done when:** every client's suite passes with the default on (0.25 is the agent's time; the Swift run is a VM job). **Deps:** P1QT2, P1WB2, P1AN2, P1SW2, P1TM.

#### Client WPs (one dialog per feature, in each client)

Each client gets two WPs (Swift three), in this order: **first** the snapshot family (W3, W4, W5, W6, W7, W8) and **second** the
Installer and safety family (S1, S2, S4, S5, S8, U5, W9, T6, undo groups). The two of one client never run in one wave.
**Keys (U3):** each client's first WP adds the keys it lacks from Qt's table (`MenuTable.qml`: preset 1–9, commit, revert, mute,
bypass, undo, redo, and the new A/B key), not while a text field has focus; Android for hardware keyboards
(`ConsoleKeyMap.kt`), Swift as `.commands` and iPad keys. No core key table in this phase: Qt's is the reference.
**Limits pages (F3):** every second WP's tests narrow a limit below a current value and show the clamp report.

**P1QT1: Qt: snapshots, A/B, import preview, safe apply, offline files.** [DONE 594a3e36] Size 3. Wave P1-D. **Owns:** new
`SnapshotsController` and `qml/snapshots/*` through the registrar; `FilesController` and `qml/files/*` (import preview, Open
File…, Save as Template…); `MenuTable.qml`/`Menus.qml` entries (this wave's one menu WP). **Implements:** a Snapshots window
(capture, list, rename, note, recall, delete, export) with an A/B toggle (key: the shortcut table's next free entry) and the diff
between A and B; the import dialog shows P1DF's diff and block choices before applying, with "Apply with rollback" and a countdown
bar (Keep, Revert); Open File… opens a document offline with a "File:" badge in the status bar. **Done when:** smoke sections for
each on the demo device, both schemes; no new modal for a success. **Deps:** P1SN, P1AB, P1DF, P1SA, P1OF. **Clients:** Qt.
-- fixup p1cfix (OD-P1g) -- A plain snapshot recall over DSPi Link is guarded too: when `apply_guard_default()` is true
(Link to a hub with `apply_guard`), recall through `apply_guarded_snapshot`/`apply_guarded_snapshot_document`, not
`snapshot_recall`, which the core never guards by itself; on USB it stays the user's choice. While a guard waits, the A/B calls
are refused ("finish the safe apply first: Keep or Revert"): disable the toggle during the countdown; a revert ends the compare.
-- fixup hwlk1f (owner) -- Also: the System window shows `system.set`'s `warnings` after Save (a unit without the audio link;
`link_system_save` returns only the revision today, so this needs the save's answer through the core).

**P1QT2: Qt: Installer page, lock, tokens, AutoEQ preview, notices, groups.** [DONE 53d28d1e] Size 2. Wave P1-E. **Owns:** new
`qml/settings/pages/InstallerPage.qml` (Settings, beside Advanced; the Mode menu stays hidden), `LimitsController`, edits in
`ToolsController` (protect all onto P1PR), `AutoEqController`/`qml/tools/AutoEq*` (preview, channel pair), `NoticeCenter` (P1NT
keys), the Clear All and direct-routing callers (P1UG groups), the confirmation dialogs (P1TK tokens). **Implements:** the Limits
editor, PIN set, clear and unlock, the locked-user view when a device carries a lock; changes apply at once; the S2 warning row.
**Done when:** smoke: set a limit, a refused write shows its notice; set a PIN, lock, only volume, mute and preset work; no PIN,
nothing is ever asked; Clear All is one undo; tokens required on in Qt. **Deps:** P1LM, P1LK, P1PR, P1AQ, P1NT, P1TK, P1UG,
P1QT1. **Clients:** Qt.

**P1WB1: Web: snapshots, A/B, import preview, safe apply, offline files.** [DONE 594a3e36] Size 3.25. Wave P1-D. **Owns:** new
`apps/web/src/screens/snapshots/`, the scene bar's host-side half (`screens/system/` scene bar: WB6e said it moves to snapshots and
A/B when the core has them; the hub's system scenes stay), the files screen's import path. **Implements:** as P1QT1, with files
through browser download and upload; the A/B toggle in the scene bar; Ctrl/Cmd keys not while a field has focus; 375 px without
sideways scroll. **Done when:** unit and e2e tests (Chromium, Firefox, narrow) on the demo device; wasm exports listed. **Deps:**
P1SN, P1AB, P1DF, P1SA, P1OF; **WB6k** (files and AutoEQ slice) for the import path, or this WP builds the import part of WB6k's
scope and WB6k drops it. **Clients:** web.
-- fixup p1cfix (OD-P1g) -- A plain snapshot recall over DSPi Link is guarded too: when `apply_guard_default()` is true
(Link to a hub with `apply_guard`), recall through `apply_guarded_snapshot`/`apply_guarded_snapshot_document`, not
`snapshot_recall`, which the core never guards by itself; on USB it stays the user's choice. While a guard waits, the A/B calls
are refused ("finish the safe apply first: Keep or Revert"): disable the toggle during the countdown; a revert ends the compare.

**P1WB2: Web: Installer, lock, power-on, stale, notices, tokens, AutoEQ preview, groups.** [DONE 53d28d1e] Size 3. Wave P1-E. **Owns:**
new `apps/web/src/screens/installer/`, the mode switch onto P1LK's manifest (`screens/system/model.ts`), the `FleetBackend` pinned
mode (`apps/web/src/fleet/types.ts`, P1HL), `ui/texts.ts` notices onto P1NT, the limiter "All outputs" menu (P1PR), "stale since"
on meters (`SystemStatus.stale`, `age_ms`). **Implements:** Limits and power-on (S5: startup slot, master-volume mode, output
configuration mode, aux power-on states, DAC mute timing, the S/PDIF and ADAT mute-until-lock line, "Test power-on" reloading the
startup slot; changes apply at once), PIN and lock, the locked-user view as the simple page; tokens on; groups for Clear All and
Direct 1:1. **Done when:** e2e for each on the demo device and through the fake hub with a pinned mode. **Deps:** P1LM, P1LK, P1HL,
P1PR, P1AQ, P1NT, P1TK, P1UG, P1WB1; WB6i (settings) for the power-on fields, or this WP carries them. **Clients:** web.

**P1AN1: Android: snapshots, A/B, import preview, safe apply, offline files.** [DONE 594a3e36] Size 3. Wave P1-D. **Owns:** a new
`feature-snapshots` module (or `feature-presets/snapshots/`), the import flow in `feature-presets`, the app-bar menu entry.
**Implements:** as P1QT1, phone layout first (Simple shows Snapshots read-mostly: recall and A/B; Advanced edits), SAF for files,
no visible layout growth. **Done when:** view-model and Robolectric tests, shots into `work/wp-P1AN1/shots/`; Gradle through the
slot. **Deps:** P1SN, P1AB, P1DF, P1SA, P1OF; **AN42** (undo, in progress) merged first, since both touch the app bar. **Clients:**
Android.
-- fixup p1cfix (OD-P1g) -- A plain snapshot recall over DSPi Link is guarded too: when `apply_guard_default()` is true
(Link to a hub with `apply_guard`), recall through `apply_guarded_snapshot`/`apply_guarded_snapshot_document`, not
`snapshot_recall`, which the core never guards by itself; on USB it stays the user's choice. While a guard waits, the A/B calls
are refused ("finish the safe apply first: Keep or Revert"): disable the toggle during the countdown; a revert ends the compare.
-- fixup hwlk1f (owner) -- Also: the System screen shows `system.set`'s `warnings` after Save (a unit without the audio link;
`link_system_save` returns only the revision today, so this needs the save's answer through the core).

**P1AN2: Android: Installer, lock, mode switch, power-on, notices, tokens, AutoEQ preview, groups.** [DONE 53d28d1e] Size 3.5. Wave P1-E.
**Owns:** `feature-settings` Installer and power-on pages, the mode switch from the manifest in the top-bar menu, `DangerousOps*`
in `bridge` (onto P1TK's tokens; the classified list stays), `AlertHost` onto P1NT, the limiter menu (P1PR), AutoEQ preview,
"stale since" on Home meters. **Implements:** as P1QT2 plus the mode switch (Simple default on the phone, per device) and S5.
**Done when:** unit and Robolectric tests, `ReadOnlyTest` and `WriteGuardedCore` updated; a no-PIN run asks nothing. **Deps:**
P1LM, P1LK, P1PR, P1AQ, P1NT, P1TK, P1UG, P1AN1. **Clients:** Android.

**P1SW1: Swift: snapshots, A/B, import preview, safe apply, offline files.** [DONE ce44e4d0] Size 3.5. Wave P1-E (VM). **Owns:** new
`Snapshots*.swift` views and a `CoreSnapshots` wrapper; the file import path (SW17's) gains the preview. **Implements:** as P1QT1, on
macOS and iOS (iOS: sheets at half height, Simple first). **Done when:** macOS and iOS test bundles on the VM. **Deps:** P1SN,
P1AB, P1DF, P1SA, P1OF; **the SW chain SW11–SW14** (the domains on the core's setters, so a recall and a revert show everywhere),
SW17 (done). **Clients:** Swift.
-- fixup p1cfix (OD-P1g) -- A plain snapshot recall over DSPi Link is guarded too: when `apply_guard_default()` is true
(Link to a hub with `apply_guard`), recall through `apply_guarded_snapshot`/`apply_guarded_snapshot_document`, not
`snapshot_recall`, which the core never guards by itself; on USB it stays the user's choice. While a guard waits, the A/B calls
are refused ("finish the safe apply first: Keep or Revert"): disable the toggle during the countdown; a revert ends the compare.

**P1SW2: Swift: Installer, lock, mode switch, power-on, tokens, AutoEQ preview, groups.** [DONE d6b7bb22] Size 2.5. Wave P1-F (VM).
**Owns:** new Installer and power-on views, the mode switch, the confirmation dialogs (tokens), the AutoEQ window (SW18's), the
Clear All, linked-pair and direct-routing callers (groups; review sw9). **Implements:** as P1AN2 (macOS Advanced by default, iOS
Simple); checks the crossfeed graph no longer flips (P1XF). **Done when:** test bundles on the VM. **Deps:** P1LM, P1LK, P1PR,
P1AQ, P1TK, P1UG, P1XF, P1SW1; SW18. **Clients:** Swift.

**P1SW3: Swift: notices and the read-only floor.** [DONE db3a9ad5] Size 2.5. Wave P1-G (VM). **Owns:** the alert sites across
`apps/apple/DSPi Console/` (about 70), a notice host view, the read-only state. **Implements:** U5: success alerts removed, the
rest non-modal notices from P1NT's table, modals kept only where a decision is asked; S8: complete Qt's read-only floor (every control off,
reads continue, banner; today banner and feature hiding), "stale since" on meters. **Done when:** test bundles; an alert
count before and after in the report; the V17, V26, V31 and V33 fixtures read-only. **Deps:** P1NT, P1SW2 (same files); runs
before or after SW22 (accessibility), never beside it (same files). **Clients:** Swift.

#### Waves

No two WPs in one wave own the same files; binding wiring is additive and the generated files are regenerated at merge.

| Wave | WPs (parallel) | Size | Needs |
|---|---|---|---|
| P1-A | P1UG, P1XF, P1DF, P1NT (AN42 runs now, outside this plan) | 4.5 | `main` |
| P1-B | P1SN, P1OF, P1TK, P1PR, P1HA | 9 | P1-A |
| P1-C | P1AB, P1SA, P1LM | 6.5 | P1-B |
| P1-D | P1LK, P1AQ, P1HL; P1QT1, P1WB1, P1AN1 | 13.75 | P1-C; WB6k for P1WB1; AN42 for P1AN1 |
| P1-E | P1QT2, P1WB2, P1AN2, P1TM; P1SW1 (VM) | 13 | P1-D; SW11–SW14 for P1SW1 |
| P1-F | P1SW2 (VM) | 2.5 | P1-E; SW18 |
| P1-G | P1SW3 (VM), P1TZ | 2.75 | P1-F (no shared files between the two) |
| | **Total: 26 WPs** | **52** | core 21.5 + P1TZ 0.25, hub 3, Terminal 1, Qt 5, web 6.25, Android 6.5, Swift 8.5 |

Sizes are this plan's units (a size-1 WP took 10–35 minutes of agent time, about an hour with review). Each wave ends with one
Fable batch review; the full gate runs after P1-C (the core complete) and after P1-G. The coordinator decides at P1-D's start
whether P1WB1 waits for WB6k or builds its import part (WB6k is late in the WB6 order). Swift's three WPs follow the VM queue and
can trail the other clients without blocking them.

#### Owner decisions for phase 1

**DECIDED 2026-11-13 (owner): all of OD-P1a to OD-P1m as recommended below**, with one addition: in code and terminology the audio link is **alink** and DSPi Link is **dlink** (dlink may be skipped, but the audio link's shorthand is never bare "link"). User-facing text keeps "Audio link" and "DSPi Link". New code uses alink names now; existing audio-link `link_*` names are renamed in dedicated WPs (ALINK1 console, ALINK2 firmware) once no other WP owns those files.

| # | Decision | Recommendation |
|---|---|---|
| **OD-P1a** | **Snapshot or scene.** PRO1 called a numbered snapshot a "scene"; the hub's audio-link system already has scenes (frame-timed ops across units, SW2). | Keep **Snapshot** for host-side whole-device states and **Scene** for the hub's; a hub scene may recall a snapshot in phase 2 |
| **OD-P1b** | **Where snapshots live** (PRO1 question 2). | Locally per client and serial in this phase, in one index format (`snapshot_store.rs`) so a hub copy can merge later; on the web "local" is the browser (download and upload); a shared copy on the hub in phase 2, the hub's winning |
| **OD-P1c** | **A forgotten PIN** (`docs/UI-MODES.md` 9.1). On a hub an Admin clears it. Off a hub, how much friction? (1) a **recovery code shown once when the PIN is set**, for the hand-over note; (2) deleting the per-serial profile file, documented in the integrator guide as the last resort; (3) a "Remove lock" button in the unlock dialog behind a typed word, which the locked user can read too | (1) with (2) as the documented last resort; not (3), which makes the lock a speed bump for the person it is meant to steer |
| **OD-P1d** | **The lock does not travel with the device.** Profiles and PINs are host-side and on the hub; the firmware has no store for them, so the same DSPi on another console is unlocked. | Accept; the unlock dialog says "this lock is on this console; the device itself is open"; a device-side marker is a firmware item for later (with FW2) |
| **OD-P1e** | **Qt's Installer entry** while Window > Mode… stays hidden. | Settings > Installer, like the Core Voltage page under Settings > Advanced; unhide the Mode menu when the owner says |
| **OD-P1f** | **Tokens required by default** (P1TZ) changes the C ABI's behaviour for any outside user of it. | Flip after every client has adopted it; no outside users are known |
| **OD-P1g** | **Safe apply's default**, and whether a plain snapshot recall over Link (bulk only) is guarded too. | On by default over Link (hub timer) for imports and recalls that carry a hardware or non-bulk part, opt-in on USB; 30 s, adjustable; an A/B toggle is never guarded; a plain recall over Link guarded too (the "apply on site with a way back" case) |
| **OD-P1h** | **The K340 template's content.** It would be derived from the owner's K340 configuration (its tuning, names and pair map). | Ship the routing, pair map, crossover, names and BUG1's derived labels, without the owner's room EQ; the owner approves the file |
| **OD-P1i** | **Locked-user over Link.** With roles Admin, Control and Viewer only, a venue phone is Control within the limits, or Viewer. | Control within the hub's limits, with the lock's scope (volume, mute, preset) applied by the hub to Control clients of a locked device, bulk images refused (P1HL); Viewer for a phone that only watches |
| **OD-P1j** | **Preset-slot loads under a profile.** A slot holds values the host never clamped. | Clamp after the load and report (the default in P1LM); alternatives: refuse slots whose stored values are outside the profile, or accept and warn |
| **OD-P1k** | **A corrupt or missing profile.** | A loud notice; fall back to the last good copy if one exists (fails closed), otherwise run unlimited with the notice standing (the default in P1LM). The alternative, refusing writes until an installer restores it, risks a hard lockout |
| **OD-P1l** | **Limits on the hub's own system scenes** (SW2: master volume and preset load across units). | Yes, the same check in P1HL |
| **OD-P1m** | **An installer hand-over sheet**: one page generated from the profile (limits, contact, the recovery code of OD-P1c, what the lock is and is not), about 0.5; PRO1's "hand it over" has no WP otherwise (E6, the manual, is later). | Add it as a WP after P1LK, in the core's wording, shown and printed by each client's Installer page |

-- WP PAIRADM -- **Admin on a hub (owner, 2026-11-14; review-hubadmin item 7, option C).** Pairing with a hub's PIN gives at most the Control role, enforced on the hub; `uconsole hub --pair-admin` (HWLK1F's admin pairing window) is removed and refused with a message pointing to `hub role`. The only way to Admin (needed for the hub's limits, installer PINs and pinned modes under OD-P1c and OD-P1i) is on the hub's machine: `uconsole hub clients` and `uconsole hub role <cid> admin` with the hub stopped, or `clients` and `role <cid> admin` at the running hub's prompt, which applies live. The notice row `installer.hub_role` says so on every read-only Installer page.

---

## 6. Hardware and owner-only tasks

**Only the owner:** contacts Weeb Labs; makes any repo public; uploads to a store; picks licences; types passwords. **No agent plans or performs these.** Agents prepare material when asked.

**Rules for every hardware step:**
- back up settings and flash before any flash;
- **never a 1.15 V build on the K340**: it needs 1.20 V;
- at most 80 % volume, and test audio only through the guarded player;
- **the coordinator runs flashes.** Subagents have refused to flash on relayed approval, rightly; they prepare, the coordinator acts;
- **core-voltage tests run on the K340 only and stay at 1.20 V, unless the owner approves another voltage at that step;**
- run `scripts/check_usb_config.py` on any firmware with a descriptor change;
- the Mac VM needs a GUI login and an unlocked host.

| Task | Needs | WPs |
|---|---|---|
| Phone adb port (changes each time) | Owner tells the port | AN13, AN15, AN16, AN20, AN22 |
| Play upload, forms, closed-test updates | Owner | AN14 (done: 0.12.0), AN22 (AN15's release), AN16 |
| ESP32 bridge wired to a DSPi | Owner: is one available? | XC4, XC5 |
| Real iPhone or iPad, signing team | Owner | SW16 |
| Apple silicon Mac; Developer ID if distributing | Owner | SW-M5 |
| `macr` restart, DSPi moved between VMs | Coordinator; owner agrees to the restart | XC1, QD25, SW5, SW6, SW15, SW-M1's check, HB7 |
| Windows clean machine, Android Chrome | Owner, or `winvm` with the K340 | WB8 |
| Real flash on the K340 | Coordinator; backups first | QD24, QD25, TM13 |
| Core-voltage test on the K340 | Coordinator; backups first | XC6 |
| Feel check on real hardware | Owner | SW-V1 |

---

## 7. Waves and gates

Every wave needs `core-merge` on `main`. Within a wave no two WPs own the same file (section 1), unless the wave table sequences them ("A, then B") and B's Deps name A. A batch of 4–8 WPs goes to one Fable reviewer.

| Wave | WPs (parallel) | Needs |
|---|---|---|
| 0 | XC0, XC1, XC2, XC3; MR1 (Android half); LC1; CG9 | `main`; XC1 the owner's go-ahead |
| 1 | MR2; AN1; CG1–CG8, CG10–CG12; HB0; WB0; QD0 first, then QD1, QD2, QD4, then QD5; QD30; TM1, TM2, TM4, TM7, TM10, TM14, TM15, TM17 | wave 0 |
| 2 | WB1; **HB1–HB3 after HB0's review**; MR1 (Swift half), then SW1, then SW2; SW-U1 after MR1; QD3, QD6, QD7, QD8, QD11, QD12, QD18, QD27, QD29; TM3, TM5, TM8, TM9, TM12, TM16, TM18; AN2, AN3, AN6, AN8, AN25 | wave 1 |
| 3 | HB5, HB6; SW3, then CG23–CG26, then SW4; CG27, CG28; AN34 then AN35, AN36; QD9, QD10, QD13, QD14, QD16, QD17, QD19, QD28; TM6, TM11; AN4, AN5, AN7, AN14 (on AN13's partial checks), AN17, AN18, AN19, AN23 | wave 2; **M0 at its end** |
| 4 | HB7; SW5, SW6 then SW7, SW-M1 then SW-M2, SW17–SW20, SW23; QD21a; TM13; AN9–AN12 | wave 3 |
| 5 | SW8, QD21b, then SW-V1 **(then OD3)**; QD24, QD25, QD35; AN15 then AN22, then AN16; AN13's rest; XC4, XC5, XC6 | wave 4 |
| 6 | SW9–SW14; SW-M3 after HB7 | OD3 decided: Swift (2026-11-13) |
| 7 | QD22 then QD23; AN20 then AN21; SW15, SW16, SW22, SW-M4, SW-M5; HO1 | everything else in that client |
| P1-A to P1-G | PRO phase 1: section 5.9's wave table | `main` with phase 0 (P0W, P0W2, P0M, P0U, P0E) |
| After | the owner's order: web (WB2-WB9, section 5.5), then optimisation (OPT0-OPT9, 5.7), then polish (PL-*, 5.8). Per resource: web WPs start beside wave 7 when workers would otherwise idle | the plan's WPs on that resource |

WPs added after this table, as review and hardware follow-ups, ran where their findings came up: CG29-CG31, HB8, QD13b,
QD36-QD38, TM25, TM26, AN37-AN39, SW25, SWF1-SWF3, DOC1, HO1a (all done). The table is a plan, not a log; the status
section at the top is the record.

**Review gates:**
1. **This plan** gets a Fable review before wave 0.
2. **After each wave:** the full `ci/check.sh --all`, MR2's matrix once it exists, a code-quality review, and an adversarial review, especially the owner's Link rule and "no write in read-only".
3. **HB0** and **SW3** get a design review before work builds on them.
4. **M0:** the full matrix passes, every row run, and the owner reviews it. It does not block CG, HB or QD work in flight. **Reached 2026-11-08** (gate-3 on `bc1f9ff4`, section 0.3); the owner's review is next.
5. **Before each release:**

| Gate | Must pass | Then |
|---|---|---|
| **R-Qt** | Section 3's Qt column, QD24, QD25; no setter on read-only; Link rule on a real hub; OD4 images and CG8 | Owner decides to publish |
| **R-Term** | TM13; TM10's round-trip with Qt's and Mac's documents | Owner decides |
| **R-And** | AN13 on the phone; `tools/ci.sh release`; no secrets in logs; Data safety matches; source for the shipped build (the Play link points at public `dspi-android`, which won't hold it: an owner step per release, a source tarball or a public mirror of `apps/android`, OD1) | Owner uploads each test update (AN14 done; AN22 with AN15, then AN16) |
| **R-Mac** | SW-M1, SW-M2, SW-M4, SW-M5, SW8–SW14, HO1. **If OD3 stops:** SW4–SW7, SW-M1, SW-M2, SW-M5, HO1. Either way, it **does not wait for HB or SW-M3** | Owner; OD-DevID if distributing |
| **R-iOS** | SW15, SW16, HO1, LC1's inventory current; review video; **the SysEx ID question answered (below)**; **the owner has confirmed the licence terms** | **Weeb Labs submit it, on their account.** Agents do neither |
| **R-Web** | WB2-WB8; the demo, WebUSB (Chromium) and Link routes pass in headless browsers (WB5-WB7); each WB8 row passes or is recorded; the web column of `docs/PARITY.md` (WB9); no write in read-only; the owner's Link rule; standalone and embedded builds both pass (owner's decision 2, section 5.5) | Owner decides to publish (publishing is only the owner's act) |

---

## 8. Decisions

### Recorded (owner's, not open)
- **Purpose:** one project, built from one base, for every client.
- **Frontends:** SwiftUI on macOS and iOS over the Rust core, UI only. Qt on Windows and Linux. (Old OD2 is closed.)
- **Feature floor:** the macOS app, for all clients.
- **Link hub on mobile:** none on iOS or in Android's Play build. A sideload Android hub build is a possible later item (section 9).
- **Terminal:** skips graphical-only features; covers non-graphical equivalents, JSON import and export included.
- **Rust Link hub:** in scope for Qt.
- **No DriverKit;** no USB firmware update from iOS.
- **Store (was OD-Store):** the owner keeps the Play listing and upload key, so `applicationId se.bofr.dspi_console_android` stays. The Apple side (App Store, team, bundle IDs, signing) goes through Weeb Labs' developer account.
- **Licence direction (was the OD-Lic route):** dual-license only the parts needed for the App Store build, from Weeb Labs and from the owner, each with an iOS-friendly licence. The rest stays GPL. Exact terms: owner and Weeb Labs. LC1 supplies the inventory.
- **Repo (OD-Repo):** monorepo.
- **Disabled Mac features (OD-Floor-disabled):** do not count, "not for now at least". Later items.
- **Rust hub on the Mac (OD-Hub):** yes, once it is in the other versions (SW-M3). R-Mac does not wait for it.
- **Visibility (OD1):** "private until discussion with weeblabs". Source tarballs on closed-test releases.
- **Bundled firmware (OD4):** stock, built from a tag. K340 owners use a local `.uf2`, under CG8.
- **Qt undo (OD5):** "no for now". No WP; section 1's rule keeps it possible. **Superseded (owner, 2026-11-12):** undo in all apps, on the core's journal (P0W); Qt has it since QP18, Android in AN42 (section 5.9).
- **Localisation (OD6):** "have the plumbing there, but just english for now".
- **Core voltage in GUIs (OD7):** writes under an Advanced menu, plus a calibration flow, in Qt, Android and Swift, with logic in the core (CG11, QD28, AN23, SW23, XC6). This replaces the earlier "status only" recommendation.
- **Qt self-update (OD8):** check-and-link, opt-in.
- **SysEx ID (OD9):** "no for now". No registration; `0x7D` stays. **This limits public MIDI releases, iOS included:** the MIDI Association's policy says `0x7D` is not for public products. **The question comes back before R-iOS**, and before any public firmware with MIDI.
- **Windows C# (OD10):** retired after `docs/PARITY.md`'s rows are settled. That is our line; Weeb Labs' repo stays theirs.
- **`dspi hub` (OD11):** yes, with service placeholders (TM11).
- **Web (2026-11-07):** scheduled after the plan's WPs: the Rust core in wasm and a minimal client in this repo, no external project. Then optimisation, then polish.
- **Hub web page (2026-11-08, was HANDOVER question 4):** keep the upstream DSPi-Link page in `dspi hub` (HB5); replace it with ours only if it clashes or lacks what is needed.
- **Web framework and model (2026-11-08):** a natural fit with the Rust core in wasm. Main use: a web hub collecting several DSPi installations. It runs standalone and is embeddable in a server-based fleet version with sign-in and access control: multi-device and multi-hub, pluggable auth, no single-device assumptions (section 5.5).
- **Earlier decisions kept:** no separate Android Link patch; R8 and edge-to-edge in separate releases.

### For the owner

| # | Decision | Recommendation |
|---|---|---|
| **OD-P1a to OD-P1m** | PRO phase 1's questions: snapshot or scene, where snapshots live, PIN recovery, the lock's reach, Qt's Installer entry, tokens by default, safe apply's default, the K340 template, locked users over Link, slot loads, a corrupt profile, limits on hub scenes, a hand-over sheet. | Section 5.9 |
| **OD-Lic** | **The exact iOS terms. Deferred by the owner.** The direction is decided (above). Yours and Weeb Labs'. | **Not legal advice.** Settle from LC1's list when the time comes: more code in the iOS binary means more to license. Also ask Weeb Labs for a Console-Linux licence and which of their Mac app's `LICENSE` (GPL) and README (MIT) holds. Remove §7 terms on what Weeb Labs would own. R-iOS stays gated. |
| **OD-WL** | Propose the core-based frontend to Weeb Labs, and when. **Deferred by the owner.** | **At M0**, with its matrix and parity table, and SW-V1's evidence if you want the Swift case stronger. Only ever your act. |
| **OD-DevID** | Apple Developer ID ($99/yr). **Deferred by the owner.** | **Only if we distribute a Mac binary.** Otherwise Weeb Labs sign theirs. |
| **OD3** [DECIDED 2026-11-13] | **Final call on Swift against Qt on the Mac, and how far Swift's move goes.** **Decided by the owner: the macOS Qt build is dropped, so Swift is the Mac and iOS app, and SW9–SW14 go ahead.** Owner: "come back with visual checks and feel". Working assumption: Swift. Comes with SW-V1 after SW8. It also decides SW9–SW14 (9–15.5 more). | **Decide on SW-V1 and SW8's evidence.** If it goes to Qt on the Mac: the Qt `.dmg` is buildable (QD21b); add back the macOS halves of QD12, QD13, QD18 and a hardware pass (about 1.5), and iOS stays Swift regardless. If Swift stops short of SW14, R-Mac uses its fallback. |

---

## 9. Unknowns and later work

| Item | Rough size | Pinned by |
|---|---|---|
| Swift domain replacement (SW8–SW14) | 12–20.5, could be 30 | SW3 (map), SW8 (hardest part first). Mirror and thread problems could change the approach |
| Android control-surface editor (AN9–AN12) | 5.5–9.5 | AN8. Phone layout for a 5 k-line desktop tab; IR and display need hardware; `FakeDevice` work may add 0.5 |
| Web build | 39–62 with a new UI (WEB-STUDY.md); scheduled (section 5.5) | WB2 (runtime) and WB3 (binding) first; WB6's slices are most of it |
| Rust hub (HB) | 12, could be 18 | HB0. Multi-device ownership in the core is the risk |
| nusb `from_fd` on Android | 0–2 | AN13: first run on a phone. No rusb fallback remains |
| Licence terms | 0 to rewriting a part | LC1. A contributor who cannot be reached, or an incompatible third-party part, would mean replacing that part in the iOS build |
| Upstream Swift merges | 0.5–1 each, recurring | SW-U1's dry run |
| Final Swift or Qt call on Mac | 0, or about 1.5 back in Qt | SW-V1, OD3 |
| Accessibility work | ±50 % | The first screen of each |
| Real ESP32 behaviour | unknown | XC4. Fixes file into `dspi-link` as new WPs |

**Not in any wave** (when the plan, the web, optimisation and polish are done, these come next, or the owner drops them):
- **Android sideload build with a hub**, for a custom player: 3–5. A UniFFI wrapper for `dspi-hub` (shared with SW-M3), NSD registration, a foreground-service hub, settings; its own application id; no Play distribution. Nothing is needed now.
- **Coach marks and hints:** QD15 1.5, SW21 1, AN19 0.5; `OnboardingCatalogue` into CG10. Off in the shipped Mac build; "not for now at least".
- **Qt undo and redo:** 1–2. "No for now."
- **Terminal `dspi ext vreg calibrate`** over CG11: 0.25, if wanted; the session crate already holds the logic.
- **`fw.install` in hub and client:** 1–2. No hub implements it. Would give iOS its only firmware route.
- Hub with TLS: 2–4. No hub has it; would also allow browser access from `https://`.
- Android `MediaSession` and volume keys (optional in the audit): 1.
- `P2` in `PLAN.md` (shared protocol tables).
- App Store and code-signing work beyond the WPs above.
- For the owner to raise with Weeb Labs, not from here: H1–H4 in `LINK-HUB-FINDINGS.md`, the `0x00` entry in `policy/commands.json`.
