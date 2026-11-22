# Changelog

Release notes of each version: `CHANGELOG-0.14.0.md`, `CHANGELOG-0.13.0.md`.

## Unreleased

### Desktop (Qt)
- Clicking or tabbing into a number in the band list, the output's control strip, an input's preamp or the band bar
  under the graph selects all its text, so typing replaces the value; a second click places the cursor. (WP QTTWEAK)
- The "Filter Response" heading above the graph is gone; the graph takes its space (screen readers still name it).
  (WP QTTWEAK)
- An output's control strip is one row where the window is wide enough: both inputs side by side, the gain and delay
  sliders between their labels and values, mute beside the limiter. Wider still (about 1600 px and up), it sits in the
  output's name row, between the name and Enabled. Narrow windows keep the two-row strip. At 1366x768 the band list
  gains about 75 px. (WP QTTWEAK)
- The main window's minimum height is 660 px (was 780), so it fits a 1366x768 screen with a title bar and a taskbar.
  (WP QTTWEAK)
- Tool windows no longer show a scroll bar for content that would fit (owner report: Headphone Crossfeed).
  Each window that holds a form (Crossfeed, Loudness, Psychoacoustic Bass, Subharmonic, Tube, Leveller,
  Upmixer, Signal Generator, Statistics, System, Measure and every Settings page) takes the height its content
  needs when it opens and when its content changes: never less than its usual size, and no more than the main
  window's size and the screen's, less 48 px on each side. Only content taller than that scrolls. It widens
  only for content wider than it. Crossfeed opens at 780×586 instead of 780×470, for example. The window can
  still be resized by hand, larger or smaller; that size stays while it is open (content that grows may enlarge
  it, nothing shrinks it), and it fits its content again when opened again. Lists of records
  (events, history, snapshots, the AutoEQ list) and the Matrix window keep their own sizes. The Import Filters,
  New from Template and Import Device Configuration dialogs show their whole list when the main window has room
  (they stopped at 320, 320 and 200 px). (WP QTPOP)
- Window > System… has an **Installation** tab on a hub (Advanced) for setting up the hub's installation: it lists the
  DSPi units the hub has seen that are not in it, with **Join** (name, room, role, what feeds it), and edits each
  unit's name, room, role and feed, the senders and fan-outs, rooms, zones and groups. What the hub would refuse is said
  on the field; what Save would change is listed beside it. Only an Admin of the hub can save; everyone else reads. If
  the installation was changed elsewhere meanwhile, Save is withdrawn and Reload offered. (WP INSTQ)
- Your speaker layout: which output drives what (full range, sub, woofer, mid, tweeter). Getting Started has a
  Speakers step and Settings has a Speakers page, both pre-filled from the DSPi's filters and routing; Confirm keeps
  it. The Measure tables start from it, and a role, speaker or group changed there is saved to it without confirming
  it. Measure shows a banner while the layout is not confirmed, also when it came with an imported configuration.
  (WP ROLEQ)
- Choosing another DSPi (on USB, or the demo) first stops what plays on the one that is left: a signal from the Signal
  Generator (faded), a measurement or the live view; an auto-tune not kept yet goes back. Measure no longer takes the
  placement chosen for another device. (WP MDQT2c)
- A second layout, **Modern**: one window with a rail of sections (Overview, Equaliser, Routing, Effects, Measure, Analyse,
  Library, System, Devices, Settings), every tool on a page with tabs, the open device as a tab at the top, the volume,
  meters, Mute and Bypass EQ in a dock along the bottom, and no menu bar (the uConsole button holds the menus, and every
  keyboard shortcut works). States that limit what can be done show in a strip under the top bar; other notices are behind
  the bell. Any tool page can be opened in its own window. Settings › Appearance switches between Classic and Modern at any
  time, and arranges Modern; the device, the selected channel and open tool windows stay. **New installs start in Modern**;
  an install whose Getting Started was finished keeps Classic. An install made with a development build since the layout
  switch came (or whose Getting Started was never finished) was counted as new and now opens in Modern: choose "Switch to
  the Classic Layout" in the uConsole menu to go back. Modern's pages are still being filled in. (WP LAY3)
