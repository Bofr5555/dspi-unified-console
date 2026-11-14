# Changelog

Release notes of earlier versions: `CHANGELOG-0.13.0.md`.

## Unreleased

### Names
- The project's apps are called **uConsole for DSPi**. Release files start with
  `uConsole-for-DSPi-<version>-`; packages, installed files and the desktop app's command use
  `uconsole-for-dspi`. The Google Play app keeps its listing name until its next update.
- Settings, the Windows install folder and registry entries, and the Linux desktop entry use
  the new names. Nothing is carried over from the earlier names.

### Desktop app (Windows and Linux)
- The Qt app was rebuilt in QML from the macOS console's layout and wording, on the shared
  core, and replaces the earlier port. It needs Qt 6.8 or newer; the Qt 5 build and the Qt
  `.dmg` are gone (the Swift app is the macOS app).
- New compared with the earlier port: read-only mode for firmware this version cannot
  write; the crossfeed pair map; device name, source names and output latency in Settings;
  clock diagnostics and output ring health in Stats; separate C0 and C1 CPU bars; the
  bundled 1.1.6 firmware, with a custom `.uf2`; the demo device; a Core Voltage page under
  Settings > Advanced (on firmware with the extension channel); Edit > Undo, Redo and
  History; Help > Save Diagnostics; an update check, off until switched on (Settings >
  Updates), in the Windows files, the AppImage and the tar.gz only.
- On Windows the settings are an INI file in `%APPDATA%\uconsole-for-dspi\`; the
  uninstaller removes it and the app's firewall rule.
- Packages: Windows installer and zip; AppImage, a tar.gz with an installer, `.deb`
  (Debian 13), `.rpm` (Fedora 43 and 44) and an AUR `PKGBUILD`. None are code-signed.

### All clients (shared core)
- One change history in the core: every write is recorded, and undo and redo are
  available to every client. Undo skips a value another console has since changed and
  says which.
- Meters share one peak hold, clip hold and fall rate, computed in the core.
- The core keeps a per-device event history (connects, clips, resets) in the client's data
  folder.
- Parameter names, units and help lines follow one glossary (`docs/GLOSSARY.md`); levels
  against full scale read in dBFS. Five upmixer ranges now match what the firmware accepts.
- Loading a preset with out-of-range values clamps them and reports each one.
- A diagnostics bundle (one zip: versions, the device's state, stats, events, the OS and
  USB view; Link tokens removed) can be saved from the desktop app and with
  `uconsole diag`.
- Each release's `SHA256SUMS` is signed with minisign; the release notes give the public
  key and how to check a download.

### Terminal
- The command is now `uconsole`. `dspi` and `dspi-term` are no longer built or installed,
  and nothing is kept for the old name: scripts, shell completions and service files that
  call `dspi` need the new name. Regenerate completions with `uconsole completions <shell>`.
- Its files moved with it: settings and Link pairing tokens to `~/.config/uconsole/` (the
  platform's configuration directory), the AutoEQ database to the same directory, the
  hub's store to `~/.local/share/uconsole/hub` (the platform's local data directory), and
  a web console installed beside it to `share/uconsole/web/`. Nothing is moved over from
  the old directories.
- The hub's service files are named after the command (`uconsole-hub.service`,
  `se.bofr.uconsole.hub.plist`, `uconsole-hub-task.xml`).
- `uconsole --third-party-licences` prints the licence texts of the Rust crates compiled
  into it; the release archives carry them as `THIRD-PARTY-NOTICES.txt`.

### All clients (draft notices)
- About no longer shows the first-person donation paragraph or "Made with love by Weeb
  Labs"; the credit line reads "DSPi and its firmware by Weeb Labs", and the links section
  says where to support Weeb Labs' work.
- The licence notice says that the program as a whole is distributed under the GNU GPL
  version 3, with Bofr5555's own contributions under version 3 or later and the section 7
  terms; it names the source archive published with each release instead of a repository
  link, and credits the firmware's third-party parts, Noto Sans Mono and AutoEq.

### Integrators (behaviour change of the C ABI and the bindings)
<!-- -- WP P1TZ -- -->
- Confirmation tokens are always required (OD-P1f). Every dangerous operation (preset
  save, copy and delete, parameter save, factory reset, output configuration save,
  bootloader, configuration import, stored core voltage, firmware install) is refused
  with `confirmation_required`, and nothing is sent, unless its `_confirmed` call is given
  a token minted with `confirm_token` (C: `dspi_confirm_token`) after the person
  confirmed. The plain calls (`dspi_save_preset`, `dspi_config_import`, `save_preset`,
  `write_param("preset.save")`, ...) are always refused so.
- The switch is gone: C `dspi_require_confirmation` and `dspi_confirmation_required`,
  UniFFI `require_confirmation` and `confirmation_required`, and the web Worker's
  `requireConfirmation` and `confirmationRequired` are removed. To migrate, mint a token
  per operation and call the `_confirmed` form: [docs/integrator/README.md](docs/integrator/README.md#confirmation-tokens).
  Every client in this project already does.
