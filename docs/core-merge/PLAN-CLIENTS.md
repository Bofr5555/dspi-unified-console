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
| Web | Decided by a study | WB0 |

**Start:** when `core-merge` reaches `main`. Owner's standing instruction: start
the first wave then, without asking again. **Every WP depends on that merge**, and
WP branches are cut from `main`. Hardware, flash, release and password steps keep
their own approvals (section 6). **Contacting Weeb Labs or making anything public is
only ever the owner's act. No agent plans or does either.**

Sizes are relative units, for comparing WPs. Measured in the core merge: a size-1 WP took 10–35 minutes of agent time, about an hour with review and fixups. They are estimates, and elapsed time depends on parallelism and on hardware and owner steps.

| Track | WPs | Size (units) | Depends on |
|---|---|---|---|
| MR: one repo, one build | 2 | 2.25 | `main` |
| LC, HO: licence inventory, handover | 2 | 1.75 | `main`; HO1 late |
| CG: core gaps | 12 | 9 | `main` |
| HB: Rust Link hub | 7 | 12 | `main` |
| QD: Qt (Linux, Windows) | 28 | 22.5 | `main`, CG, HB for the hub page |
| TM: Terminal | 18 | 9.75 | `main`, CG, HB for `dspi hub` |
| AN: Android | 24 | 25.75–29.75 | `main`, MR1, CG9 |
| SW: Swift (macOS and iOS) | 29 | 32.75–43.25 | `main`, MR1, CG3, CG4 |
| WB: web | 1 + stubs | 2 (study) | `main` |
| XC: audit, cross-client tests | 7 | 3.75 | hub hardware, K340 |
| **Total** | **130** | **about 121–136**, not counting the web build | |

**M0, the first milestone, is about 12–14 days after `main`** (section 0.3). The
largest unknowns are Swift domain replacement, the Android control-surface editor
and the web build (section 9).

---

## 0. One base

### 0.1 One repo

Target layout: `crates/`, `apps/desktop`, `apps/terminal`, `apps/android`,
`apps/apple`. **Owner decided: monorepo** ("yes"). This replaces the earlier "separate branch of the
Console fork" rule.

**MR1: Import with history.** Size 0.75. Two halves.
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

**MR2: One entry point.** Size 1.5.
- **Owns:** `ci/check.sh --all-clients` (or `xtask`, which is an empty stub today), `ci/matrix.sh`, `ci/run.sh`, `ci/container/`.
- **Implements:** from one commit, build and test everything, then print a matrix:

| Client | Where |
|---|---|
| Linux Qt, Terminal | container |
| Windows Qt, Terminal | `xwin` in a container; run in `winvm` when scheduled |
| Android AAB | Gradle container |
| wasm check | container |
| macOS Swift app, iOS Simulator, Qt `.dmg` (build only) | `macvm` through `launchctl asuser`: GUI login, host unlocked, one heavy job at a time. **"Not run" until SW2 adds these rows**, and later whenever the VM is down |

- **Reproducible images:** `ci/run.sh`'s base images `localhost/dspi-qtport:4` and `localhost/dspi-termport-xwin:1` have no Containerfile in the repo (`ci/container/Containerfile` only builds on top of the first), so a clean machine cannot recreate them. **Commit a Containerfile for every image**, with pinned bases and verified downloads.
- **CG3's iOS check:** Linux containers have no Apple SDK, so the container step is
  `cargo check --target aarch64-apple-ios` only, with no link. If a dependency's build
  script needs the SDK, it moves to the VM. Real linking is always `macvm` (SW1).
- **Drift step:** regenerate `dspi_core.h`, the Kotlin contract and bindings, and the Swift
  bindings, then fail on any diff. **Generated code is never hand-edited.**
- **Done when:** a run from a clean checkout gives the matrix with every non-Apple row, and the macOS and iOS
  rows reading "not run"; an edited generated file fails it; **every image builds from its committed Containerfile with an empty podman and download cache.** **The full matrix is M0's gate.**
- **Deps:** MR1, CG9.

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

**LC1: Licence inventory, iOS first.** Size 0.75. **Owns:** `docs/licensing/IOS-INVENTORY.md`, a check script in `ci/`, `NOTICE.md`,
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