- A theme setting (Settings › Appearance, and Modern's top bar): the system's, light or dark. (WP LAY3)
- While a measurement runs, Modern cannot be switched to Classic (that closed the Measure page and stopped it without a
  word): Settings › Appearance names the device it runs on. (WP RBFIX)
- A name with markup in it (a device's, a hub's, a room's, a channel's) is shown as it is written. (WP RBFIX)
- Modern opens several devices at once: a DSPi chosen (on USB, through a hub, or the demo) opens in a tab of its own
  beside the others, and each tab keeps its device's edits, pages, history, drafts and running measurement. Closing a
  tab asks about that device's unsaved changes; quitting asks about each open device. Classic still shows one device
  and replaces it. (WP MDQT2a, MDQT2b, MDQT2c, INT3)
- Modern layout: the actions are where they act. A right-click (or the menu key) on a channel, a band (its row or its dot
  on the graph), the graph or a preset slot opens its menu: rename, copy and paste, mute, identify, routing, headphone
  profile and Clear PEQ on a channel; shape, slope, bypass, invert, copy and paste of one band, and reset on a band or
  the selected bands; add a band here, the selection, spectrum, phase, pop-out and settings on the graph; recall, save,
  rename, startup preset, copy and clear on a slot, also in the preset picker's list. Undo has a ▾ with the recent
  changes and History. The app button's menu holds only the global actions (files, settings, help, quit). A device tab's
  Close is greyed while a firmware update or a core voltage calibration runs there, with the reason. (WP LAY3b)

### Android
- DSPi Link is reachable from the main page (owner report on 0.14.0: only through the settings
  cogwheel). With no DSPi connected, the strip under the top bar says "No DSPi connected. Plug one
  in over USB, or connect over DSPi Link." and shows **Connect over DSPi Link** (opens Networking,
  the same page as Settings › Networking) and **Try the demo**. The device list always offers
  "DSPi Link…", with DSPi Link off too. DSPi Link stays opt-in: it is turned on only with
  Networking's switch; hubs are listed only while it is on. (WP ANLINK)
- The volume buttons change the DSPi's User Volume (the Home "User Volume" slider) while the app
  is in front and a DSPi is the current device (USB, DSPi Link or the demo): 1 dB a press, held to
  repeat, -60 to 0 dB. The slider moves; elsewhere a short "User Volume" note replaces Android's
  volume panel. The mute button toggles the User Volume's mute. With no DSPi, a read-only
  connection, or the app in the background, the buttons change this device's volume as before.
  On by default; Settings › Advanced › Volume Buttons turns it off. (WP ANVOL)
- An unencrypted DSPi Link connection is no longer a red strip: it is amber (a warning tone with WCAG AA contrast in
  light and dark), still ends in "(unsecured)" and keeps its info button. The info dialog offers "I understand, don't
  highlight this hub" (on a phone also the strip's overflow menu): that hub's strip then has the normal colours, with
  the words and the info button kept. The choice is remembered per hub (its id, and for a hub added by address the
  address), dropped when the hub is next seen encrypted or a different hub answers at that address, and can be undone in
  the info dialog or in Settings › Networking › Acknowledged Unsecured Hubs (per hub or Forget All). The lost-connection
  banner stays red. (WP ANSEC)
- On a phone, a long hub or device name no longer pushes "(unsecured)" out of sight; the name is shortened instead.
  (WP ANSEC)
- Settings › Networking no longer says all DSPi Link traffic is unencrypted: DSPi Link connections to a uConsole for
  DSPi hub are encrypted and pairing pins the hub's key; those to Weeb Labs' DSPi Console or a DSPi bridge are not.
  The privacy policy and the Play description say the same. (WP ANSEC)
