# Making the DSPi consoles portable: Linux, Windows and macOS

*2026-11-06.* This covers Weeb Labs' Linux DSPi Console (Qt) and
DSPi-Terminal, ported so that one code base builds natively on Linux,
Windows and macOS. It also covers how the two came
together in this repository.

**Two periods.** The port was done on the old desktop core (`dspi-core-linux`:
rusb, libusb, `dspi_core.dll`). Then the core merge
([`core-merge/PLAN.md`](core-merge/PLAN.md)) replaced that core and deleted it.
Summary and sections 1, 6, 7 and 11 describe the state after the merge.
Sections 2, 4 (the results table), 5, 8, 9 and 10 are the record of that earlier
work: **their results were not repeated on the merged core unless section 4 says
so.** The root [`README.md`](../README.md) has the current layout and builds.

Ground rules for the work:
- the Linux and Windows builds and tests ran in podman containers;
- the macOS ones ran in the macOS VM (section 9);
- nothing was pushed by me;
- the only device touched was the DSPi passed through to the macOS VM,
  and that was read-only (section 9), on the old core and the Terminal stack.

The USB path on real Windows (WinUSB) is **not verified on hardware**, on either
core. On the merged core no real device has been used on any platform.

## Summary

- **Desktop app.** Qt and QML, built with CMake, against **Qt 6 (preferred) or
  Qt 5.15**. It links the C ABI crate `dspi-capi` (library `dspi_capi`, header
  `dspi_core.h`) over `dspi-core`. There is no libusb any more: USB is nusb.
- **Windows x86_64.** Cross-compiled from Linux with Fedora's MinGW Qt 6. The core
  ships as `dspi_capi.dll`. The deliverables are a runnable folder, a zip and a
  per-user NSIS installer. All are built on the merged tree; none has been started
  on it (the Wine and Windows 11 runs in section 10 were on the old core).
