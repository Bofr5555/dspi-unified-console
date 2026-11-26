# Changelog

Release notes of each version: `CHANGELOG-0.15.0.md`, `CHANGELOG-0.14.0.md`, `CHANGELOG-0.13.0.md`.

## 0.15.0 (2026-11-26)

One version for every client but the Swift app: the Qt desktop app (Windows, Linux and macOS), Android (versionCode
15), the Terminal (`uconsole`), the web console and the hub. The Swift app for macOS, iPhone and iPad is still
deferred. The firmware is unchanged: the fork's v1.1.6-bofr1, as for 0.14.0. Everything below was merged after the
0.14.0 release (tag `v0.14.0`, `7d681a73`, 2026-11-20) up to 2026-11-26. The release notes are
`CHANGELOG-0.15.0.md`; they also say what was tested where, and the known limits.

### Layouts (desktop)
- A second layout, **Modern**: a rail of sections (Overview, Equaliser, Routing, Effects, Measure, Analyse, Library,
  System, Devices, Settings), every tool a page with tabs that can also open in a window of its own, the open devices
  as tabs at the top, the volume, meters, Mute and Bypass EQ in a dock, and no menu bar: actions are where they act
  (right-click a channel, a band, the graph or a preset slot; the menu key too), and every keyboard shortcut works.
  Limiting states show in a strip under the top bar; other notices are behind the bell.
- Settings › Appearance switches between Classic and Modern at any time (the device, the selected channel and open
  tool windows stay), sets the theme (system, light or dark) and arranges Modern (a darker grey, the inspector, the
  band bar under the graph). **New installs start in Modern**; an install whose Getting Started was finished keeps
  Classic. While a measurement runs, the layout is not switched.
- **Ctrl+K** (or the search in the top bar) opens the command palette: any menu command, page, channel, setting or
  open device, with where it is and its shortcut. The Interrupt Monitor now clears with **Ctrl+L**.
- Modern's Equaliser: **PEQ** (the channel in one strip over the graph, the selected band's values on the graph),
  **Crossover** (every output's curve over one table of high and low passes) and **Delay & Gain** (gain, delay,
  distance at 343 m/s, arrival bar, polarity, mute and limiter of every output). An **inspector** shows the selected
  band with every shape laid out flat, and the channel: what an input feeds, an output's crossover, limiter and
  Linkwitz Transform. Under 1280 px it opens over the page.
- Modern's **Overview** has one card per channel (its speaker's name and role, a small response, its level, its
  chips); **Routing** has the matrix with its inspector; **Effects** marks each effect that is on and shows the signal
  chain in the DSPi's order with a switch per effect; **Analyse** marks a playing Signal Generator and stops it from
  any of its pages; **Measure**, **System** and **Settings** have their parts as tabs; **Library** has Presets,
  Snapshots, History, Files and AutoEQ; **Devices** has This Computer, Hubs (the hub's key, your role, PIN and pairing
  again) and Firmware.
- Modern's **AI** button shows the agents' state and opens a popover with the switch, the connected agents, their
  latest changes with Undo, and Pause.
- Undo has a ▾ with the recent changes and the History.

### One design language
- Modern and the web console follow `docs/DESIGN-LANGUAGE.md`: surfaces told apart by tone instead of outlines (the
  focus ring stays), the accent only for on, chosen and the one main action, notices with a tint and an edge,
  floating menus and popovers with a shadow, forms at the left and at most 760 px wide, shared empty states, list and
  form rows, and graph axes with their labels inside the plot. Neutral grey surfaces, with Darker grey as an option.
  Classic's look is unchanged.

### Matrix mixer
- Redrawn, on the Routing page and in the Matrix window (both layouts) and in the web console: fixed tiles with the
  gain and polarity, dots for empty cells, "From inputs / To outputs" in the corner, column footers (gain, delay, fed
  by), Enabled and Mute in each column's header, sticky headers, and a dense grid (chosen, or past 12 outputs).
- Full keyboard: arrows, Home/End, Page Up/Down, Space, Enter or digits (typing "-6" on a cell is −6 dB), + and −
  (Ctrl for 3 dB), I, M, E, Delete, Shift+F10 and Escape; a menu per cell.
- **Link L/R pairs** (off by default, kept per device) mirrors an edit to the pair's other crosspoint as one undo
  step; **Direct 1:1**, **Clear** (asks past four crosspoints) and **Dense** as tools.

