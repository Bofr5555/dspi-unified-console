# DSPi Console 0.13.0

Release date: 2026-11-09. One version for every client.

0.13.0 is the first release of the unified console: the Android app, the
desktop app for Linux, Windows and macOS, the `dspi` Terminal and the web
console are now built from one shared core. A fix or feature in the core
reaches every client at once, and every client reads and writes the same
preset files.

For Android this is the update after 0.12.0 on Google Play. For the other
clients it is the first release from this project; their notes describe what
each one can do, and what is new compared with Weeb Labs' original consoles
they grew from.

The clients work with stock DSPi firmware from Weeb Labs. Features marked
"extension firmware" need a firmware build with the extension channel and are
hidden otherwise.

Licence: GPL-3.0-or-later, with additional terms under section 7, shown in
every client's About. The notices are drafts until approved.

## All clients

- **DSPi Link is faster and more tolerant.** Meters start within seconds of
  connecting, even on a weak or lossy network (before, they could take well
  over a minute). A device behind a hub shows as soon as its snapshot arrives
  and the rest loads in the background, instead of being read twice. Short
  drop-outs no longer end a session: the connection is checked every 5
  seconds and closed only after 30 seconds of silence.
- **Outputs say what they play.** An output that still has its factory name
  is labelled from the routing and the filters, for example "R · Low", and
  output pairs read "Woofers L | R". A name you set yourself always wins.
- **Fractional delays are kept** (for example 0.5 ms), down to 0.0001 ms.
- **Delay limits come from the device**: 21 ms on RP2040 and 42 ms on RP2350.
  A longer value is refused with a message that says why, and importing a
  preset with a longer delay names each delay it shortened.
- An edit made right after loading a preset now stays marked as unsaved.
- A delay changed from another console now shows straight away.
- Reset Channel Names gives I2S outputs their I2S names (it gave them S/PDIF
  names).
- The demo device reports firmware 1.1.6, so it no longer shows an outdated
  firmware banner.

## Android

Changes since 0.12.0:

- **Faster DSPi Link meters**, as above: on a weak Wi-Fi network the first
  meter reading now arrives within seconds. A lost connection shows a banner
  with Retry and Disconnect, and the PIN prompts are clearer.
- **TalkBack support.** Sliders, meters (read in 3 dB steps), switches, the
  matrix crosspoints ("USB 1 L to OUT1"), the EQ graph's band handles (with
  actions) and the help tips all read out what they are. The layout is
  unchanged.
- **Output labels** beside each OUT pill ("R · Low") and paired cards
  ("Woofers L | R"); they fit at larger font sizes too.
- **Fractional delays** are shown with two decimals. Delay sliders move in
  whole milliseconds, like the macOS Console; typed values stay exact.
- **Delay limits from the device** (21 ms on RP2040, 42 ms on RP2350; the app
  used to offer up to 85 ms). Pasting a channel copied from a device with a
  longer limit no longer fails.
- **A smaller download**: about 5 MB per device instead of about 12 MB.
- **Spectrum bars** on Home and on the channel pages, and an "RTA Bars" switch
  in the graph menu. When another console drives the analyser, the app follows
  it and offers Take control.
- **One Devices menu** for USB, known hubs, DSPi Link and Demo. Demo is
  available with a DSPi attached, and switching devices asks about unsaved
  changes.
- **Getting Started and What's New.** Existing users are not sent through
  setup again.
- **Firmware**: the DSPi 1.1.6 images are bundled, with a safety check before
  writing, a banner when the firmware does not match, and Export Configuration
  offered before an update. The AutoEQ database can be downloaded.
- **Control surfaces** can be edited in full: bindings, IR remote buttons
  (with learning), display pages, channel groups, macros and auxiliary
  outputs.
- **Core voltage calibration** under Settings > Advanced (extension firmware).
- **Keyboard shortcuts** with a hardware keyboard (Ctrl or Meta with the
  macOS Console's letters, Ctrl+C and Ctrl+V for a channel, Space for the
  signal generator). The input strips follow the USB host's channel count.
- The signal generator keeps running when you leave its page.
- Linkwitz Transform and crossover filter-type changes are checked properly.

A sideload APK is also offered. It installs as a separate app, "DSPi Console
(APK)", next to the Google Play version rather than over it.

## Desktop: Linux, Windows and macOS

The desktop app grew from Weeb Labs' DSPi Console for Linux. It now runs on
the shared core and is offered for Windows and macOS as well.