- **MSVC.** `dspi_capi.dll` and `dspi.exe` build for `x86_64-pc-windows-msvc` from
  Linux with cargo-xwin. **The Qt app has not been linked with MSVC** (see "MSVC Qt
  builds" below).
- **macOS.** A universal app and `.dmg` were built and run on the old core
  (section 9). `build-macos.sh` is updated for `dspi_capi` but **has not been run on
  the merged tree**.
- **Terminal.** Builds for Linux, Windows GNU and Windows MSVC. On the merged tree
  it reaches USB, the demo device, USB MIDI and DSPi Link hubs, and reconnects by
  itself.
- **One repository, one core.** Everything lives in this monorepo, with history, in
  one Cargo workspace. Protocol constants are generated from Bofr5555/DSPi
  `bofr-main` `04122d3` for every client.

## 1. What the desktop app is built with

| | |
|---|---|
| GUI | Qt Quick / QML (about 150 QML files) plus 17 C++ files: a bridge, custom QQuickItems for plots, and controllers. `QApplication` with Fusion as the fallback style, plus a custom `qml/style` (ToolTip). |
| Qt | was 5.15 only; now 6.x (tested 6.11.2) or 5.15 (tested 5.15.18) |
| Build | CMake (AUTOMOC/AUTORCC); resources in `qt/resources.qrc` |
| Core | Rust crate `dspi-capi` (`crates/dspi-capi/`, over `crates/dspi-core/`): `staticlib` and `cdylib`, 167 `extern "C"` functions, header `include/dspi_core.h` from cbindgen in its `build.rs`. Linked statically on Linux and macOS; a DLL on Windows. |
| USB | nusb 0.2, pure Rust over usbfs, WinUSB and IOKit. No C library and no driver step. |
| Optional | KDE KWindowSystem (blur and shadow): KF5 with Qt 5, KF6 with Qt 6 |

### 1.1 Linux-specific parts (inventory)

| What | Where | Status |
|---|---|---|
| **udev rule and `pkexec` install of it** (`70-dspi.rules`, "DSPi Needs Permission" dialog) | `DSPiBridge::installUdevRule`, `main.qml`, `CpuSection.qml`, `packaging/linux/` | Linux only. Other platforms now get their own "can't be opened" advice. |
| **Bootloader drive**: `/dev/disk/by-label`, `/proc/self/mountinfo`, `udisksctl mount` | `FirmwareUpdater.cpp` | macOS already had a `getfsstat` path. Windows now uses `QStorageInfo` by label. |
| UF2 write with POSIX `open`, `write` and `fdatasync` | `FirmwareUpdater.cpp` | Windows: `_wopen(O_BINARY)` and `_commit`. Text mode would have corrupted the UF2. |
| Wayland cursor theme from KDE config or `gsettings` (`popen`) | `main.cpp` | Gated to Linux (not compiled on Windows) |
| `RESOURCE_NAME`, `setDesktopFileName`, the `.desktop` file, metainfo, AppImage hooks | `main.cpp`, `packaging/linux` | Harmless elsewhere, left as is |
| KDE blur and shadow (KWindowEffects, KWindowShadow) | `WindowEffects.cpp` | Optional at build time |
| Frameless windows with a client-side titlebar (`startSystemMove` / `startSystemResize`) | `main.qml`, `AppWindow.qml`, `WindowTitleBar.qml` | Works on Windows (Aero Snap through `startSystemMove`); see 3 |
| Fonts: Noto Sans (`sans-serif` fallback), `monospace` | `main.cpp`, `main.qml` | Windows: Segoe UI and Consolas |
| Settings via `QSettings` / `Qt.labs.settings` | throughout | XDG `.conf` on Linux; Windows now INI in `%APPDATA%` |
| AutoEQ cache | `QStandardPaths::AppDataLocation` | already portable |
| Packaging: AppImage (linuxdeploy), .deb and .rpm (CPack), AUR `PKGBUILD` | `packaging/` | Linux; Qt 6 package dependencies added |
| No ALSA, PipeWire or D-Bus use in the app | | n/a |
| macOS code already upstream: NSVisualEffectView, CoreText text, AppKit system colours (`MacSystemColors.mm`), `getfsstat` | `main.cpp`, `*.mm` | built in phase 2 |

## 2. What changed (`portable` commits, now in this repository's history)

*On the old core. Items 1, 4, 7 and 8 name things the merge has since replaced: the
library is `dspi_capi`, rusb and libusb are gone, and wIndex is handled in
`dspi_transport::usb`.*

1. **Build: say where the core is, and link it as a DLL on Windows.**
   - CMake cache variables `DSPI_CORE_DIR`, `DSPI_CARGO_TARGET_DIR` and
     `DSPI_RUST_TARGET`.
   - On Windows the core is `dspi_core.dll`, linked through its import
     library and copied next to the exe.
   - rusb is `vendored` on Windows and macOS.
2. **Build with Qt 6 as well as Qt 5.15.**
   - `find_package(QT NAMES Qt6 Qt5)`; `-DDSPI_QT_MAJOR` forces one.
   - In Qt 6 the custom style is a QML module (`qml/style/qmldir`, Fusion
     as the fallback).
   - `QtCompat.h` covers the event-position renames.
   - `AA_EnableHighDpiScaling` is set on Qt 5 only.
   - KF5 or KF6 for KDE.
   - Package dependencies per Qt version.
   - `Qt.labs.settings` still works on 6.11; it logs a deprecation note.
     QtCore's `Settings` would drop Qt 5, so it was left for later.
3. **Firmware update on Windows**: drive found by label, binary-mode
   write, `_commit`.
4. **Core: the vendor interface in wIndex on Windows.**
   - WinUSB hands an interface-recipient request only to the interface the
     handle owns. So on Windows the low byte of wIndex is always 2, as the
     C#/WinUI console does.
   - The firmware ignores wIndex on vendor requests. On Linux the core still
     sends 0 for "global" requests, unchanged.
   - A DSPi that libusb can't open on Windows (`NotSupported` or `Busy`: no
     WinUSB driver, or another program holds it) now shows as present but
     inaccessible, instead of being invisible.
5. **Windows conventions** (section 3).
6. **Windows release build from Linux**: `deploy-mingw.sh` (the
   windeployqt equivalent), the NSIS per-user installer,
   `build-windows.sh` and a `Containerfile`.
7. **cbindgen under cross builds.** The build script drops the target
   rustc flags before `cargo metadata`. Without this, cargo-xwin's
   `-C linker-flavor=lld-link` broke the host run.
8. **macOS**: links the core's built-in libusb instead of Homebrew's. Not
   yet built on a Mac.

Monorepo commits on top:
- the root workspace;
- the core header goes to `crates/dspi-core-linux/include/`;
- CMake defaults and the Linux release scripts point at the new layout;
- `dspi-link` fixture paths;
- the README and the CI draft.

## 3. Platform conventions

### Applied for Windows

