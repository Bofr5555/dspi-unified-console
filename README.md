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

Prebuilt versions are on the
[Releases page](https://github.com/Bofr5555/dspi-unified-console/releases) (tag `v<version>`).
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
   the DSPi is plugged into, then open **http://&lt;that machine&gt;:11915/** from any
   browser on the network. The page links to the console (`/console/`) and to the hub page
   (`/hub/`). Pair with the hub's PIN when asked.
3. **On your own web server:** serve the unpacked folder over HTTPS
   ([`apps/web/DEPLOY.md`](apps/web/DEPLOY.md) has the details, also for embedding it in a
   fleet product). USB works there in Chromium; hubs can only be reached from a page the
   hub itself serves (option 2), because a hub accepts only its own pages.

The Swift app in `apps/apple/` is the macOS download above; only its iOS build is not
distributed, because an unsigned iOS build cannot be installed.

## Status

As of 2026-11-08, on `main` at `73ce0a89`. The detail, one row per work package,
is in [`docs/core-merge/PLAN-CLIENTS.md`](docs/core-merge/PLAN-CLIENTS.md#status-2026-11-08).
Every client is tested in CI against the simulated device, mocks and a fake hub.
Hardware runs are recorded in `docs/core-merge/HW-*.md` and
`docs/HIL-NEW-CORE.md`; what they still lack is listed under
[What is owed](#what-is-owed).

| Client | Where | Status |
|---|---|---|
| Core | `crates/` | In use by every client except the Swift app's writes and transports. The core gaps the clients asked for are done (CG1 to CG37). Recent: writes by parameter path (`write_param`), one filter-type name table, delays kept to 0.0001 ms, and one delay limit per platform (42 ms on RP2350, 21 ms on RP2040): a longer delay, NaN or infinity is refused with a message, and an import names each delay it shortened. |
| Qt, Linux and Windows | `apps/desktop/` | *(2026-11-13.)* Rebuilt in QML from the macOS console's layout and words, on the core through the C ABI, and swapped in for the earlier port (Qt 6.8 or newer; no Qt 5, no macOS build). Beyond the Mac: read-only mode for other firmware, the Core Voltage page, undo and redo with a History window, an update check in the project's own downloads, and diagnostics bundles. Hardware pass on the K340 on Linux and Windows (QP22: `docs/core-merge/HW-QT.md`). Packages: `.deb`, `.rpm`, AppImage, a tar.gz with an installer and an Arch `PKGBUILD` for Linux; a zip and an installer for Windows (cross-built from Linux). None are signed or published. |
| Android | `apps/android/` | On the core through UniFFI. **0.12.0, the first release on the core, is on Google Play.** On `main` for the next release: R8 shrinking, accessibility (TalkBack labels and roles, with the layout unchanged), faster Link meters (they start in seconds, not minutes, on a lossy network), fractional delays kept, delay sliders in whole milliseconds, and the delay limit from the core. Open: edge-to-edge, localisation. |
| Terminal | `apps/terminal/` | Done but for a physical replug in its hardware pass (TM13). Refuses delays past the limit and non-finite numbers. |
| Swift, macOS and iOS | `apps/apple/` | On the core for reads: the bulk read, notifications, the view model's mirror, the unsaved-changes list and the delay limit. Read-only is enforced everywhere. iOS imports and exports files through the system document picker. Still Swift-only: writes, USB, MIDI, Link, firmware updates and the hub. Moving those depends on the owner's look-and-feel decision (OD3). |
| Link hub | `crates/dspi-hub/` | Server, pairing, roles, registry, advertising, conformance and a C ABI are merged; `uconsole hub` runs it. With `--web` it serves `/` (a landing page without scripts), `/console/` (our web console) and `/hub/` (Weeb Labs' DSPi-Link page, built from a pinned commit). Connection budgets: plain HTTP 48 in all and 16 per address; WebSocket 16 in all and 4 per address before sign-in, so web pages cannot use up the app's slots. |
| Web | `apps/web/`, `crates/dspi-wasm/` | The same core in wasm, in one Worker per session, without threads. Works: the core's demo device, USB through WebUSB (Chromium, tested on a mock), DSPis behind hubs over Link (Chromium and Firefox), hosting by a hub, by `uconsole web` or by any static server. Screens: main window, EQ (with Linkwitz), crossover, outputs and matrix. Next: the remaining screens (WB6), Web MIDI (WB4c), hardware runs (WB8) and its gate (WB9). Not published. |
| Link over the ESP32 bridge | `crates/dspi-link/`, `tools/esp-rig/` | LK1 and LK2 are merged: meters no longer wait behind start-up reads, and Link tolerates short silences (5 s ping, closes only after a long silence). LK3 (an emulated bridge: the real bridge firmware in Espressif's QEMU, its UART tunnelled to a DSPi, with set baud rates and network loss) and LK4 (a faster connect, and a silent bridge session replaced as soon as a new one connects) are on their branches, not merged yet. Target: at least as fast as Weeb Labs' own Link client. |
| CI | `ci/` | `ci/check.sh` is the gate (27 steps with `--all`, the web console included); `ci/matrix.sh` builds and tests every client from one commit, one row each, with the macOS Swift app and the iOS Simulator on a macOS VM, and the Qt release packages installed in clean containers. The ESP32 rig comes with LK3 as an opt-in step (`--step esp-rig`), not in `--all`. The GitHub Actions file is a draft and not enabled. |

What each client lacks against the macOS reference is in
[`docs/PARITY.md`](docs/PARITY.md), section 5.

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
| Limits profiles | An installer narrows what a system may do (per output gain, high-pass, delay range, limiter ceiling; PEQ boost; master volume). One range table in the core for every editor | ✓ (the page keeps the profile) | ✓ | ✓ | not yet | `limits` |
| Installer lock and PIN | A PIN on this console (or the hub's) leaves volume, mute and presets to the user; a recovery code is shown once. Not a security boundary: real access control is the hub's roles | reads and unlocks a hub's lock; no PIN of its own | ✓ | ✓ | not yet | `lock`; holds every write (`set`, `eq`, scripts, the TUI) |
| Confirmation tokens | Dangerous calls (preset save, copy, delete, factory reset, import, firmware writes) run only with a token their confirmation minted | ✓ | ✓ | ✓ | ✓ for the import | ✓ (`--yes` mints it) |
| Offline documents and templates | Open a `.dspipreset` with no device and edit it as one; generic templates (stereo, crossovers and the like) and build templates ship with the app, and your own can be saved | ✓ | ✓ | ✓ | ✓ | `open` (templates: not yet) |
| Document diff and partial import | What a file would change, block by block, and an import of the chosen blocks (volume levels and hardware off unless chosen) | ✓ | ✓ | ✓ | ✓ | `diff`, `import --only`, `--preview` |
| Undo groups | Compound edits (Clear All, a linked pair's preamp, Direct 1:1, Protect all outputs) are one undo step. Undoing Protect all leaves the limiters on, and every client says so | ✓ | ✓ | ✓ | not yet | ✓ (the TUI's compound edits, imports, `autoeq apply`) |
| AutoEQ preview | A headphone profile's curve before it is applied, outputs' gain untouched by default, the suggested trim shown | ✓ | ✓ | ✓ | not yet | `autoeq preview`, `autoeq apply` |

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

**Not done yet:**

- **The hub's profile and PIN from the clients (WP HUBADMIN).** The clients edit and lock
  their own copy; setting a hub's profile, PIN or pinned modes from a client, and hearing a
  hub's `limits.changed`/`mode.changed` mid-session, come with HUBADMIN.
- **Swift:** the limits editor, the installer lock, undo groups and the AutoEQ preview.

### What is owed

- **Hardware passes:** a physical replug on Linux (terminal and Qt); the
  firmware path (a flash) from Qt, and the desktop hand checks QP22 lists
  ([`docs/core-merge/HW-QT.md`](docs/core-merge/HW-QT.md)); the Swift app on an Apple silicon Mac and on an iPhone or iPad; the
  web console against real USB and real hubs; the ESP32 bridge suite with a
  real bridge.
- **Phone checks for the next Android release:** R8, TalkBack and Link meters
  (checklist in [`docs/HIL-NEW-CORE.md`](docs/HIL-NEW-CORE.md)), and what 0.12.0
  still owes: the Link half, older firmware, background and notifications.
- **Signing:** none of the Qt packages or the `.dmg` is signed (Authenticode or
  Developer ID); that is the owner's decision. Every release's `SHA256SUMS` is
  signed with the owner's minisign key ([`docs/RELEASING.md`](docs/RELEASING.md)).
- **Publishing:** only Android 0.12.0 is out. The other clients are to be
  published as GitHub releases at the next Android release (not iOS). Making
  the repository public, any store upload and any contact with Weeb Labs are
  the owner's.
- **Decisions:** the Swift app's look and feel (OD3), and the licence terms for
  an iOS build ([Licensing](#licensing)).

## Layout

| Path | What |
|---|---|
| `crates/dspi-proto/` | Pure protocol, no I/O: constants generated from the firmware headers vendored in `firmware/`, the V32 bulk codec, read-only mapping of other wire formats, extension channel codecs, MIDI SysEx framing. |
| `crates/dspi-transport/` | The `Transport` trait and its backends: USB (nusb), MIDI, Link, the demo device, a scripted mock. |
| `crates/dspi-link/` | The DSPi Link client (WebSocket), with a fake hub for tests. |
| `crates/dspi-session/` | Session logic: probe, device state, notifications, writes, analyser, control surfaces, presets, filter files, AutoEQ. |
| `crates/dspi-core/` | The embedder API: one `DspiCore` object for a GUI or a binding, with the device manager, hotplug, hub discovery, Link sessions and firmware files. |
| `crates/dspi-capi/` | C ABI over `dspi-core` for the Qt app (`include/dspi_core.h`, generated by its `build.rs`). |
| `crates/dspi-ffi/` | UniFFI library over `dspi-core` for Kotlin and Swift. |
| `crates/dspi-wasm/` | The wasm binding over `dspi-core` for the web console (WebUSB and Link). |
| `crates/dspi-hub/` | The DSPi Link hub ([`docs/core-merge/HUB.md`](docs/core-merge/HUB.md)). |
| `crates/dspi-cmd/`, `crates/dspi-tui/` | The terminal's command grammar and interface (ratatui). |
| `apps/desktop/` | The Qt 6 (6.8 or newer) and QML app, CMake (`qt/`). Its docs are `README.md` and `Documentation/behaviour.md`. |
| `apps/desktop/packaging/` | Qt packages: `build-release.sh` (every package), `linux/` (`.deb`, `.rpm`, AppImage, tar.gz, udev rule), `arch/`, `windows/` (zip and installer). |
| `apps/android/` | The Android app, Kotlin and Compose, Gradle. |
| `apps/apple/` | The Swift app, one Xcode project with macOS and iOS targets. |
| `apps/terminal/` | The `uconsole` terminal client, a TUI and CLI (package `dspi-term`). |
| `apps/web/` | The web console, TypeScript and Preact, Playwright tests ([`apps/web/DEPLOY.md`](apps/web/DEPLOY.md) for hosting). |
| `tools/esp-rig/` | The emulated ESP32 Link bridge rig (on `wp/LK3` and `wp/LK4`, not yet on `main`). |
| `assets/` | Data shared by every client, `about.json` (the About pages) and `onboarding/`. |
| `tests/fixtures/` | Filter-file fixtures and the DSPi Link fixtures (`link/`, from WeebLabs/DSPi-Link). |
| `xtask/` | Developer tasks, and the udev rule they install. |
| `docs/` | `core-merge/` (the core plan `PLAN.md`, the client plan `PLAN-CLIENTS.md`, the hub, the web study, the hardware records), `PARITY.md`, `PORTABILITY.md`, `SWIFT-SEAMS.md`, `UI-MODES.md` (Simple, Advanced and Installer), `licensing/`, `terminal/`. |
| `ci/` | `check.sh` and `run.sh` (the checks and their container wrapper), `matrix.sh` (every client from one commit), the container files, and a **draft** GitHub Actions file that is not enabled. |

## Building and testing

Every toolchain runs in a container; nothing is installed on the host.
`ci/run.sh 'COMMAND'` runs a command in the build container for this worktree,
with its caches under `~/.cache/dspi-build`. `ci/check.sh` runs named checks
through it. The images are listed in `ci/container/images.conf`, and
`ci/container/build-images.sh` builds any that are missing.

```
ci/check.sh                               # the gate: fmt, clippy, tests, Windows GNU, wasm-core, Android contract, licences, deny
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
| Swift, macOS and iOS | not in a container: `ci/macvm.sh macos` or `ios` runs it in the macOS VM | `apps/apple/build-xcframework.sh`, then `apps/apple/ci-test.sh macos` or `ios` |
| Web | `--step web` (Chromium and Firefox on the core's demo) | `apps/web/README.md`; `uconsole web --demo` serves it on localhost |
| Swift, macOS `.dmg` | not in a container: `ci/macvm.sh swift-dmg` builds the release `.dmg` and starts it on the demo device | `apps/apple/package-macos.sh`, then `apps/apple/test-dmg.sh` |
| Bindings | `--step kotlin`, `--step swift`, `--step drift` | `ci/compile-bindings.sh` |

Other checks: `--step test`, `proptest`, `deny`, `licences`, `wasm`, `wasm-core`,
`ios-check`. On Linux the workspace needs ALSA development files for MIDI; the
Linux Qt app needs the udev rule in `apps/desktop/packaging/linux/` for USB access.
The Qt app has no macOS build; the Swift app is the macOS client.

## The core

Qt reaches `dspi-core` through the C ABI (`dspi-capi`); Android through UniFFI
(`dspi-ffi`), as does the Swift app for its reads; its writes and transports
are still its own Swift code. The terminal uses the session crates
directly, with `dspi-core` and `dspi-hub` for some commands.

- **Wire format V32** (firmware 1.1.6) is read and written. Older formats, and
  newer ones, connect read-only; an image that cannot be mapped still shows the
  device and allows a firmware update.
- **Stock Weeb Labs firmware needs nothing from us.** The extension channel of
  Bofr5555's fork (`REQ_EXTENSION` 0x00: crossfeed pair map, core voltage, bulk
  source tag) is probed once per connect; on stock firmware those calls return
  `Unsupported`.

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
| Bofr5555's additions: the core merge (`crates/dspi-core`, `dspi-capi`, `dspi-ffi`, `dspi-hub`, `dspi-wasm`), the Android console, the Rust Link client (`crates/dspi-link`), the web console | GPL-3.0-or-later with the section 7 terms |
| DSPi-Terminal parts and what grew from them (`apps/terminal/`, `crates/dspi-proto`, `dspi-transport`, `dspi-session`, `dspi-cmd`, `dspi-tui`, `xtask/`) | GPL-3.0-or-later, as upstream's manifests and README state |
| `apps/apple/`, and the UI, behaviour and texts other clients follow from it | GPL-3.0 (DSPi Console for macOS; its `LICENSE` says version 3) |
| The Android files ported from DSPi Console for Windows (listed in `NOTICE.md`) | GPL-3.0 (its README says GPL v3) |
| Vendored firmware headers (`crates/dspi-proto/firmware/`) and bundled firmware images | GPL-3.0 (DSPi firmware v1.1.6), with the third-party parts in `assets/licences/firmware-THIRD-PARTY-NOTICES.txt` |
| `tests/fixtures/link/`, the hub's DSPi-Link web page | MIT, Weeb Labs |
| AutoEQ database (`crates/dspi-session/autoeq/`) | AutoEq data, MIT (`assets/licences/AutoEq-MIT.txt`); the measurement sources' own terms are an open point |
| `apps/desktop/` (the Qt app) | Not released at present; it is being ported again |

A GPL core conflicts with App Store terms, which matters for the iOS build. The
owner's direction is in `docs/core-merge/PLAN-CLIENTS.md`, section 0.4. The
inventory is `docs/licensing/IOS-INVENTORY.md`; the licence statement and the third-party
notices are in `NOTICE.md`.

"DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no
rights to them.