- The strip under the top bar is thinner: 36 dp instead of 56 dp on one line (68 dp instead of 100 dp for a phone's
  "No DSPi connected" with its two actions). Its buttons are drawn smaller (32 dp high); each still reports a 48 dp touch area, though a tap right on a neighbouring button goes to that button. (WP ANBAR)
- The strip's colours are the app's own instead of Material's default pink: a calm blue for DSPi Link, a warm grey for
  the demo and an open preset file, a cool grey for an acknowledged unsecured hub; the unsecured warning stays amber.
  Text and icons have WCAG AA contrast (4.5:1 or more) in light and dark, also on the USB strip, whose text was a
  light grey. (WP ANBAR)
- The accent blue is a shade per theme so it is readable as text everywhere (WCAG AA, 4.5:1): #005AC2 in light
  (was #007AFF), #64AEFF in dark with dark navy text on filled accent buttons, chips and switches. Selected pills and
  tiles have a fainter blue fill so their blue text stays readable. Every Material colour role is now the app's own,
  so no Material lavender or pink remains (slider tracks, the active preset row, Settings' selected page, menus,
  dialogs, snackbars). (WP ANBAR)

### Web console
- The Modern layout. The sections (Overview, Equaliser, Routing, Effects, Measure, Analyse, Library, System, Devices,
  Settings) are in a rail on the left, and each section's pages are tabs beside its title. A row of device tabs along
  the top holds the open devices, with how each is reached, its state and an unsaved mark; + opens another, and
  closing a tab asks about unsaved changes first. Each device comes back at the page it was left at. Above the pages:
  the preset picker, Undo and Redo, Save to device. Below them, on every page: the input source (new on the web), the
  volume with Mute and Bypass, and the meters. On a narrow screen the rail is a row. (WP WEBM1)
- A DSPi is open once: another entry for a DSPi that is already open (over USB and through a hub, or through two hubs)
  shows the open one and says so, rather than making a second session. (WP WEBM1)
- The System screen has an Installation tab in the Installer mode on a hub. It lists the units the hub sees that
  are not in the installation, with Join (name, room, role and what feeds it), and edits each unit's room, role and
  upstream, the rooms and the zones. The hub's own check runs as you edit and shows a problem where it is; Save lists
  what changes and needs the hub's Admin role. If someone else saved meanwhile, nothing is overwritten: the tab reads
  the installation again and offers to make your changes again on it. A hub with no installation can get its first
  one here. (WP INSTW)
- Settings › Speakers says which output drives what (full range, sub, woofer, mid,
  tweeter), suggested from the filters and confirmed by you. The Measure setup starts from it,
  and a way, speaker or group changed there is saved to it. A banner on both says when the
  layout is not confirmed, or came with an imported configuration and needs checking. The
  browser keeps it per DSPi. (WP ROLEW)

## 0.14.0 (2026-11-20)

One version for every client but the Swift app: the Qt desktop app (Windows, Linux and
macOS), Android (versionCode 14), the Terminal (`uconsole`), the web console and the hub. The
Swift app for macOS, iPhone and iPad is not part of this release (deferred by the owner); the
Mac's download is the Qt app's `.dmg`. Everything below was merged after the 0.13.0 release (tag `v0.13.0`,
2026-11-09) up to 2026-11-20.

### Names
- The project's apps are called **uConsole for DSPi**. Release files start with
  `uConsole-for-DSPi-<version>-`; packages, installed files and the desktop app's command use
  `uconsole-for-dspi`. The Android app is now called **uConsole for DSPi** on Google Play too
  (formerly DSPi Console for Android); its app ID stays `se.bofr.dspi_console_android`.
- Google Play granted the Android app production access (2026-11-20); it is being rolled out
  to production, no longer only in closed testing.
- Settings, the Windows install folder and registry entries, and the Linux desktop entry use
  the new names. Nothing is carried over from the earlier names.

### Measurement and auto-tune (new)
- Measure speakers or headphones with a calibrated microphone in the web console (Measure
  screen) and the desktop app (Tools > Measure…). The DSPi plays the test signal with its own
  generator; the console only listens.
- Microphone calibration from miniDSP UMIK-1/UMIK-2, REW, CSV and Dayton Audio (iMM-6C, UMM-6)
  files; speakers at one or several listening positions, or headphones on a coupler.
- Results against a target, a live microphone RTA, and direct adjustments from the measured
  curve ("cut this peak", "fit this range") as one undo step.
- Auto-tune proposes delay and polarity, crossover corner and levels and a target EQ, in words.
  A summed check comes first; the apply is guarded between "Before" and "After" snapshots,
  verified by measuring again, then kept or reverted, with A/B between the two.
- Safety first: the level starts quiet and ramps only until the microphone hears it; a run stops
  on clipping or on any change from elsewhere; a tweeter is measured only through its
  protective high-pass; EQ boosts are bounded; a delay is offered only when the timing is
  reliable.
- Measurement, microphone and target documents, kept locally or on the hub, and REW text export.
  `uconsole measure` imports calibrations and targets, manages documents and runs an EQ-only
  tune from a saved measurement.
- The demo device has a simulated microphone in a simulated room, so the whole flow can be tried
  without hardware.
- Android: measurement comes in a later release.

### Multi-unit system and the audio link (alink)
- The hub models an audio-link system: rooms, zones and units, the link topology, alignment,
  scenes recalled at a common frame, system alarms and a system mute. The web console has the
  System view, the scene bar and a status rail; Qt and Android show the system's status, and
  Android's System screen names units fed by S/PDIF (view only).
- USB-direct alink units (several DSPis on USB from one computer, in phase lock) in the hub,
  the core and every client, with scene timing, start-offset watch and a keep-aligned option.
  `uconsole usb-direct generate` writes a home theatre's PipeWire and WirePlumber files.
- **Several units measured together**: units on one time base (behind one alink sender, on
  USB-direct, or following one DSPi over S/PDIF) are measured with one microphone through our
  hub, in the web console's and Qt's "Several units" tab. Starts may overlap, sized for the
  expected lateness, so a session takes less time. A delay between units is offered only when
  it is known to within 10 µs, after a summed check across the units.
- The hub guards a system apply: a countdown, and it puts the installation and the units' delays
  back by itself if Keep does not come.
- The alink sender (`DSPi-Sender`) connects as its own kind of device with a limited view
  (device info, the audio link, firmware updates). The desktop app greys out every DSP menu,
  shortcut and tool window for it and says why in one banner.

### Hub and DSPi Link
- **DSPi Link is encrypted.** The hub serves TLS 1.3 on its one port and requires it by default.
  Clients pin the hub's key when they pair (PIN pairing bound to the key, with fingerprint and
  QR code; a PAKE for the native clients) and refuse a hub whose key changed; a hub once pinned
  is never reached in plaintext. Key rotation is signed by the old key
  (`uconsole hub tls rotate`). Upstream DSPi Link clients, which cannot encrypt, need the hub's
  `--upstream-compat`.