| Convention | How |
|---|---|
| No console window | `WIN32_EXECUTABLE` |
| Icon and version info | `packaging/windows/dspi-console.rc.in`: the `.ico` (like `qt/dspi-console.svg` and the `.icns`) is generated by `assets/icon/render.sh` from the project icon, and version fields come from `setApplicationVersion` |
| Per-user settings in AppData | `QSettings::setDefaultFormat(IniFormat)` gives `%APPDATA%\DSPi\DSPi Console.ini`, checked under Wine. Caches go to `%LOCALAPPDATA%\DSPi\DSPi Console`. |
| Native file dialogs | already: `Qt.labs.platform` FileDialog uses the Windows dialog |
| Ctrl shortcuts | already Ctrl-based. `StandardKey.Quit` is Alt+F4 on Windows, through Qt. |
| HiDPI | per-monitor DPI awareness from Qt. Qt 5 builds now keep fractional scales (PassThrough), which Qt 6 does by default. |
| UI font | Segoe UI (falls back to Noto Sans under Wine); Consolas for numbers |
| Window chrome | The custom titlebar keeps the app's look. Caption buttons follow the Windows layout: 46 px, full height, flush right, red close (#c42b1c). On Windows 11 the frameless windows get rounded corners (DWM attribute 33). |
| Device access help | The Linux udev dialog is replaced by "close other programs using it; very old firmware needs Zadig" |
| Installer | NSIS, per user (no admin): `%LOCALAPPDATA%\Programs\DSPi Console`, a Start menu shortcut, and an entry under Settings > Apps (HKCU). Uninstall leaves the settings. |

### Not done (cheap follow-ups)

- **DWM drop shadow** for the frameless windows. Windows 10 shows them
  flat. A proper shadow needs `WS_THICKFRAME` plus `WM_NCCALCSIZE`
  handling, which can't be checked under Wine.
- **Snap Layouts flyout** on the maximise button. It needs `WM_NCHITTEST`
  to return `HTMAXBUTTON`, the same native-event filter as the shadow.
- **Code signing** (signtool) and a `.msi`.
- **The menu shows "Ctrl+…" labels.** That is right on Windows; macOS
  needs "⌘".

### Applied for macOS (phase 2, see section 9)

| Convention | How |
|---|---|
| Native menu bar | Built from the titlebar menu's groups (`AppMenu.groups`): Device, File, AutoEQ, Effects, Tools, Window, Help. Same enabled rules and actions; Commit and Factory Reset ask first (the old macOS menu did not). Settings (⌘,) moves to the application menu through its role; Qt adds Hide and Quit (⌘Q). Duplicate QML Shortcuts are off on macOS. |
| ⌘ shortcut labels | `MenuStyle.shortcutLabel()`: "⇧⌘M" in the app's own menus and tooltips; the graph help says Command and Option. Qt maps `Ctrl+` to ⌘ for the keys themselves. |
| App bundle | "DSPi Console.app": Info.plist (name, identifier `io.github.weeblabs.DSPiConsole`, versions, `NSHighResolutionCapable`, music category, minimum macOS 13) and an `.icns` made from the SVG |
| Packaging | `macdeployqt`, ad-hoc codesign (Developer ID and the hardened runtime with `SIGN_ID`), `.dmg` |
| Window chrome, colours, text | already upstream: native titlebar with vibrancy, AppKit colours, CoreText text |
| Settings | the standard plist (`~/Library/Preferences`) |

## 4. Build results (2026-11-06)

### On the merged tree (WP Q3, `wp/Q3` at `3c88309`)

Linux containers, through `ci/check.sh`. No device.

| Step | Result |
|---|---|
| `--crate dspi-capi --crate dspi-core --crate dspi-tui --crate dspi-ffi` | fmt and clippy clean, 1202 tests passed |
| `--step qt`, `--step qt5` | The app builds on Qt 6 and Qt 5.15 |
| `--step qt-offscreen` | Qt 6, the real app with its smoke driver on the demo device: 127 checks, 0 failures, 0 QML errors |
| `--step qt5-offscreen` | The same on Qt 5.15: 110 checks, 0 failures, 0 QML errors |
| `--step qt-windows` | MinGW Qt 6 app, folder (580 files, 127 MB), zip (50 MB), installer (34 MB). Built, not started. |
| `--step windows-msvc` | `dspi_capi.dll`, `dspi.exe` (dev profile). Built, not run. |

The whole workspace's test step last ran at the wave 3 merge (`d190535`): 2182
passed. `--step android` and `--step proptest` passed in the same round.

**The smoke driver does not reach** the spectrum analyser, control surfaces, signal
generator or buffer statistics, past their no-device paths: `FakeDevice` does not
model them, and the driver prints `NOT EXERCISED`. The Qt code for these is compiled
against the new core and has not run with real data on it.

**Not run on the merged tree:** `build-macos.sh`, any Qt link with MSVC,
`build-release.sh` (Linux packages), the `dspi-windows` image's route to
`build-windows.sh`, Wine, real Windows, and any real device, hub or MIDI port.

