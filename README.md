# dspi-unified-console

One repository with one Rust core and every DSPi console client built on it:
the Qt app for Windows and Linux, the Android app, the `uconsole` terminal client,
the Swift app for macOS and iOS (partly moved onto the core), a web console,
and a Rust DSPi Link hub.

[DSPi](https://github.com/WeebLabs/DSPi) is Weeb Labs' open-source DSP firmware
for the Raspberry Pi Pico and Pico 2. Weeb Labs also wrote the original DSPi
Consoles (macOS in Swift, Windows in C#, Linux in Qt), DSPi-Terminal and DSPi
Link. This repository grew from that work: the core is built on DSPi-Terminal's
crates and follows the macOS DSPi Console's behaviour, the Swift app is a fork of
the macOS DSPi Console, and the Android app and the Rust Link client come from
Bofr5555's Android console ([Credits](#credits)). The Qt app was rebuilt from the
macOS console on the core and replaced the earlier port on `main` (2026-11-13). The clients talk to
stock Weeb Labs firmware, and to the extension channel of Bofr5555's firmware
fork where it is present ([The core](#the-core)).

The goal is one project, built from one base, for every client. This repository
is **private** and is not an official Weeb Labs repository. The macOS DSPi
Console remains the reference for behaviour and look
([`docs/PARITY.md`](docs/PARITY.md)).

## Download and install

Prebuilt versions are to be on the
[Releases page](https://github.com/Bofr5555/dspi-unified-console/releases) (tag `v<version>`).
*(2026-11-19: only the Android app has been published, on Google Play in closed testing; the
other files below are named as they will be when they are released, see [What is owed](#what-is-owed).)*
The app is called **uConsole for DSPi**; every release file starts with `uConsole-for-DSPi-<version>`,
and packages, commands and installed files use `uconsole-for-dspi` (the Terminal's command stays `uconsole`).
The Google Play app is still called *DSPi Console* until its next update.
Every client shares the version number. Check a download against `SHA256SUMS`.
The desktop installers, the `.dmg` files and the Terminal binaries are **not code-signed** yet,
so each system warns the first time (how to get past it is below).

| System | Download | How |
|---|---|---|
| **Windows** 10/11 (x86_64) | `uConsole-for-DSPi-<version>-windows-x86_64-setup.exe` (or the portable `…-windows-x86_64.zip`) | Run the installer. SmartScreen warns about an unknown publisher: *More info* → *Run anyway*. No driver step on a clean machine. |
| **macOS** 14.6+ (Intel and Apple silicon) | `uConsole-for-DSPi-<version>-macos.dmg`, the native Swift app | Open the `.dmg` and drag the app to Applications. The first start is blocked (unsigned): *System Settings* → *Privacy & Security* → *Open Anyway*. The app is *uConsole for DSPi.app*; Weeb Labs' own Mac app is *DSPi Console.app*, and each keeps its own settings. |
| **Debian 13, Ubuntu 25.04 or newer** | `uconsole-for-dspi_<version>_amd64.deb` | `sudo apt install ./uconsole-for-dspi_<version>_amd64.deb` (installs the USB rule too). |
| **Fedora 43 or 44** | `uconsole-for-dspi-<version>-1.fc<N>.x86_64.rpm` (one per Fedora release) | `sudo dnf install ./uconsole-for-dspi-<version>-1.fc<N>.x86_64.rpm` (installs the USB rule too). |
| **Other Linux** (Ubuntu 24.04 and newer, x86_64) | `uConsole-for-DSPi-<version>-x86_64.AppImage`, or `uConsole-for-DSPi-<version>-linux-x86_64.tar.gz` with an installer | Make the AppImage executable and run it. For USB access install the rule once: the tar.gz's `install.sh --udev` does it, or copy `70-uconsole-for-dspi.rules` to `/etc/udev/rules.d/` and run `sudo udevadm control --reload`, then replug. |
| **Arch Linux** | the AUR package `uconsole-for-dspi` (`apps/desktop/packaging/arch/PKGBUILD`), built from `uConsole-for-DSPi-<version>-desktop-source.tar.gz` | `makepkg -si` in the package's folder. |
| **Android** 8+ | Google Play: *DSPi Console for Android* (renamed with its next update). Without Play: `uConsole-for-DSPi-<version>.apk` | The APK is a separate app, *uConsole for DSPi (APK)*, so it installs next to the Play version. Allow installs from your browser or file manager when asked. |
| **iOS / iPadOS** | none | Not distributed (an unsigned build cannot be installed). |
| **Terminal** (`uconsole`, every system) | `uConsole-for-DSPi-<version>-terminal-<system>.tar.gz` / `.zip`: Linux x86_64 and aarch64 (glibc 2.39+), Windows x86_64, macOS universal | Unpack and run `uconsole`. On Linux, `sudo uconsole --install-udev` once for USB access. The aarch64 build has no MIDI. |
| **Web console** | `uConsole-for-DSPi-<version>-web.tar.gz` | See below. |

The source code of each release is `uConsole-for-DSPi-<version>-source.tar.gz` (the Qt packages':
`…-desktop-source.tar.gz`).

### The web console

The web console runs in the browser and is served by the `uconsole` Terminal program or by a
web server. Pick one:

1. **On this computer, with USB and hubs:** unpack the web download into a folder named
   `web` next to the `uconsole` program (or pass `--console <folder>`), run `uconsole web`, and open
   **http://localhost:11916/console/** in Chrome or Edge. USB needs a Chromium-based browser
   (Firefox and Safari have no WebUSB); on Linux the USB rule above is needed.
2. **From a hub on the network:** run `uconsole hub --web --console <folder>` on the machine
   the DSPi is plugged into, then open **https://&lt;that machine&gt;:11915/** from any
   browser on the network. The hub's certificate is its own (self-signed), so the browser warns
   once: accept it after comparing the fingerprint the page shows with the one the hub prints
   (`uconsole hub tls fingerprint`). The page links to the console (`/console/`) and to the hub page
   (`/hub/`). Pair with the hub's PIN when asked. A PIN gives the Control role; to make a
   console an Admin (for the hub's limits and installer PINs), run `uconsole hub clients`
   and `uconsole hub role <cid> admin` on the hub's machine, or type `clients` and
   `role <cid> admin` at the running hub's prompt.
3. **On your own web server:** serve the unpacked folder over HTTPS
   ([`apps/web/DEPLOY.md`](apps/web/DEPLOY.md) has the details, also for embedding it in a
   fleet product). USB works there in Chromium; hubs can only be reached from a page the
   hub itself serves (option 2), because a hub accepts only its own pages, or through the
   server's own `wss://` proxy to them (DEPLOY.md). Option 1 is the recommended way to hubs: `uconsole web`
   reaches them over TLS pinned to each hub's key, so the browser shows no warning
   ([`docs/integrator/security.md`](docs/integrator/security.md), The browser).

The Swift app in `apps/apple/` is the macOS download above; only its iOS build is not
distributed, because an unsigned iOS build cannot be installed.

## Status

As of 2026-11-19, on `main` at `11625ba5`. The detail, one row per work package,
is in [`docs/core-merge/PLAN-CLIENTS.md`](docs/core-merge/PLAN-CLIENTS.md#status-2026-11-19)
and, for the PRO phases and the later plans, in the plan files under
[`docs/core-merge/`](docs/core-merge/) (`PLAN-P2.md`, `PLAN-P3.md`, `PLAN-P5.md`,
`PLAN-AT.md`, `PLAN-PG.md`, `PLAN-SEC.md`), each WP marked there with its merge.
Every client is tested in CI against the simulated device, mocks, fakes of both
firmware generations and a fake hub. Hardware runs are recorded in `docs/core-merge/HW-*.md` and
`docs/HIL-NEW-CORE.md`; what they still lack is listed under
[What is owed](#what-is-owed). The status as of 2026-11-08 (`73ce0a89`) is in the
history of this file.

| Client | Where | Status |
|---|---|---|
| Core | `crates/` | In use by every client, the Swift app included (its opt-out paths aside). The core gaps the clients asked for are done (CG1 to CG37). Since 2026-11-08: confirmation tokens are required for every dangerous call (P1TZ); the client read-only mode (ROMODE); the PRO features of phases 1 to 3 and 5 (below); DSPi Link over TLS 1.3 with the hub's key pinned at pairing (PLAN-SEC); the measurement maths in the new crate `dspi-measure` and the workflow in `dspi-core`'s `measure` module; and both firmware generations, 1.1.6 and upstream's paged 1.1.7, decided once per connection (PLAN-PG). Earlier: writes by parameter path (`write_param`), one filter-type name table, delays kept to 0.0001 ms, and one delay limit per platform (42 ms on RP2350, 21 ms on RP2040). |
| Qt, Linux and Windows | `apps/desktop/` | *(2026-11-13.)* Rebuilt in QML from the macOS console's layout and words, on the core through the C ABI, and swapped in for the earlier port (Qt 6.8 or newer; no Qt 5, no macOS build). Beyond the Mac: read-only mode for other firmware, the Core Voltage page, undo and redo with a History window, an update check in the project's own downloads, and diagnostics bundles. Since then: the PRO phase 1 dialogs, the hub's limits and PIN (HUBADMIN), shared snapshots, the hub's history, the alarm banner and Identify (P2QT), pairing bound to the hub's key (SECQT), measurement and auto-tune in Tools > Measure… with a "Several units" tab (P5QCAP, P5QUI, P5QUI2, ATQT), and the device's generation and the update check in Settings > About (PGQT). Hardware pass on the K340 on Linux and Windows (QP22: `docs/core-merge/HW-QT.md`). Packages: `.deb`, `.rpm`, AppImage, a tar.gz with an installer and an Arch `PKGBUILD` for Linux; a zip and an installer for Windows (cross-built from Linux). None are signed or published. |
| Android | `apps/android/` | On the core through UniFFI. **0.12.0, the first release on the core, is on Google Play, in closed testing**; `main` is at 0.13.0 (versionCode 13), not uploaded. On `main` for the next release: R8 shrinking, accessibility (TalkBack labels and roles, with the layout unchanged), faster Link meters, fractional delays kept, delay sliders in whole milliseconds, the delay limit from the core; the PRO phase 1 screens and the hub's limits and PIN; shared snapshots, the hub's history, the alarm banner and Identify (P2AN); pairing bound to the hub's key (SECAN); device info, the update check and the core's write gate (PGAND); S/PDIF units shown in the System screen, view only. Measurement comes in a later batch (PLAN-P5). Open: edge-to-edge, localisation. |
| Terminal | `apps/terminal/` | Done but for a physical replug in its hardware pass (TM13). Every write goes through the core (TMCORE), so limits, the installer lock and confirmation tokens hold for it. New commands since 2026-11-08: the PRO phase 1 commands, `uconsole hub` services, REST, TLS, groups and the firmware rollout (P2TM, SECTM, P3TM), `uconsole measure` (P5TM), and `uconsole info` and `raw --id` for the generation (PGTM); see [`apps/terminal/README.md`](apps/terminal/README.md) and `uconsole --help`. |
| Swift, macOS and iOS | `apps/apple/` | On the core by default since 2026-11-15/16: USB on the Mac (SW-M1), USB MIDI on iPhone, iPad and the Mac's MIDI paths (SW5), the DSPi Link client (SW6, with its lost-connection banner SW7), firmware updates (SW-M2), and the Mac's hub, now the Rust hub over UniFFI with TLS required by default (SW-M3, P2SW, SECSW). The DSP domains, presets, files, undo and the PRO phase 1 features run on the core's calls (SW8 to SW14, P1SW1 to P1SW3). The opt-out paths (`-CoreTransport NO`, `-CoreHub NO`) are still in the app, 1.1.6 only, to be retired (SWS13). Not yet: the device's generation and the update check (PGSWIFT), measurement (later batch). The look-and-feel decision (OD3) is still the owner's. |
| Link hub | `crates/dspi-hub/` | `uconsole hub` runs it; the Mac app hosts it too. With `--web` it serves `/` (a landing page without scripts), `/console/` (our web console) and `/hub/` (Weeb Labs' DSPi-Link page, built from a pinned commit). Since 2026-11-08: TLS 1.3 on its one port, required by default, with PIN pairing bound to its key and a PAKE for native clients (PLAN-SEC); the phase 2 services and the phase 3 fleet (below), with the REST mirror under `/api/v1/` frozen as v1 on 2026-11-16; measurement documents (P5HUB); measuring several units, `system.measure.*`, and the guard of a system apply, `system.guard.*` (PLAN-AT); devices of both generations and DSPi Link 1.2 and 1.3 clients (PLAN-PG). Connection budgets: plain HTTP 48 in all and 16 per address; WebSocket 16 in all and 4 per address before sign-in. |
| Web | `apps/web/`, `crates/dspi-wasm/` | The same core in wasm, in one Worker per session, without threads. Works: the core's demo device, USB through WebUSB (Chromium, tested on a mock), DSPis behind hubs over DSPi Link with TLS (Chromium and Firefox; `uconsole web` pins the hub's key for the page), hosting by a hub, by `uconsole web` or by any static server. The UI floor is complete (WB6a to WB6l): main window, EQ (with Linkwitz), inputs, crossover, outputs and matrix, effects, the System view, scene bar and status rail, Overview, Snapshots, Files and AutoEQ, Control Surfaces, Events, Installer, the tools (Signal Generator, Analyser, Stats for nerbs, Interrupt Monitor), Measure, Device and Settings; on a hub also the Hub screen (devices, health board, alarms, history, groups, bulk apply, firmware rollout) and Hub Settings. Next: Web MIDI (WB4c, later) and its gate (WB9). Not published. |
| Link over the ESP32 bridge | `crates/dspi-link/`, `tools/esp-rig/` | LK1 to LK4 are merged (LK3 and LK4 on 2026-11-09): meters no longer wait behind start-up reads, Link tolerates short silences, a faster connect, and the emulated bridge (the real bridge firmware in Espressif's QEMU, its UART tunnelled to a DSPi) as the opt-in step `ci/check.sh --step esp-rig`. A real bridge has not been run (XC4, XC5). Target: at least as fast as Weeb Labs' own Link client. |
| CI | `ci/` | `ci/check.sh` is the gate (26 steps with `--all`, the web console included); `ci/matrix.sh` builds and tests every client from one commit, one row each, with the macOS Swift app and the iOS Simulator on a macOS VM, and the Qt release packages installed in clean containers. The ESP32 rig is the opt-in step `--step esp-rig`, not in `--all`. The GitHub Actions file is a draft and not enabled. |

What each client lacks against the macOS reference is in
[`docs/PARITY.md`](docs/PARITY.md), section 5; sections H to K there cover what goes
beyond it (the audio link system, measurement, both firmware generations).

### PRO features, phase 1 (on `main`, 2026-11-14)

Phase 1 of the PRO plan ([`docs/core-merge/PLAN-CLIENTS.md`](docs/core-merge/PLAN-CLIENTS.md),
section 5.9) is merged. Each feature lives once in the core; the clients show it.
The integrator documents are in [`docs/integrator/`](docs/integrator/)
(`document-format.md`, `limits-profiles.md`, `installer-lock.md`).

| Feature | What it does | Web | Qt | Android | Swift | Terminal |
|---|---|---|---|---|---|---|
| Snapshots | Whole-device snapshots kept on this computer, one folder per device: capture, name, note, recall, export, import | ✓ (kept in the page) | ✓ | ✓ | ✓ with the core attached | `snapshot` |
| A/B compare | Two snapshots (or the device now and one), toggled with one bulk write and diffed; keep either side. The toggle key is a plain `B` | ✓ | ✓ | ✓ | ✓ | `ab` |
| Safe apply | A recall or an import with a way back: Keep, or it goes back by itself (30 s by default, 5 to 300 s in Advanced). Over DSPi Link the hub puts it back too if the console goes away | ✓ | ✓ | ✓ | ✓ | `--guard`, on by default over DSPi Link |
| Limits profiles | An installer narrows what a system may do (per output gain, high-pass, delay range, limiter ceiling; PEQ boost; master volume). One range table in the core for every editor | ✓ (the page keeps the profile) | ✓ | ✓ | ✓ with the core attached | `limits` |
| Installer lock and PIN | A PIN on this console (or the hub's) leaves volume, mute and presets to the user; a recovery code is shown once. Not a security boundary: real access control is the hub's roles | reads and unlocks a hub's lock; no PIN of its own | ✓ | ✓ | ✓ with the core attached | `lock`; holds every write (`set`, `eq`, scripts, the TUI) |
| Confirmation tokens | Dangerous calls (preset save, copy, delete, factory reset, import, firmware writes) run only with a token their confirmation minted | ✓ | ✓ | ✓ | ✓ with the core attached | ✓ (`--yes` mints it) |
| Offline documents and templates | Open a `.dspipreset` with no device and edit it as one; generic templates (stereo, crossovers and the like) and build templates ship with the app, and your own can be saved | ✓ | ✓ | ✓ | ✓ | `open` (templates: not yet) |
| Document diff and partial import | What a file would change, block by block, and an import of the chosen blocks (volume levels and hardware off unless chosen) | ✓ | ✓ | ✓ | ✓ | `diff`, `import --only`, `--preview` |
| Undo groups | Compound edits (Clear All, a linked pair's preamp, Direct 1:1, Protect all outputs) are one undo step. Undoing Protect all leaves the limiters on, and every client says so | ✓ | ✓ | ✓ | ✓ with the core attached | ✓ in the TUI (compound edits, imports); a one-shot command is its own process, so `uconsole undo` has nothing to undo |
| AutoEQ preview | A headphone profile's curve before it is applied, outputs' gain untouched by default, the suggested trim shown | ✓ | ✓ | ✓ | ✓ with the core attached | `autoeq preview`, `autoeq apply` |

**Terminal commands** (`uconsole --help` lists them all; each one-shot command is one
session of the core, so an unlock lasts for that command):

```sh
uconsole open site.dspipreset                     # edit a document with no device
uconsole snapshot save "Before" --note "as found"
uconsole snapshot list
uconsole diff site.dspipreset                     # what it would change on the device
uconsole import site.dspipreset --only eq,crossover --guard 60   # type keep, or it goes back
uconsole ab "Before" 2 --keep b                   # compare, end on B
uconsole --link hub.local snapshot recall 1       # guarded for 30 s by default over DSPi Link
uconsole --link hub.local snapshot recall 1 --no-guard
uconsole limits set profile.dspipreset            # also: limits show | clear
uconsole lock status                              # also: unlock | set-pin | clear-pin
uconsole autoeq preview <id>                      # then: autoeq apply <id> [ch | l r]
uconsole status
```

Every write of the Terminal goes through the core, so the same rules apply as in the
other clients: `uconsole set`, `uconsole eq`, scripts of them and the TUI are held to the
limits profile and the installer lock (refused with the core's words, exit code 3), a
dangerous write (a preset save or delete, a save, a factory reset, the bootloader) asks
on a terminal and takes `--yes` in a script, and a compound edit in the TUI is one undo
step. A hub's device is opened as the core's own DSPi Link session: the hub's lock shows
as `on the hub`, and a guarded recall or import is guarded by the hub too (`hub: true`
in `--json`), with the hub's backstop line.

Over DSPi Link (`--link`) a snapshot recall and an import of a whole-device document are
guarded by default for 30 s; `--guard S` sets the time, `--no-guard` opts out. Locally (USB,
MIDI, the demo) nothing is guarded unless `--guard` is given. A guard waits for a person:
a piped `keep` keeps at once. The installer PIN is never echoed and never taken on the
command line (`DSPI_INSTALLER_PIN` or `--installer-pin -` in a script).

**Not done yet** (as of 2026-11-14; both are done since):

- **The hub's profile and PIN from the clients (WP HUBADMIN).** Done 2026-11-14 (`d4c9dad2`): every
  client sets a hub's profile, PIN and pinned modes and hears the hub's `limits.changed` and
  `mode.changed` mid-session.
- **Swift without the core.** Done 2026-11-15/16: the Mac's USB, USB MIDI, the Swift Link client and the
  Mac's hub run on the core by default (SW-M1, SW5, SW6, SW-M3), so the PRO features above work there
  without a flag. Only the opt-out paths (`-CoreTransport NO`, `-CoreHub NO`) lack them.

### PRO phase 2: the hub as a server (on `main`, 2026-11-15)

Phase 2 ([`docs/core-merge/PLAN-P2.md`](docs/core-merge/PLAN-P2.md)) is merged, P2SW (the Mac hub)
on 2026-11-16. The hub keeps running with no console open and is useful on its own; the contract is
[`docs/integrator/hub-api.md`](docs/integrator/hub-api.md), sections 1 to 18. Each family can be turned
off (`uconsole hub --no-health`, `--no-inventory`, ...).

| Family | What it does |
|---|---|
| Inventory and health | Every DSPi the hub has seen, online or not, kept by serial; each one's health (load, clip indicator, temperature, uptime, restarts) |
| Identify | Makes one device blink its LED, optionally with a quiet tone on one output |
| History | A shared event history of the hub's devices, with who did what |
| Alarms | Offline, restarted, clipping, too hot and the rest, raised and cleared by the hub, acknowledged by a person; rules and thresholds set by an Admin |
| Notifications out | Webhooks (HTTPS), push and MQTT targets for alarms and schedule results; off until an Admin adds one |
| Parameters and snapshots | Parameters read and written by name, presets and snapshots by name, and a shared copy of each device's snapshots on the hub (the hub's copy wins) |
| Schedules | Rules the hub runs by itself at a local time or once: a preset, a snapshot, a scene, the volume or mute |
| MQTT and Home Assistant | The hub's MQTT client, with Home Assistant's discovery |
| REST and API keys | `/api/v1/` on the same port (`--rest`), for API keys an Admin mints with a role |

Every write path (the API, REST, MQTT, schedules, a hub-side recall) goes through the same roles,
limits and lock checks as a console. The web console's Hub and Hub Settings screens show all of it;
Qt, Android and the Swift app get what a single-device user needs: shared snapshots, the hub's
history, the alarm banner and Identify.

### Security: DSPi Link over TLS (on `main`, 2026-11-16)

The security plan ([`docs/core-merge/PLAN-SEC.md`](docs/core-merge/PLAN-SEC.md), waves SEC-A to SEC-D)
is merged; the summary for users and integrators is
[`docs/integrator/security.md`](docs/integrator/security.md).

- **The hub serves TLS 1.3 on its one port** (11915: the WebSocket, the web pages, the REST mirror),
  `--tls require` by default; `--tls on` also takes unencrypted clients, which cannot pair, and
  `--upstream-compat` is the stated downgrade for upstream DSPi Link clients.
- **Clients pin the hub's key when they pair** and accept only that key from then on: PIN pairing bound
  to the key (fingerprint and QR code), a PAKE for the native clients, key rotation signed by the old
  key (`uconsole hub tls rotate`). A hub once pinned is never reached in plaintext; a hub that is not
  encrypted shows as "(unsecured)".
- **Outbound:** webhooks over HTTPS and MQTT over TLS. **Downloads:** the bundled firmware images are
  checked against pinned digests in CI (`--step firmware-digests`), and every release's `SHA256SUMS`
  is signed with minisign (below).
- The device links themselves (USB, USB MIDI, the audio link) stay local and unencrypted; the plan says why.

### PRO phase 3: the fleet in the web console (on `main`, 2026-11-16)

Phase 3 ([`docs/core-merge/PLAN-P3.md`](docs/core-merge/PLAN-P3.md)) is merged; the contract is
[`docs/integrator/hub-api.md`](docs/integrator/hub-api.md), sections 19 to 21. From any browser on the
network, served by the hub:

- **Groups** of devices, and group writes: a volume with per-device offsets, a preset map, a shared
  snapshot applied to a group with a way back per device (bulk apply).
- **The health board** (counters, clip events a minute, last change) and the **Overview**, one device's
  outputs and meters in one table.
- **The firmware rollout** (`uconsole hub --rollout`, served over TLS only): an Admin updates the
  firmware of the hub's own USB devices with nobody at the rack, one device at a time, the settings
  exported first, the device checked after it restarts (on a K340 its stored 1.20 V core setting), an
  alarm if it does not come back. It writes through the core's PICOBOOT writer and never changes a
  core voltage.
- **The hub API v1 is frozen** (2026-11-16): additive only from now on.

Qt, Android and the Swift app get nothing in this phase (the multi-device view is the web console's).

### PRO phase 5: measurement and auto-tune (on `main`, 2026-11-19)

Phase 5 ([`docs/core-merge/PLAN-P5.md`](docs/core-merge/PLAN-P5.md), waves P5-A to P5-E) is merged. The
user's page is [`docs/manual/measurement.md`](docs/manual/measurement.md), the contract
[`docs/integrator/measurement.md`](docs/integrator/measurement.md). Phase 4 is not planned (only if the
owner asks for it).

- **The DSPi plays the test signal itself** with its own generator, so no console ever plays test
  audio; the console contributes only the microphone's samples. The maths is the crate `dspi-measure`,
  the workflow `dspi-core`'s `measure` module, the same for every client.
- **A calibrated microphone:** miniDSP UMIK-1/UMIK-2, REW, CSV and Dayton Audio (iMM-6C, UMM-6)
  calibration files become `mic` documents; levels in dB SPL are marked approximate.
- **Placements:** speakers at the listening position (one or several positions, near-field), or
  headphones on a coupler with their own lower levels and default targets.
- **Results and live:** each output's response against a target, the microphone RTA, and direct
  adjustments from the measured curve ("cut this peak", "fit this range") as one undo step.
- **Auto-tune:** delay and polarity, crossover corner and levels, then a target EQ fit, as one
  proposal in words; the summed check is mandatory before anything is applied; the apply is guarded
  between a "Before" and an "After" snapshot, then verified by measuring again, then Keep (two steps
  after a failed verify) or Revert, with A/B between the two.
- **Safety:** the level starts quiet and ramps until the microphone hears it, never to full scale; a
  run stops on clipping and on any change from elsewhere; a tweeter is measured only through its
  protective high-pass; a proposal's changes are bounded (an EQ boost of at most +3 dB for speakers);
  and a delay is offered only when the timing is reliable.
- **Documents:** `measurement`, `mic` and `target` documents, local and on the hub, and REW text export.

| Client | What it offers |
|---|---|
| Web | the Measure screen (Advanced and Installer): Measure, Results, Live, Auto-tune, Documents; the Passive RTA for a Viewer or a read-only connection |
| Qt | Tools > Measure…, the same tabs |
| Terminal | `uconsole measure`: calibration and target import, documents, REW export, and a tune from a saved measurement (it opens no microphone, so off the demo device only an EQ-only tune, unverified) |
| Demo device, every client | a "Simulated microphone" in a simulated room, for the whole flow without hardware |
| Android, Swift | later batch |

Only the owner's start-level runs with a real microphone and the K340 have been done (P5SPIKE,
2026-11-18); a full session on hardware is owed (P5HIL).

### Several units measured together (on `main`, 2026-11-19)

PLAN-AT ([`docs/core-merge/PLAN-AT.md`](docs/core-merge/PLAN-AT.md)) is merged on the console side;
its firmware half is on the fork's `bofr-main`. The integrator's page is
[`docs/integrator/multi-unit-measurement.md`](docs/integrator/multi-unit-measurement.md), the user's
[`docs/manual/measurement.md`](docs/manual/measurement.md) section 10.

- **One time base.** Units that share a clock (behind one alink sender, on USB-direct from one USB
  controller in phase lock, or following one DSPi over S/PDIF) share a sample counter. Each unit's own
  generator starts on a sample of it, so one microphone measures them all, one after the other.
- **S/PDIF carries the time base** between DSPi units: one DSPi as the S/PDIF source, the others
  following it.
- **Through our hub only:** the hub runs the session (`system.measure.*`: a fresh check of every unit,
  the settle wait, the schedule, aborts on every unit at once) and guards the apply (`system.guard.*`:
  a countdown, and the hub puts the installation and the units' delays back by itself if Keep does not
  come). Control measures; Admin applies.
- **Lateness is planned for:** units that play up to a second later than the others (their output
  delays, a USB-direct stream's start offset) are spaced for; a delay between units is offered only
  when it is known to within 10 µs, after a summed check across the units.
- **The web console and Qt** have a "Several units" tab. Without hardware:
  `uconsole --demo hub --alink --with-measure-units` serves simulated units on one time base, one on
  another, and S/PDIF units, with the demo hub's simulated microphone.

Nothing of it has run on real boards yet (ATHIL).

### Both firmware generations (on `main`, 2026-11-19)

PLAN-PG ([`docs/core-merge/PLAN-PG.md`](docs/core-merge/PLAN-PG.md), waves PG-A to PG-D) is merged:
every component speaks the old protocol, 1.1.6 with or without this fork's extension channel, and
upstream's paged protocol of firmware 1.1.7. The client's side is
[`docs/integrator/protocol-generations.md`](docs/integrator/protocol-generations.md), the user's
[`docs/manual/notices.md`](docs/manual/notices.md).

- **The core decides the generation once per connection**, before anything else is sent (the probe
  `SYS.PROTOCOL_INFO`), and an adapter turns its 1.1.6-shaped calls into paged requests, so every client
  on the core connects to both. This fork's extensions travel on page 0 (`REQ_EXTENSION` 0x00) or, on
  paged firmware, on the private page 0xE0 (tunnel `0xE010`).
- **DSPi Link 1.2 and 1.3:** the core's Link client talks to both kinds of hub; our hub serves devices
  of both generations, and answers `0x86` to any request that names a page toward a legacy device.
- **The update check before the bootloader entry** (no image for a board without BOOTSEL, the lineage
  phrase before leaving this fork's firmware, the generation change said) and **the core's write
  gate**, which judges every firmware write on the target it read itself.
- **The alink sender** (`firmware/DSPi-Sender`) connects as its own device kind with a limited
  connection: device info, the audio link block and the update. Recognising the later dual-stack
  sender on its paged stack is pending (in review).
- **Device info** (generation, protocol version, pages, envelope, board): web Device screen, Qt and
  Android Settings > About, `uconsole info`. Raw requests by command id: `uconsole raw --id`, and the
  web and Qt raw dialogs. Swift: not yet (PGSWIFT).

The port of this fork's firmware onto 1.1.7 (FWPG1 to FWPG6) waits for upstream to tag 1.1.7. No real
1.1.7 board has been run yet (PGSMOKE, PGHIL).

### What is owed

- **Hardware passes:** a physical replug on Linux (terminal and Qt); the
  firmware path (a flash) from Qt, and the desktop hand checks QP22 lists
  ([`docs/core-merge/HW-QT.md`](docs/core-merge/HW-QT.md)); the Swift app on an Apple silicon Mac and on an iPhone or iPad; the
  web console against real USB and real hubs; the ESP32 bridge suite with a
  real bridge (XC4, XC5).
- **Hardware runs of the later plans** (each the coordinator's with the owner's go): one firmware
  rollout of the K340 through the hub with the image it runs (P3HIL); a full measurement and auto-tune
  session with the microphone (P5HIL); several real units on one time base, S/PDIF between real units
  (ATHIL); a real 1.1.7 board (PGSMOKE, PGHIL).
- **Phone checks for the next Android release:** R8, TalkBack and Link meters
  (checklist in [`docs/HIL-NEW-CORE.md`](docs/HIL-NEW-CORE.md)), and what 0.12.0
  still owes: the Link half, older firmware, background and notifications.
- **Client work still open:** the Swift app's generation and update check (PGSWIFT) and the retirement
  of its opt-out paths (SWS13); measurement on Android and in the Swift app (later batch); Web MIDI
  (WB4c) and the web's gate (WB9); optimisation (OPT0 to OPT9) and polish (PL-*).
- **Signing:** none of the Qt packages or the `.dmg` is signed (Authenticode or
  Developer ID); that is the owner's decision. Every release's `SHA256SUMS` is
  signed with the owner's minisign key: its public half is published as
  [`docs/release/minisign.pub`](docs/release/minisign.pub) (2026-11-18), and the secret half is kept by the
  owner, outside the repository ([`docs/RELEASING.md`](docs/RELEASING.md)). The Swift app's own updater
  still carries a test key, to be swapped at its first signed release.
- **Publishing:** only the Android app is out (0.12.0, Google Play, closed testing). The other clients
  are to be published as GitHub releases at the next Android release (not iOS). Making
  the repository public, any store upload and any contact with Weeb Labs are
  the owner's.
- **Decisions:** the Swift app's look and feel (OD3), and the licence terms for
  an iOS build ([Licensing](#licensing)).

## Layout

| Path | What |
|---|---|
| `crates/dspi-proto/` | Pure protocol, no I/O: constants generated from the firmware headers vendored in `firmware/`, the V32 bulk codec, read-only mapping of other wire formats, extension channel codecs, MIDI SysEx framing, and upstream 1.1.7's paged command registry (vendored) with its planner. |
| `crates/dspi-transport/` | The `Transport` trait and its backends: USB (nusb), MIDI, Link, the demo device, a scripted mock. |
| `crates/dspi-link/` | The DSPi Link client (WebSocket), with a fake hub for tests. |
| `crates/dspi-session/` | Session logic: probe, device state, notifications, writes, analyser, control surfaces, presets, filter files, AutoEQ. |
| `crates/dspi-core/` | The embedder API: one `DspiCore` object for a GUI or a binding, with the device manager, hotplug, hub discovery, Link sessions, firmware files and the update check, the PRO features (snapshots, limits, the installer lock, tokens) and the measurement workflow (`measure`). |
| `crates/dspi-measure/` | The measurement maths of PRO phase 5, pure and without I/O: the stimulus and timing, responses and the RTA, microphone calibration and targets, the bounded EQ fit, alignment and the crossover corner, the simulated room, the measurement documents. |
| `crates/dspi-capi/` | C ABI over `dspi-core` for the Qt app (`include/dspi_core.h`, generated by its `build.rs`). |
| `crates/dspi-ffi/` | UniFFI library over `dspi-core` for Kotlin and Swift. |
| `crates/dspi-wasm/` | The wasm binding over `dspi-core` for the web console (WebUSB and Link). |
| `crates/dspi-hub/` | The DSPi Link hub ([`docs/core-merge/HUB.md`](docs/core-merge/HUB.md); its API: [`docs/integrator/hub-api.md`](docs/integrator/hub-api.md)), with its services, the fleet, the rollout and the audio link system model. |
| `crates/dspi-cmd/`, `crates/dspi-tui/` | The terminal's command grammar and interface (ratatui). |
| `apps/desktop/` | The Qt 6 (6.8 or newer) and QML app, CMake (`qt/`). Its docs are `README.md` and `Documentation/behaviour.md`. |
| `apps/desktop/packaging/` | Qt packages: `build-release.sh` (every package), `linux/` (`.deb`, `.rpm`, AppImage, tar.gz, udev rule), `arch/`, `windows/` (zip and installer). |
| `apps/android/` | The Android app, Kotlin and Compose, Gradle. |
| `apps/apple/` | The Swift app, one Xcode project with macOS and iOS targets. |
| `apps/terminal/` | The `uconsole` terminal client, a TUI and CLI (package `dspi-term`). |
| `apps/web/` | The web console, TypeScript and Preact, Playwright tests ([`apps/web/DEPLOY.md`](apps/web/DEPLOY.md) for hosting). |
| `tools/esp-rig/` | The emulated ESP32 Link bridge rig (`ci/check.sh --step esp-rig`). |
| `tools/alink-scope/`, `tools/shots/` | The alink timing tool on a PicoScope mixed-signal scope (WP SCOPE1), and the screenshot sheets of the Terminal and the web console. |
| `firmware/manifest.json`, `tools/firmware_manifest.py` | The SHA-256 of every firmware image the apps bundle, checked by `--step firmware-digests`. |
| `assets/` | Data shared by every client, `about.json` (the About pages) and `onboarding/`. |
| `tests/fixtures/` | Filter-file fixtures and the DSPi Link fixtures (`link/`, from WeebLabs/DSPi-Link). |
| `xtask/` | Developer tasks, and the udev rule they install. |
| `docs/` | `core-merge/` (the core plan `PLAN.md`, the client plan `PLAN-CLIENTS.md`, the plans of the later phases `PLAN-P2.md`, `PLAN-P3.md`, `PLAN-P5.md`, `PLAN-AT.md`, `PLAN-PG.md`, `PLAN-SEC.md`, the hub, the web study, the hardware records), [`integrator/`](docs/integrator/README.md) (the integrator guide), `manual/` (the user manual's first pages: measurement, notices), `release/` (the minisign public key, the release notes template), `GLOSSARY.md`, `PARITY.md`, `PORTABILITY.md`, `SWIFT-SEAMS.md`, `SWIFT-STAYS.md`, `UI-MODES.md` (Simple, Advanced and Installer), `RELEASING.md`, `licensing/`, `terminal/`. |
| `ci/` | `check.sh` and `run.sh` (the checks and their container wrapper), `matrix.sh` (every client from one commit), the container files, and a **draft** GitHub Actions file that is not enabled. |

## Building and testing

Every toolchain runs in a container; nothing is installed on the host.
`ci/run.sh 'COMMAND'` runs a command in the build container for this worktree,
with its caches under `~/.cache/dspi-build`. `ci/check.sh` runs named checks
through it. The images are listed in `ci/container/images.conf`, and
`ci/container/build-images.sh` builds any that are missing.

```
ci/check.sh                               # the gate: fmt, clippy, tests, Windows GNU, wasm-core, Android contract, licences, deny, firmware digests
ci/check.sh --all                         # every container step, the web console included
ci/check.sh --crate dspi-core             # fmt, clippy and tests of one crate
ci/check.sh --step qt --step terminal     # named steps; the list is at the top of ci/check.sh
ci/matrix.sh                              # build and test every client from one commit, and print the matrix
```

Per client:

| Client | In a container | Entry point |
|---|---|---|
| Terminal | `ci/check.sh --step terminal` | `cargo build --release -p dspi-term`; `cargo run -p dspi-term -- --demo dump` runs on the simulated device |
| Qt, Linux | `--step qt`; `qt-offscreen` runs the app with its smoke driver on the demo device; `qt-ts` checks the translation source | `cargo build --release -p dspi-capi`, then CMake on `apps/desktop/qt` ([`apps/desktop/README.md`](apps/desktop/README.md)); `uconsole-for-dspi --demo` |
| Qt, Windows | `--step qt-windows` (MinGW app, zip and installer, the installer tested under Wine); `--step windows-msvc` builds the core and terminal with MSVC | `apps/desktop/packaging/windows/build-windows.sh` |
| Qt release packages | not in `ci/check.sh`: `ci/release-desktop.sh --test` (matrix row `qt-packages`) builds every package in its own image and installs the Linux ones in clean containers | `apps/desktop/packaging/build-release.sh`, `linux/test-packages.sh` |
| Android | `--step android` (core libraries), `android-tests`, `android-aab` | `apps/android/tools/ci.sh` |
| Swift, macOS and iOS | not in a container: `ci/macvm.sh macos` or `ios` runs it in the macOS VM; `ci/macvm.sh build macos` or `build ios` only compiles the app and its tests (`xcodebuild build-for-testing`), a cheap check after a merge | `apps/apple/build-xcframework.sh`, then `apps/apple/ci-test.sh macos` or `ios` (`--build-only` to compile without testing) |
| Web | `--step web` (Chromium and Firefox on the core's demo) | `apps/web/README.md`; `uconsole web --demo` serves it on localhost |
| Swift, macOS `.dmg` | not in a container: `ci/macvm.sh swift-dmg` builds the release `.dmg` and starts it on the demo device | `apps/apple/package-macos.sh`, then `apps/apple/test-dmg.sh` |
| Bindings | `--step kotlin`, `--step swift`, `--step drift` | `ci/compile-bindings.sh` |

Other checks: `--step test`, `proptest`, `deny`, `licences`, `firmware-digests`, `wasm`, `wasm-core`,
`ios-check`, `conformance-ts` (upstream's TypeScript Link client against the hub), and the opt-in
`esp-rig`. On Linux the workspace needs ALSA development files for MIDI; the
Linux Qt app needs the udev rule in `apps/desktop/packaging/linux/` for USB access.
The Qt app has no macOS build; the Swift app is the macOS client.

## The core

Qt reaches `dspi-core` through the C ABI (`dspi-capi`); Android and the Swift app through
UniFFI (`dspi-ffi`; the Swift app's opt-out paths are its own Swift code); the web console through
the wasm binding (`dspi-wasm`). The terminal uses the session crates for its interface and grammar,
writes through `dspi-core` (TMCORE), and runs `dspi-hub` for `uconsole hub`.

- **Wire format V32** (firmware 1.1.6) is read and written. Older formats, and
  newer ones, connect read-only; an image that cannot be mapped still shows the
  device and allows a firmware update.
- **Firmware 1.1.7** (upstream's paged protocol, the same V32 image) is read and written too
  *(2026-11-19, PLAN-PG)*: the generation is decided once per connection, before anything else is
  sent, and an adapter turns the core's calls into paged requests
  ([`docs/integrator/protocol-generations.md`](docs/integrator/protocol-generations.md)). Firmware with a
  newer protocol major is refused with a notice.
- **Stock Weeb Labs firmware needs nothing from us.** The extension channel of
  Bofr5555's fork (`REQ_EXTENSION` 0x00: crossfeed pair map, core voltage, bulk
  source tag, the audio link and its time base) is probed once per connect; on stock firmware those calls return
  `Unsupported`. On paged firmware the same extensions travel on the private page 0xE0.

The design of the core (transports, C ABI, UniFFI, Link sessions, echo handling)
is in [`docs/core-merge/PLAN.md`](docs/core-merge/PLAN.md); platform notes are in
[`docs/PORTABILITY.md`](docs/PORTABILITY.md).

## Credits

Everything here builds on Weeb Labs' work: DSPi, the DSPi firmware, DSPi Console
for macOS and for Windows, DSPi-Terminal and DSPi-Link. The core merge, the Android console, the
Rust Link client and the unified console are Bofr5555's. The full list, the
same one every client's About page shows, is
[`assets/about.json`](assets/about.json) (a draft until the owner approves it);
third-party notices are in [`NOTICE.md`](NOTICE.md).

The imports, with history (each upstream's paths rewritten with
`git filter-repo` and merged with `--allow-unrelated-histories`, so
`git log --follow` works on any file):

| Component | Upstream | Commit imported |
|---|---|---|
| `apps/terminal/`, `crates/dspi-{proto,transport,session,cmd,tui}`, `xtask/`, `tests/fixtures/*.txt` | WeebLabs/DSPi-Terminal `main` | `8389b20` |
| `crates/dspi-link/`, `apps/android/` | Bofr5555's Android console (`dspi-android`) `main` | `63492af` |
| `apps/apple/` | Bofr5555's fork of the macOS DSPi Console, branch `ext-channel-116` | `48cf7c5` |
| `tests/fixtures/link/` (data) | WeebLabs/DSPi-Link | `86f10e3` |

## Licensing

> **DRAFT.** Not legal text: the statement is [`NOTICE.md`](NOTICE.md), pending the
> owner's approval. This section only summarises it.

The program as a whole is distributed under the **GNU GPL version 3**: it combines
Weeb Labs' parts under GPL-3.0 (the DSPi firmware, DSPi Console for macOS, DSPi
Console for Windows) or GPL-3.0-or-later (DSPi-Terminal), DSPi-Link under MIT, and
Bofr5555's own additions, which are licensed **GPL-3.0-or-later with three
additional terms** under GPL version 3 section 7 (attributions kept, modified
versions marked, no trademark rights). `LICENSE` holds the GPL version 3 text; the
workspace's licence field, `GPL-3.0-or-later`, is the licence of Bofr5555's own
additions. Every client shows the notice and the terms in its About, and every
release is published with the source archive of the commit it was built from
(`ci/release-source.sh`).

| Component | What applies |
|---|---|
| Bofr5555's additions: the core merge (`crates/dspi-core`, `dspi-capi`, `dspi-ffi`, `dspi-hub`, `dspi-measure`, `dspi-wasm`), the Android console, the Rust Link client (`crates/dspi-link`), the web console | GPL-3.0-or-later with the section 7 terms |
| DSPi-Terminal parts and what grew from them (`apps/terminal/`, `crates/dspi-proto`, `dspi-transport`, `dspi-session`, `dspi-cmd`, `dspi-tui`, `xtask/`) | GPL-3.0-or-later, as upstream's manifests and README state |
| `apps/apple/`, and the UI, behaviour and texts other clients follow from it | GPL-3.0 (DSPi Console for macOS; its `LICENSE` says version 3) |
| The Android files ported from DSPi Console for Windows (listed in `NOTICE.md`) | GPL-3.0 (its README says GPL v3) |
| Vendored firmware headers (`crates/dspi-proto/firmware/`) and bundled firmware images | GPL-3.0 (DSPi firmware v1.1.6), with the third-party parts in `assets/licences/firmware-THIRD-PARTY-NOTICES.txt` |
| `tests/fixtures/link/`, the hub's DSPi-Link web page | MIT, Weeb Labs |
| AutoEQ database (`crates/dspi-session/autoeq/`) | AutoEq data, MIT (`assets/licences/AutoEq-MIT.txt`); the measurement sources' own terms are an open point |
| `apps/desktop/` (the Qt app) | Rebuilt on the core (2026-11-13), not released yet; its packages are to be published with the `…-desktop-source.tar.gz` archive (`NOTICE.md`, Source code) |

A GPL core conflicts with App Store terms, which matters for the iOS build. The
owner's direction is in `docs/core-merge/PLAN-CLIENTS.md`, section 0.4. The
inventory is `docs/licensing/IOS-INVENTORY.md`; the licence statement and the third-party
notices are in `NOTICE.md`.

"DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no
rights to them.