- **The hub works on its own**: inventory and health of every DSPi it has seen, Identify (LED
  and an optional quiet tone), a shared history of who did what, alarms with acknowledgement,
  notifications by HTTPS webhook, push or MQTT (with TLS), parameters, presets and snapshots by
  name with a shared copy on the hub, schedules, MQTT with Home Assistant discovery, and a REST
  mirror under `/api/v1/` with API keys. Every write path goes through the same roles, limits
  and lock as a console.
- **The fleet in the web console**: groups and group writes (volume with offsets, preset maps,
  bulk apply of a snapshot with a way back per device), the health board, the Overview, and an
  unattended firmware rollout of the hub's own USB devices. The hub API v1 is frozen (additive
  changes only).
- The hub's limits profile, PIN and pinned modes can be set from every client.
- The hub serves devices of both firmware generations and DSPi Link 1.2 and 1.3 clients, the
  paged alink sender included.
- A revert that the hub could not complete is now said as such, naming what was not put back.

### Firmware updates and safety
- **Firmware 1.1.7** (upstream's paged 1.1.7 beta as of 2026-11-18 (later 1.1.7 betas may need an update)) is supported beside 1.1.6, decided once per
  connection, in every client but the Swift app. Device info (generation, protocol, pages,
  board) is shown in the web Device screen, Qt and Android Settings > About and `uconsole info`.
- An update check runs before every bootloader entry (no image for a board without BOOTSEL, a
  phrase to type before leaving this fork's firmware, a generation change said), and the core's
  write gate judges every firmware write on the target it read itself.
- The DSPi fork firmware this release was tested against is published as
  [v1.1.6-bofr1](https://github.com/Bofr5555/DSPi/releases/tag/v1.1.6-bofr1) (images for
  RP2040, RP2350, the K340 and the alink sender; [which image](docs/INSTALL.md#fork-firmware)).
- Images carry a kind stamp (DSP or alink sender) and a core-voltage stamp. An image of the
  wrong kind, or of an unreadable kind, is refused with no override; an unstamped image on a
  sender needs a typed confirmation. Our own stamped sender image on a sender skips the voltage
  phrase.
- Firmware is written through the core's PICOBOOT writer, and the bundled images are checked
  against pinned digests.
- **A device you can trust** (PRO phase 1), in every client: snapshots, A/B compare, safe apply
  with automatic revert, limits profiles, the installer lock and PIN, confirmation tokens for
  every dangerous operation, offline documents and templates, document diff and partial import,
  undo groups and the AutoEQ preview.
- Client read-only mode, and refused writes put back with a notice.

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
- Copy and paste of a channel's parameters through one core clipboard; a linked pair's band,
  bypass and paste edits are one undo step.
- A diagnostics bundle (one zip: versions, the device's state, stats, events, the OS and
  USB view; Link tokens removed) can be saved from the desktop app and with
  `uconsole diag`.

### Desktop app (Qt, Windows, Linux and macOS)
- The Qt app was rebuilt in QML from the macOS console's layout and wording, on the shared
  core, and replaces the earlier port. It needs Qt 6.8 or newer; the Qt 5 build is gone.
- macOS: `uConsole-for-DSPi-0.14.0-macos.dmg`, the same Qt app for Intel and Apple silicon
  (universal), macOS 12 or newer. It is signed ad hoc only, not with a Developer ID, and not
  notarised, so macOS asks once before it opens it (docs/INSTALL.md, macOS). No USB driver or
  rule is needed; pairing tokens are kept in a file, not the Keychain.
- New compared with the earlier port: read-only mode for firmware this version cannot
  write; the crossfeed pair map; device name, source names and output latency in Settings;
  clock diagnostics and output ring health in Stats; separate C0 and C1 CPU bars; the
  bundled 1.1.6 firmware, with a custom `.uf2`; the demo device; a Core Voltage page under
  Settings > Advanced (on firmware with the extension channel); Edit > Undo, Redo and
  History; Help > Save Diagnostics; an update check, off until switched on (Settings >
  Updates), in the Windows files, the AppImage, the tar.gz and the macOS `.dmg` only.
- The PRO phase 1 dialogs, the hub's limits and PIN, shared snapshots, the hub's history, the
  alarm banner and Identify, pairing bound to the hub's key, measurement (Tools > Measure…,
  with "Several units"), and the device's generation and update check in Settings > About.
- `uconsole-for-dspi --version` and `--help` answer without a display.
- On Windows the settings are an INI file in `%APPDATA%\uconsole-for-dspi\`; the
  uninstaller removes it and the app's firewall rule.

### Android
Changes since 0.13.0 (versionCode 13 on Google Play, closed testing):
- The app is now called uConsole for DSPi on Google Play (formerly DSPi Console for Android),
  and has production access there (2026-11-20); it is being rolled out to production.
- Phone Home with the volume first and a health line; channel colours as tags.
- The PRO phase 1 screens (snapshots, A/B, safe apply, limits, the installer lock, documents
  and templates, diff and partial import, AutoEQ preview), undo and redo, and Copy/Paste
  Parameters.
- The hub's limits and PIN, shared snapshots, the hub's history, alarm notices and Identify.
- Pairing bound to the hub's key, with an "(unsecured)" bar for a hub that is not encrypted.
- Device info, the update check before the bootloader entry and the core's write gate; a page
  for the alink sender; S/PDIF units in the System screen (view only).

### Web console
- The UI is complete: besides the main window, EQ, crossover, outputs and matrix there are now
  inputs, effects, the System view, scene bar and status rail, Overview, Snapshots, Files and
  AutoEQ, Control Surfaces, Events, Installer, the tools (Signal Generator, Analyser, Stats,
  Interrupt Monitor), Measure, Device and Settings; on a hub also the Hub screen (devices,
  health board, alarms, history, groups, bulk apply, firmware rollout) and Hub Settings.
- DSPi Link over TLS (Chromium and Firefox); `uconsole web` pins the hub's key for the page.
- Copy and paste of a channel's parameters; Paste names the device the copy came from.
- Not yet: Web MIDI.

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
- Every write goes through the core, so limits, the installer lock and confirmation tokens
  hold for it (a dangerous write asks on a terminal and takes `--yes` in a script).
- New commands: `open`, `snapshot`, `ab`, `diff`, `import --only/--preview/--guard`, `limits`,
  `lock`, `autoeq preview|apply`; `uconsole hub` services, REST, TLS, groups and the firmware
  rollout; `uconsole measure`; `uconsole info` and `raw --id`; `uconsole usb-direct generate`.
- `uconsole --third-party-licences` prints the licence texts of the Rust crates compiled
  into it; the release archives carry them as `THIRD-PARTY-NOTICES.txt`.
- `uconsole doctor` also accepts the udev rule a package installed, and says when the installed
  rule is an older one.

### Packaging
- Desktop: Windows installer and zip; AppImage, a tar.gz with an installer, `.deb` (Debian 13),
  `.rpm` (Fedora 43 and 44) and an AUR `PKGBUILD`. None are code-signed. The AppImage has its
  own release test on Ubuntu 24.04 and 25.04, Debian 13 and Fedora 44 with no Qt installed.
- **One udev rule for every component** (`70-dspi.rules`, from `xtask/70-dspi.rules`): the
  `.deb`, `.rpm` and AUR package (in `/usr/lib/udev/rules.d/`), the tar.gz's `install.sh
  --udev`, the desktop app's device-access dialog and `uconsole --install-udev` (in
  `/etc/udev/rules.d/`) all install the same file. It lets every account open a DSPi
  (`MODE="0666"`, so headless machines and the hub's service work too), keeps logind's
  `uaccess` tag, and covers older firmware's `2e8a:feaa` and the RP2350 bootloader `2e8a:0003`.
  Before, the Terminal and the desktop app wrote different rules to the same path. A rule
  installed by the tar.gz before 0.14.0 as `70-uconsole-for-dspi.rules` can be removed.
- Each release's `SHA256SUMS` is signed with minisign; the release notes give the public
  key and how to check a download.
- Not in this release: the Swift app for macOS, iPhone and iPad.

### All clients (draft notices)
- About no longer shows the first-person donation paragraph or "Made with love by Weeb
  Labs"; the credit line reads "DSPi and its firmware by Weeb Labs", and the links section
  says where to support Weeb Labs' work.
- The licence notice says that the program as a whole is distributed under the GNU GPL
  version 3, with Bofr5555's own contributions under version 3 or later and the section 7
  terms; it names the source archive published with each release instead of a repository
  link, and credits the firmware's third-party parts, Noto Sans Mono and AutoEq.

### Integrators (breaking change of the C ABI and the bindings)
<!-- -- WP P1TZ -- -->
- Confirmation tokens are always required (OD-P1f). Every dangerous operation (preset
  save, copy and delete, parameter save, factory reset, output configuration save,
  bootloader, configuration import, stored core voltage, firmware install) takes a token
  minted with `confirm_token` (C: `dspi_confirm_token`) after the person confirmed, through
  its `_confirmed` call; without a good one it is refused with `confirmation_required` and
  nothing is sent.
- Removed: the calls without a token. C: `dspi_save_preset`, `dspi_copy_preset`,
  `dspi_delete_preset`, `dspi_save_params`, `dspi_factory_reset`, `dspi_save_output_config`,
  `dspi_enter_bootloader`, `dspi_config_import`, `dspi_config_import_selected`,
  `dspi_core_vreg_cal_commit`, `dspi_core_vreg_manual_set`, `dspi_core_vreg_op`. UniFFI
  (Kotlin, Swift): `save_preset`, `copy_preset`, `delete_preset`, `save_params`,
  `factory_reset`, `save_output_config`, `enter_bootloader`, `config_import`,
  `config_import_selected`, `core_vreg_cal_commit`, `core_vreg_manual_set`, `core_vreg_op`
  (now `core_vreg_op_confirmed`: a SET, which stores the voltage, takes a `vreg_save`
  token; trials, AUTO and CLEAR take none). Use the `_confirmed` form of each (an
  import: `config_import_confirmed(text, options, eq_channels, token)`, `eq_channels`
  all bits for every channel).
- The firmware write is one core method, behind the firmware guard, that takes a
  `firmware_flash` token: `flash_confirmed` (C `dspi_flash_confirmed`). It works with no
  device open, so a board already in BOOTSEL (no DSPi connection) is still flashed: pass
  `firmware_guard_board_in_bootloader()` (C: `board_required_mv` 0) and the phrase the
  guard's verdict asks for, and mint the token on any core once the bootloader drive is
  there. Removed: the free `flash_to_drive` and `flash_to_drive_guarded` (C
  `dspi_flash_to_drive`, `dspi_flash_to_drive_guarded`); there is no write without the
  guard. Without a token, or with one for another operation, nothing reaches the drive.
- Removed: the switch. C `dspi_require_confirmation` and `dspi_confirmation_required`,
  UniFFI `require_confirmation` and `confirmation_required`, the web Worker's
  `requireConfirmation` and `confirmationRequired`.
- `write_param` (C `dspi_write_param`) refuses the gated paths `preset.save`,
  `preset.delete`, `dev.save` and `dev.save.io` with `confirmation_required`;
  `write_param_confirmed` (C `dspi_write_param_confirmed`, UniFFI `write_param_confirmed`,
  the web Worker's `write` with `token`) takes them with a token.
- Raw requests (`raw_get`, `raw_set`; the Terminal's `raw`) refuse the opcodes of the
  dangerous operations (`REQ_PRESET_SAVE`, `REQ_PRESET_DELETE`, `REQ_SAVE_PARAMS`,
  `REQ_SAVE_OUTPUT_CONFIG`, `REQ_FACTORY_RESET`, `REQ_ENTER_BOOTLOADER`) with
  `invalid`, naming the `_confirmed` call to use; nothing is sent. A raw request carries no
  token. Every other opcode goes out as before.
- The import's token refusal (C `dspi_config_import_confirmed`, the web Worker's
  `configImport`) carries `code` `confirmation_required`, as every other `_confirmed`
  reply.
- Migration: [docs/integrator/README.md](docs/integrator/README.md#confirmation-tokens).
  Every client in this project already uses the `_confirmed` forms.