### On the old core

Paths below were in a session scratch directory (`Q`), which has not been kept.

| Target | Result | Artifact |
|---|---|---|
| Core tests, Linux | 59/59 | |
| Core tests, windows-gnu under Wine | 59/59 | |
| Core tests, windows-msvc (cargo-xwin) under Wine | 59/59 | `Q/target-msvc/x86_64-pc-windows-msvc/release/dspi_core.dll` |
| Desktop, Linux, Qt 5.15.18 | builds; runs offscreen and under Xvfb (xcb); no QML errors | `Q/build/linux-qt5/DSPiConsole` |
| Desktop, Linux, Qt 6.11.2 | same; with KF5 and KF6 blur both compile | `Q/build/linux-qt6/DSPiConsole` |
| Desktop, Windows x86_64, MinGW Qt 6.11.2 | builds; runs under Wine 11.20 (wizard, then "No Devices"); settings INI in AppData | folder `Q/dist-mono/DSPi-Console-1.1.6-beta4-windows-x86_64/`, `.zip` (51 MB), `-setup.exe` (34 MB) |
| NSIS installer under Wine (`/S`) | installs to `AppData\Local\Programs\DSPi Console`, adds the Start menu link and the HKCU uninstall key | |
| Desktop, Windows, MinGW Qt 5 | **not built**: Fedora 44's `mingw64-qt5-qtquickcontrols2` lacks its import library (`libQt5QuickControls2.dll.a`). Qt 6 is the Windows target anyway. | |
| Monorepo `cargo test --workspace --all-features` (Linux) | 1378 passed: Terminal 1297, core 59, dspi-link 22 | |
| Monorepo, windows-gnu workspace build | builds `dspi.exe`, `dspi_core.dll`, `xtask.exe`; dspi-link tests 22/22 under Wine | |

Screenshots:
- `Q/logs/wine-qt6.png` and `Q/logs/wine-mono.png` (Windows build under Wine);
- `Q/logs/linux-qt6.png`.

The Windows bundle is 127 MB unpacked: 580 files, about 60 DLLs plus the
QML modules. Fedora's MinGW Qt is built with ICU (31 MB of data), glib,
harfbuzz and OpenSSL. An official Qt build, or MSVC, would be roughly half
that.

### MSVC Qt builds

- **Core and terminal:** `cargo xwin build --target x86_64-pc-windows-msvc`
  works in a Linux container (`ci/check.sh --step windows-msvc`). The Microsoft CRT
  and SDK come from
  Microsoft under the licence the owner accepted.
- **The Qt app with MSVC, without Windows,** is possible in principle but
  was not attempted. It would need:
  - Qt's prebuilt `win64_msvc2022_64` archives from download.qt.io;
  - clang-cl and lld-link against the xwin SDK;
  - a Linux host Qt of the same version for moc, rcc and qmlcachegen
    (`QT_HOST_PATH`).

  That is fragile and unsupported by Qt.
