# Making the DSPi consoles portable: Linux, Windows, then macOS

*2026-11-06.* This covers Weeb Labs' Linux DSPi Console (Qt) and
DSPi-Terminal, ported so that one code base builds natively on Linux and
Windows, with macOS as the next phase. It also covers how the two came
together in this repository.

Ground rules for the work:
- every build and test ran in podman containers;
- nothing was pushed;
- no USB device, phone or macOS VM was touched.

All Windows checks ran under Wine, so the USB path on real Windows (WinUSB)
is **not verified on hardware**.

## Summary

- **Desktop app.** The app is Qt and QML, built with CMake. It was Qt 5.15
  only; it now builds against **Qt 6 (preferred) or Qt 5.15**, which took
  small, mechanical changes. The Rust core `dspi-core` uses rusb/libusb and
  a cbindgen C ABI.
- **Windows x86_64.** The app is cross-compiled from Linux with Fedora's
  MinGW Qt 6. The core is built for `x86_64-pc-windows-gnu` and ships as
  `dspi_core.dll`. Under Wine 11:
  - the app starts;
  - it shows the first-launch wizard;
  - after "Skip Setup" it shows the no-device state ("No Devices").

  The deliverables are a runnable folder, a zip and a per-user NSIS
  installer. The installer was also tested under Wine.
- **MSVC.** The core and the terminal client build for
  `x86_64-pc-windows-msvc` from Linux with cargo-xwin, and their tests pass
  under Wine. A full MSVC Qt build needs Qt's MSVC binaries, which in
  practice means a Windows runner (see "MSVC Qt builds" below).
- **DSPi-Terminal.**
  - It builds and runs on Linux, Windows GNU and Windows MSVC; all 1297
    tests pass on Linux and under Wine.
  - Its protocol headers are refreshed to Bofr5555/DSPi `bofr-main`
    `af001ec` (v1.1.6 final plus the extension channel).
  - It gained `dspi ext list | xfeed-map | vreg`.
- **One repository.** Everything now lives in this monorepo, with history,
  in one Cargo workspace. On Linux, 1378 tests pass.

## 1. What the desktop app is built with

| | |
|---|---|
| GUI | Qt Quick / QML (about 150 QML files) plus 17 C++ files: a bridge, custom QQuickItems for plots, and controllers. `QApplication` with Fusion as the fallback style, plus a custom `qml/style` (ToolTip). |
| Qt | was 5.15 only; now 6.x (tested 6.11.2) or 5.15 (tested 5.15.18) |
| Build | CMake (AUTOMOC/AUTORCC); resources in `qt/resources.qrc` |
| Core | Rust crate `dspi-core` (`crates/dspi-core-linux/`): `staticlib` and `cdylib`, 150 `extern "C"` functions, header from cbindgen |
| USB | rusb 0.9 over libusb. Linux uses the system libusb; Windows and macOS now build libusb from source into the core (rusb `vendored`). |
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
| Icon and version info | `packaging/windows/dspi-console.rc.in`: the `.ico` is generated from `qt/dspi-console.svg`, and version fields come from `setApplicationVersion` |
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

### For macOS (phase 2)

