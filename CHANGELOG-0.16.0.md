# uConsole for DSPi 0.16.0

Release date: 2026-11-26. One version for every client but the Swift app.

0.16.0 is the first release since 0.14.0 (a 0.15.0 was prepared but not published; everything it had is here). It
changes how the desktop app looks and how far it reaches. The desktop app has a second layout, **Modern**, which you
can switch with the Classic one at any time and which opens several DSPi units side by side; the desktop app and the
web console share one design language. An **AI agent** on your computer (Claude Desktop, Claude Code, Cursor or
another MCP client) can work with your DSPi: read it, change it, measure it and tune it, under rules it cannot switch
off that protect your speakers.

And it is about everyday use: the app builds your crossovers and routes from the speakers you confirmed, in one step
that one Undo takes back; A/B in the top bar compares what is saved on the device with what you hear now; the
Overview lists what is left to set up, every device tab says when it has unsaved changes, and Simple mode and a DSPi
locked by its installer open on **Home**, a remote with volume, mute, source and presets. Outputs are called by your
speakers' names, and the demo is a tuned two-way speaker.

This release carries the desktop app for Windows, Linux and macOS, Android (versionCode 16), the Terminal
(`uconsole`), the web console and the hub. The Swift app for macOS, iPhone and iPad is still **not** part of it; on
a Mac, the desktop app is the Qt app, as a `.dmg`.

