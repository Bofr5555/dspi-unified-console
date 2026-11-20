# uConsole for DSPi 0.14.0

Release date: 2026-11-20. One version for every client but the Swift app.

0.14.0 turns the console into a tool for whole installations. It still works
as a plain console for one DSPi, but the headline features are new: measuring
and auto-tuning speakers with a microphone, systems of several DSPi units on
the audio link, a hub that runs on its own and is encrypted, and support for
upstream's paged firmware 1.1.7 beside 1.1.6 (the 1.1.7 beta as of 2026-11-18;
later 1.1.7 betas may need an update).

This release carries the desktop app for Windows and Linux, Android
(versionCode 14), the Terminal (`uconsole`), the web console and the hub. The
Swift app for macOS, iPhone and iPad is **not** part of 0.14.0, so there is no
macOS download this time.

The project's apps are now called **uConsole for DSPi**: release files start
with `uConsole-for-DSPi-0.14.0-`, packages and the desktop command are
`uconsole-for-dspi`, and the Terminal's command is `uconsole`. The Android app
is now called uConsole for DSPi on Google Play (formerly DSPi Console for
Android), and Google Play granted it production access: it is being rolled out
to production, no longer only in closed testing.

The clients work with stock DSPi firmware from Weeb Labs. Features marked
"extension firmware" need a firmware build with the extension channel and are
hidden otherwise: the DSPi fork firmware, release
[v1.1.6-bofr1](https://github.com/Bofr5555/DSPi/releases/tag/v1.1.6-bofr1), is the one
0.14.0 was tested against.

Licence: GPL-3.0-only for the program as a whole (Bofr5555's contributions
GPL-3.0-or-later with additional terms under section 7), shown in every
client's About. The notices are drafts until approved.

## Measurement and auto-tune (new)

- **Measure with a calibrated microphone** in the web console (Measure) and
  the desktop app (Tools > Measure…). The DSPi plays the test signal with its
  own generator; the console only listens. Calibration files from miniDSP
  UMIK-1/UMIK-2, REW, CSV and Dayton Audio (iMM-6C, UMM-6) are read.
- Speakers at one or several listening positions, or headphones on a coupler.
- Results against a target, a live microphone RTA, and direct adjustments from
  the curve ("cut this peak", "fit this range") as one undo step.
- **Auto-tune** proposes delay and polarity, the crossover corner and levels,
  and a target EQ, explained in words. A summed check comes first; the apply is
  guarded between "Before" and "After" snapshots, verified by measuring again,
  and then kept or reverted, with A/B between the two.
- **Safe by design**: the level starts quiet and rises only until the
  microphone hears it; a run stops on clipping or on a change from elsewhere;
  a tweeter is measured only through its protective high-pass; EQ boosts are
  bounded; a delay is offered only when the timing is reliable.
- Measurement, microphone and target documents, kept locally or on the hub,
  and REW text export. `uconsole measure` handles documents and runs an
  EQ-only tune from a saved measurement.
- The demo device has a simulated microphone in a simulated room, so the whole
  flow can be tried without hardware.

Measurement on Android comes in a later release.

## Several DSPi units: the audio link (alink)

- The hub models a whole system: rooms, zones and units, the link topology,
  alignment, scenes recalled at a common frame, system alarms and a system
  mute. The web console shows it in the System view, the scene bar and a
  status rail; the desktop app and Android show the system's status.
- **USB-direct units**: several DSPis on USB from one computer, kept in step,
  with a keep-aligned option. `uconsole usb-direct generate` writes the
  PipeWire and WirePlumber files for a home theatre.
- **Several units measured together** with one microphone, through the hub
  ("Several units" in the web console and the desktop app): units behind one
  alink sender, on USB-direct, or following one DSPi over S/PDIF share one time
  base. Starts may overlap, so a session is quicker. A delay between units is
  offered only when it is known to within 10 µs.
- The hub guards a system-wide apply and puts everything back by itself if
  Keep does not come.
- The alink sender connects as its own kind of device, with device info, the
  audio link and firmware updates. The desktop app greys out the DSP menus and
  tools for it and says why.

## Hub and DSPi Link

- **Encrypted.** The hub serves TLS 1.3 and requires it by default. Clients
  pin the hub's key when they pair (PIN pairing with a fingerprint or QR code)
  and refuse a hub whose key changed. Weeb Labs' DSPi Link clients, which
  cannot encrypt, need the hub's `--upstream-compat`.
- **Works on its own**, with no console open: inventory and health of every
  DSPi it has seen, Identify, a shared history of who did what, alarms,
  notifications (HTTPS webhooks, push, MQTT), presets and snapshots by name,
  schedules, Home Assistant through MQTT, and a REST API with API keys. Every
  path obeys the same roles, limits and lock as a console.
- **The fleet in the web console**: groups and group writes, bulk apply with a
  way back per device, the health board, the Overview, and an unattended
  firmware rollout of the hub's USB devices. The hub API v1 is frozen.
- The hub's limits profile and PIN can be set from every client.
- A revert the hub could not complete now says so, and what was not put back.

## Firmware updates and safety

- **Firmware 1.1.7** (upstream's paged 1.1.7 beta as of 2026-11-18; later 1.1.7
  betas may need an update) works beside 1.1.6; each client shows the device's
  generation in its device info.
- Before the board enters its bootloader, an update check says whether the
  image fits: a board without BOOTSEL gets no image, leaving this fork's
  firmware asks for a typed phrase, and a generation change is named. The core
  then checks every firmware write against the board it read itself.
- Images say what they are (a DSP or an alink sender) and which core voltage
  they need; an image of the wrong kind is refused.
- **A device you can trust**, in every client: snapshots, A/B compare, safe
  apply that reverts by itself, limits profiles for installers, the installer
  lock and PIN, confirmation for every dangerous operation, offline documents
  and templates, document diff and partial import, undo groups, and the AutoEQ
  preview.
- Undo and redo in every client, through one change history; undo skips a
  value another console has since changed and says which.

## Desktop: Windows and Linux

The desktop app was rebuilt in QML from the macOS console's layout and
wording, on the shared core (Qt 6.8 or newer). New: read-only mode for
firmware it cannot fully write, the crossfeed pair map, clock diagnostics and
output ring health, the bundled 1.1.6 firmware, the demo device, the Core
Voltage page (extension firmware), Undo, Redo and History, Save Diagnostics,
measurement, the hub's shared snapshots, history, alarms and Identify, and an
optional check for new releases (off by default; Windows files, AppImage and
tar.gz only). There is no macOS build of the Qt app.

## Android

Changes since 0.13.0:

- The app is now called uConsole for DSPi on Google Play (formerly DSPi
  Console for Android), and Google Play granted it production access: it is
  being rolled out to production, no longer only in closed testing.
- Home on a phone puts the volume first, with a health line.
- Snapshots, A/B compare, safe apply, limits, the installer lock, offline
  documents and templates, diff and partial import, the AutoEQ preview, undo
  and redo, and Copy/Paste Parameters.
- The hub's shared snapshots, history, alarm notices and Identify, and its
  limits and PIN.
- Pairing bound to the hub's key, with an "(unsecured)" bar for a hub that is
  not encrypted.
- Device info, the update check before a firmware update, a page for the alink
  sender, and S/PDIF units in the System screen.

## Terminal (`uconsole`)

- The command is now `uconsole` (it was `dspi`); its settings, pairing tokens
  and hub store moved with it, and the hub's service files are named after it.
  Scripts and completions that call `dspi` need the new name.
- Every write goes through the core: limits, the installer lock and
  confirmations hold for it.
- New: `open`, `snapshot`, `ab`, `diff`, partial and guarded `import`,
  `limits`, `lock`, `autoeq preview`; the hub's services, REST, TLS, groups and
  firmware rollout; `measure`; `info` and `raw --id`; `usb-direct generate`;
  `--third-party-licences`.

## Web console

The web console now has every screen: besides EQ, outputs, crossover and the
matrix, also inputs, effects, the System view, Overview, Snapshots, Files and
AutoEQ, Control Surfaces, Events, Installer, the tools, Measure, Device and
Settings, and on a hub the Hub screen and Hub Settings. DSPi Link runs over
TLS. Web MIDI is not there yet.

## Linux: one USB rule

Every part of uConsole for DSPi now installs the same udev rule,
`70-dspi.rules`: the `.deb`, `.rpm` and AUR packages, the tar.gz's
`install.sh --udev`, the desktop app's permission dialog and
`sudo uconsole --install-udev`. It lets every account on the computer open a
DSPi (also headless machines and the hub's service) and covers the bootloader
a firmware update uses. By hand: `sudo cp 70-dspi.rules /etc/udev/rules.d/`,
`sudo udevadm control --reload-rules`, then replug. A rule the tar.gz put in
`/etc/udev/rules.d/70-uconsole-for-dspi.rules` before 0.14.0 can be removed.

## Downloads

| System | File |
|---|---|
| Windows 10/11 (x86_64) | `uConsole-for-DSPi-0.14.0-windows-x86_64-setup.exe`, or the portable `uConsole-for-DSPi-0.14.0-windows-x86_64.zip` |
| Debian 13, Ubuntu 25.04 or newer | `uconsole-for-dspi_0.14.0_amd64.deb` |
| Fedora 43, 44 | `uconsole-for-dspi-0.14.0-1.fc43.x86_64.rpm`, `uconsole-for-dspi-0.14.0-1.fc44.x86_64.rpm` |
| Other Linux (x86_64, glibc 2.39+) | `uConsole-for-DSPi-0.14.0-x86_64.AppImage`, or `uConsole-for-DSPi-0.14.0-linux-x86_64.tar.gz` with an installer |
| Arch Linux | the AUR package `uconsole-for-dspi` |
| Android 8+ | Google Play, or `uConsole-for-DSPi-0.14.0.apk` |
| Terminal (`uconsole`) | `uConsole-for-DSPi-0.14.0-terminal-PLATFORM.tar.gz` / `.zip` |
| Web console | `uConsole-for-DSPi-0.14.0-web.tar.gz` |

macOS, iPhone and iPad: not in this release.

The Windows files, the AppImage and the tar.gz check this project's releases for
updates when that is switched on (Settings > Updates); the `.deb`, the `.rpm`
and the AUR package are updated by their package managers. None of the files
are code-signed: Windows SmartScreen warns the first time the app is opened.

Every file of this release is listed in `SHA256SUMS`, which is signed:
`SHA256SUMS.minisig`. The source code of exactly this release is
`uConsole-for-DSPi-0.14.0-source.tar.gz`; the Qt packages' (Windows and Linux)
is `uConsole-for-DSPi-0.14.0-desktop-source.tar.gz`. Licence: see `NOTICE.md`.

## Check a download

The public key that signs every release:

```
RWTiNwy5h2MBuviJDs7LBs3wi2Bh/YdJyh5pXdS3XKo8wRt1igYdrGgY
```

It is also in the repository as
[`docs/release/minisign.pub`](https://github.com/Bofr5555/dspi-unified-console/blob/main/docs/release/minisign.pub).
Compare the two before you trust either.

1. Download `SHA256SUMS`, `SHA256SUMS.minisig` and the files you want into one
   folder.
2. Check the signature with [minisign](https://jedisct1.github.io/minisign/)
   (packaged for most systems: `apt install minisign`, `dnf install minisign`,
   `brew install minisign`, `scoop install minisign`):

   ```sh
   minisign -Vm SHA256SUMS -P RWTiNwy5h2MBuviJDs7LBs3wi2Bh/YdJyh5pXdS3XKo8wRt1igYdrGgY
   ```

   It must say `Signature and comment signature verified`.
3. Check the files against the signed list:

   - Linux: `sha256sum --ignore-missing -c SHA256SUMS`
   - macOS: `shasum -a 256 --ignore-missing -c SHA256SUMS`
   - Windows (PowerShell): `Get-FileHash .\FILE -Algorithm SHA256`, and compare
     the hash with the file's line in `SHA256SUMS`.

   Each file you downloaded must say `OK`.

A file that fails either check is not this release: do not run it.