**HO1: Handover pack.** Size 1. Late: before R-Mac and R-iOS. **Owns:** `README.md` top-level map, `Documentation/`
per app, `docs/HANDOVER.md`.
- **Parity against each console replaced:** macOS, Windows C#, Linux, Terminal, as rows of `docs/PARITY.md`. Windows C#
  rows to settle, from the audit: undo and redo (Qt has none; C# has IO undo; owner: no for now), the 0xFC/0xFD pair mask
  (`DspDevice.cs:288` against Qt's `crossfeedOutputPairMask`), settings depth, the bulk-endpoint debug window, 12 PEQ bands
  against 10.
- **Stale docs:** `apps/terminal/README.md`, `docs/terminal/platforms.md` and `apps/desktop/packaging/arch/PKGBUILD` still describe the pre-merge state. Bring them up to date, and retire `apps/desktop/Documentation/parity-plan.md` in favour of `docs/PARITY.md`.
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
  write.** Wording comes from CG10.
- **After a reconnect** the client calls `rta_start` again if its analyser was running. A listener callback must
  not call a `set_*_listener` method: it self-deadlocks (review 3-A).
- **C15's surface to use:** `link_connect`'s fifth argument `expected_hub_id`, error code `wrong_hub`, and
  `SystemStatus.stale` and `age_ms` for stale meters.
- **Blocking calls stay off the UI thread**, `poll_hotplug` included (F1 note).
- **Same-file ownership:**
  - Qt: **one WP per wave may edit `DSPiBridge.{h,cpp}` and `main.qml`** (W1 QD0, W2 QD6, W3 QD17). All others add a
    controller and QML file through QD0's registrar and banner host.
  - Terminal: `apps/terminal/src/main.rs`'s command table takes additive edits under a `// -- WP <id> --` marker.
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
| M53 | Stats for Nerds, Reset Watermarks, Output Ring Health (`StatsView.swift:206-223,478-504,918-1187`) | partial: no ring health; CG12, QD29 | partial: no ring health; CG12, TM16 | partial: no ring health; CG12, AN25 | have |
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

**CG1: C ABI for Link and MIDI.** Size 1.
- **Owns:** `crates/dspi-capi/src/fns_link.rs`.
- **Implements:** every `link_*` function (with `expected_hub_id`), the listener and the free functions; `dspi_open_midi` behind `midir`.
- **Done when:** `ci/check.sh --crate dspi-capi` passes; `link_close` works from a listener thread; Q1's C ABI rules hold (failure values, null and short buffers). **Deps:** `main`.

**CG2: Find hubs in the core.** Size 1.
- **Owns:** `crates/dspi-link/src/discovery.rs`, `crates/dspi-core/src/link_browse.rs`, `crates/dspi-capi/src/fns_browse.rs`.
- **Implements:** browse `_dspi._tcp` through `mdns-sd` behind a feature, off for mobile; start, stop, results listener; resolve by hub id (F5). HB5 reuses the responder half.
- **Done when:** tests pass against a fake advertiser; mobile libraries do not link it; the dependency reason is stated. **Deps:** `main`.

**CG3: USB as a feature.** Size 1.
- **Owns:** `usb` feature in `crates/dspi-transport/Cargo.toml` and `crates/dspi-core/Cargo.toml`; its gates in `device.rs`, `firmware.rs`, `crates/dspi-transport/src/usb.rs`; an `ios-check` step in `ci/check.sh`.
- **Implements:** on for desktop, Android and macOS (`nusb` has `macos_iokit`); off for iOS.
- **Done when:** `dspi-ffi` passes `cargo check` for `aarch64-apple-ios` with `usb` off, as in section 0.2; default builds are unchanged. **Deps:** `main`.

**CG4: Raw commands for bindings.** Size 0.5.
- **Owns:** `crates/dspi-core/src/raw.rs`.
- **Implements:** `raw_get` and `raw_set` on `DspiCore` (exported), so Swift that has not moved yet shares the core's one link (SW4). `raw_set` goes through `Writable`. Adds a §1.2 line in `PLAN.md`.
- **Done when:** tests cover read-only refusal and bulk-session exclusion. **Deps:** `main`.

**CG5: Analyser following state.** Size 0.5.
- **Owns:** `crates/dspi-core/src/rta.rs`.
- **Implements:** driving or following another session, with the origin, from `RTA_CONFIG`, like `case following(origin:)` at `SpectrumAnalyser.swift:84`; `take_control`.
- **Done when:** a fake-hub test shows another session's config switches state, our own does not, and Take control works. **Deps:** `main`.

**CG6: AutoEQ update with a host-supplied fetch.** Size 1.
- **Owns:** `crates/dspi-session/src/autoeq.rs` (extended, **no new file**), `crates/dspi-core/src/autoeq.rs` (new binding).
- **Implements:** update, rebuild from GitHub, import and Reset to Built-in, through a fetch callback (reference `AutoEQManager.swift:248,331,500`, `DSPi_ConsoleApp.swift:11391`). No HTTP client in the core.
- **Done when:** tests pass with a scripted fetch, including rate limit and partial failure; exported to C and UniFFI. **Deps:** `main`.

**CG7: Preset diff at the floor.** Size 0.5.
- **Owns:** the diff section of `crates/dspi-core/src/presets.rs`.
- **Implements:** Swift's table, `PresetSnapshot.swift`: preamps and pair map added; user volume and DAC mute as Swift treats them.
- **Done when:** a test covers each row against Swift's wording. **Deps:** `main`.

**CG8: Firmware guard.** Size 0.75.
- **Owns:** `crates/dspi-core/src/firmware_guard.rs`.
- **How an image's voltage is read:** firmware with `core_vreg` boots at max(stored, build default) (`core_vreg.c:59-62`), so a stored or built 1.20 V holds. A stock image has no `core_vreg` code at all, so a stored value does nothing and it runs at 1.15 V. The guard scans the UF2 for the build's marker: its `binary_info` stamp, or a `core_vreg` marker. **Absent means unknown, treated as 1.15 V.** Today's images carry no such stamp, so all count as unknown; adding one to `bofr-main` is a small firmware item for the coordinator, and nothing depends on it.
- **Which boards are at risk:** the connected board reports a build default or live voltage above 1.15 V on `EXT_CORE_VREG`. A board already in the bootloader is unknown, so ask.
- **Rule:** at-risk board plus unknown or lower image: **blocked without typed confirmation.** Every updater calls it (QD11, TM3, AN18, SW-M2).
- **Done when:** tests cover K340-like, stock, no-extension and in-bootloader cases; exported to C and UniFFI. **Deps:** `main`.

**CG9: Compile the generated bindings in CI.** Size 0.5. **Wave 0**, since AN1 depends on it.
- **Owns:** `kotlin` and `swift` steps in `ci/check.sh`, the CI image's Containerfile, `crates/dspi-ffi/tests/bindgen_wp_f1.rs`.
- **Implements:** compile F1's Kotlin with `kotlinc` in the container. Compile the Swift with an official Swift toolchain in the container if it works there, otherwise as a VM step. Downloads pinned and verified. A Kotlin compile would have caught review 3-A's breaks.
- **Done when:** both steps pass, and a deliberately broken namespace fails them. **Deps:** `main`.

**CG10: Shared wording and onboarding data.** Size 0.5. **Owns:** `crates/dspi-core/src/link_text.rs`, `assets/onboarding/{whatsnew,getting_started}.json`.
- **Implements:** one `LinkPhase` wording table as stable keys plus English: `NeedsPairing` (by `pin_source`), `NoDevice` ("Connected to hub, no DSPi found"), `ChooseDevice`, `Reconnecting`, `Refused`, `Closed`. Clients localise by key.
- **Done when:** every phase has text; exported to C and UniFFI. **Deps:** `main`.

**CG11: Core-voltage calibration.** Size 1.5. Owner: writes go "under an advanced menu (we could put a 'calibrate core voltage' there as well)".
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

**CG12: Output ring health.** Size 0.25. From XC0 (M53).
- **Owns:** new `crates/dspi-session/src/ring_health.rs`, a binding in `crates/dspi-core/src/stats.rs`.
- **Implements:** Fill Target, Underrun Episodes, Refused Blocks, Lowest Fill, Worst Input Gap, Worst USB Gap and Over-Budget Steps, from `REQ_OUTPUT_LATENCY` index 2 with `REQ_GET_STATUS` 26–31 (reference `StatsView.swift:206-223,478-504`), plus the ring-scaled fill thresholds (`:1222`). In the session because the Terminal does not link `dspi-core`. Exported to C and UniFFI. SW14 uses it.
- **Done when:** tests cover a device without the status (no section), a `UInt32.max` lowest fill (no value yet) and a short reply. **Deps:** `main`.

### 4.2 HB: the Rust Link hub

In scope on Qt regardless (owner), and used by Terminal and, later, the Mac. No OS or UI code in the crate, so a sideload Android build needs only a binding; no work is added for that now.

**Reference:** Swift hub files, 3 984 lines (`LinkServer`, `LinkHub`, `LinkSessionHandler`, `CommandRouter`, `AuthStore`, `PollScheduler`, `NotifyRelay`, `SnapshotCache`, `LinkPolicy`, `LinkService`, `DeviceRegistry`, `HubTransport`, `USBHubDevice`, `HubDevice`, `PairingPrompt`); the ESP32 hub; the spec; `fakehub.rs` (1 251 lines, test grade). Earlier studies said 10–15.

**The hard part is device ownership.** One `DspiCore` owns one connection today. A hub owns all local devices and serves many sessions, and the app must be one of them. Design: the hub owns the transports and the app opens an in-process transport through `open_transport`, with its own session id, so the echo rule is unchanged.

**HB0: Design and skeleton.** Size 2.
- **Owns:** `crates/dspi-hub/` (new), `docs/core-merge/HUB.md`, and the core changes it needs in `crates/dspi-core/src/{device.rs,hotplug.rs}`.
- **Implements:** threads, ownership, the in-process transport, interfaces for later HB WPs, `todo!` markers as in `PLAN.md`.
- **Done when:** the app-through-hub path passes a demo-device test. **A Fable review of the design passes before wave 2's HB1–HB3.** **Deps:** `main`.

**HB1: Server, handshake, auth, roles.** Size 2. **Owns:** `crates/dspi-hub/src/{server,auth,store}.rs`.
- **Implements:** WebSocket server, hello, `/dspi/v1/info`, session limits. **PIN is the default; open access is opt-in.** Window PIN, tokens, rate limits, revoke; close codes 4001, 4002, 4003; ping every 10 s; 30 s dead-peer limit.
- **Done when:** L1's client and fixtures pass, including wrong PIN answered with `unauthenticated`, as real hubs do. **Deps:** HB0.

**HB2: Commands and notifications.** Size 2.5. **Owns:** `crates/dspi-hub/src/{router,policy,lock,relay}.rs`.
- **Implements:** per-device ordered CMD with `max_inflight`; `policy/commands.json` by role; locks; NOTIFY with `origin` per session; resync. Documents how `0x00` is classed (upstream's policy lacks it, so admin-only).
- **Done when:** tests cover two clients seeing each other's writes, an own echo dropped, and a vanished client's lock released by timeout. **Deps:** HB0.

**HB3: Registry, snapshot, polls, rename.** Size 2. **Owns:** `crates/dspi-hub/src/{registry,snapshot,polls}.rs`. **Done when:** C15's meter feed works against it; unplug and replug tests pass. **Deps:** HB0.

**HB4:** moved to later work: `fw.install`, which no hub implements.

**HB5: Advertising and web UI.** Size 1. **Owns:** `crates/dspi-hub/src/{advertise,web}.rs`, bundled assets. **Implements:** Bonjour through CG2's library; the MIT DSPi-Link web bundle at a pinned tag, **on only while sharing and bound to the hub's port.** **Done when:** the bundle's version and hash are pinned, and a browser test reaches the demo device. **Deps:** HB1, CG2.

**HB6: C ABI for the hub.** Size 1. **Owns:** `crates/dspi-capi/src/fns_hub.rs`. **Implements:** start, stop, settings, client list, role, revoke, pairing window, status listener. **Done when:** a Qt offscreen test starts a demo hub and the Terminal connects. **Deps:** HB1–HB3.

**HB7: Conformance.** Size 1.5. **Owns:** `crates/dspi-hub/tests/conformance/`. **Implements:** our client, the TypeScript client and the Swift client (VM) against it; L1's faults; fixtures. **Done when:** all three pass, and differences from the Swift hub are recorded. **[HW]**: `macr` (XC1). **Deps:** HB1–HB6.

---

## 5. Client tracks

### 5.1 Qt: Linux and Windows

Q3 makes it run on the core. macOS leaves this track, with Swift as the working assumption: **the Qt `.dmg` stays a buildable, unpublished CI artefact until the owner's final call at OD3.** Paths are under `apps/desktop/qt/`.

**QD0: Seam and C ABI pitfalls.** Size 0.25. Do the pitfalls inside Q3 if it is still open. **Owns:** `src/ConfigFiles.cpp`, `src/DSPiBridge.cpp:1409-1433`, new `src/Registrar.h`, new `qml/BannerHost.qml`, `main.qml`.
- **Implements:** `ConfigFiles.cpp:17-21` retries after a short buffer, so an import applies twice: a reply too long is now a failure. `DSPiBridge.cpp:1409-1433` holds `state()` across writes: re-read it. The registrar and banner host let later WPs add without editing the bridge.
- **Done when:** a test shows one import applies once. **Deps:** `main`.

**QD1: `CsController` surfaces `error`.** Size 0.25. **Owns:** `src/CsController.cpp`. **Done when:** a refused `cs_apply` shows its `error`. **Deps:** `main`.

**QD2: Pair-map editor and dimmed rows.** Size 1. **Owns:** `qml/CrossfeedWindow.qml`, `src/ExtController.{h,cpp}`. **Implements:** Default, K340 and Custom (reference `CrossfeedPairMapView.swift`), hidden on a stall. Dimmed rows show the preset's own values from the table the window holds. **Done when:** a demo test edits and reads back a map; an invalid pair is refused. **Deps:** QD0.

**QD3: Core-voltage status.** Size 0.5. **Owns:** a status block in `qml/settings/pages/OverviewPage.qml`. **Implements:** status; writes are QD28. **Done when:** live mV and hang count show on the demo device; hidden on stock firmware. **Deps:** QD2.

**QD4: Input-source and device names.** Size 0.5. **Owns:** source-name rows in `qml/settings/pages/InputsPage.qml`; a Device Name row in `qml/settings/pages/GlobalParametersPage.qml`, as on the Mac. **Done when:** edits round-trip on the demo device; another client's `NOTIFY_NAMES` updates them. **Deps:** QD0.

**QD5: Output latency, clock diagnostics, C0/C1 bars.** Size 0.5. **Owns:** an Output Latency row in `qml/settings/pages/GlobalParametersPage.qml`, as on the Mac (`DSPi_ConsoleApp.swift:1912`), `src/StatsController.cpp`, `qml/StatsWindow.qml`, `qml/components/CpuSection.qml`. **Done when:** options come from the device; a stall hides the section; both CPU bars show. **After QD4**, which adds to the same page. **Deps:** QD0, QD4.

**QD6: Read-only mode.** Size 1. **Owns:** `qml/components/ReadOnlyBanner.qml`, a `writable` bridge property, compat branch of `main.qml`. **Implements:** older and newer firmware show all settings, disable every control, keep reading, offer the update (`WIRE-FORMAT-COMPAT.md` §4.4). **Done when:** V17, V26, V31, V33 fixtures run offscreen; no setter is called (test); clip clear and bootloader still work. **Deps:** QD0.

**QD7: Demo mode.** Size 0.25. **Owns:** the demo entry through the registrar. **Done when:** selectable with no hardware; clearly labelled. **Deps:** QD0.

**QD8: Link controller and token store.** Size 1. **Owns:** `src/LinkController.{h,cpp}`, `src/TokenStore.{h,cpp}`. **Implements:** wraps CG1, off the UI thread, with `expected_hub_id`. Tokens in Credential Manager or Secret Service, falling back to a 0600 file with a warning; deleted only on `token_refused`. **Done when:** a fake-hub test covers pairing, reuse after restart, `wrong_hub`, and `rate_limited` keeping the token. **Deps:** CG1.

**QD9: Device list, discovery, PIN prompt.** Size 1.5. **Owns:** `qml/DevicesPopup.qml`, `qml/PairingDialog.qml`. **Implements:** one list of USB, demo and hubs; non-blocking PIN prompt; every `LinkPhase`, CG10's text. **Done when:** runs offscreen against the fake hub. **Deps:** QD8, CG2, CG10.

**QD10: Lost-connection banner.** Size 0.5. **Owns:** `qml/components/LinkBanner.qml`. **Done when:** in a fake-hub fault test: banner at once, mirror readable, a write fails and is not sent later, abort works during a dial, analyser restarts. **Deps:** QD8, CG10.

**QD11: Firmware updater.** Size 1. **Owns:** `src/FirmwareUpdater.{h,cpp}`, `qml/FirmwareUpdateWindow.qml`, `firmware/`. **Implements:** images per OD4; custom `.uf2` with `uf2_inspect`; CG8's guard; `firmware_version_matches` replacing `FwVersion` compare; export configuration first; hidden on STM32H723. **Done when:** offscreen tests cover wrong-chip file, guard and fake bootloader drive. **No real flash here. Deps:** CG8.

**QD12: Tray.** Size 0.75. **Owns:** `src/TrayController.{h,cpp}`. **Implements:** Linux and Windows `QSystemTrayIcon`; close to tray; start minimised; the Mac's menu items (`MenuBarMode.swift:104-126`), including Share on This Network and Allow a New Client (`:117-120`); hub keeps serving. Those two stay hidden until QD14 registers its controller. **Done when:** an offscreen test covers the state machine, including no-tray fallback; checked once in `winvm` and on the host desktop. **Deps:** QD0.

**QD13: Open at login and keep awake.** Size 0.5. **Owns:** `src/SystemIntegration.{h,cpp}`. **Implements:** XDG autostart and Windows Run key; sleep inhibit while sharing. **Done when:** unit tests check the written entry and the inhibit call; each is verified on its OS. **Deps:** QD12.

**QD14: Networking page.** Size 1.5. **Owns:** `qml/settings/pages/NetworkingPage.qml`, `src/HubController.{h,cpp}`. **Implements:** `NetworkingSettingsView.swift`: share toggle, hub name, access (PIN by default), port, paired clients, roles, revoke, pairing code with expiry, other hubs, background options. Connects QD12's two hub menu items. Says a PIN crosses the network in plain text without TLS. **Done when:** offscreen on a demo hub, and a real client pairs. **Deps:** HB6, QD8.

**QD30: Popover cursor on the graph gear.** Size 0.25. **Owns:** `qml/components/PopoverWindow.qml`. **Implements:** owner's report on KDE Wayland: with a `PopoverWindow` open (graph gear; check the spectrum strip's too), the pointer over the gear doesn't return to the hand until wiggled, because the popup holds a grab. **Done when:** the cursor stays right without a wiggle and a click on the gear closes the popover once, on Wayland and X11; the owner retests. **Deps:** `main`.

**QD29: Output ring health.** Size 0.25. **Owns:** the section in `qml/StatsWindow.qml`, `src/StatsController.cpp`, after QD5, which owns both in wave 1. **Done when:** the section and ring-scaled thresholds show on the demo device; hidden when the status is absent. **Deps:** CG12, QD5.

**QD15:** moved to later work: coach marks (owner: "not for now at least").

**QD16: App self-update.** Size 1. **Owns:** `src/AppUpdater.{h,cpp}`. **Implements:** check our releases, compare, show notes, open the page. **The check is opt-in, with a privacy line saying what is sent.** In-place install only after OD8. **Done when:** tested on a local feed; off in distro packages. **Deps:** QD0.

**QD17: Shortcuts.** Size 0.5. **Owns:** shortcuts in `qml/main.qml`, `qml/SignalGeneratorWindow.qml`. **Reference:** the list in `DSPi_ConsoleApp.swift` around lines 11700–12100. **Done when:** every Mac shortcut has a Ctrl equivalent, in a test table. **Deps:** QD0.

**QD18: Host audio format.** Size 1. **Owns:** `src/HostAudioFormat*.{h,cpp}`. **Implements:** idle channel count by WASAPI and by PipeWire or ALSA, falling back to the device's report (reference `HostAudioFormatMonitor.swift`). **Done when:** a fake backend tests the fallback; each OS is checked once. **Deps:** QD0.

**QD19: Analyser over Link.** Size 0.25. **Owns:** `qml/SpectrumAnalyserWindow.qml`. **Done when:** following state and Take control work on the fake hub. **Deps:** CG5, QD8.

**QD20:** folded into `docs/PARITY.md` rows (XC0, HO1).

**QD21: Packaging.** Size 1.5. **Owns:** `apps/desktop/packaging/**`. **Implements:** Linux on Qt 6 (raises the glibc floor), AUR source URL, QML precompile, installer check that the app is not running, keep Qt dynamic or add the LGPL relinking provision. The `.dmg` stays unpublished. **Done when:** artefacts build. **Deps:** Q3.

**QD22: Accessibility.** Size 2. **Alone in the last wave**: touches most of 131 QML files. **Owns:** all of `qml/`. **Done when:** `Accessible.*` on custom controls in `components/` and the windows; keyboard-only and screen-reader pass on Linux. **Deps:** other QD UI WPs.

**QD23: Localisation plumbing.** Size 1.75. **After QD22. Owns:** all QML, `translations/`. **Implements:** `qsTr` and the `.ts` build, **English only for now** (owner). **Done when:** a pseudo-locale run shows no untranslated string or clipped layout in main windows. **Deps:** QD22.

**QD24: Hardware pass, Linux.** Size 0.5. **QD25: Windows.** Size 1. Both **[HW][COORD]**. Stock USB pass-through per `~/vm/windows/USB-PASSTHROUGH.md`, with the bootloader device attached too. **Owns:** `docs/core-merge/HW-QT.md`. **Done when:** connect, read, write, notifications, replug, firmware path and Link pass on the K340 under section 6's rules. **Deps:** QD0–QD14.

**QD27: AutoEQ on the core.** Size 0.5. **Owns:** `src/AutoEqLibrary.{h,cpp}` (453 lines), `qml/AutoEqRebuildWindow.qml`. **Implements:** re-point at CG6, with Qt supplying the fetch; remove the duplicated logic. **Done when:** existing behaviour passes against a scripted fetch. **Deps:** CG6.

**QD28: Advanced core voltage.** Size 1. **Owns:** `qml/settings/pages/CoreVoltagePage.qml`, reachable from `AdvancedPage.qml`, a `VregController`. **Implements:** thin UI over CG11: status, manual ops, the calibration flow, CG11's warnings. **Done when:** offscreen on the demo device with simulated hangs, every state shows; no path reaches a write without the confirmation; not reachable outside Advanced. **Deps:** CG11, QD3.

### 5.2 Terminal (`apps/terminal`, `crates/dspi-tui`, `crates/dspi-cmd`)

Owner decided: it skips what needs a graphical UI (on-graph editing, onboarding) and covers the non-graphical equivalents, including JSON import and export.

**TM1: Pair-map row.** Size 0.5. **Owns:** `screens/crossfeed.rs`. **Done when:** Default, K340, Custom work on the demo device; hidden on stock; not confused with the `Pairs` mask row. **Deps:** `main`.

**TM2: Core-voltage screen.** Size 0.5. **Owns:** `settings/overview.rs`. **Implements:** TUI status; writes stay in the CLI, as they are. **Done when:** demo render test. **Deps:** `main`.

**TM3: Firmware flashing.** Size 1. **Owns:** `apps/terminal/src/flash.rs`, `dspi-core` as a dependency for `firmware.rs` only. **Implements:** `dspi flash [FILE]` with `uf2_inspect` and CG8's guard. **`--force` does not override the guard**; only a separate `--accept-core-voltage-risk` flag plus the typed confirmation does. Suggests `dspi export` first. **Done when:** a fake bootloader drive test passes; `--force` alone is refused on an at-risk board. **Deps:** CG8.

**TM4: Read-only fallback.** Size 1. **Owns:** connect path in `crates/dspi-tui/src/shell/connect.rs`, `apps/terminal/src/main.rs`. **Implements:** `probe_any`, display in all screens, a firmware banner, writes refused in one place; `--json` reports `read_only`. **Done when:** V17, V31, V33 fixtures render; no write leaves (mock transport). **Deps:** `main`.

**TM5: Analyser overlay and following.** Size 1. **Owns:** `graph.rs`, `screens/spectrum.rs`. **Done when:** curves on the graph; following state and a take-control key work on the fake hub. **Deps:** CG5.

**TM6: Bar strip.** Size 0.5. **Owns:** `widgets/bar_strip.rs`. **Done when:** third-octave strip renders in ASCII and braille. **Deps:** TM5.

**TM7: Tube Basic/Advanced.** Size 0.25. **Owns:** `screens/tube.rs`. **Done when:** a toggle with Swift's row set. **Deps:** `main`.

**TM8: Lost-link status line.** Size 0.25. **Owns:** status line in `shell/mod.rs`. **Done when:** in a fake-hub test: status line, abort key, failed write not replayed. **Deps:** CG10. Wave 2, since CG10 lands in wave 1.

**TM9: Find hubs.** Size 0.5. **Owns:** `dspi link scan`, picker in `shell/connect.rs`. **Done when:** a fake-advertiser test; `--json` output. **Deps:** CG2.

**TM10: JSON import and export.** Size 1.
- **Owns:** `crates/dspi-session/src/preset_file.rs`, `cmd_export` and `cmd_import` in `apps/terminal/src/main.rs`, a `cs` command.
- **What exists:** `.dspipreset`, a filter text format, `dump --json`, `--dry-run`.
- **Gaps (R1's check):** format is chosen by extension only, so `dspi export x.json` writes the filter text; no `-` for stdin or stdout; `--json` has no effect on import or export; filter banks have no JSON form; the preset document has no control-surface block and appears to lack the pair map (compare `dspi_core::files` and `config_import`); `import_preset`'s doc is stale; Save Master Volume and Save Output Configuration to be checked.
- **Done when:** round-trip with Qt's and the Mac's documents in the shared fixtures; `dspi cs export|import` work on the demo device. **Deps:** `main`.

**TM11: `dspi hub`.** Size 1. **Owns:** `apps/terminal/src/hub.rs`, `apps/terminal/packaging/service/`. **Implements:** a headless hub with pairing, clients and roles as CLI commands, and sleep inhibit. Owner: "include some placeholder systemd service / windows background / mac eq". So three documented placeholders with install notes: a systemd user unit, a Windows scheduled task (with a note on a service wrapper), and a launchd plist. **PIN pairing stays the default in all three.** **Done when:** a Link client connects; `systemd-analyze verify` passes in a container, `plutil -lint` in `macvm`, and the task imports in `winvm`. **Deps:** HB6.

**TM12: AutoEQ update.** Size 0.25. **Owns:** `screens/autoeq.rs`, `crates/dspi-cmd` command. **Implements:** CG6, removing the `curl` call. **Done when:** update, rebuild and reset work on a scripted fetch, with no external process. **Deps:** CG6.

**TM13: K340 hardware pass.** Size 0.5. **[HW][COORD]**. **Owns:** `docs/core-merge/HW-TERMINAL.md`. **Done when:** USB replug and real MIDI ports end to end pass (R1's open item). **Deps:** TM1, TM2, TM4.

**TM15: Closed pipes.** Size 0.25. **Owns:** output handling in `apps/terminal/src/main.rs`, under its marker. **Implements:** `dspi --demo dump | head -5` panics with "failed printing to stdout: Broken pipe". Handle a closed pipe across all CLI output: stop quietly, with the usual exit status. **Done when:** a test pipes every command that prints into a closed reader, with no panic. **Deps:** `main`.

**TM14: No tmpfs in tests.** Size 0.25. **Owns:** tests in `crates/dspi-tui/src/{actions.rs,settings/config.rs,settings/mod.rs}`. **Implements:** five `temp_dir()` uses move to a target-dir path. **Done when:** none remain, and tests pass. **Deps:** `main`.

**TM16: Output ring health.** Size 0.25. **Owns:** `screens/stats.rs`. **Done when:** the section and ring-scaled thresholds render on the demo device; absent status hides it. **Deps:** CG12.

**TM17: Device names and latency in the TUI.** Size 0.5. **Owns:** `settings/global.rs`, `settings/inputs.rs`. **Implements:** Device Name and Output Latency on Global Parameters and source names on Inputs, over the existing `dev.name`, `out.latency` and `in.name`. **Done when:** edits round-trip on the demo device; hidden when absent. **Deps:** `main`.

**TM18: Volume mode and Identify.** Size 0.25. **Owns:** the `VolumeModeToggle` arm in `crates/dspi-tui/src/live.rs`, `settings/config.rs`, `identify_command` in `screens/mod.rs`. **Implements:** the volume mode is saved; Identify as `Commands.swift:3688-3704`: -12 dB, two passes, clamped to the valid-channel mask. **Done when:** a test restores the mode after a restart; a test checks Identify's commands. Wave 2, since TM14 owns tests in `settings/config.rs` in wave 1. **Deps:** `main`.

### 5.3 Android (`apps/android`)

The swap is contained: UI talks to the `core-api` interface, and only `ffi` sees UniFFI. Owner: the next Play update waits for this.

**Fast lane, independent of Qt:** AN1 → {AN2, AN3, AN6} → {AN4, AN5} → AN13 → AN14.
- **First update** = AN1, AN2, AN3, AN13. **AN4–AN6 go in the second update** (AN22), which the 14-day closed test needs anyway.
- XC2 must pass before Link ships against a Mac hub. **It depends on XC1, which needs the owner's go-ahead for the `macr` restart.**
- **So the lane cannot stall silently:** AN13 runs its USB half first, with no hub. Its Link half runs against XC2's Mac hub or the ESP32 bridge (XC4). If neither is available, the coordinator reports that, and the Link half and any Link change are deferred to AN22.

**AN1: New crates in place of `dspi-ffi` and the patched core.** Size 2.5.
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

**AN2: Read-only on older firmware.** Size 0.5. **Owns:** `feature-home/.../HomeModel.kt`, `README.md`. **Implements:** V17–V31 now decode; `WriteGuardedCore` stays; fixes the README claim. The banner still fires on the wire format only: **the first Play update must show no version banner on a K340 or on stock final firmware** (the comparison is AN18's). **Done when:** fixture tests; banner per `WIRE-FORMAT-COMPAT.md` §4.4. **Deps:** AN1.

**AN3: Link engine on the core's supervisor.** Size 1. **Owns:** `link/.../{LinkSessionEngine,LinkCoreOps}.kt`.
- **Implements:** delete Android's own reconnect, backoff and token deletion, which would fight C15; follow `link_status`; implement `on_link_status_changed` and `on_link_token_issued`; delete a token only on `token_refused`; pass `expected_hub_id`; handle `wrong_hub`; remove `setListener` from inside callbacks (`LinkSessionEngine.kt:164,178,214`); `rta_start` again after a reconnect; dial off the serial FFI queue.
- **Done when:** `link` tests are rewritten against the fake hub with L1's faults; `rate_limited` keeps the token. **Deps:** AN1.

**AN4: Network handling.** Size 1. **Owns:** `link/.../{LinkNetworkMonitor,NsdHubBrowser,LinkSettings}.kt`. **Implements:** `onLost` and network change call `link_retry_now`; browse while reconnecting and re-resolve via `link_set_url` (F5); test F10's items. `WifiLock` needs no new permission; **re-check Data safety if sockets get bound to Wi-Fi.** **Done when:** unit tests with a fake monitor; F10 items recorded as used or rejected. **Deps:** AN3.

**AN5: Link UI.** Size 1. **Owns:** `ui-common/.../LinkBanner.kt`, `app/.../{LinkDialogs,AppNetworkingController,ConnectionStrip}.kt`. **Implements:** owner's banner on every screen; wrong-PIN by `pin_source`; "no DSPi found" (F9), CG10's text. **Done when:** Compose tests per `LinkPhase`. **Deps:** AN3, CG10.

**AN6: Settings rows.** Size 1. **Owns:** `feature-settings/.../{GeneralSettingsTab,PinOverviewTab}.kt`, source picker in `feature-home`. **Implements:** core voltage status, device name, input-source names, output latency. **Done when:** demo tests round-trip; hidden when absent. **Deps:** AN1.

**AN7: One device list.** Size 1. **Owns:** `app/.../{AppMode,EntryNavigator,UsbStartGate}.kt`. **Implements:** a user's device switch calls `withUnsavedChanges` first (M42). **Done when:** USB, Link and demo in one list; demo reachable with a DSPi attached; a test shows a switch with a dirty preset asks first. **Deps:** AN3.

**AN8: Control-surface spike.** Size 0.5. **Owns:** a report. **Implements:** a phone layout against Swift's 5 270-line tab and Qt's `settings/cs` and pages. Sets AN9–AN12's sizes. **Done when:** a reviewed screen map exists. **Deps:** AN1.

**AN9: CS bindings.** Size 2–4. **AN10: IR learn.** Size 1. **AN11: Display pages.** Size 1–2. **AN12: Groups, macros, aux.** Size 1.5–2.5. **Owns:** one file set each under `feature-settings/.../cs/`. **Implements:** `cs_snapshot`, `cs_apply`, `cs_pin_uses`. **Done when:** runs on the demo device (`FakeDevice`'s CS support looks minimal: check, and extend); IR and display need **[HW]**; an `error` reply is shown. **Deps:** AN8.

**AN13: Phone test.** Size 1. **[HW][OWNER][COORD]**. **Owns:** `docs/HIL-NEW-CORE.md`. **Requires** the owner's adb port. **Done when:** the earlier HIL set passes on the nusb `from_fd` path (unproven on hardware): USB, notifications, old firmware, Link with a Mac hub, 45 s background, no secrets in logcat. Side by side with `applicationIdSuffix ".hil"`: never uninstall the Play build. **Deps:** AN1, AN2, AN3; USB half needs nothing more; Link half needs XC2 or XC4, or is deferred to AN22 (see the fast lane).

**AN14: First new-core update.** Size 0.5. **[OWNER]**. **Owns:** version, notes, store text. **Implements:** the unreleased crossfeed pair fix ships here; README; versionCode bump; source tarball on the release (OD1). **Done when:** an AAB is ready for the owner to upload. **Deps:** AN13, R-And.

**AN15: R8.** Size 1. **Owns:** `app/build.gradle.kts`, keep rules. **Done when:** a release build passes a phone test **[HW]**. Play deadline February 2027; own release. **Deps:** AN14.

**AN16: Edge-to-edge.** Size 1. **Owns:** `app/.../MainActivity.kt`, insets. **Done when:** every route checked on the phone **[HW]**. Own release. **Deps:** AN14.

**AN17: Analyser bar strip and following.** Size 0.75. **Owns:** a bar-strip file and overlay state in `feature-analysis/`. **Done when:** Compose tests on the demo device; following and Take control work on the fake hub. **Deps:** AN1, CG5.

**AN18: Small floor items.** Size 1.25. **Owns:** `feature-firmware/`, the version comparison in `feature-home/.../{HomeModel,FirmwareMismatchBanner}.kt`, AutoEQ update UI, a host-format reader, `feature-tools/` generator lifetime, graph keyboard map. **Implements:** images per OD4; **CG8's guard called on Android's Kotlin flashing path, PICOBOOT included**; export first; CG6's update with a Kotlin fetch; `AudioManager` channel count; generator keeps running off-screen; hardware shortcuts; the mismatch banner compares firmware versions like the Mac (`DSPViewModel.swift:2099-2106`), arriving with the updated images (M41). **Firmware built with our extension channel counts as acceptable:** the K340's `bofr-main` build reports a matching version plus a build stamp, so it must not get a false "update" prompt. **Done when:** a unit test proves the flash path cannot skip the guard; a test shows the banner for another version on the same wire format, and none for a matching version with a build stamp; each item has a test. **Deps:** AN1, CG6, CG8.

**AN19: Getting Started and What's New.** Size 1. **Owns:** `feature-onboarding/`. **Done when:** first run shows once; replay from Help; data from CG10. **Deps:** CG10.

**AN20: Accessibility.** Size 2. **Owns:** semantics across `feature-*`. **Done when:** a TalkBack pass on graph, matrix and meters **[HW]**. **AN21: Localisation plumbing.** Size 1.75. **Owns:** `strings.xml` across modules, **English only for now** (owner). **Done when:** a pseudo-locale run shows no hard-coded string. Both alone in the last wave. **Deps:** other AN UI WPs.

**AN22: Second update.** Size 0.25. **[OWNER]**. **Owns:** version, notes. **Implements:** AN4–AN6, and AN13's Link half if it was deferred. **Done when:** a phone re-test passes **[HW]**, and an AAB is ready. **Deps:** AN4, AN5, AN6, AN14.

**AN23: Advanced core voltage.** Size 1. **Owns:** `feature-settings/.../CoreVoltageScreen.kt`, reached from `AdvancedSettingsTab.kt`. **Implements:** thin UI over CG11. The soak survives the screen going off or the process dying, by re-reading `trial_active`. **Done when:** Compose tests on the demo device with simulated hangs; not reachable outside Advanced. **Deps:** CG11, AN6.

**AN25: Output ring health.** Size 0.25. **Owns:** the section in `feature-analysis/.../StatsView.kt`, the stats controller in `bridge/`. **Done when:** a Compose test shows the section and ring-scaled thresholds on the demo device; hidden when absent. **Deps:** CG12, AN1.

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

**SW1: XCFramework.** Size 1. **Owns:** `apps/apple/build-xcframework.sh`. **Implements:** in the VM, add `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios` (the VM is Intel) and the macOS slices. `usb` off for iOS, on for macOS. Library size recorded. **Done when:** it links in the Simulator and on macOS. **Deps:** CG3, MR1.

**SW2: Swift package and compile test.** Size 1. **Owns:** `DSPi Console/Core/Generated/`, Xcode project lines. **Implements:** bindings into the app; `init_logging`; one run per namespace. Adds the macOS and iOS rows to MR2's matrix. **Done when:** a test calls `core_info()` and opens the demo device on both targets, and both rows run in the matrix. **Deps:** SW1, CG9, MR2.

**SW3: Seam map.** Size 0.5. **Owns:** `docs/SWIFT-SEAMS.md`. **Implements:** each wire-using function mapped to a core method; those with none become core WPs; thread and mirror decisions; re-sizes SW8–SW14. **Done when:** Fable has reviewed it. **Deps:** SW2.

**SW4: `CoreBackedTransport`.** Size 1.5. **Owns:** `DSPi Console/Core/CoreBackedTransport.swift`. **Implements:** `DeviceTransport` over CG4 and the core's notifications; the core's `FakeDevice` under demo mode. **Done when:** existing `FakeTransportTests` and wire tests pass behind a launch flag on both targets. **Deps:** SW3, CG4.

**SW5: CoreMIDI through the core.** Size 1. **Owns:** `MIDI/CoreMidiPipe.swift`, reducing `MIDI/MIDITransport.swift` to I/O. **Implements:** `MidiSender`, `MidiInputHandle` with `close_port`, the relay port. **Done when:** `midi_vectors.json` passes; relay run **[HW]** on the K340. **Deps:** SW4.

**SW6: Link through the core.** Size 1.5. **Owns:** `Link/LinkClient*.swift` replaced by `Core/CoreLink.swift`; keeps `HubBrowser.swift`, `LinkTokenStore.swift`, the pairing sheet. **Implements:** Bonjour stays in Swift (the entitlement belongs to it); `expected_hub_id`; `link_retry_now` on foreground; `rta_start` after a reconnect. Checks the Local Network prompt with Rust's BSD sockets. **Done when:** the run in `A7-LINK-CLIENT-REPORT.md` passes **[HW]**. **Deps:** SW4, XC1.

**SW7: Banner and errors.** Size 0.5. **Owns:** `Core/LinkBannerView.swift`. **Done when:** the owner's rule holds in a fake-hub run; CG10's wording. **Deps:** SW6, CG10.

**SW8: Reads and the mirror.** Size 3–5. **SW-V1 and OD3 follow it.** **Owns:** `Commands.swift` "Bulk Parameter Transfer" and "Notification → State Mirroring" (about 840 lines). **Implements:** view-model state from `DspState`, notifications from the core, older firmware read-only. **Done when:** every view shows correct values on the demo device and fixtures; no echo flicker. **Deps:** SW4.

Then one WP per domain. Each **owns** its `Commands.swift` sections and wire use in its views; **done when** wire tests are ported, the domain has no `transport.` calls, and both targets work on the demo device. SW3 re-sizes them.

| WP | Domain | Size |
|---|---|---|
| SW9 | Volume, EQ, crossover, outputs, matrix, slider fast path | 1.5–2.5 |
| SW10 | Effects (loudness to upmixer) | 1.5–2.5 |
| SW11 | Pins, I2S, ADAT, inputs, UART/I2C, names | 1.5–2 |
| SW12 | Control surfaces | 1.5–3 |
| SW13 | Presets, `PresetDocument`, `PresetSnapshot` | 1.5–3 |
| SW14 | Analyser (following state), Stats, signal generator, monitor | 1.5–3 |

**SW15: Simulator and relay test.** Size 1. **[HW][COORD]**. **SW16: Real iPhone or iPad.** Size 0.5. **[HW][OWNER]**: no device has ever run this app. **Owns:** `docs/SWIFT-HW.md`. **Done when:** USB-C MIDI, Link, background and return pass. **Deps:** SW5, SW6, SW14.

**iOS view-layer gaps** (parity 3.1, section 3). Views only; alongside the domains:

**SW17: File import and export.** Size 1. **Owns:** `iOS/DocumentPicker.swift`, the `notYet` block in `DSPi_ConsoleApp.swift`. **Done when:** filter files and `.dspipreset` round-trip against shared fixtures. **Deps:** SW2.

**SW18: AutoEQ and firmware window.** Size 0.75. **Owns:** iOS AutoEQ menu, firmware wizard step. **Implements:** Favourites, CG6's update; hide the dead firmware window and point to a computer, including the mismatch banner's Update button (`ContentView.swift:1075-1077`) and the Tools entry (`iOS/IOSApp.swift:218-220`); no `.uf2` images in the iOS target. **Done when:** UI tests cover the menu, show no firmware entry on iOS, and show the banner cannot open the window; the iOS bundle has no `.uf2`; the Mac is unchanged. **Deps:** CG6, SW2.

**SW19: Touch graph.** Size 1.5. **Owns:** `iOS/PeqTouchEditor.swift`, a touch HUD. **Implements:** marquee, pull from curve, shape chooser, value chip. **Done when:** UI tests on iPhone and iPad cover each gesture. **Deps:** SW2.

**SW20: Hardware keyboard and host format.** Size 0.75. **Owns:** `iOS/IOSApp.swift` commands, `iOS/AudioRouteMonitor.swift`. **Done when:** a UI test covers the iPad shortcut table; the route count falls back to the device report. **Deps:** SW2.

**SW21:** moved to later work: coach marks.

**SW23: Advanced core voltage.** Size 1. **Owns:** `Core/CoreVoltageView.swift`, reached from the Advanced settings page. **Implements:** thin UI over CG11, both targets; extends today's status view (`CoreVoltageStatusView.swift`). iOS needs no background mode: on return it re-reads `trial_active`. **Done when:** UI tests on both targets with simulated hangs; not reachable outside Advanced. **Deps:** CG11, SW2.

**SW22: Accessibility and localisation plumbing.** Size 2.75, English only for now. **Owns:** all views. Alone, last. **Done when:** a VoiceOver pass in the Simulator; a pseudo-locale run. **Deps:** other SW WPs.

**Mac-only additions:** about 3.5–5.5.

**SW-M1: USB through the core.** Size 1–1.5. **Owns:** `Core/CoreUsb.swift`; reduces `USBDevice.swift` and `USBNotificationReader.swift`. **Implements:** `nusb`'s `macos_iokit` through CG3's `usb` feature, including hot-plug. **Done when:** a demo run and a mock test pass; the macOS target has no IOKit vendor path left outside the hub. **Deps:** SW4.

**SW-M2: Firmware through the core.** Size 0.5–1. **Owns:** `FirmwareInstaller.swift`, `BootloaderLocator.swift`, reduced to UI over C11's `firmware.rs` and CG8's guard. **Done when:** `FirmwareInstallerTests` pass; a fake drive test; guard cannot be skipped. **No real flash here. Deps:** SW-M1, CG8.

**SW-M3: Hub over `dspi-hub`.** Size 1–2. **Owns:** a UniFFI wrapper for the hub, `NetworkingSettingsView.swift` re-pointed; Swift hub files removed at the end. **Owner decided: "once it comes into the other versions it should come in here as well".** It does not block R-Mac. The Swift hub stays as the reference harness until HB7 passes, then retires. **Done when:** HB7's Swift client passes against the wrapper from the Mac app. **Deps:** HB7, SW-M1.

**SW-M4: What stays Swift.** Size 0.5. **Owns:** `docs/SWIFT-STAYS.md`. **Implements:** an audit that each item is UI or OS integration only, with no protocol: menu-bar mode, login item, keep-awake, `AppUpdateChecker` and installer, `HostAudioFormatMonitor`, Metal renderers. **Done when:** Fable has reviewed it, and a grep for opcodes outside `Core/` is empty. **Deps:** SW14.

**SW-M5: Real Mac hardware pass.** Size 0.5. **[HW][OWNER]**: an Apple silicon run has never happened, and the VM and known spare Macs are Intel. **Owns:** `docs/SWIFT-HW.md`. **Done when:** the checklist passes under section 6's rules. Developer ID only if we distribute a binary (OD-DevID). **Deps:** SW-M1, SW-M2.

**SW-V1: Visual and feel evidence for OD3.** Size 0.75. **Owns:** `docs/SWIFT-VS-QT-MAC.md`, screenshots under it. Owner: "come back with visual checks and feel to make a final decision, i assume swift will be needed for now".
- **Implements:** side-by-side screenshots of the main window, graph editing, settings, dialogs and menus, Qt `.dmg` against the Swift app on the Mac VM, on the demo device. Notes on frame rate and responsiveness, with the VM's limits stated. Also a current iOS screenshot set. A short **section for the owner to try on real hardware [OWNER]**: a checklist of feel items (drag, scroll, native menus, text, sliders).
- **Done when:** the report goes to the owner at OD3's point, with no recommendation hiding the evidence. **Deps:** SW8, MR2's `.dmg` row.

**SW-U1: Upstream tracking rule.** Size 0.25. **Owns:** `docs/UPSTREAM-TRACKING.md` (section 0.1). **Done when:** a dry-run merge of a later upstream commit is recorded. **Deps:** MR1.

Store items (review video, TestFlight, the SysEx ID question) are later: `APPSTORE-MIDI-RISK.md` applies then.

### 5.5 Web

**WB0: Study.** Size 2. **Owns:** `docs/core-merge/WEB-STUDY.md`, throwaway spikes under `~/.cache/dspi-build/work/wp-wb0/`. Must judge, with a working spike where marked:

| Option | Questions |
|---|---|
| Rust core in wasm + a web UI **(spike)** | The session's 17 transport-using files block on `&mut dyn Transport`: about 170 call sites and 45 uses of threads or `Instant`. Compare (a) an async twin of the trait; (b) a request and reply split (`PLAN.md` §1.3); (c) blocking in a Worker with `Atomics.wait`, needing COOP and COEP; (d) JavaScript Promise Integration. Which leaves `dspi-session` unchanged for other clients? `nusb` has a wasm32 WebUSB backend. |
| Qt for WebAssembly **(spike)** | Do the Qt app and the C ABI core build? Download size? Same async problem. |
| Reuse what exists | sigman78's console (MIT, Svelte, WebUSB, demo mode, 1.1.5+) and the DSPi-Link web UI (MIT, which HB5 serves). What do they need for extensions and Link? Licence fit (OD-Lic)? |
| Link in the browser | Browsers block `ws://` from `https://`, and no hub has TLS. Is a hub-served page the only route? |
| WebUSB | Chromium only. Check on stock and bofr-main, per OS. **Why does sigman78's README still mention Zadig on Windows, given the firmware's MS OS 2.0 descriptor?** |
| Web MIDI | Chromium, Firefox with permission. Firmware's MIDI build only. |

**Done when:** a recommendation with sizes fills the stubs. **Deps:** `main`.

**Stubs, unsized until WB0:** WB1 transport and session route; WB2 UI; WB3 hosting and origin; WB4 hardware test. Rough range by route: reuse 3–8, Qt for WebAssembly 8–15, Rust and a new UI 40–70.

### 5.6 Audit and cross-client tests

**XC0: Finish the floor audit.** Size 0.5. **Wave 0.** **Owns:** `docs/PARITY.md`, moved into the repo from `dspi-mobile/docs/FEATURE-PARITY.md`, and the references to the old path in `docs/core-merge/PLAN.md` and `docs/PORTABILITY.md` §11. **Implements:** walk the Mac's menus and every settings page; turn every "hit" in section 3 into read code, per client; add anything missing with a WP or a reason. **Done when:** no "hit" remains, and Fable has spot-checked 10 rows. **Deps:** `main`.

**XC1: `macr` restart.** Size 0.5. **[COORD]**, **with the owner's go-ahead.** **Implements:** patched QEMU and libusb per `qemu-usb/REPORT.md`; check `br0` and Bonjour from the phone; re-attach the DSPi over QMP; host unlocked. **Done when:** the Mac hub is reachable from the LAN and the 6 KB read works. **Prerequisite for every Mac test. Deps:** none.

**XC2: Mac hub ping test.** Size 0.25. **[HW][COORD]**. **Owns:** a report. **Done when:** the Mac hub answers pings, so L1's 5 s probe does not kill healthy sessions; idle past 35 s survives; rate limit keeps the token. **Before AN13's Link half on a Mac hub. Deps:** XC1.

**XC3: Real-hub test harness.** Size 0.5. **Owns:** `crates/dspi-link/tests/real_hub/`, opt-in by environment. **Implements:** the scripted parts of `LINK-HUB-FINDINGS.md` section 5. **Done when:** it runs against the fake hub with the same assertions and skips cleanly with no hub set. **Deps:** `main`.

**XC4: ESP32 bridge suite.** Size 1. **[HW][OWNER][COORD]**: **is an ESP32 on hand and wired to a DSPi?** **Owns:** a report. **Implements:** all 9 items: idle, wrong PIN ×5, Wi-Fi pulls at 5, 20, 32, 60 s, all slots full, DHCP change, board off, GETs over 132 B, cut after `auth.pair`, Doze. **Done when:** each is pass or fail, with fixes filed. **Deps:** XC3, AN5 for phone cases.

**XC5: Meters over a bridge.** Size 0.5. **[HW][OWNER][COORD]**. **Owns:** a report. **Done when:** CPU, frame time and audio are measured with meters on, using `poll.subscribe`, against kingsnake's report. **Deps:** XC4.

**XC6: Core voltage on the K340.** Size 0.5. **[HW][COORD]**, **K340 only, backups first.** **Owns:** a report. **Implements:** back up settings and flash; through Qt, then Android and Swift if available: status; a short trial at the build default (1.20 V) with abort; a pass result with `TRIAL_END`; a `SET` to the same value, verified, and re-read after power cycle. **Not in this WP without the owner's approval at that step:** a real hang test (a trial at 1.15 V) or anything above 1.20 V. The hang path is proven on `FakeDevice`. **Done when:** the report records results, and `live_vreg` and settings match the backup at the end. **Deps:** CG11, QD28.

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
| Play upload, forms, closed-test updates | Owner | AN14–AN16, AN22 |
| ESP32 bridge wired to a DSPi | Owner: is one available? | XC4, XC5 |
| Real iPhone or iPad, signing team | Owner | SW16 |
| Apple silicon Mac; Developer ID if distributing | Owner | SW-M5 |
| `macr` restart, DSPi moved between VMs | Coordinator; owner agrees to the restart | XC1, QD25, SW5, SW6, SW15, HB7 |
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
| 2 | **HB1–HB3 after HB0's review**; MR1 (Swift half), then SW1, then SW2; SW-U1 after MR1; QD3, QD6, QD7, QD8, QD11, QD12, QD18, QD27, QD29; TM3, TM5, TM8, TM9, TM12, TM16, TM18; AN2, AN3, AN6, AN8, AN25 | wave 1 |
| 3 | HB5, HB6; SW3 then SW4; QD9, QD10, QD13, QD16, QD17, QD19, QD28; TM6; AN4, AN5, AN7, AN13 then AN14, AN17, AN18, AN19, AN23 | wave 2; **M0 at its end** |
| 4 | HB7; SW5, SW6 then SW7, SW-M1 then SW-M2, SW17–SW20, SW23; QD14, QD21; TM11, TM13; AN9–AN12, AN22 | wave 3 |
| 5 | SW8 then SW-V1 **(then OD3)**; QD24, QD25; AN15, AN16; XC4, XC5, XC6 | wave 4 |
| 6 | SW9–SW14; SW-M3 after HB7 | the owner's call at OD3 |
| 7 | QD22 then QD23; AN20, AN21; SW15, SW16, SW22, SW-M4, SW-M5; HO1 | everything else in that client |

**Review gates:**
1. **This plan** gets a Fable review before wave 0.
2. **After each wave:** the full `ci/check.sh --all`, MR2's matrix once it exists, a code-quality review, and an adversarial review, especially the owner's Link rule and "no write in read-only".
3. **HB0** and **SW3** get a design review before work builds on them.
4. **M0:** the full matrix passes, every row run, and the owner reviews it. It does not block CG, HB or QD work in flight.
5. **Before each release:**

| Gate | Must pass | Then |
|---|---|---|
| **R-Qt** | Section 3's Qt column, QD24, QD25; no setter on read-only; Link rule on a real hub; OD4 images and CG8 | Owner decides to publish |
| **R-Term** | TM13; TM10's round-trip with Qt's and Mac's documents | Owner decides |
| **R-And** | AN13 on the phone; `tools/ci.sh release`; no secrets in logs; Data safety matches; source for the shipped build (the Play link points at public `dspi-android`, which won't hold it: an owner step per release, a source tarball or a public mirror of `apps/android`, OD1) | Owner uploads each test update (AN14, AN22, AN15, AN16) |
| **R-Mac** | SW-M1, SW-M2, SW-M4, SW-M5, SW8–SW14, HO1. **If OD3 stops:** SW4–SW7, SW-M1, SW-M2, SW-M5, HO1. Either way, it **does not wait for HB or SW-M3** | Owner; OD-DevID if distributing |
| **R-iOS** | SW15, SW16, HO1, LC1's inventory current; review video; **the SysEx ID question answered (below)**; **the owner has confirmed the licence terms** | **Weeb Labs submit it, on their account.** Agents do neither |
| **R-Web** | Set by WB0 | |

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
- **Qt undo (OD5):** "no for now". No WP; section 1's rule keeps it possible.
- **Localisation (OD6):** "have the plumbing there, but just english for now".
- **Core voltage in GUIs (OD7):** writes under an Advanced menu, plus a calibration flow, in Qt, Android and Swift, with logic in the core (CG11, QD28, AN23, SW23, XC6). This replaces the earlier "status only" recommendation.
- **Qt self-update (OD8):** check-and-link, opt-in.
- **SysEx ID (OD9):** "no for now". No registration; `0x7D` stays. **This limits public MIDI releases, iOS included:** the MIDI Association's policy says `0x7D` is not for public products. **The question comes back before R-iOS**, and before any public firmware with MIDI.
- **Windows C# (OD10):** retired after `docs/PARITY.md`'s rows are settled. That is our line; Weeb Labs' repo stays theirs.
- **`dspi hub` (OD11):** yes, with service placeholders (TM11).
- **Earlier decisions kept:** no separate Android Link patch; R8 and edge-to-edge in separate releases.

### For the owner

| # | Decision | Recommendation |
|---|---|---|
| **OD-Lic** | **The exact iOS terms. Deferred by the owner.** The direction is decided (above). Yours and Weeb Labs'. | **Not legal advice.** Settle from LC1's list when the time comes: more code in the iOS binary means more to license. Also ask Weeb Labs for a Console-Linux licence and which of their Mac app's `LICENSE` (GPL) and README (MIT) holds. Remove §7 terms on what Weeb Labs would own. R-iOS stays gated. |
| **OD-WL** | Propose the core-based frontend to Weeb Labs, and when. **Deferred by the owner.** | **At M0**, with its matrix and parity table, and SW-V1's evidence if you want the Swift case stronger. Only ever your act. |
| **OD-DevID** | Apple Developer ID ($99/yr). **Deferred by the owner.** | **Only if we distribute a Mac binary.** Otherwise Weeb Labs sign theirs. |
| **OD3** | **Final call on Swift against Qt on the Mac, and how far Swift's move goes.** Owner: "come back with visual checks and feel". Working assumption: Swift. Comes with SW-V1 after SW8. It also decides SW9–SW14 (9–15.5 more). | **Decide on SW-V1 and SW8's evidence.** If it goes to Qt on the Mac: the Qt `.dmg` is still buildable; add back the macOS halves of QD12, QD13, QD18 and a hardware pass (about 1.5), and iOS stays Swift regardless. If Swift stops short of SW14, R-Mac uses its fallback. |

---

## 9. Unknowns and later work

| Item | Rough size | Pinned by |
|---|---|---|
| Swift domain replacement (SW8–SW14) | 12–20.5, could be 30 | SW3 (map), SW8 (hardest part first). Mirror and thread problems could change the approach |
| Android control-surface editor (AN9–AN12) | 5.5–9.5 | AN8. Phone layout for a 5 k-line desktop tab; IR and display need hardware; `FakeDevice` work may add 0.5 |
| Web build | 3–70 by route | WB0 |
| Rust hub (HB) | 12, could be 18 | HB0. Multi-device ownership in the core is the risk |
| nusb `from_fd` on Android | 0–2 | AN13: first run on a phone. No rusb fallback remains |
| Licence terms | 0 to rewriting a part | LC1. A contributor who cannot be reached, or an incompatible third-party part, would mean replacing that part in the iOS build |
| Upstream Swift merges | 0.5–1 each, recurring | SW-U1's dry run |
| Final Swift or Qt call on Mac | 0, or about 1.5 back in Qt | SW-V1, OD3 |
| Accessibility work | ±50 % | The first screen of each |
| Real ESP32 behaviour | unknown | XC4. Fixes file into `dspi-link` as new WPs |

**Not in any wave:**
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
