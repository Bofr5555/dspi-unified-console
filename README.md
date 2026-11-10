# dspi-unified-console

One repository with one Rust core and every DSPi console client built on it:
the Qt app for Windows and Linux, the Android app, the `dspi` terminal client,
the Swift app for macOS and iOS (partly moved onto the core), a web console,
and a Rust DSPi Link hub.

[DSPi](https://github.com/WeebLabs/DSPi) is Weeb Labs' open-source DSP firmware
for the Raspberry Pi Pico and Pico 2. Weeb Labs also wrote the original DSPi
Consoles (macOS in Swift, Windows in C#, Linux in Qt), DSPi-Terminal and DSPi
Link. This repository grew from those: the core comes from DSPi-Terminal's crates
and the Linux console's Rust core, the Qt app from DSPi-Console-Linux, the Swift
app from a fork of the macOS DSPi Console, and the Android app and the Rust Link
client from Bofr5555's Android console ([Credits](#credits)). The clients talk to
stock Weeb Labs firmware, and to the extension channel of Bofr5555's firmware
fork where it is present ([The core](#the-core)).

The goal is one project, built from one base, for every client. This repository
is **private** and is not an official Weeb Labs repository. The macOS DSPi
Console remains the reference for behaviour and look
([`docs/PARITY.md`](docs/PARITY.md)).

## Download and install

Prebuilt version **0.13.0** is on the
[Releases page](https://github.com/Bofr5555/dspi-unified-console/releases) (tag `v0.13.0`).
The app is called **DSPi uConsole** (it was DSPi Console until the 0.13.0 files were rebuilt on 2026-11-10).
The Google Play app is still called *DSPi Console* until its next update.
Every client shares the version number. Check a download against `SHA256SUMS`.
The desktop installers, the `.dmg` files and the Terminal binaries are **not code-signed** yet,
so each system warns the first time (how to get past it is below).

| System | Download | How |
|---|---|---|
| **Windows** 10/11 (x86_64) | `DSPi-uConsole-0.13.0-windows-x86_64-setup.exe` (or the portable `…-windows-x86_64.zip`) | Run the installer. SmartScreen warns about an unknown publisher: *More info* → *Run anyway*. No driver step on a clean machine. |
| **macOS** 14.6+ (Intel and Apple silicon) | `DSPi-uConsole-Swift-0.13.0-macos.dmg`, the native Swift app (recommended). The alternative is the Qt app, `DSPi-uConsole-0.13.0-macos.dmg`, which also runs on macOS 13 | Open the `.dmg` and drag the app to Applications. The first start is blocked (unsigned): *System Settings* → *Privacy & Security* → *Open Anyway*. Both apps are called *DSPi uConsole.app*: to keep both, rename one in Applications. Weeb Labs' own Mac app is *DSPi Console.app*. Each keeps its own settings. |
| **Debian 13, Ubuntu 25.04 or newer** | `dspi-uconsole_0.13.0_amd64.deb` | `sudo apt install ./dspi-uconsole_0.13.0_amd64.deb` (installs the USB rule too). |
| **Fedora 42 or newer** | `dspi-uconsole-0.13.0-1.x86_64.rpm` | `sudo dnf install ./dspi-uconsole-0.13.0-1.x86_64.rpm` (installs the USB rule too). |
| **Other Linux** (Ubuntu 24.04 and newer, x86_64) | `DSPi-uConsole-v0.13.0-x86_64.AppImage`, or `DSPi-uConsole-v0.13.0-linux-x86_64.tar.gz` with an installer | Make the AppImage executable and run it. For USB access install the rule once: the tar.gz's `install.sh` does it, or copy `70-dspi.rules` to `/etc/udev/rules.d/` and run `sudo udevadm control --reload`, then replug. |
| **Arch Linux** | `dspi-uconsole-0.13.0-arch-PKGBUILD.tar.gz` (or the test-built `…pkg.tar.zst`) | `makepkg -si` in the unpacked folder. |
| **Android** 8+ | Google Play: *DSPi Console for Android* (renamed with its next update). Without Play: `DSPi-uConsole-APK-0.13.0.apk` | The APK is a separate app, *DSPi uConsole (APK)*, so it installs next to the Play version. Allow installs from your browser or file manager when asked. |
| **iOS / iPadOS** | none | Not distributed (an unsigned build cannot be installed). |
| **Terminal** (`dspi`, every system) | `dspi-0.13.0-<system>.tar.gz` / `.zip`: Linux x86_64 and aarch64 (glibc 2.39+), Windows x86_64, macOS universal | Unpack and run `dspi`. On Linux, `sudo dspi --install-udev` once for USB access. The aarch64 build has no MIDI. |
| **Web console** | `dspi-uconsole-web-0.13.0.tar.gz` | See below. |

If you installed the first 0.13.0 files (named DSPi Console), the renamed ones install next to them, not over
them: remove the old copy yourself (`sudo apt remove dspi-console`, `sudo dnf remove dspi-console`,
`sudo pacman -R dspi-console`, *DSPi Console* in Windows' Settings › Apps, or the old app in Applications).
The Qt app keeps its settings.

### The web console

The web console runs in the browser and is served by the `dspi` Terminal program or by a
web server. Pick one:

1. **On this computer, with USB and hubs:** unpack the web download into a folder named
   `web` next to the `dspi` program (or pass `--console <folder>`), run `dspi web`, and open
   **http://localhost:11916/console/** in Chrome or Edge. USB needs a Chromium-based browser
   (Firefox and Safari have no WebUSB); on Linux the USB rule above is needed.
2. **From a hub on the network:** run `dspi hub --web --console <folder>` on the machine
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
| Qt, Linux and Windows | `apps/desktop/` | On the core through the C ABI. Its track is merged, including accessibility (names, roles, keyboard), localisation plumbing (every string through `qsTr`/`tr`), sliders that commit from the keyboard, and the delay limit with a refusal banner. Packages: `.deb`, `.rpm`, AppImage (Qt 6.8 LTS) and an Arch `PKGBUILD` for Linux; a zip and an installer for Windows (cross-built from Linux); a universal `.dmg` for macOS (built in CI, launch-tested on x86_64 only). None are signed or published. |
| Android | `apps/android/` | On the core through UniFFI. **0.12.0, the first release on the core, is on Google Play.** On `main` for the next release: R8 shrinking, accessibility (TalkBack labels and roles, with the layout unchanged), faster Link meters (they start in seconds, not minutes, on a lossy network), fractional delays kept, delay sliders in whole milliseconds, and the delay limit from the core. Open: edge-to-edge, localisation. |
| Terminal | `apps/terminal/` | Done but for a physical replug in its hardware pass (TM13). Refuses delays past the limit and non-finite numbers. |
| Swift, macOS and iOS | `apps/apple/` | On the core for reads: the bulk read, notifications, the view model's mirror, the unsaved-changes list and the delay limit. Read-only is enforced everywhere. iOS imports and exports files through the system document picker. Still Swift-only: writes, USB, MIDI, Link, firmware updates and the hub. Moving those depends on the owner's look-and-feel decision (OD3). |
| Link hub | `crates/dspi-hub/` | Server, pairing, roles, registry, advertising, conformance and a C ABI are merged; `dspi hub` runs it. With `--web` it serves `/` (a landing page without scripts), `/console/` (our web console) and `/hub/` (Weeb Labs' DSPi-Link page, built from a pinned commit). Connection budgets: plain HTTP 48 in all and 16 per address; WebSocket 16 in all and 4 per address before sign-in, so web pages cannot use up the app's slots. |
| Web | `apps/web/`, `crates/dspi-wasm/` | The same core in wasm, in one Worker per session, without threads. Works: the core's demo device, USB through WebUSB (Chromium, tested on a mock), DSPis behind hubs over Link (Chromium and Firefox), hosting by a hub, by `dspi web` or by any static server. Screens: main window, EQ (with Linkwitz), crossover, outputs and matrix. Next: the remaining screens (WB6), Web MIDI (WB4c), hardware runs (WB8) and its gate (WB9). Not published. |
| Link over the ESP32 bridge | `crates/dspi-link/`, `tools/esp-rig/` | LK1 and LK2 are merged: meters no longer wait behind start-up reads, and Link tolerates short silences (5 s ping, closes only after a long silence). LK3 (an emulated bridge: the real bridge firmware in Espressif's QEMU, its UART tunnelled to a DSPi, with set baud rates and network loss) and LK4 (a faster connect, and a silent bridge session replaced as soon as a new one connects) are on their branches, not merged yet. Target: at least as fast as Weeb Labs' own Link client. |
| CI | `ci/` | `ci/check.sh` is the gate (27 steps with `--all`, the web console included); `ci/matrix.sh` builds and tests every client from one commit, one row each, with the macOS Swift app, the iOS Simulator and the Qt `.dmg` on a macOS VM. The ESP32 rig comes with LK3 as an opt-in step (`--step esp-rig`), not in `--all`. The GitHub Actions file is a draft and not enabled. |

What each client lacks against the macOS reference is in
[`docs/PARITY.md`](docs/PARITY.md), section 5.

### What is owed

- **Hardware passes:** a physical replug on Linux (terminal and Qt); Link,
  notifications from another client and the firmware path from Qt on Linux and
  Windows; the Swift app on an Apple silicon Mac and on an iPhone or iPad; the
  web console against real USB and real hubs; the ESP32 bridge suite with a
  real bridge.
- **Phone checks for the next Android release:** R8, TalkBack and Link meters
  (checklist in [`docs/HIL-NEW-CORE.md`](docs/HIL-NEW-CORE.md)), and what 0.12.0
  still owes: the Link half, older firmware, background and notifications.
- **Signing:** none of the Qt packages or the `.dmg` is signed; a Developer ID
  is the owner's decision. Packages still lack the LGPL text and Qt source
  offer.
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
| `apps/desktop/` | The Qt 6 (or 5.15) and QML app, CMake (`qt/`). Its docs are in `Documentation/`. |
| `apps/desktop/packaging/` | Qt packages: `linux/` (`.deb`, `.rpm`, AppImage, udev rule), `arch/`, `windows/` (zip and installer), `macos/` (`.dmg`). |
| `apps/android/` | The Android app, Kotlin and Compose, Gradle. |
| `apps/apple/` | The Swift app, one Xcode project with macOS and iOS targets. |
| `apps/terminal/` | The `dspi` terminal client, a TUI and CLI (package `dspi-term`). |
| `apps/web/` | The web console, TypeScript and Preact, Playwright tests ([`apps/web/DEPLOY.md`](apps/web/DEPLOY.md) for hosting). |
| `tools/esp-rig/` | The emulated ESP32 Link bridge rig (on `wp/LK3` and `wp/LK4`, not yet on `main`). |
| `assets/` | Data shared by every client, `about.json` (the About pages) and `onboarding/`. |
| `tests/fixtures/` | Filter-file fixtures and the DSPi Link fixtures (`link/`, from WeebLabs/DSPi-Link). |
| `xtask/` | Developer tasks, and the udev rule they install. |
| `docs/` | `core-merge/` (the core plan `PLAN.md`, the client plan `PLAN-CLIENTS.md`, the hub, the web study, the hardware records), `PARITY.md`, `PORTABILITY.md`, `SWIFT-SEAMS.md`, `licensing/`, `terminal/`. |
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
| Qt, Linux | `--step qt`, `qt5`; `qt-offscreen`, `qt5-offscreen` run the app with its smoke driver on the demo device | `cargo build --release -p dspi-capi`, then CMake on `apps/desktop/qt` ([`apps/desktop/README.md`](apps/desktop/README.md)); `DSPiConsole --demo` |
| Qt, Windows | `--step qt-windows` (MinGW app, zip and installer); `--step windows-msvc` builds the core and terminal with MSVC | `apps/desktop/packaging/windows/build-windows.sh` |
| Android | `--step android` (core libraries), `android-tests`, `android-aab` | `apps/android/tools/ci.sh` |
| Swift, macOS and iOS | not in a container: `ci/macvm.sh macos` or `ios` runs it in the macOS VM | `apps/apple/build-xcframework.sh`, then `apps/apple/ci-test.sh macos` or `ios` |
| Web | `--step web` (Chromium and Firefox on the core's demo) | `apps/web/README.md`; `dspi web --demo` serves it on localhost |
| Qt, macOS | not in a container: `ci/macvm.sh qt-dmg` builds the `.dmg` and starts it on the demo device | `apps/desktop/packaging/macos/ci-dmg.sh` |
| Swift, macOS `.dmg` | not in a container: `ci/macvm.sh swift-dmg` builds the release `.dmg` and starts it on the demo device | `apps/apple/package-macos.sh`, then `apps/apple/test-dmg.sh` |
| Bindings | `--step kotlin`, `--step swift`, `--step drift` | `ci/compile-bindings.sh` |

Other checks: `--step test`, `proptest`, `deny`, `licences`, `wasm`, `wasm-core`,
`ios-check`. On Linux the workspace needs ALSA development files for MIDI; the
Linux Qt app needs the udev rule in `apps/desktop/packaging/linux/` for USB access.
`apps/desktop/packaging/macos/` builds the Qt app as a `.dmg` (the `qt-dmg` row);
the Swift app remains the macOS client.

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

Everything here builds on Weeb Labs' work: DSPi, the DSPi firmware, the DSPi
Consoles, DSPi-Terminal and DSPi-Link. The core merge, the Android console, the
Rust Link client and the unified console are Bofr5555's. The full list, the
same one every client's About page shows, is
[`assets/about.json`](assets/about.json) (a draft until the owner approves it);
third-party notices are in [`NOTICE.md`](NOTICE.md).

The imports, with history (each upstream's paths rewritten with
`git filter-repo` and merged with `--allow-unrelated-histories`, so
`git log --follow` works on any file):

| Component | Upstream | Commit imported |
|---|---|---|
| `apps/desktop/`, and the old core since merged into `crates/dspi-core` and `dspi-capi` | WeebLabs/DSPi-Console-Linux `main` | `4683a2c` |
| `apps/terminal/`, `crates/dspi-{proto,transport,session,cmd,tui}`, `xtask/`, `tests/fixtures/*.txt` | WeebLabs/DSPi-Terminal `main` | `8389b20` |
| `crates/dspi-link/`, `apps/android/` | Bofr5555's Android console (`dspi-android`) `main` | `63492af` |
| `apps/apple/` | Bofr5555's fork of the macOS DSPi Console, branch `ext-channel-116` | `48cf7c5` |
| `tests/fixtures/link/` (data) | WeebLabs/DSPi-Link | `86f10e3` |

## Licensing

**The project's licence statement is [`NOTICE.md`](NOTICE.md)** (WP LIC1, pending
the owner's approval of its wording): `GPL-3.0-or-later` (owner, 2026-11-08) with
three additional terms under GPL version 3 section 7 (attributions, modified
versions marked, no trademark rights), which apply to the material Bofr5555
added and to every client. `LICENSE` holds the GPL version 3 text, as usual for
"or later", and the workspace's licence field says `GPL-3.0-or-later`. Every
client shows the notice and the terms in its About. Final terms for Weeb Labs'
parts are open between the owner and Weeb Labs. This table says what is known;
it is not legal text.

| Component | What applies now | Open point |
|---|---|---|
| DSPi-Terminal parts and what grew from them (`apps/terminal/`, `crates/dspi-proto`, `dspi-transport`, `dspi-session`, `dspi-cmd`, `dspi-tui`, `xtask/`) | GPL-3.0-or-later, stated in upstream's manifests and README | Upstream has no LICENSE file. `xtask` has no licence field. |
| DSPi-Console-Linux parts and their ports (`apps/desktop/`, `crates/dspi-core`, `dspi-capi`) | GPL-3.0-or-later here, as the working assumption | **Upstream has no licence file and no licence field.** The Android console's `NOTICE.md` records "MIT per Weeb Labs, file pending". To be confirmed by Weeb Labs. |
| `crates/dspi-ffi` | GPL-3.0-or-later (the workspace field) | New here, over the parts above. |
| `crates/dspi-link/` | GPL-3.0-or-later (Bofr5555; confirmed by the owner), the workspace field | Its old workspace said `UNLICENSED`. |
| `tests/fixtures/link/` | MIT, Weeb Labs | `LICENSE.DSPi-Link` next to the files. |
| Vendored firmware headers (`crates/dspi-proto/firmware/`) and bundled firmware images (`apps/desktop/qt/firmware/*.uf2`) | GPL-3.0 (DSPi firmware) | |
| AutoEQ database (`apps/desktop/qt/autoeq/`, `crates/dspi-session/autoeq/`) | MIT (AutoEq measurements, jaakkopasanen/AutoEq), as recorded | Upstream terms to be read (client plan LC1). |
| Parts ported from Weeb Labs' DSPi Console for macOS (`apps/apple/`, and the UI, behaviour and texts other clients port from it) | GPL-3.0: its `LICENSE` says GPL-3.0, without "or later" | Open, for the owner and Weeb Labs: how that fits the project's GPL-3.0-or-later statement. |

A GPL core conflicts with App Store terms, which matters for the iOS build. The
owner's direction is in `docs/core-merge/PLAN-CLIENTS.md`, section 0.4. The
inventory is `docs/licensing/IOS-INVENTORY.md`; the licence statement and the third-party
notices are in `NOTICE.md`.

"DSPi" and "DSPi Console" are Weeb Labs' names. This repository grants no
rights to them.