### Presets
- Modern's top bar has one preset button: a popover with every slot (Active, unsaved, Startup), Recall, Save, Copy
  Here and the slot's menu; Up and Down load the next slot, Enter or Space open it. The web console has the same.
- Loading a preset yourself is guarded when your speaker setup has a confirmed tweeter or midrange and the master
  volume is kept apart from presets: the device is silent while the preset loads, a tweeter or midrange the preset
  leaves without its high-pass is muted, and the volume comes back; a notice says what was muted (desktop and web).

### Several devices at once
- Modern opens each DSPi you choose (USB, a hub's unit, the demo) in a tab of its own; each tab keeps its device's
  edits, pages, history, drafts and running measurement. Closing a tab asks about that device's unsaved changes;
  quitting asks about each, and asks first while a firmware update or core-voltage calibration runs on any of them.
- Classic shows one device and lists the others under Open devices; the tabs are there again back in Modern. Window
  > System… has **Open in tab** for each unit of a hub.
- Choosing another device stops what plays on the one left: the Signal Generator (faded), a measurement or the live
  view; an auto-tune not kept goes back.
- Measure › Several units uses a unit's open tab; a refusal on a device not shown is a notice naming that device.
- Web console: device tabs, each coming back at the page it was left at; a DSPi is open once (a second way to the
  same unit shows the open tab).

### Speaker setup
- **Settings › Speakers** and a Speakers step in Getting Started: which output drives what (full range, sub, woofer,
  midrange, tweeter), pre-filled from the filters and routing, kept per device, **Confirm** to keep it. The web console
  has the same page, kept in the browser per DSPi. Measure starts from it and shows a banner while it is not confirmed
  or came with an imported configuration ("set outside this app").
- **Detect…** plays quiet sweeps on each output with the Measure microphone (above 1 kHz at most −24 dBFS; below 1
  kHz only on an output that is no tweeter, at most −50 dBFS and down to 157 Hz; only a driver still flat at 177 Hz,
  whatever the floor, is swept on down to 20 Hz at most −24 dBFS, never under the limits profile's high-pass minimum), with a warning first and
  **Stop**. A detected setup says "Detected, not confirmed"; each row has a marker (detected, uncertain, nothing heard,
  off, you, suggested) with its reason, and **Show Details** lists the evidence. A role you correct is yours.
- A measurement holds every output to the strictest of the role asked for, your confirmed setup's and what its filters
  show, one unit or several: a tweeter without a crossover yet is never played below 1 kHz, and a tweeter's
  measurement starts no lower than your limits profile's high-pass minimum for it. A sub is never a tweeter's
  measurement reference while a midrange is there.
- A layout kept but not confirmed is named on the main screen (Classic's banners, Modern's strip).

### AI agents (desktop app)
- **Settings › AI Agents** (off until you turn it on): an AI agent on this computer (Claude Desktop, Claude Code,
  Cursor, VS Code and other MCP clients) can read the open devices and change them. It listens on `127.0.0.1` and
  `[::1]` only (port 47860); from another computer, through SSH (`docs/mcp-ssh-forwarding.md`). Each agent program is
  paired once with a code, or uses a token made for it; removing a client stops its token. The agent starts the app
  with `--mcp`, or `uconsole mcp`, which relays to the running app. The endpoint follows MCP 2026-07-28 and the
  earlier version, checked against the official conformance suite. User manual: `docs/manual/ai-agents.md`.
- **Questions**: "About risky changes" (the default: louder, unmute, a preset or snapshot, a flash write, the input
  source, a tweeter's protection, test signals, every unit of an installation), "About every change", or "Never".
  Test signals are asked once per device: **Allow** covers that agent's measurements there for 10 minutes, shown with
  the time left and **Revoke**; it belongs to the agent's pairing and to that one device. An agent's save asks one
  question that names the preset, one for several devices.
- **Hard rules, under every setting**: a tweeter in your setup (confirmed, detected or stated in your words), and any
  output whose high-pass is at 1 kHz or above, keeps a high-pass of at least 1 kHz (or the profile's higher corner),
  12 dB/octave or steeper, whenever it can play; a midrange is held at 80 Hz; each change is judged on the state it
  leaves. A tweeter you left playing without a high-pass may be turned down by an agent, never up (gain, volume, a
  route, an effect, an EQ boost, a preset). Agents never update firmware, enter the bootloader or factory reset, change
  the limits profile, PIN or lock, hardware or core-voltage settings, play the signal generator or Identify, pair or
  administer hubs, delete presets, snapshots or measurements, send raw commands or touch files by path. An installer's
  lock tells the agent what still passes and where to unlock, never the PIN.
- **Presets**: an agent's load is silent while the app checks it, and is put back if it would leave a tweeter
  unprotected; a refused load leaves the device on its preset. No agent preset loads on a device that saves the
  master volume with each preset. A hub scene that loads presets is judged the same way.
- **What you see**: a strip under the top bar (both layouts) names the agent, what it does and where, with Pause and
  Stop; the AI button, Classic's robot icon and the tray icon show that it works; its changes are marked with a
  notice and **Undo**, listed in the activity (Device › AI Agents…) and tagged **AI** in the History. The screen
  follows its work (device tab, page, Measure), never while you drag, type or have a dialog open. A question about
  another device brings its tab to the front.
- **When you step in** the agent is told with its next call: what you changed, undid, loaded, opened, stopped or
  paused. A control you take back is refused to it until it asks; a change you make while its measurement runs stops
  it; after Stop, it is told you stopped it. **Undo** takes back the agent's newest whole call.
- **Measuring**: an agent can check and start a measurement, find an output's low end, propose, apply and verify an
  auto-tune, and measure several units. Measure runs it with your microphone ("Requested by AI · name"); one
  microphone, so it waits while anything else measures. Roles come from your speaker setup. Loudest measurement:
  85 dB SPL by default (60 to 95, with a dB SPL calibration), and never above −12 dBFS at a speaker or −30 dBFS on
  headphones; an agent may choose your calibrations and targets and only lower the limits. It keeps a tune only
  after a verify that passed.
- **Roles from your words**: one question shows the whole table, your words quoted; an output not named is treated as
  a tweeter; over a confirmed setup only when your words say what changed. Before a test signal below 1 kHz on an
  output your words made a midrange, woofer, sub or full range, its top end is checked; one that behaves like a
  tweeter is held as one ("tweeter-like"). Under Never: Settings › AI Agents › **Agents may set speaker roles from my
  words** (off by default).
- **Detection by an agent**: on a device nobody set up, with the same sweeps and one question; refused on a confirmed
  setup; an output where nothing was heard may not be routed, switched on or unmuted by an agent. Under Never:
  **Agents may detect speakers** (off by default).
- Agents can also: set a room's or zone's volume (each unit's limits hold; louder asks), open a DSPi the app knows
  and close a tab they opened, read a hub's schedules, edit a hub's installation in a draft of their own (sources,
  zone sources, slots, scenes, programme sources; saving follows the questions setting), and read this computer's
  sound settings (`dspi_get_host_audio`: whether a DSPi is an output, the default, its format, which apps play to it,
  the system volume; PipeWire or ALSA, Windows audio, Core Audio). They never change the computer's sound settings.
- Settings › AI Agents also has the USB devices agents may open (remembered), the loudest measurement, and the
  activity. Every number an agent reads or gives counts from 1, as on the screen.

### Installation setup on a hub
- **System › Installation** (Window > System… in Classic; the web console's System screen in the Installer mode):
  the units the hub sees and has not placed (**Join**: name, room, role, what feeds it); each unit's name, room, role
  and feed; senders and fan-outs, rooms, zones and groups; an S/PDIF source and the outputs it sends its time base on.
  The hub's checks show on the field; Save lists what changes and needs the hub's Admin role; if someone saved
  meanwhile, nothing is overwritten and **Make Them Again** redoes your changes. A hub without an installation can get
  its first one there.
- The audio link's sources (a sender's channels), what each zone plays (every unit of the zone on the link takes the
  source's channels as its slots; a clash is refused), a unit's own slots, scenes with actions (a `source` action
  switches a zone's source at once), and programme sources (which computer or box feeds which input). Before Save, a
  notice names the units whose hand-made slots a zone's source replaces; after Save, **Align** is offered.
- The System view sets a room's or zone's volume unit by unit, asking first when one gets louder.
- Settings › This Computer shows whether each open DSPi is an output of this computer, the default or not, its
  format, what plays to it and whether the system volume moves it.

### Outputs nothing plays to
- An output no input is routed to is "unused": the views that only show leave it out (the Overview's cards, the
  meters, the spectrum's chooser, Stats, Measure's live view; Android's Home, Spectrum and Stats; the Terminal's
  Overview, Spectrum and Stats; the web's signal chain), and the views that edit it name it "3 · None" unless you
  named it. The Equaliser's channel list, the History, Measure and an agent's notices name outputs as the Overview does (by
  their speaker, where the setup names one).
- An unused output that the signal generator, a measurement or Identify plays on keeps its meter, its spectrum entry
  and its Stats rows while it plays, in every client.

### Desktop (Qt), other changes
- Tool windows take the height their content needs (never less than their usual size, no more than the screen's) and
  scroll only beyond that; a size set by hand stays while open. Crossfeed opens at 780×586 instead of 780×470.
- Classic: clicking or tabbing into a number selects it; the "Filter Response" heading is gone; an output's control
  strip is one row on a wide window (in the name row from about 1600 px); the minimum height is 660 px (was 780), so
  the window fits a 1366×768 screen.
- A name with markup in it is shown as written. Graph axes read "0", not "+0". Measurement refusals, device events
  and History rows (one row per measurement) are in plain words. On a Linux desktop Qt has no theme for, the file
  dialogs are the system's (the portal's).

### Web console
- The Modern layout (the web's only one), laid out as the desktop's Modern page by page: one top bar (app menu,
  device tab, presets, search, Undo, Redo, Save to Device, the bell), device tabs, the rail (or tabs along the top),
  and the dock (input source, volume with User and Master, Mute, Bypass EQ, meters). Overview, Equaliser (PEQ,
  Crossover, Delay & Gain, Inputs, the inspector), Routing (the new matrix), Effects with the desktop's graphs and
  gauges, Analyse (Spectrum, Signal Generator, Stats), Measure (Measure, Results, Live, Auto-tune, Documents, Several
  units), Library (Presets, Snapshots, History with Undo and Redo, Files, AutoEQ), System (Units, Clock and Links,
  Installation, Events), Devices, Settings (by group, with search) and Control Surfaces.
- Ctrl+K (Cmd+K on a Mac) opens the command palette; a right-click (or the menu key, or Shift+F10) opens a channel's,
  preset slot's, device tab's, band's or the graph's menu; the bell opens the notifications. Settings › Appearance
  places the sections and device tabs and sets the theme; the browser keeps it.
- On a phone's width: 44 px tap targets, device names shortened with an ellipsis, Search in the app menu.
- Settings › Speakers, Detect…, and the stated and detected roles, as on the desktop.
- When an AI agent of a desktop app works on a hub's device: a strip on every screen, a mark on the device's tab, its
  test-signal permission with the time left, and the screen following its work.
- A number field writes only what you typed, and reads a decimal comma, "−" and a unit. The installer's limits and
  the signal generator's decimal fields are text fields.
- Not yet on the web: Settings › Updates, Networking, AI Agents and This Computer; Devices › Hubs and Firmware; the
  EQ's phase view; the crossfeed pair map.

### Android
Changes since 0.14.0 (versionCode 14 on Google Play):
- The volume buttons change the DSPi's User Volume while the app is in front and a DSPi is the current device: 1 dB a
  press, held to repeat, −60 to 0 dB; the mute button toggles its mute. Settings › Advanced › Volume Buttons turns it
  off (on by default). Otherwise the buttons change the phone's volume as before.
- DSPi Link from the main page: with no DSPi connected, the strip says so and offers **Connect over DSPi Link** and
  **Try the demo**; the device list always offers "DSPi Link…". DSPi Link stays off until switched on.
- Reconnect to the last hub at app start (Settings › Networking, on by default), and a faster DSPi Link start,
  above all to a DSPi bridge (fewer round trips; start-up reads no longer hold up a tap).
- An unencrypted DSPi Link connection is amber instead of red, still "(unsecured)"; "I understand, don't highlight this
  hub" gives it the normal colours, per hub, undone in the info dialog or Settings › Networking › Acknowledged
  Unsecured Hubs. A long name no longer hides "(unsecured)" on a phone.
- Settings › Networking, the privacy policy and the store description say which DSPi Link connections are encrypted
  (to a uConsole for DSPi hub) and which are not (to Weeb Labs' DSPi Console or a DSPi bridge).
- The connection strip is thinner (36 dp) and in the app's own colours; the accent blue is a shade per theme readable
  as text (WCAG AA); no Material lavender or pink remains.
- Unused outputs are left out of Home, its meters, the Spectrum chips and Stats.
- A number field and the compact editor write only what you typed.
- `android.hardware.wifi` is no longer required (Wi-Fi is only for DSPi Link), and the in-app privacy link points at
  this repository.

### Terminal
- `uconsole mcp`: the AI agents' stdio relay to the running desktop app, one token per agent program (`--port`).
- Unused outputs are left out of the Overview grid, the Spectrum chooser and Stats; the side bar names them "None".
- `uconsole measure` takes the roles from the device's speaker setup.

### Hub and DSPi Link
- The installation keeps sources, zone sources (they route: the units' slots follow), scenes with a `source` action,
  and programme sources; the hub checks them as it checks the rest.
- The hub tells every console when an AI agent works on one of its devices.
- A hub once met encrypted is dialled encrypted only, until it is paired again or forgotten; a hub known to answer
  without encryption is dialled without the TLS try first.

### Fixes
- **A user's report on 0.14.0**: a number typed into a gain, delay or filter field went back to the old value when you
  clicked another field after Enter (the main window and the matrix): now Enter and leaving both keep what you typed,
  Escape drops it, and a decimal comma or point, "−" and a unit are read. The preset box showed the first preset while
  another was active (after a change, Save, Copy to, Rename), and the source box after a rename.
- Number fields in every client write only what you typed; tabbing through a field no longer writes its rounded text
  back.
- **Preset copy**: the web console's Copy to… on a stored preset overwrote that preset with the live settings. Copy
  is from the active preset only, and the core refuses any other source.
- Tool windows no longer show a scroll bar for content that fits (Headphone Crossfeed).
- The core names the crossfeed preset "Jan Meier", as the desktop app does.
- Terminal: Preset › Copy to… could overwrite preset 1 when the device did not say which preset was active. Copy to…,
  Save, Rename, Set as Default and Clear wait until the active preset is known; Copy to… reads it again right before
  its first write and refuses a mismatch by the core's rule. An active slot of 10 or more counts as unknown.
- Speaker detection: a raised floor (an agent's `floor_hz`, the limits profile's high-pass minimum) moved the gate of
  the sweep down to 20 Hz up with it. Now the quiet sweep goes on to 177 Hz first whatever the floor, the sweep to
  20 Hz never goes below the profile's minimum, and an output whose top end was never heard is swept at the quiet
  level. The question, the warning and Settings › AI Agents say exactly that.
- Agents read from 1 the numbers still from 0: `dspi_preset_copy`'s result, the copy refusal and "did not confirm",
  `dspi_get_limits`' outputs, an installation's speaker, upstream and fan-out outputs (taken from 1 too); an unknown
  preset slot reads null, not 256; the guide names the documents that keep their file's 0-based fields.
- Web: Auto-tune and Several units show the core's refusal words; Settings › Speakers names each output once; the
  Spectrum's bars no longer cover their dB scale.
- Desktop: a channel has one name on a page (Delay & Gain, the Crossover table and the Spectrum chooser say "Centre
  woofer" as the Equaliser does; Settings › Speakers, Events and an agent's question "Out 1 (Centre woofer)").
- Events: a connection says how it came ("Connected to the demo", "over USB", "through a hub"), not "over fake".
- The relay, the endpoint's texts, `uconsole mcp` and the AI agents manual name the app "uConsole for DSPi".

### Integrators
- Additive C ABI: `dspi_core_set_*` (one core per open device with one shared hotplug scan), `dspi_mcp_*` (the agents'
  server), `dspi_speaker_layout_*`, `dspi_installation_*`, `dspi_revert_entry`, `dspi_preset_load_notice`,
  `dspi_system_volume_plan`/`_set`, `dspi_measure_rolloff_*`; nothing removed (`crates/dspi-capi/include`).
- The output label carries `unused` and `hidden` (unused and no test signal playing on it; UniFFI record, the C ABI's
  and the web's JSON), an unused output with its default
  name is labelled "N · None", and `output_cards` leave unused outputs out.
- A preset copy whose source is not the active slot is refused as an invalid argument before its token is spent.
- Every history entry has an actor (`id`, `actor`, `actorLabel`); undo by entry id.

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
