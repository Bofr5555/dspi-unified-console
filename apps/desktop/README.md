# The desktop app (Windows and Linux)

The Qt app for Windows and Linux, built on the project's Rust core through its
C ABI (`crates/dspi-capi`, header `include/dspi_core.h`). Its screens follow
the Mac console (`apps/apple`): the same layout, the same words, the same
behaviour, with the Windows and Linux additions listed in `docs/PARITY.md`.

- Qt 6.8 or newer, Qt Quick with the Basic style and the app's own components.
  No Qt 5.
- The system's window frame with the menu bar inside the window.
- Light and dark follow the system; dark is the look when the system says
  nothing.
- The name people see is in one place: `src/AppIdentity.h`.
- Settings are kept under the organisation `DSPi` and the application
  `DSPi Console`, as the previous desktop app kept them.

## Building

Toolchains run in the project's containers (`ci/run.sh`), never on the host:

    ci/check.sh --step qt             # the app, the .ts check
    ci/check.sh --step qt-offscreen   # the app with its smoke driver, on the demo device
    ci/check.sh --step qt-ts          # only the .ts check

By hand, in the container:

    cargo build --release -p dspi-capi --features link,midir,mdns,hub
    cmake -S apps/desktop/qt -B /work/build -G Ninja -DDSPI_CARGO_TARGET_DIR=/target [-DDSPI_SMOKE=ON]
    cmake --build /work/build

`DSPiConsole --demo` opens the built-in demo device.

## Layout

    qt/CMakeLists.txt        one QML module, UConsole: every QML file, C++ type and icon
    qt/src/main.cpp          the app's start and end
    qt/src/AppIdentity.h     the display name, the settings names, the project's links
    qt/src/core/             the core wrapper: Core, CoreWorker, DeviceModel, the list models
    qt/src/controllers/      Registrar, Shell; one controller per feature
    qt/src/platform/         the icon provider, the token store, tray and system integration
    qt/qml/Theme/            Theme (every colour, size and font) and ChannelPalette
    qt/qml/components/       the shared controls: slider, field, pill, card, popover,
                             banner, icon, menus, tooltip, tool window
    qt/qml/main/             the main window, the menu table, the banner host, the
                             sidebar, the right pane, the device button
    qt/icons/lucide/         the Lucide icons the app ships (manifest.txt, fetch-lucide.sh)
    qt/icons/own/            icons drawn for the app, in Lucide's style
    qt/smoke/                the offscreen smoke driver (NAMES.md: object names)
    qt/translations/         dspi-uconsole_en.ts, generated (English only)
    packaging/               installers and packages per platform

## How the app talks to the core

- **One thread calls the core:** `CoreWorker`. Every blocking call (reads,
  writes, scans, hot-plug polls, files) is posted to it and its result comes
  back to the GUI thread as a queued call. Nothing in QML calls the core.
- **The state** is read on the worker as a whole snapshot (`DeviceSnapshot`:
  a copy of the core's state record, the device's name, the output labels) and
  applied by `DeviceModel` on the GUI thread. QML binds to `DeviceModel`'s
  properties and list models (`channels`, `presets`, `BandListModel`).
- **Notifications:** the core's listener thread only schedules; the worker
  processes the batch (`dspi_process_notifications`) and reads a snapshot when
  something changed.
- **Writes** go one way: a controller validates and calls
  `DeviceModel::write()` with the setter; the setter runs on the worker, a
  fresh snapshot follows, and a refusal emits `writeFailed`.

## Adding a feature

A work package adds its files without editing the shell:

- a **controller** with `Registrar::Controller<T>` (constructor
  `T(const AppContext &, QObject *)`), seen by QML as a singleton of module
  `UConsole`;
- a **banner** with `Registrar::Banner("<QML type>", order)`: a `Banner` that
  decides its own `visible`;
- a **tool window** with `Registrar::Window("<action id>", "<QML type>")`: a
  `ToolWindow`, opened by its menu row;
- a **menu row's action** with `ctx.shell->addHandler("<action id>", this, ...)`;
  every row of the Mac's menus is in `qml/main/MenuTable.qml` already and does
  nothing until something handles its id;
- a **smoke section** with `SmokeDriver::Section` in its own file under
  `qt/smoke/`.

New files go into the lists in `CMakeLists.txt` under the work package's
marker. `src/controllers/Registrar.h` has the details.

## Parked files

Kept from the previous desktop app and not built yet; the work package named
adapts each to `Core` and `DeviceModel`, or replaces it:

| File | Work package |
|---|---|
| `src/controllers/LinkController.h` | QP13 |
| `src/platform/SystemIntegration.h`, `SystemIntegration.cpp` | QP14 |
| `src/platform/FirewallController.cpp` | QP14 |
| `src/platform/HostAudioFormatWindows.cpp` | QP18 |
| `smoke/LinkFakeHub.h`, `smoke/SmokeDriverLink*.cpp`, `SmokeDriverDevices.cpp`, `SmokeDriverRtaFollow.cpp` | QP13, QP9 |
| `smoke/SmokeDriverNetworking.cpp`, `SmokeDriverTray.cpp`, `SmokeDriverSystem.cpp` | QP14 |
| the other `smoke/SmokeDriver*.cpp` section files | the work package of their screens; QP19 for the rest |

`src/platform/TokenStore.*` is built (it needs nothing of the app) and wired by QP13.