The clients work with stock DSPi firmware from Weeb Labs. Features marked "fork firmware" need the DSPi fork
firmware and are hidden otherwise. The firmware has not changed since 0.14.0: release
[v1.1.6-bofr1](https://github.com/Bofr5555/DSPi/releases/tag/v1.1.6-bofr1) is still the one to use (images for
RP2040, RP2350, the K340 and the alink sender; which image for which board:
[`docs/INSTALL.md`](https://github.com/Bofr5555/dspi-unified-console/blob/main/docs/INSTALL.md#fork-firmware)).

Licence: GPL-3.0-only for the program as a whole (Bofr5555's contributions GPL-3.0-or-later with additional terms
under section 7), shown in every client's About. The notices are drafts until approved.

**Please read [What was tested, and where](#what-was-tested-and-where) before you let an AI agent near real
speakers.** The agents' safety rules have been tested at length, but only against simulated speakers so far, and
nothing new in this release has run on a real DSPi yet.

## The Modern layout, switchable with Classic (desktop)

- **Modern**: one window with a rail of sections on the left (Overview, Equaliser, Routing, Effects, Measure,
  Analyse, Library, System, Devices, Settings). Every tool is a page with tabs, and any page can be opened in a
  window of its own. The open device is a tab at the top. The volume, the meters, Mute and Bypass EQ are in a dock
  along the bottom. There is no menu bar: each action is where it acts (right-click a channel, a band, the graph or
  a preset), and every keyboard shortcut still works.
- **Ctrl+K** (or Search in the top bar) opens a command palette: type part of a name to find a command, a page, a
  channel, a setting or another open device.
- The **Equaliser** has three pages: PEQ (the channel's controls in one strip over the graph), Crossover (every
  output's curve over one table of high and low passes) and Delay & Gain (every output's gain, delay, distance,
  arrival, polarity, mute and limiter). An inspector on the right shows the selected band and channel.
- The **Overview** has one card per channel: its speaker's name and role, a small response curve, its level.
- States that limit what you can do show in a strip under the top bar; other notices are behind the bell.
- **Classic** is the layout of earlier versions (menu bar, tool windows) and looks as it did.
- **Settings › Appearance** switches between them at any time (in Modern also the uConsole menu: "Switch to the
  Classic Layout"); the device, the selected channel and open tool windows stay. It also sets the theme (the
  system's, light or dark) and, for Modern, a darker grey and whether the inspector shows.
- **New installs start in Modern.** An install that finished its Getting Started keeps Classic; one whose Getting
  Started was never finished counts as new.

## One design language (desktop and web)

Modern's screens and the web console follow one set of rules (written down in `docs/DESIGN-LANGUAGE.md`): surfaces
are told apart by their tone rather than by outlines; blue is kept for what is on, what is chosen and the one main
action on a page; forms sit at the left; empty pages, notices, popovers and graph axes look the same everywhere.
Classic's colours are unchanged.

## A new matrix mixer

The matrix mixer is redrawn, on the Routing page and in its window (both layouts) and in the web console:

- A crosspoint in use is a tile with its gain and polarity; an empty one is a dot. Each output's column shows its
  gain, delay and what feeds it, with Enabled and Mute in its header; headers stay in view while you scroll.
- The keyboard does everything: arrows, Home/End, Page Up/Down, Space, Enter or digits to type a gain ("-6" is
  −6 dB), + and −, I (invert), M (mute), E (enable), Delete, and a menu per cell.
- **Link L/R pairs** (off by default, kept per device) mirrors an edit to the pair's other crosspoint, as one undo
  step. **Direct 1:1**, **Clear** (asks first past four crosspoints) and a **Dense** view for many outputs.
- The Routing page's inspector follows the grid: the crosspoint, the output or the input you are on.

## Presets in one popover (Modern and web)

The preset name in the top bar is one button. It opens every slot as a list, with its state (Active, unsaved,
Startup) and Recall, Save, Copy Here and the slot's menu. Up and Down load the next slot. It replaces the preset box
and its separate button.

## Several devices at once

- In Modern, a DSPi you choose (on USB, through a hub, or the demo) opens in a **tab of its own** beside the
  others. Each tab keeps its own edits, pages, undo history, drafts and running measurement. Closing a tab asks
  about that device's unsaved changes; quitting asks about each.
- Classic still shows one device; the others stay open and are listed under Open devices.
- Choosing another device first stops what plays on the one you leave (a signal generator fades out, a measurement
  stops, an auto-tune not yet kept goes back).
- The web console has device tabs too. A DSPi is open once: a second way to the same unit (USB and a hub, or two
  hubs) shows the tab that is already open.
- Measuring several units uses the tabs that are open instead of a second connection to them.
- With more than one device open, the dock offers **All devices**: volume, mute and preset recall then act on every
  open device, each within its own limits and keeping its own offset. Home's volume follows the link too.

## Your speaker setup

- **Settings › Speakers** (and a Speakers step in Getting Started) says which output drives what: full range, sub,
  woofer, midrange or tweeter. It is suggested from the device's filters and routing, and you **Confirm** it. The
  web console has the same page; the browser keeps it per DSPi.
- **Detect…** finds it for you with the measurement microphone: quiet test sweeps on each output, at most −24 dBFS
  above 1 kHz, and below 1 kHz only on an output that is clearly no tweeter, at most −50 dBFS and no lower than
  157 Hz; only a driver still playing flat at 177 Hz, as no tweeter does, is then swept down to 20 Hz, at most
  −24 dBFS. A raised floor (your limits profile's) never starts that last step sooner. **Stop** keeps what
  was found. A detected setup is marked "Detected, not confirmed" until you confirm or correct it, each row with
  its evidence. A full-range driver or a small dome midrange is detected as a tweeter: correct those rows.
- Measurement starts from this setup, and holds every output to the strictest of the role asked for, your setup's
  and what its filters show: a tweeter is never played below 1 kHz, also before it has a crossover.
- Loading a preset yourself is guarded when the setup has a tweeter or midrange (and the master volume is kept
  apart from presets, the default): the device is silent while the preset loads, a tweeter or midrange that the
  preset leaves without its high-pass is muted, and a notice says so.

## Crossovers and routes from your speakers (desktop)

- Once **Settings › Speakers** is confirmed, **Equaliser › Crossover › From Speaker Layout…** opens one sheet with
  each speaker's transitions ("woofer ◂▸ tweeter"; a three-way has two; a sub its low-pass), a frequency and a slope
  for each, and **Same as Left** for a speaker shaped like an earlier one. It starts from what the outputs already
  have. **Apply** writes every output's high- and low-pass in one step, and **one Undo** puts them all back.
- **Routing › From Speaker Layout…** routes each output from its speaker's side (L, R, centre, sub, surrounds, by the
  inputs' names). A centre or sub fed from both sides gets each side 6 dB below what fed it before (at most
  −6 dB), so it never plays louder than it did.
- **A tweeter is protected on the way:** its high-pass never goes below 1 kHz at 12 dB/octave (or the installer's
  higher limit); a lower value is refused with the reason, and a route to a tweeter or mid driver that has no
  high-pass is refused unless the same Apply sets its crossover.
- Settings › Speakers offers **Crossovers and Routing…** once the layout is confirmed, and so does the first run's
  Speakers step.

## A/B: as saved against now (desktop and web)

- The top bar's **A | B** compares the preset **as saved on the device** (A) with **what you hear now** (B), from your
  first edit: click A, or press **B**. No snapshot to make first. Library › Snapshots keeps the compare of any two.
- A toggle never jumps the volume: the listening volume, mute and the master volume stay as they are on both sides.
- While you listen to A, your edits on B still count as **unsaved**: switching preset, closing the tab, quitting or
  a linked recall asks first, and Save to Device saves B.
- If the saved preset leaves a tweeter without its high-pass, A holds that tweeter muted, as loading that preset
  would.

## Knowing where you are (desktop)

- The **Overview** has a setup list for the device shown, "Set up this DSPi · 2 of 5 done": speakers confirmed,
  inputs routed, crossovers set, measured, saved to the device, each with its one action.
- Each **device tab** shows a dot and "N unsaved" when it has unsaved changes; the count's tooltip lists them, and
  the Save question names the device, the preset slot and its name. **Save All…** saves every open device with
  unsaved changes after one question. **Factory Reset** left the Save menu; it is in a device tab's menu and in
  Devices › This Computer.
- After an auto-tune or a trial is kept, a notice says it is not saved on the device yet, with **Save to Device**.
- Outputs are named as your speakers everywhere in Modern: "Left woofer · 1" in the Crossover and Delay & Gain
  tables, the matrix, the Routing inspector and the Installer page. The command palette finds them by "tweeter" or
  "left". Outputs nothing plays to fold under one "N unused" row; one of your speakers, one with a crossover or EQ
  band, and one a test signal plays on never fold.
- **Settings** opens on Device › Speakers; developer facts moved to Advanced. **Help › Keyboard Shortcuts** lists the
  keys that move between the top bar, the sections, the page, the inspector and the dock (F6).
- **Measure and Auto-tune** keep their actions in a bar that is always in view (Keep, Revert, Verify, Stop, Next
  Position), and a refused start shows one row per reason with its fix ("Lower to −12 dB", "Open the Crossover").
- One speaker table, edited in one place (Settings › Speakers and the first run); Measure and Auto-tune show it in
  one line with **Edit in Speakers…**.
- The first run cannot dead-end: you can pass the board step without a board, the Speakers step offers **Later**,
  and the Welcome page opens the demo.
- **Settings › Appearance › Accessibility**: **Outline controls** (a line round every field and button) and
  **Control size** (Comfortable on a touch screen). Off and Automatic by default: with a mouse nothing changes.

## Home: a remote for listening, and the installer's lock (desktop, web, Android)

- **Simple mode** opens on **Overview › Home**: the device, one big volume with Mute, the source, the presets as a
  Recall list, and one health line. Simple's dock has no master volume and no Bypass EQ.
- A DSPi **locked by its installer** opens on Home too. In Modern, on the web and on Android nothing the lock refuses
  is offered: the other pages are read-only with a notice and Go to Home, and Save, Undo, the shortcuts and palette
  rows that would write are off. The locked user may change the **volume, mute, source and presets**.
- Under the lock the **speaker layout is the installer's**: the roles the tweeter guards trust cannot be changed
  until the installer unlocks, on every client.
- The Classic layout is unchanged under the lock: it stays live, and what the lock covers is refused after the click.

## Solo an output (desktop and web)

- **Solo** in the Routing inspector and in an output's menu plays only that output: the others are muted on the
  device until **End Solo** (in the strip that says so). A solo never unmutes anything and never reaches a preset,
  a snapshot or a save: it ends before anything is saved, loaded or measured, and before the device is closed. A web
  page that is closed or reloaded during a solo asks first and ends the solo as it goes. If the device or its DSPi
  Link connection is lost during a solo, the app lifts the solo's mutes when it connects to that DSPi again, before
  it looks at what is unsaved.

## A demo that shows what DSPi does

- The demo is now a tuned two-way speaker: a crossover at 2.2 kHz, a room-mode cut, a few EQ bands, the tweeters a
  little quieter and later, outputs named "Left woofer" … "Right tweeter", and a confirmed speaker layout. Nothing
  is unsaved when it opens. **Reset Demo** (desktop, web and Android) starts it again.

## AI agents (desktop app)

An AI agent on this computer can read your devices and change them for you: "make the left woofer 2 dB quieter",
"why does output 3 sound thin?", "measure the speakers and set the crossovers". It talks to the desktop app over MCP
(the Model Context Protocol). You see every change live, as if you had made it, and you can undo each one. The
manual page, [`docs/manual/ai-agents.md`](docs/manual/ai-agents.md), has the configuration for Claude Desktop,
Claude Code, Cursor and VS Code.

- **Off until you turn it on** (Settings › AI Agents). The app listens on this computer only; nothing on your
  network can reach it. From another computer, go through SSH (`docs/mcp-ssh-forwarding.md`).
- **Each agent is paired once**, with a code you compare, or with a token you make for it. Remove a client and its
  token stops at once.
- An agent starts the app with `--mcp`, or the Terminal's `uconsole mcp`; either one relays to the desktop app you
  have open, which must be running.
- **You see what it does**: a strip under the top bar names the agent, what it does now and on which device, with
  **Pause** and **Stop**. A change it makes is marked for a moment, with a notice and **Undo**; the History tags it
  **AI**. The screen follows its work to the device and page it changes, never while you drag, type or have a
  dialog open.
- **It is told when you step in**: what you changed, undid or stopped. A control you take back is refused to it
  until it asks; a setting you change while its measurement runs stops the measurement.
- **What it may do**: the equaliser, crossovers, gain, delay, polarity, mute, routing, effects, volume and source,
  presets and snapshots, A/B, files as text, AutoEQ, names, a hub's installation and a room's or zone's volume; it
  can ask the app to open a device the app knows. It reads, but never changes, this computer's sound settings: it
  can tell you whether the DSPi is the default output and which apps play to it.
- An agent that runs on a cloud service sends what it reads there, as with anything you show it.

### Questions in the screen's words, and connecting

- **Questions in the screen's words**: "Claude Code wants to make Right woofer louder (Demo 1)", each change as
  "Right woofer: gain −5.0 dB → −2.0 dB", the changes it asks about marked ▲, and an honest last line: a save on the
  device says Undo cannot take it back. An output the agent named itself is shown with its number ("Sub (quieter) ·
  Out 3").
- **Questions raised together are one sheet**: one agent's questions within about 1.5 s come as one sheet, grouped by
  device, with one Allow that answers exactly the rows on the sheet: a sheet takes another question only while
  everything fits its 12 rows, and the rest is the next sheet. Up to 8 of an agent's questions may wait (was 3).
- **Refusals you can see**: when the app refuses an agent ("Kept Right tweeter's high-pass at 1 kHz"), the agent
  strip says so for a moment. **Stop Agents** is the one Stop; Disconnect no longer forgets a client.
- The strip says when an agent's work is **not saved** on the device yet, with Save.
- Connecting: a three-step manual page, a pairing code you can compare in the app and in the agent's chat, and
  Settings › AI Agents › **Install in Claude Desktop…**, which writes an extension file for Claude Desktop (see
  [Known limits](#known-limits)). `dspi_get_guide` gives every agent the same guide.

### What protects your speakers

These hold whatever an agent is told, and the agent cannot change them:

- **Questions.** Under the default, **About risky changes**, the agent waits for your answer before it makes
  anything louder (a gain, a volume, an unmute, a route, an effect that adds level), loads a preset, writes the
  device's memory, changes the input source, weakens a tweeter's protection, plays a test signal, or changes every
  unit of an installation. A question about test signals is asked once per device: **Allow** lets that agent
  measure there for 10 minutes, with **Revoke** at any time. You can also have every change asked about, or none.
- **The tweeter floor.** An output that your speaker setup calls a tweeter (confirmed, detected, or set from your
  words), and any output whose high-pass is at 1 kHz or above, must keep a high-pass of at least 1 kHz (or your
  limits profile's higher corner), 12 dB/octave or steeper, whenever it can play. A midrange is held the same way
  at 80 Hz. Every change is judged on what it leaves, under every setting, "never ask" included. A tweeter you left
  playing without its high-pass may be turned down by an agent, never up.
- **Never, whatever the setting**: firmware updates, the bootloader or a factory reset; the limits profile, the
  installer PIN or lock; hardware and core-voltage settings; the signal generator; pairing and DSPi Link security;
  deleting presets, snapshots or measurements; raw device commands; files by path.
- **Your limits profile** holds for agents as for everyone, and an installer's lock tells the agent what still
  passes, never the PIN.
- **Presets.** An agent's preset load is silent while the app checks the result, and is put back if it would leave a
  tweeter unprotected. On a device that saves the master volume with each preset, an agent may not load presets.

### Speaker roles from your words, and detection by an agent

- Tell an agent "L and R front speakers, three way, tweeter, mid, woofer, left then right, outputs 1–6", and it can
  set that table. You see the whole table in one question, with your words quoted. An output your words did not
  name is treated as a tweeter. The roles protect at once; Settings › Speakers says "Set by AI from your words, not
  checked" until you confirm them there.
- **Trust but check**: before a test signal plays below 1 kHz on an output your words made a midrange, woofer or
  sub, the app checks its top end. If it behaves like a tweeter, nothing below 1 kHz plays there and the agent tells
  you. (This check covers test signals; music is routed as you said.)
- An agent can ask to **detect** the speakers on a device nobody has set up, with the same quiet sweeps and one
  question with a warning. It never replaces a setup you confirmed.
- Under "never ask", both need their own switch in Settings › AI Agents, off by default.

### Measuring and auto-tune by an agent

- An agent can check and start a measurement, find where an output's response falls off at the low end, propose an
  auto-tune from a measurement, apply it and verify it, and measure several units together.
- The app does the measuring, with your microphone, in its own **Measure** page, which says "Requested by AI ·
  name". A person has to put the microphone where the agent says. The result is filed with your measurements.
- Roles always come from your speaker setup; without one, the agent is told to ask you (or to set roles from your
  words, or to detect them).
- **How loud**: an agent's measurement is capped at 85 dB SPL by default (Settings › AI Agents; 60 to 95), which
  needs a microphone calibrated in dB SPL, and never above −12 dBFS at a speaker (−30 dBFS on headphones). An agent
  may lower these limits, never raise them.
- An agent keeps a tune only after a verify that passed; any other tune only you can keep.

## Installation setup on a hub (desktop and web)

- **System › Installation** (Window > System… in Classic; the web console's System screen) sets up a hub's
  installation: the units the hub sees and has not placed yet (**Join**, with name, room, role and what feeds it),
  rooms, zones and groups, senders and fan-outs. The hub's own checks show on the field as you edit; Save lists what
  it changes and needs the hub's Admin role. If someone else saved meanwhile, nothing is overwritten, and **Make Them
  Again** redoes your changes on the new version.
- For the audio link (fork firmware): the sources (a sender's channels), what each zone plays (every unit of the
  zone takes the source's channels as its slots), a unit's own slots, scenes that switch a zone's source, and where
  the programme comes from. After Save, **Align** makes the units play the new channels.
- The System view sets the volume of a room or a zone, unit by unit, and asks first when a unit would get louder.
- Settings › This Computer (desktop) shows whether each open DSPi is one of this computer's sound outputs, the
  default or not, and what plays to it.

## The web console: the same pages as the desktop's Modern layout

The web console now has the Modern layout (its only one) and lays out every page as the desktop's Modern does: the
top bar with device tabs, the preset popover, Undo, Redo and Save to Device; the rail of sections; the dock with
the input source, volume, Mute, Bypass and meters. The Equaliser (PEQ, Crossover, Delay & Gain with the inspector),
Effects with their graphs, Analyse › Spectrum, Measure and Auto-tune, Library (Presets, Snapshots, History, Files,
AutoEQ), System (Units, Clock and Links, Installation, Events), Devices and Settings use the desktop's words and
groups. Ctrl+K (Cmd+K on a Mac) opens the command palette, a right-click opens a menu, and the bell opens the
notifications. When an AI agent works on a hub's device through a desktop app, the web console shows it in a strip
and on the device's tab.

Not on the web yet: Settings › Updates, Networking, AI Agents and This Computer; the Devices page's Hubs and
Firmware tabs. AutoEQ lists headphones once you search.

- Device tabs you can tell apart (name, then where it is), a Start page that says what each Connect does, skip
  links, and a rail with one Tab stop.
- **Recovery that tells the truth**: while a device is away the page is fenced and says so; a hub that answers as a
  different hub at the same address is named; the notice goes when the device is back.
- A hub-served page starts on **Devices › Fleet**: every DSPi of the hub with its state, alarms and firmware,
  Identify and Open. The hub page has parts (Devices, Alarms, Updates, Groups, Hub) and one table.
- **Snapshots survive a reload**: they are kept in this browser per DSPi; Export… still gives a file.
- The EQ graph shows the **phase** (Settings › Graphing), Crossfeed has Qt's **output-pair picker**, and an output's
  **polarity** can be inverted from the Routing inspector.
- The Crossover opens on an output and says which outputs are off; the filters use one vocabulary (Bell, Q, Low
  pass, High pass).
- Add to Home Screen installs the console as an app.

## Outputs nothing plays to are out of the way

An output that no input is routed to is left out of the views that only show: the meters, the Overview, the
spectrum, Stats and Measure's live view, in the desktop app, the web console, Android (Home, Spectrum, Stats) and
the Terminal (Overview, Spectrum, Stats). Where it must show (the matrix, Delay & Gain, output choosers, Speakers) it
is named "3 · None" unless you named it. When another console or a control surface starts the signal generator, every output of that device is shown while
it plays.

## Android

Changes since 0.14.0:

- **The volume buttons** set the DSPi's User Volume while the app is in front and a DSPi is connected: 1 dB a
  press, held to repeat; the mute button mutes it. Settings › Advanced › Volume Buttons turns this off; with no
  DSPi, the buttons work as usual.
- **DSPi Link from the main page**: with no DSPi connected, the strip offers **Connect over DSPi Link** and **Try
  the demo**, and the device list always offers "DSPi Link…".
- **Reconnects to the last hub** when the app starts (Settings › Networking, on by default), and DSPi Link starts
  faster, especially to a DSPi bridge. A hub that was once encrypted is only ever reached encrypted.
- An unencrypted DSPi Link connection is marked in **amber** instead of red, and you can stop highlighting a hub you
  trust ("I understand, don't highlight this hub"; undo it in Settings › Networking). Settings › Networking, the
  privacy policy and the store text now say which hubs are encrypted.
- The app's own colours instead of Material's defaults, readable in light and dark; a thinner connection strip.
- Unused outputs are left out of Home, Spectrum and Stats (above).
- A number field writes only what you typed.
- The app no longer requires Wi-Fi hardware to be installed (it is needed only for DSPi Link).
- The in-app privacy policy link points at this repository.
- **Mute** beside the volume on Home; the notification's steps are 1 dB.
- A tap the installer's lock refuses says why, with **Unlock…**.
- **Settings › Speakers** and a Speakers step in Getting Started: the role and side of each output, the tweeter's
  floor, Confirm. Under the lock it is read-only.
- A touch **band editor** (a sheet with − and + per value), the crossover in two rows above the bands with **Same for
  the right side**, legend pills that choose the channel.
- Notices when the USB DSPi goes away and comes back; Exit demo and Allow USB access are buttons on the bar.
- **Phones open in Advanced** (as do iPhone and iPad, whose app is not in this release).

Measurement, AI agents and the Modern layout are not on Android.

## Terminal (`uconsole`)

- `uconsole mcp` relays an AI agent to the desktop app (one token per agent program; `--port` for another port).
- Unused outputs are left out of the Overview, Spectrum and Stats, and named "None" in the side bar.
- `uconsole measure` takes the roles from the device's speaker setup.
- **Plain mode** (`uconsole --plain`, or `TERM=dumb`): the interface as sentences, one command per line, for screen
  readers and scripts. Piped, every line runs, nothing waits for an answer (an `ab` or a guarded recall keeps), and
  the exit code is the first failure's; `--plain` with a command on the line is refused.
- Ctrl-P search finds words ("crossover", "mute"); `uconsole` with nothing plugged in waits for a DSPi and offers the
  demo and DSPi Link; the `:` line runs `snapshot`, `ab`, `diff`, `lock`, `limits` and `status`; numbers take `k`
  (`2k`).
- Tab goes channels → page → footer; Enter on a route edits its gain and never connects; `[` `]` `{` `}` and `m` set
  the volume and mute from any page; unused outputs fold.

## Hub and DSPi Link

- The hub keeps the installation's sources, zone sources and programme sources; a scene can switch a zone's source,
  and the hub then writes the zone's units' slots at once.
- An AI agent's work on a hub's device is told to the hub, so the other consoles show it.
- A hub that was once met encrypted is never dialled in plaintext again (until you pair it again or forget it).
- An installer lock the hub holds lets the input source through, as the console's lock does.

## One word for one thing (every client)

- **Save to Device** everywhere (Classic's menu said Commit Parameters). Filters say **Bell** (not Peaking), **Q**,
  and **Low pass** / **High pass**. The unsaved list says "Out 1 · L gain". A snapshot applied for a while is a
  **trial**. What's New opens once with the app's own news.

## Fixes

- **Reported by a user on 0.14.0** (Windows, Classic layout):
  - A number typed into a gain, delay or filter field went back to the old value when you clicked another field
    after Enter, in the main window and the matrix mixer: the field showed the old value while the device had the
    new one, and leaving the field wrote the old one back. Now Enter and leaving both keep what you typed, Escape
    drops it, and a number may have a decimal comma or point, the minus sign "−" and its unit ("-3,5 dB").
  - The preset box showed the first preset while another was active, after a change, after Save, Copy to and
    Rename; the source box likewise after a rename.
  - From the same user: clicking or tabbing into a number selects it, so typing replaces it; the "Filter Response"
    heading is gone; an output's controls are one row on a wide window; the main window fits a 1366×768 screen.
- **Number fields everywhere** (desktop, web, Android, the matrix tiles) write only what you typed. Before, tabbing
  through a field could write its rounded text back (a 4.1234 ms delay became 4.1108 ms).
- **Preset copy**: the web console offered "Copy to…" on every stored preset and overwrote that preset with the
  live settings. Copy now works from the active preset only, in every client, and the core refuses anything else.
- Tool windows no longer show a scroll bar for content that fits (Headphone Crossfeed, for one); each opens at the
  size its content needs.
- Quitting asks first while a firmware update or core-voltage calibration runs on any open device.
- A name with markup in it (of a device, hub, room or channel) is shown as written.
- The Interrupt Monitor clears with Ctrl+L (Ctrl+K is the command palette).
- On a Linux desktop Qt has no theme for, the file dialogs are the system's.
- Measurement refusals, device events and History rows are in plain words; a measurement is one History row.
- Graph axes read "0 dB", not "+0", and thin their labels evenly.
- Two reviews before this release found problems in the new features (A/B, agents' questions, the lock, Solo,
  From Speaker Layout and others); they are fixed, and `CHANGELOG.md` lists them.

## What was tested, and where

- **Every client** is tested automatically against simulated DSPi devices, a simulated room and microphone, fakes
  of both firmware generations and a fake hub: about 8,700 checks of the desktop app's screens in both layouts, the
  web console in Chromium and Firefox (also at a phone's width), Android's unit tests, and the MCP server against
  the official MCP conformance suite.
- **AI agents** were tested with a real AI agent (Claude Code), given only the MCP tools, driving the real desktop
  app against a **simulated home theatre**: three DSPi units, twelve drivers, a room and a noisy microphone. The
  stories: measuring and tuning from scratch, "make the tweeters go lower", a sub on another device, undo while you
  hold a control, presets and snapshots with every change asked, a whole evening of requests, an installer's lock, a
  preset that would expose a tweeter, roles from the user's words (with a wiring trap), and speaker detection
  (with traps: a 700 Hz tweeter, a dome midrange, unwired outputs, a swapped pair). The simulator watched every
  driver: in the first five full runs (807 test-signal starts), no tweeter was played below 1 kHz, nothing played
  on an output that drives nothing, and every level stayed far below its cap.
- **Speaker detection of the Koss K340** headphone amplifier was run against a **simulated** K340, not the real
  one.
- **Real hardware**: none of the new features since 0.14.0 have run on a real DSPi yet. The real K340 on USB was last
  checked for 0.14.0 (Linux, Windows, and an Intel Mac). Android 0.16.0 has not run on a phone (its new features
  are tested in unit tests). An agent starting the app on Windows over stdio (Claude Desktop) has not been tried
  on a real Windows. Reading this computer's sound settings has run on Linux only; on Windows and macOS it is built
  but untried.
- **The everyday-use features** (From Speaker Layout, A/B as saved, the setup list, Save All, Home, the installer's
  lock, a fresh install, the product demo) were walked in the real desktop app against the same simulated home
  theatre, and the recorded agent stories were replayed against it. The full CI ran green on the release commit.
- The Claude Desktop extension (Install in Claude Desktop…) has **not** been installed in a real Claude Desktop. The
  iPhone and iPad defaults changed in code, but the Swift app's tests have not run (it is not in this release).

So for a first session with an agent on real speakers: set a limits profile, keep the volume low, confirm your
speaker setup first, and keep the default questions.

## Known limits

- **Install in Claude Desktop…** writes an extension that has never been tried in a real Claude Desktop, and it is
  not signed. If Claude Desktop does not take it, use Copy JSON in Settings › AI Agents and the manual's steps.
  After an AppImage moves, make the extension again.
- An agent that starts the app with `--mcp` (Claude Desktop, Claude Code) handles one request at a time: while a
  question waits for you, its other requests wait too, and its questions never come together as one sheet (an agent
  over HTTP gets one sheet).
- From Speaker Layout and the Home page are on the desktop (Home also on the web and Android); the web and Android
  have no From Speaker Layout yet.
- The Terminal's A/B is "now against a snapshot", not "as saved".
- If the desktop app is killed during a solo, the outputs it muted stay muted on the device (not saved) until you
  unmute them; a web page's solo ends on a reload only as far as the browser lets it finish. Another console on the
  same hub sees a lost solo's mutes until this one reconnects.
- Agents run only in the desktop app. Measurement and the Modern layout are not on Android; the Swift apps are still
  deferred. Not yet: Web MIDI; Apple Developer ID and Windows code signing; Apple silicon has never run the Mac app.
- An agent on another computer needs SSH forwarding.
- Auto-tune with the "house" target can cut a tweeter's level more than needed; look at the proposal before you
  apply it.
- The detection takes a full-range driver or a small dome midrange for a tweeter, and does not probe an output that
  is off or muted.
- Roles from your words are checked before test signals only; music is routed as you stated it. Confirm the roles
  once the measurements look right.
- Installation setup, a room's or zone's volume and showing an agent's work to other consoles need a hub.

## Downloads

| System | File |
|---|---|
| Windows 10/11 (x86_64) | `uConsole-for-DSPi-0.16.0-windows-x86_64-setup.exe`, or the portable `uConsole-for-DSPi-0.16.0-windows-x86_64.zip` |
| macOS 12 or newer (Intel and Apple silicon) | `uConsole-for-DSPi-0.16.0-macos.dmg` (not notarised: open it once by hand, `docs/INSTALL.md`) |
| Debian 13, Ubuntu 25.04 or newer | `uconsole-for-dspi_0.16.0_amd64.deb` |
| Fedora 43, 44 | `uconsole-for-dspi-0.16.0-1.fc43.x86_64.rpm`, `uconsole-for-dspi-0.16.0-1.fc44.x86_64.rpm` |
| Other Linux (x86_64, glibc 2.39+) | `uConsole-for-DSPi-0.16.0-x86_64.AppImage`, or `uConsole-for-DSPi-0.16.0-linux-x86_64.tar.gz` with an installer |
| Arch Linux | the AUR package `uconsole-for-dspi` |
| Android 8+ | Google Play, or `uConsole-for-DSPi-0.16.0.apk` |
| Terminal (`uconsole`) | `uConsole-for-DSPi-0.16.0-terminal-PLATFORM.tar.gz` / `.zip` |
| Web console | `uConsole-for-DSPi-0.16.0-web.tar.gz` |

iPhone and iPad: not in this release.

The Windows files, the AppImage, the tar.gz and the `.dmg` check this project's releases for updates when that is
switched on (Settings › Updates); the `.deb`, the `.rpm` and the AUR package are updated by their package managers.
None of the files are code-signed: Windows SmartScreen warns the first time the app is opened, and macOS refuses the
app until it is opened once by hand.

Every file of this release is listed in `SHA256SUMS`, which is signed: `SHA256SUMS.minisig`. The source code of
exactly this release is `uConsole-for-DSPi-0.16.0-source.tar.gz`; the Qt packages' (Windows, Linux and macOS) is
`uConsole-for-DSPi-0.16.0-desktop-source.tar.gz`. Licence: see `NOTICE.md`. How to install each file:
`docs/INSTALL.md`.

## Check a download

The public key that signs every release:

```
RWTiNwy5h2MBuviJDs7LBs3wi2Bh/YdJyh5pXdS3XKo8wRt1igYdrGgY
```

It is also in the repository as
[`docs/release/minisign.pub`](https://github.com/Bofr5555/dspi-unified-console/blob/main/docs/release/minisign.pub).
Compare the two before you trust either.

1. Download `SHA256SUMS`, `SHA256SUMS.minisig` and the files you want into one folder.
2. Check the signature with [minisign](https://jedisct1.github.io/minisign/) (packaged for most systems:
   `apt install minisign`, `dnf install minisign`, `brew install minisign`, `scoop install minisign`):

   ```sh
   minisign -Vm SHA256SUMS -P RWTiNwy5h2MBuviJDs7LBs3wi2Bh/YdJyh5pXdS3XKo8wRt1igYdrGgY
   ```

   It must say `Signature and comment signature verified`.
3. Check the files against the signed list:

   - Linux: `sha256sum --ignore-missing -c SHA256SUMS`
   - macOS: `shasum -a 256 --ignore-missing -c SHA256SUMS`
   - Windows (PowerShell): `Get-FileHash .\FILE -Algorithm SHA256`, and compare the hash with the file's line in
     `SHA256SUMS`.

   Each file you downloaded must say `OK`.

A file that fails either check is not this release: do not run it.
