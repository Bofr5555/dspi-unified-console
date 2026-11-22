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
- Settings are kept under the organisation and application
  `uconsole-for-dspi` (`AppIdentity.h`).
- `--version` and `--help` answer on the terminal with no display (they are read before the window
  system starts), so they work over SSH and in a package test.
- With the alink sender open (the device that feeds the audio link, which has no DSP), one banner says so and
  every DSP menu row, shortcut and tool window is greyed, never hidden; device info, the audio link block and
  the firmware update stay.

Installing a release: [`docs/INSTALL.md`](../../docs/INSTALL.md) (Windows, the Linux packages, the AppImage and
the one USB rule every component shares). What the app offers against the other clients, and which features need
the DSPi fork firmware (†): [`docs/FEATURES.md`](../../docs/FEATURES.md).

## Notes for Linux users

**The system volume doesn't reach the DSPi.** With PipeWire this needs the
DSPi card profile from the firmware repository
([`tools/linux-pipewire-card-profile`](https://github.com/WeebLabs/DSPi/tree/main/tools/linux-pipewire-card-profile)):
run its `install-dspi-profile.sh`, then unplug and reconnect the DSPi.
Without it PipeWire may show the DSPi as two outputs, or pick the IEC958
profile with software volume, and the DSPi never sees a change. The profile is
for PipeWire only; the classic PulseAudio daemon does not read it.

**No blurred sidebar, on KDE or elsewhere.** The old Qt app drew a translucent,
blurred sidebar on KDE Plasma (KWin's blur effect, and on Wayland also
`kwayland-integration`). This app draws its sidebar solid on every desktop and
uses no KDE libraries, so nothing needs installing for it, and it looks the
same on X11 and Wayland.

## Building

Toolchains run in the project's containers (`ci/run.sh`), never on the host:

    ci/check.sh --step qt             # the app, the .ts check
    ci/check.sh --step qt-offscreen   # the app with its smoke driver, on the demo device
    ci/check.sh --step qt-ts          # only the .ts check

By hand, in the container:

    cargo build --release -p dspi-capi --features link,midir,mdns,hub
    cmake -S apps/desktop/qt -B /work/build -G Ninja -DDSPI_CARGO_TARGET_DIR=/target [-DDSPI_SMOKE=ON]
    cmake --build /work/build

`uconsole-for-dspi --demo` opens the built-in demo device.

## Layout

    qt/CMakeLists.txt        one QML module, UConsole: every QML file, C++ type and icon
    qt/cmake/wp/             each work package's own file list (<wp>.cmake)
    qt/src/main.cpp          the app's start and end
    qt/src/AppIdentity.h     the display name, the settings names, the project's links
    qt/src/core/             the core wrapper: DeviceRegistry, DeviceSession, SessionModel (per device);
                             Core, DeviceModel, Poller (the active one); CoreWorker, the list models
    qt/src/controllers/      Registrar, Shell; one controller per feature
    qt/src/platform/         the icon provider, the token store, tray and system integration,
                             the update checker (AppUpdater), the bootloader locator,
                             device access (DeviceAccess: the udev rule), the measurement
                             microphone (AudioCapture, capture only) and the P5SPIKE switches
                             (MeasureSpike: --audio-inputs, --measure-spike; WP P5QCAP)
    qt/src/render/           the drawn items: BodePlot (the graph), PeqOverlay, Meter, PinGrid,
                             RtaSurface and RtaBars (the spectrum), TubeIllustration
    qt/qml/Theme/            Theme (every colour, size and font) and ChannelPalette
    qt/qml/components/       the shared controls (below)
    qt/qml/main/             the main window, the menu table, the banner host, the
                             sidebar, the right pane, the device button
    qt/icons/lucide/         the Lucide icons the app ships (manifest.txt, fetch-lucide.sh)
    qt/icons/own/            icons drawn for the app, in Lucide's style
    qt/smoke/                the offscreen smoke driver (NAMES.md: the naming rule;
                             names/<wp>.md: each package's names)
    qt/tools/contrast.py     the theme's WCAG contrast check (ci/check.sh --step qt)
    qt/translations/         uconsole-for-dspi_en.ts, generated (English only)
    packaging/               installers and packages per platform

## How the app talks to the core

- **One core per open device** (WP MDQT1): `DeviceRegistry` over the core's
  `CoreSet`, a `DeviceSession` each (its core, `CoreWorker`, `SessionModel` and
  stash), one of them active. `Core`, `DeviceModel`, `Poller` and
  `LinkController` are facades over the active one: ask them at each call, never
  keep a handle or a worker, and read again on `activeChanged`. Classic replaces
  the active device on its own core; `DeviceRegistry.keepOpen` opens beside it.
  The main session's core is there with no device too.
  A controller's per-device state is a `sessionState<State>()` (SessionState.h,
  in the session's stash; WP MDQT2a): what a job brings back goes into the State
  it was asked with. On `activeLeaving` what is half-edited is finished or let go
  of, on the device it was made on; a `QuestionDialog` about the device is
  `ofDevice`.
  A job goes on on the device it was started on (WP MDQT2c): Measure and Tune are
  facades over a `MeasureSession` and a `TuneSession` per device, with a
  measurement thread each. What has no end of its own (the signal generator, the
  live view, solo) stops on `activeLeaving`; what plays on a device that is closed
  or replaced stops on `sessionStopping`, posted to its worker (`drainFirst`).
  `DeviceSession::setJob` makes its tab busy.
  Views that span devices use the registry directly (WP XDEV1): Several units
  judges a unit open in a tab by that tab's core (no second session) and any
  other by an extra core of the set (`DeviceRegistry::openExtra`: no tab, its
  serial held while it is chosen); the System view's "Open in tab" shows a
  unit's tab or opens one through the hub (`LinkController::openInTab`). A write
  refused on a device that is not the active one is a notice naming it
  (`backgroundRefused`). Leaving Modern keeps the other devices open:
  `DeviceRegistry::kOthersStayOpenInClassic` is that one policy point.
  `sessionClosing` is never emitted for the main session.
- **One thread calls a core:** its `CoreWorker`. Every blocking call (reads,
  writes, scans, files) is posted to it and its result comes back to the GUI
  thread as a queued call. The hot-plug poll (one for every core) and the opens
  run on the registry's own threads. Nothing in QML calls the core.
- **The state** is read on the worker as a whole snapshot (`DeviceSnapshot`:
  a copy of the core's state record, the device's name, the output labels) and
  applied by `DeviceModel` on the GUI thread. QML binds to `DeviceModel`'s
  properties and list models (`channels`, `presets`, `BandListModel`).
- **Notifications:** the core's listener thread only schedules; the worker
  processes the batch (`dspi_process_notifications`) and reads a snapshot when
  something changed.
- **Writes** go one way: a controller validates and calls
  `DeviceModel::write(what, setter, done)`; the setter runs on the worker, a
  fresh snapshot follows, and a refusal emits `writeFailed`. `what` is also the
  coalescing key: a write whose key matches one still queued replaces it
  (latest wins), so name a write by its target (`"output gain 3"`). A slider
  dragged over a slow link then sends its newest value, not a backlog.
- **Periodic reads** go through `Poller` (`AppContext::poller`), never a timer of
  a controller's own. The base runs two polls: the meters every 60 ms
  (`Poller.peaks`, `clipFlags`) and the status every second (`cpu0`, `cpu1`,
  `activeInputs`). A package adds its own:

      m_poll = ctx.poller->add(QStringLiteral("subharmonic meter"), 100, this,
          [](FfiCore *c) { float v = 0; return dspi_fetch_subharm_meter(c, &v) ? v : NAN; },  // worker
          [this](float v) { setMeter(v); });                                                 // GUI
      ctx.poller->setEnabled(m_poll, windowShown);

  A tick is skipped while the poll's last read is in flight; nothing polls
  while no device is open; over a slow transport (`Poller.slowTransport`, set
  by the Link client) every poll runs at 2 Hz at most. Polls are never paused
  for anything else.

## Components

`qml/components/`, each with `Accessible.role` and `Accessible.name` and themed
for both schemes; screens use these, not the Basic style's controls:

| Component | For |
|---|---|
| `AppButton` | push buttons; `primary` (accent), `destructive`, `iconName`, `toolTip` |
| `IconButton` | borderless icon buttons; `secondaryClicked` (right-click) |
| `AppSwitch`, `AppCheckBox` | on/off settings; the check box also partly checked |
| `AppComboBox` | popup choices (presets, sources, settings pickers); `label` |
| `AppSpinBox` | a number with steppers in real units: `realValue`, `realFrom/To/Step`, `decimals`, `unit`, `committed` |
| `CustomSlider` | sliders: `valueMoved` live (≤ 30/s), `committed` on release; right-click resets |
| `BorderedField` | text fields; `committed`, Escape restores |
| `AppTabBar` + `AppTabButton` | segmented choices (PEQ / XO) |
| `AppScrollBar` | `ScrollBar.vertical: AppScrollBar {}` |
| `SectionHeading` | section titles (`caps`, `rule`) |
| `Card`, `Pill` | grouped controls; channel labels and curve toggles |
| `PopoverPanel` | popovers anchored to a control |
| `QuestionDialog` | questions before a change: `acceptText`, `rejectText`, `alternativeText`, `destructive`, `answered(answer)` |
| `Banner` | the banners under the menu bar |
| `Icon`, `HelpTip` | Lucide icons tinted by the theme; tooltips |
| `AppMenu`, `AppMenuItem`, `AppMenuBar` | menus |
| `ToolWindow` | tool windows: remembered size, the menu table's shortcuts (`TableShortcuts`) |
| `LevelMeter` | a dBFS meter: three bands or one colour, the core's hold line and clip hold, a gain reduction variant (QP18) |
| `NumericField` | a number typed in its unit ("2k", "2.5 kHz", "+=3"), nudged by Up/Down, held to its range with a badge (QP18) |
| `StatusBar` | a window's foot: connection, scene, mode, then the caller's items (QP18) |

A finished action that needs no answer is a notice (`NoticeCenter.post(text)`,
shown in the banner stack), never a modal "Success" alert; a question is a
`QuestionDialog`. Undo and redo are the core journal's (`HistoryController`).

Colours come from `Theme` tokens only. Text tokens reach WCAG AA (4.5:1) on
every surface; `tools/contrast.py` checks it in `ci/check.sh --step qt`.

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
  nothing until something handles its id (one handler per id);
- a **hardware setting held for the save bar** with a `SaveParticipant`
  added with `HardwareController::addSaveParticipant(participant, order)` (QP11); an edit of
  the output configuration made elsewhere calls
  `HardwareController::beginOutputEdit()` before it writes;
- **Qt modules and libraries** through the base's lists in the package's
  cmake file: `list(APPEND DSPI_QT_COMPONENTS Network)`,
  `list(APPEND DSPI_APP_LIBS Qt6::Network)`, and for anything else per target
  a `function(<name> target)` appended to `DSPI_APP_HOOKS`;
- a **smoke section** with `SmokeDriver::Section` in its own file under
  `qt/smoke/`.

`src/controllers/Registrar.h` has the details.

## Shared files: the merge rule

Work packages run in parallel on branches cut from `qtport`. So that their
merges do not conflict, nothing generated or listed is edited by two of them:

- **Files to build:** each package lists its sources, QML files, singletons
  and smoke sections in its own `qt/cmake/wp/<wp>.cmake` (`cmake/wp/README.md`).
  `CMakeLists.txt` includes them all; packages do not edit it.
- **Icons:** each package names the icons it adds in
  `qt/icons/lucide/manifest.d/<wp>.txt` and commits the SVGs
  `icons/fetch-lucide.sh` copies; `manifest.txt` is the base's.
- **The translation file** `qt/translations/uconsole-for-dspi_en.ts` is not
  committed by a package. The coordinator regenerates it on `qtport` after
  each merge (QP18 during its pass), and `ci/check.sh --step qt-ts` checks it
  there. On a package's branch `--step qt` skips the `.ts` check
  (`DSPI_TS_CHECK=1` forces it).
- **Smoke object names:** each package documents its names in
  `qt/smoke/names/<wp>.md`; `NAMES.md` holds only the rule.
- **The menu table** (`qml/main/MenuTable.qml`) has every row already; a
  package fills a row through `Shell` and edits the table only to add a row,
  under its `// -- WP <ID> --` marker.
- **The parked files' table** below is updated by the coordinator when a
  package that adapts a parked file is merged.

## Parked files

None left: QP19 converted or retired the last smoke files (the core-voltage
states the old `SmokeDriverExt.cpp` checked are listed in
`Documentation/behaviour.md` §20 for the core-voltage page's work package).

QP14 built `src/platform/SystemIntegration.*` and `FirewallController.*`, and
its sections are `SmokeNetworking.cpp`, `SmokeTray.cpp`, `SmokeSystem.cpp`;
QP15 removed `SmokeDriverAutoEq.cpp`.

`src/platform/TokenStore.*` is built (it needs nothing of the app) and wired by QP13.