- Global menu bar (`Platform.MenuBar`, or a native `QMenuBar` with the app
  menu's items).
- ⌘ labels in `AppMenu` and `ActionMenu`. Qt already maps `Ctrl+` to Cmd.
- An app bundle (`MACOSX_BUNDLE`, `Info.plist`, `.icns`), `macdeployqt`,
  codesign and notarisation.
- Settings stay in the standard plist.

## 4. Build results (2026-11-06)

Paths below are in the scratch directory
`/tmp/claude-1000/-home-bofr-prog-dspi-k340-amplifier/8d838519-065f-490c-bea1-470a339f3928/scratchpad/qtport/`
(`Q`) unless absolute.

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
  works in a Linux container. The Microsoft CRT and SDK come from
  Microsoft under the licence the owner accepted.
- **The Qt app with MSVC, without Windows,** is possible in principle but
  was not attempted. It would need:
  - Qt's prebuilt `win64_msvc2022_64` archives from download.qt.io;
  - clang-cl and lld-link against the xwin SDK;
  - a Linux host Qt of the same version for moc, rcc and qmlcachegen
    (`QT_HOST_PATH`).

  That is fragile and unsupported by Qt.
- **On a hosted `windows-2022` runner** it is routine:
  `install-qt-action` (Qt's official MSVC build), `cargo build -p
  dspi-core`, CMake with Ninja under the MSVC environment, `windeployqt`,
  and optionally `signtool`. See `ci/github-actions-build.yml`. The CMake
  file already links `dspi_core.dll.lib` when `MSVC` is set.
- **Not compiled with MSVC yet, so expect small fixes:** the desktop C++
  sources have only been compiled by GCC (MinGW and Linux). The code was
  kept to standard C++17 and Qt, with the POSIX calls gated.

### USB on Windows

- The desktop core uses libusb's WinUSB backend (vendored libusb 1.0.2x).
  It does no driver installation and assumes no Zadig.
- The firmware's MS OS 2.0 descriptors (vendor code 0x01, a function
  subset for interface 2) make Windows bind WinUSB to interface 2 on first
  plug-in. The audio interfaces stay with `usbaudio2.sys`.
- The core claims interface 2. Control transfers now always carry wIndex
  low byte = 2 on Windows. Notifications are read on bulk IN 0x83 on the
  same interface.
- WinUSB is exclusive per interface. While the C#/WinUI console or `dspi`
  has the device open, this app sees it as inaccessible and says so.
- The terminal uses nusb (pure Rust), WinUSB directly, interface 2,
  wIndex 2, and transfers capped at 4 KB, so it sends chunked.
- **Unverified on real Windows hardware** for both apps. The first test
  with a DSPi should:
  1. check that Device Manager shows "DSPi" under Universal Serial Bus
     devices (WinUSB);
  2. connect, read the state, change a value;
  3. check that notifications arrive (turn the volume on the PC);
  4. run a firmware update.

## 5. DSPi-Terminal

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
  extension channel). Wire format V32, 247 opcodes, CS caps v22.
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
  - `core/` → `crates/dspi-core-linux/`;
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
- The core keeps its package name `dspi-core`; the directory says
  `-linux` to tell it from the Terminal stack.
- The C header is generated into `crates/dspi-core-linux/include/`.

**Two protocol stacks for now.**
- The desktop core's constants are still hand-transcribed at 1.1.6 beta 4.
  For example, it keeps the 0x54–0x57 channel gain/mute constants, which
  nothing calls.
- The Terminal stack is generated from `bofr-main`.
- The intended end state, per `RUST-CORE-CONSOLIDATION.md`:
  1. move the desktop's C ABI onto `dspi-proto` / `dspi-session` (S9 there);
  2. then delete `crates/dspi-core-linux`.

## 7. Remaining gaps and open questions

1. **Licences.**
   - DSPi-Console-Linux has no licence file upstream.
   - DSPi-Terminal states GPL-3.0-or-later but has no LICENSE file.
   - dspi-link's terms need to be confirmed (its old workspace said
     `UNLICENSED`).

   These need settling before the repository is published.
2. **Hardware.** No Windows (or Linux) USB run of either app yet. See the
   checklist in 4.
3. **MSVC Qt build:** only on a Windows runner (or a fragile cross setup).
4. **Linux release packaging** still targets Qt 5 (Ubuntu 22.04
   AppImage/.deb, Fedora 42 .rpm). Moving it to Qt 6 means a newer base
   (Ubuntu 24.04), which raises the glibc floor.
5. **AUR `PKGBUILD`** builds from the upstream DSPi-Console-Linux
   repository's layout. It needs a new source URL and paths if the
   monorepo becomes the source.
6. The desktop core's protocol constants are not generated from
   `bofr-main` (see 6).
7. Windows: shadow, Snap Layouts, signing (see 3).
8. `Qt.labs.settings` deprecation on Qt 6.11 (runtime note only).

## 8. macOS effort (phase 2, in the macOS VM)

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