- **On a hosted `windows-2022` runner** it should be routine:
  `install-qt-action` (Qt's official MSVC build), `cargo build -p
  dspi-capi`, CMake with Ninja under the MSVC environment, `windeployqt`,
  and optionally `signtool`. See `ci/github-actions-build.yml`, a draft that has
  never run. The CMake file links `dspi_capi.dll.lib` when `MSVC` is set.
- **Not compiled with MSVC yet, so expect small fixes:** the desktop C++
  sources have only been compiled by GCC (MinGW and Linux). The code was
  kept to standard C++17 and Qt, with the POSIX calls gated.

### USB on Windows

- Both apps now use the same code: `dspi_transport::usb`, nusb over WinUSB directly.
  It does no driver installation and assumes no Zadig.
- The firmware's MS OS 2.0 descriptors (vendor code 0x01, a function
  subset for interface 2) make Windows bind WinUSB to interface 2 on first
  plug-in. The audio interfaces stay with `usbaudio2.sys`.
- The transport claims interface 2 and sends wIndex 2. Transfers are capped at
  4 KB, so bulk goes chunked.
- WinUSB is exclusive per interface. While another console has the device open,
  it cannot be opened here.
- **Unverified on real Windows hardware.** The first test with a DSPi should:
  1. check that Device Manager shows "DSPi" under Universal Serial Bus
     devices (WinUSB);
  2. connect, read the state, change a value;
  3. check that notifications arrive (turn the volume on the PC);
  4. run a firmware update.

## 5. DSPi-Terminal

*The port as it was done. Since then the crates have grown into the shared core
(README, "The core"): `--demo`, `--midi` and `--link`, reconnect, and an ext probe on
connect. On Linux the default build now needs ALSA headers for midir;
`--no-default-features` drops that.*

**Source.** Repository `WeebLabs/DSPi-Terminal`, port branch `portable`
based on `main` @ `8389b20`, now `apps/terminal/` plus `crates/` here.

**What it is.** A Rust workspace:
- `dspi-proto`: protocol constants generated from vendored firmware
  headers;
- `dspi-transport`: USB over nusb, plus a mock transport;
- `dspi-session`, `dspi-cmd` and `dspi-tui`: session logic, command
  grammar, and the interface (ratatui + crossterm);
- the `dspi` binary.

There is no C dependency and no libusb, so cross-builds are plain cargo
builds.

| Target | Built | Tests | Smoke test |
|---|---|---|---|
| Linux x86_64 | yes | 1297 pass | `--help`, `--version`, `list`, `doctor`, the TUI with `--fixture` in a pty |
| Windows x86_64, MinGW | yes | 1297 pass under Wine | the same commands under Wine 11 |
| Windows x86_64, MSVC (cargo-xwin) | yes | not run | the same commands under Wine 11 |
| macOS | phase 2 (upstream already runs it on Apple silicon) | | |

**Platform notes.**
- **USB.** nusb uses WinUSB, IOKit and usbfs directly. As above, it uses
  interface 2, wIndex 2 and a 4 KB transfer cap.
- **Linux** needs a udev rule (`sudo dspi --install-udev`). That code is
  gated with `cfg!(target_os = "linux")`.
- **Terminal.** crossterm on every OS. Windows Terminal and ConEmu get
  truecolour and braille; the classic console gets 16 colours.
- **Config paths** come from `dirs::config_dir()`: XDG,
  `~/Library/Application Support`, or `%APPDATA%`. Fixed here: the AutoEQ
  data used to follow `HOME` on Windows.
- **`dspi --fixture`** runs the interface on a built-in example device, for
  headless and Wine tests.

**Protocol pin.**
- **Before:** WeebLabs/DSPi `release/v1.1.6` @ `557bce7` (1.1.6 beta 4).
- **After:** Bofr5555/DSPi `bofr-main` @ `af001ec` (1.1.6 final plus the
  extension channel). Wire format V32, 247 opcodes, CS caps v22. Now `04122d3`,
  which adds `EXT_NOTIFY_FEATURES` (`crates/dspi-proto/firmware/PROVENANCE.toml`).
- **Opcodes 0x54–0x57** moved from the legacy channel gain/mute to
  source/device names. They are probed with a 32-byte read, so older
  firmware reads as "absent".
- **New rows:** output latency (0x03), clock diagnostics (0x82) and
  `ext.list` (0x00).
- **New CS nouns:** PRESET_SAVE and OUTPUT_LATENCY.
- **Build warning:** the generator now warns that
  `SIGGEN_MULTITONE_MAX` is platform-conditional and is not emitted.

**Extension channel (`REQ_EXTENSION` 0x00).**
- `dspi ext list`
- `dspi ext xfeed-map [L:R …]`: the K340 map `2:0 3:1` is sent as
  `02 13 00 00` and always read back.
- `dspi ext vreg [set|trial|trial-end|auto|clear]`

On stock firmware a stalled read is reported as "not supported", and a
stalled write as "refused" (exit 3). There are 11 mock-transport tests;
none has run against a K340 yet.

**Remaining.**
- Hardware runs on Windows and Linux.
- The MSVC test run.
- TUI screens for the new names, latency and ext commands (CLI only for
  now).
- The `SOURCE` choice still lacks S/PDIF 2–4.
- The TUI config directory (`dspi`) and the AutoEQ data directory
  (`dspi-term`) have different names.

## 6. The monorepo

**Import method.** `git filter-repo` path rewrites, then
`git merge --allow-unrelated-histories`. The script is
`Q/mono/import.sh`, and it ran in the build container. Mapping:

- **DSPi-Console-Linux** (`portable` = `main` `4683a2c` plus 8 commits):
  - `core/` → `crates/dspi-core-linux/` (deleted since, by the merge);
  - everything else → `apps/desktop/`.
- **DSPi-Terminal** (`portable` = `main` `8389b20` plus 5 commits):
  - `crates/dspi/` → `apps/terminal/`;
  - `docs/` → `docs/terminal/`;
  - library crates, `tests/`, `xtask/` and the workspace files keep their
    paths, because `include_str!` paths in the crates expect them;
  - the rest → `apps/terminal/`, including its upstream workflows, which
    are inactive there.
- **dspi-android** `main` `63492af` (last change to these paths: `401ca8b`), only:
  - `rust/dspi-link/` → `crates/dspi-link/`;
  - `fixtures/link/` → `tests/fixtures/link/`.

**Workspace.**
- The root `Cargo.toml` is Terminal's, grown to all members.
- The C header is generated into `crates/dspi-capi/include/` (gitignored).

**One protocol stack.** This section used to describe two. Since the merge:
- every client gets its constants from `dspi-proto`, generated from `bofr-main`;
- the desktop's C ABI runs over `dspi-core`, on `dspi-session`;
- `crates/dspi-core-linux` is deleted. `git show d190535` still has it;
- the library is `dspi_capi`, not `dspi_core`: that name would collide in
  `target/` with `dspi-core`'s own library. The header keeps the name `dspi_core.h`.

## 7. Remaining gaps and open questions

1. **Licences. Nothing is final.** The repository is private; the owner's code is
   GPL-3.0-or-later (owner, 2026-11-08); final terms for Weeb Labs' parts are open
   between the owner and Weeb Labs (README, "Licensing").
   - DSPi-Console-Linux has no licence file upstream. The Android console's
     `NOTICE.md` records "MIT per Weeb Labs, file pending".
   - DSPi-Terminal states GPL-3.0-or-later but has no LICENSE file.
   - `dspi-link`'s manifest has no licence field (its old workspace said
     `UNLICENSED`).
2. **Hardware.** No run of the merged core on any real device, hub or MIDI port.
   See the checklist in 4.
3. **MSVC Qt build:** only on a Windows runner (or a fragile cross setup). Never
   linked.
4. **macOS:** `build-macos.sh` not run on the merged tree.
5. **The demo device** has no spectrum analyser, control surfaces, signal generator
   or buffer statistics, so Qt's paths for them have not run on the new core. These
   need a device.
6. **Linux release packaging** still targets Qt 5 (Ubuntu 22.04
   AppImage/.deb, Fedora 42 .rpm). Moving it to Qt 6 means a newer base
   (Ubuntu 24.04), which raises the glibc floor.
7. **AUR `PKGBUILD`** builds from the upstream DSPi-Console-Linux
   repository's layout. It needs a new source URL and paths if the
   monorepo becomes the source.
8. **Build images:** done. `ci/container/build-images.sh` builds every image
   `ci/run.sh` uses from its committed Containerfile.
9. Windows: shadow, Snap Layouts, signing (see 3).
10. `Qt.labs.settings` deprecation on Qt 6.11 (runtime note only).

## 8. macOS effort (estimate made before phase 2; on the old core)

**Already present upstream.**
- Full macOS code paths: native titlebar and vibrancy, CoreText text,
  AppKit system colours, and the `getfsstat` drive lookup.
- `isMacOS` styling throughout the QML.
- A previous Homebrew Qt 5 build.

**What is left.**
- Build with Qt 6 (Homebrew `qt` or qt.io). `MacSystemColors.mm` and the
  objc calls should be unaffected, but this needs checking.
- The vendored libusb; run the core tests.
- An app bundle: `MACOSX_BUNDLE`, `Info.plist` with
  `NSHighResolutionCapable` and the version, an `.icns` made from the SVG,
  and `macdeployqt`.
- Menu bar and ⌘ labels.
- A universal binary (`lipo` of the aarch64 and x86_64 cores) or two builds.
- Smoke runs with screenshots; the terminal's `cargo test` and `--fixture`
  run.

**Estimate.**
- About **1.5–2.5 agent-days** for desktop and terminal together.
- Plus about 0.5 day for a hardware check, if a DSPi can be passed through
  to the VM.
- Signing and notarisation need an Apple developer ID; that is the owner's
  call.

## 9. macOS phase 2 (done, 2026-11-06, on the old core)

The work ran in the Intel macOS 26.7.2 VM (`ssh macvm`), one heavy job at a
time. The code moved between the host and the VM as git bundles.

**What I installed in the VM** (all in the user's home, nothing system-wide):
- rustup from rustup.rs in `~/.cargo`, `~/.rustup`: stable 1.99.1, plus
  the `aarch64-apple-darwin` target.
- A Python venv `~/qtport/venv` (the system Python 3.9) with `aqtinstall`
  3.3.0, `cmake` 4.4.4 and `ninja` 1.13, all from PyPI.
- Qt 6.11.2 `clang_64` (universal) in `~/qtport/Qt` (1.4 GB), installed
  with aqtinstall from download.qt.io.
- Xcode 26.6 was already there. Homebrew is not used.

**Results.**

| What | Result |
|---|---|
| `cargo test --workspace --all-features` on macOS (x86_64) | 1378 passed, 0 failed |
| Desktop app, Qt 6.11.2 | Builds with no errors and no QML warnings except the `Qt.labs.settings` deprecation |
| Universal app | `DSPi Console.app` is x86_64 + arm64 (`lipo`). The .dmg is 71 MB; the bundle 161 MB unpacked. The arm64 half has not been run: the VM is Intel. |
| Run in the GUI session, DSPi attached | It connected to the DSPi (2AC1E7B6) over the VM's USB pass-through and read the platform, status and the full parameter set (chunked 2048-byte reads, which the pass-through allows). It showed preset 1, user volume −23 dB, the K340 crossovers and CPU 8 %, plus the "newer firmware" banner (the app targets 1.1.6 beta 4, the device runs 1.1.6 final). |
| Menu bar | Opened by QMP clicks: the application menu (Preferences… ⌘, / Hide / Quit ⌘Q) and Tools (⇧⌘M, ⇧⌘A, …) |
| Terminal client | `dspi list`, `dspi doctor` ("interface claimed, RP2350 firmware 1.1.6, wire V32, 17 channels"), `dspi ext list` (0x0001 crossfeed pair map, 0x0002 core voltage), `dspi ext vreg` (live 1.20 V, hang_count 0), `dspi ext xfeed-map` (`02 13 00 00`: pairs 2:0, 3:1, the K340 map) |
| TUI | `--fixture` in a pty: draws and quits on `q` |
| Universal terminal binary | `lipo` of the aarch64 and x86_64 builds |

**No writes to the device.** The desktop app's connect path only reads.
I checked the core: its fetches are GETs, and the analyser's one write
happens only when the analyser window is open, which it wasn't. The
terminal ran read commands only.

**Screenshots** are in the host scratch directory under
`qtport/mac/`:
- `bundle-final.png` (the main window, connected);
- `menu-tools.png` and `menu-app.png` (the menus);
- `shot1.png` (the first, unbundled run).

**Artifacts** are in `qtport/mac/` on the host and `~/qtport/dist/` in
the VM: `DSPi-Console-1.1.6-beta4-macos.dmg` and the universal `dspi`.

**Fixed on the way.**
- **The Qt 5 build had been broken since the Qt 6 commit.** Qt 5.15
  rejects `import … auto` in `qml/style/qmldir`, so there was no ToolTip
  type and no main window. The unversioned import fixes it on both Qt
  versions; Linux was rechecked on 5.15 and 6.11.
- **`macdeployqt` errors.** It copies SQL drivers that link client
  libraries a Mac doesn't have; the build script deletes those three. Its
  "ERROR: no file at …" lines for them are expected.

**Left for macOS.**
- Signing with a Developer ID and notarisation: needs the owner's Apple
  account.
- A run on Apple silicon.
- Qt names the settings item "Preferences…"; macOS 13 and later call it
  "Settings…".
- No Mac-specific USB permission step is needed (no driver, no
  entitlement for a non-sandboxed app). A sandboxed or App Store build
  would need the USB entitlement.

## 10. Windows 11 run (real Windows, 2026-11-06, on the old core)

Tested in a Windows 11 Pro VM (build 26300) with no GPU: Qt renders on
WARP, which is Direct3D 11 in software. The screen is 1280 × 800 at 100 %.
No DSPi was attached, so only the no-device paths were tested. The
artifacts were rebuilt from `main` in the containers.

**What worked.**
- **Installer:** the NSIS installer installed per user (no UAC prompt)
  and upgraded over an existing install. It was not downloaded, so there
  was no SmartScreen prompt; an unsigned download will get one.
- **Startup:** the first-launch wizard leads to the main window ("No
  Devices"), and settings go to `%APPDATA%\DSPi\DSPi Console.ini`.
- **Window chrome and menus:**
  - Windows-style caption buttons; maximise and restore work.
  - The app menu and its submenus work.
  - The native Windows file dialog opens (AutoEQ > Update Database >
    Import File).
  - Ctrl+, and Ctrl+Shift+A work.
  - Settings and the Spectrum Analyser render correctly under WARP.
- **Uninstall:** removes the program folder, the Start menu entry and
  the Apps entry, and keeps the settings.
- **The zip** runs from any folder.
- **Performance under WARP:**
  - About 1 s to the first frame once the QML cache exists; 6–9 s on the
    first launch after an install (QML compiled at run time, plus
    Defender scanning 580 new files).
  - Idle: about 1.3 % of one core, 136 MB working set.
- **Terminal:**
  - `dspi --version`, `list` ("No DSPi found.", exit 2), `doctor`
    ("USB subsystem reachable", then no device) and `ext` (no device),
    for both the MinGW and the MSVC build.
  - The `--fixture` TUI works in Windows Terminal (truecolour, braille)
    and in conhost (16 colours, blocks) and quits on `q`.

**Fixed after the run.**
- **The window could sit under the taskbar.** A frameless window opened at
  the screen's corner at its full 950 × 813, so on a 1280 × 800 screen
  Skip Setup and Continue were under the taskbar. It is now kept inside
  the work area and centred (Windows only).
- **"Run DSPi Console" on the installer's last page** goes through the
  shell (`ExecShell`). The installer now also has the app's own side
  artwork instead of NSIS's default.
- **About said "for Linux"**; it now names the platform.
- **The MSVC `dspi.exe` didn't start on a clean Windows**
  (STATUS_DLL_NOT_FOUND, VCRUNTIME140.dll). MSVC builds now link the C
  runtime statically (`.cargo/config.toml`). The MinGW builds were
  unaffected.

**Not checked here.**
- Windows 11's rounded corners: Windows turns them off without GPU
  acceleration.
- Real HiDPI: the display scaling setting was left alone; Qt 6 is per-
  monitor DPI aware.
- Everything that needs a DSPi (see 4, "USB on Windows").

**Follow-ups.**
- Precompile the QML (Qt 6 `qt_add_qml_module` or `qmlcachegen`) to cut
  the first launch.
- An installer check that the app isn't running during an upgrade.
- A drop shadow or border for the frameless tool windows.
- Signing.

## 11. Parity after the merge

The full survey is [`docs/PARITY.md`](PARITY.md), moved here from
`dspi-mobile/docs/FEATURE-PARITY.md` by client plan WP XC0
(`docs/core-merge/PLAN-CLIENTS.md`). Its matrix was written before the merge, by
reading code; its section 5 is XC0's floor audit, on this tree. In the matrix, the
macOS, iOS, Android and Windows C# columns still stand: **none of those apps is on
this core yet.** Its UI-only rows for Qt and the Terminal also stand (accessibility,
localisation, tray, RTA overlay in the TUI and so on).

This table is what the merge changed. Row numbers are the survey's. "Core" means
`dspi-core` or, for the Terminal, the session crates. A check here means the code
exists and is tested against simulated devices; no row has run on hardware on this
tree.

| # | Feature | Core | C ABI | Qt UI | Terminal |
|---|---|---|---|---|---|
| A1 | USB vendor | nusb | yes | yes | yes |
| A2 | USB MIDI | `open_midi`, midir feature | **no** | no | `--midi` |
| A3 | Link client | `link_*`; no mDNS | **no** (CG1) | no (QD9) | `--link`; no mDNS |
| A7 | Demo device | `open_demo`, stateful | `dspi_open_demo` | `--demo` | `--demo` |
| A8 | Reconnect after replug | `poll_hotplug` | yes | yes | now automatic |
| A9 | Wire V32 constants | generated from `bofr-main` | yes | yes | yes |
| A10 | Older and newer firmware | read-only mirror (V16..V31, newer prefix) | `compat` | banner; the bridge still gates its calls on `COMPAT_OK`, and the read-only mode has not been brought to the UI | still refused (strict `probe`) |
| A11 | `EXT_LIST` probe on connect | yes | `dspi_ext_status` | not called | yes, on the overview line |
| A12 | Live sync, echoes | echo rule per link, two bulk modes | yes | through the C ABI | yes, with attribution |
| B2, B3 | Source names, device name | yes | yes | **not called** | yes |
| B14 | Crossfeed pair map | yes | yes | **not called** | CLI |
| B22 | Core voltage | yes | yes | **not called** | CLI (full), label on the overview |
| B23, C2 | Output latency, clock diagnostics | yes | yes | **not called** | yes |
| C1 | RTA | engine, worker | yes | code in place, not run on this core | panel |
| C4 | Signal generator | yes | yes | code in place, not run on this core | yes |
| E6 | Control surfaces | editing model in session | JSON | code in place, not run on this core | yes |
| F1 | Firmware update | `uf2_inspect`, `flash_to_drive` | yes | **not called**: its own updater, bundled beta4 images only | finds the drive; copy by hand |

"Not called" means the Qt sources do not reference the C function: the core side is
done and the UI is client plan work (track QD). The survey's section 4 expected
the move to the Terminal crates to close those Qt rows; it closed the core side, and
the rows stay open until the UI exists.