What it does: channel pages with ten EQ bands and every filter type
(including Linkwitz Transform), editing on the graph, the spectrum analyser
(overlay, bars, and a window of its own), the dashboard, the matrix mixer,
every effect, limiters, presets and `.dspipreset` files shared with the
other consoles, AutoEQ, control surfaces, the tools, firmware updates and
DSPi Link.

New compared with the original Linux console:

- **DSPi Link client**: a Devices list, finding hubs on the network, PIN
  prompts, a lost-connection banner, and following the analyser of another
  console.
- **Sharing as a hub**: a Networking page with roles and revoking access, a
  tray icon with close to tray, open at login, and keeping the system awake
  while sharing.
- **Demo mode**, and a read-only mode for firmware it cannot fully drive.
- Crossfeed pair map, output latency, buffer health, input source and device
  names; core voltage status and calibration (extension firmware).
- Every macOS Console shortcut, with Ctrl in place of Command. The input
  strips follow the system's audio format (PipeWire or ALSA on Linux, WASAPI
  on Windows).
- **Accessibility**: screen-reader names and roles, keyboard navigation, and
  sliders that commit from the keyboard.
- The output labels and delay rules above (labels in full, "Out 1 · R ·
  Low").
- An optional check for new releases, off by default.

Downloads:

- Linux: `.deb` (Debian 13, Ubuntu 25.04 and later), `.rpm` (Fedora 42 and
  later), AppImage and a `.tar.gz` with the AppImage, the udev rule and an
  installer (Ubuntu 24.04 and later), and an Arch Linux `PKGBUILD`.
- Windows (x86_64): a zip that runs as is, and a per-user installer.
- macOS: one `.dmg` for Apple silicon and Intel.

None of the files are signed. Windows SmartScreen and macOS Gatekeeper warn
the first time the app is opened.

## Terminal (`dspi`)

The Terminal grew from Weeb Labs' DSPi-Terminal: a full-screen console laid
out like the desktop app, plus a scriptable command line (get and set, EQ,
mixer, dump, watch, export and import, AutoEQ, undo, shell completions, JSON
output, dry runs).

New compared with DSPi-Terminal:

- `--demo`, `--midi` and `--link` work with every command; `dspi link scan`
  and `--hub` find hubs, and a status line shows a lost link.
- **`dspi hub`**: a hub without a window, with PIN pairing, roles and keeping
  the system awake, plus example service files for systemd, launchd and the
  Windows Task Scheduler. `dspi hub --web` and `dspi web` serve the web
  console.
- **`dspi flash`** updates the firmware, with the core-voltage safety check
  (`--force` does not skip it).
- Read-only mode for firmware it cannot fully drive; the analyser overlay
  with following and Take control; a third-octave bar strip.
- Export and import as JSON, through standard input and output, and
  `dspi cs export|import` for control surfaces. AutoEQ updates without curl.
- Crossfeed pair map, core voltage status, device and source names, output
  latency, buffer health, Tube Basic and Advanced, Identify; the volume mode
  is remembered.
- The output labels ("R·Lo") and the delay limit.
- Fixes: no crash when the output pipe closes; a multi-line Save keeps every
  line; exports no longer fill in values for reads that failed; the settings
  pages work at 40 columns; options before the command are read correctly.

Downloads: Linux x86_64, Linux aarch64 (for Raspberry Pi; without MIDI in
this build), Windows x86_64 and macOS (Apple silicon and Intel).

## Web console (preview)

The same core runs in the browser. It works with the demo device, with a
DSPi over USB through WebUSB, and with DSPis behind a hub over DSPi Link.
Screens: the main window (volume, presets, inputs and outputs), EQ with
editing on the graph, outputs, crossover, Linkwitz Transform and the matrix.

Limits in this preview:

- USB needs a Chromium-based browser (Chrome, Edge, Opera) and a page served
  over https or from localhost. DSPi Link works in Chromium-based browsers and
  Firefox; a hub accepts only pages it serves itself (`dspi hub --web`, or
  `dspi web` on the same system).
- It needs a recent browser (Chrome 137, Firefox 153 or Safari 27 and later).
- No MIDI yet, and no settings, effects, analyser, statistics or file
  screens yet.
- It has not yet been run against real hardware or a real hub.

## DSPi Link hub

The desktop app (Networking page) and `dspi hub` can share a DSPi on the local
network: PIN pairing by default, roles, revoking access, and advertising so
other consoles find the hub. It works with Weeb Labs' DSPi Link clients. With
`--web`, the hub also serves the web console and Weeb Labs' DSPi Link page.
The protocol is not encrypted: use it on networks you trust.
