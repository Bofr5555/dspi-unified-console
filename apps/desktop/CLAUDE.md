# Working on the desktop app

Read `README.md` in this directory first: layout, threading, how a feature
plugs in.

- Toolchains only in containers: `ci/run.sh '<cmd>'`. Never cmake, qmake,
  lupdate or cargo on the host. Build directories under `/work` (the
  container's view of `~/.cache/dspi-build/work/<branch>`), never in the tree
  and never on tmpfs.
- Checks: `ci/check.sh --step qt` (build and .ts), `--step qt-offscreen` (the
  smoke driver; `DSPI_SMOKE_SECTION=<name>` runs one section), `--step qt-ts`.
- The screens follow the Mac console (`apps/apple/DSPi Console/*.swift`): its
  layout and its words. Shared wording comes from the core by key
  (`dspi_link_phase_text`, the onboarding JSON, `dspi_output_labels`,
  `dspi_about_json`), never rewritten in the app.
- Every user-visible string through `qsTr` (QML) or `tr` (C++). After changing
  strings, regenerate `translations/dspi-uconsole_en.ts` in the container:
  `ci/run.sh 'cd apps/desktop/qt && lupdate-qt6 -silent -locations none -no-obsolete -source-language en -target-language en qml src -ts translations/dspi-uconsole_en.ts'`.
- No colour literals in screens: `Theme` and `ChannelPalette` tokens only.
  Icons by Lucide name through `Icon`; a new one goes into
  `icons/lucide/manifest.txt` and `icons/fetch-lucide.sh` is run again.
- Custom controls carry `Accessible.role` and `Accessible.name`.
- Nothing in QML or in a controller's GUI-thread code calls a blocking `dspi_*`
  function: post it to `Core::worker()`. Writes go through `DeviceModel::write()`.
- Object names for the smoke driver: `smoke/NAMES.md`.
- Shared files (`CMakeLists.txt` lists, `qml/main/MenuTable.qml`) take
  additive edits under a `# -- WP <ID> --` / `// -- WP <ID> --` marker.
