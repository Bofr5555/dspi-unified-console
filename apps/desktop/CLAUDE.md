# Working on the desktop app

Read `README.md` in this directory first: layout, threading, how a feature
plugs in.

- Toolchains only in containers: `ci/run.sh '<cmd>'`. Never cmake, qmake,
  lupdate or cargo on the host. Build directories under `/work` (the
  container's view of `~/.cache/dspi-build/work/<branch>`), never in the tree
  and never on tmpfs.
- Checks: `ci/check.sh --step qt` (build, contrast; the .ts only on qtport),
  `--step qt-offscreen` (the smoke driver; `DSPI_SMOKE_SECTION=<name>` runs
  one section), `--step qt-ts` (the coordinator's, on qtport).
- The screens follow the Mac console (`apps/apple/DSPi Console/*.swift`): its
  layout and its words. Shared wording comes from the core by key
  (`dspi_link_phase_text`, the onboarding JSON, `dspi_output_labels`,
  `dspi_about_json`), never rewritten in the app.
- Every user-visible string through `qsTr` (QML) or `tr` (C++). A work
  package does not commit `translations/uconsole-for-dspi_en.ts`; the coordinator
  regenerates it on qtport (README.md, "Shared files: the merge rule").
- No colour literals in screens: `Theme` and `ChannelPalette` tokens only, and
  the components in `qml/components` (README.md, "Components") instead of the
  Basic style's controls. Icons by Lucide name through `Icon`; a new one goes
  into `icons/lucide/manifest.d/<wp>.txt` and `icons/fetch-lucide.sh` is run again.
- Custom controls carry `Accessible.role` and `Accessible.name`.
- Nothing in QML or in a controller's GUI-thread code calls a blocking `dspi_*`
  function: post it to `Core::worker()`. Writes go through `DeviceModel::write()`
  (named by target: it coalesces by name); periodic reads through `Poller`.
- Object names for the smoke driver: `smoke/NAMES.md` (rule), `smoke/names/<wp>.md`.
- Files to build: your own `cmake/wp/<wp>.cmake`, never `CMakeLists.txt`.
  `qml/main/MenuTable.qml` takes additive edits under a `// -- WP <ID> --` marker.
