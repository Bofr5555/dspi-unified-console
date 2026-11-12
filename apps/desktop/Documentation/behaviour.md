# The desktop console: behaviour reference

This document describes how the Linux and Windows desktop console behaves where the Mac console
(`apps/apple`) and `docs/PARITY.md` do not already say it. It is the source for those behaviours when the desktop UI is
rebuilt. Where the Mac app covers a screen, the Mac app is the reference and this file adds only the desktop
differences.

Cite it by section, for example "behaviour.md §6.3".

## 0. How to read this file

- **Observed** means seen on a running build of the current app (offscreen smoke run, or the plain app driven by
  hand under a virtual X server with a scratch home directory, both on 2026-11-11 from `main` at aae7fa97), or in the
  2026-11-09 smoke screenshots.
- **Documented** means taken from `docs/PARITY.md`, `docs/PORTABILITY.md`, `docs/core-merge/HW-QT.md`,
  `docs/core-merge/PLAN-CLIENTS.md` (the QD entries), or the reports of the Windows firewall and discovery work. Those
  behaviours were not seen here, usually because they need Windows, a tray host, a keyring or a USB device.
- **Not observable** marks a fact that neither source shows. The implementer decides, or a later hand check settles it.
- Text in "quotes" is a label as it appears on screen. Labels are quoted only where users, tests or other clients rely
  on the exact word. Everything else here is a description: write the wording from the Mac app, from the core (by key)
  or anew.
- "The core" means `dspi-core` through `dspi_core.h`. When the core supplies a text (link phases, firmware guard,
  core-voltage warnings, refusals, onboarding, output labels), the client shows the core's text and never its own copy.

## 1. Application shell

### 1.1 Identity and start-up

- Window title and product name today: "DSPi uConsole". The executable is `DSPiConsole` (`DSPiConsole.exe` on Windows).
  The rebuilt app takes the window title and every place a text names the product from its one identity constant
  (`AppIdentity`, plan Q15), never from this paragraph: the name may change before packaging.
- Settings organisation and application are `DSPi` and `DSPi Console` (§21).
- `--demo` on the command line opens the demo device at start. Without it, the app starts with whatever USB device
  is present, or with nothing (§1.3).
- First start, when the onboarding flag is unset: the main window shows the Getting Started flow (three steps:
  Welcome, Board, Done) with "Skip Setup" and "Continue". Skipping or finishing sets the flag (§21), so the flow never
  shows again by itself. Help menu entries reopen it and What's New.
- What's New shows once per app version: the version last shown is stored (§21), and a newer version shows it again.

### 1.2 Main window

- Default size 950 × 813 (observed with no saved geometry). Sidebar 270 px wide by default; its width is stored.
- The left column holds, top to bottom: the INPUTS and OUTPUTS channel lists, the quick strip, the Preset / Source /
  volume controls, the C0 and C1 CPU bars, and the device status line (§3).
- The right pane holds the banner stack (§2), the response graph, and the overview or channel detail.
- The app menu opens from a button at the top left. Top level, in order: a header line naming the connected device
  and firmware (or that nothing is connected), Device, File, AutoEQ, Effects, Tools, Help, then Settings with Ctrl+,.
  With no device, File is disabled. Device holds "Devices…", Commit Parameters (Ctrl+S), Revert to Saved, Save Master
  Volume, Save Output Config, Firmware Update…, Factory Reset… (shown in a warning colour). Help holds Getting Started…,
  What's New, and the two GitHub links. The new app's menu bar follows the plan's menu table; this paragraph records
  only what exists today.
- Ctrl+Q quits from the main window (observed). Escape and Return shortcuts of a hidden tool window act only while
  that window has focus (QD36).

### 1.3 Nothing connected

- Sidebar lists are empty, Preset and Source show a dash, the CPU bars show a dash, the status line reads
  "No Devices" in red.
- A banner at the top of the right pane says no DSPi is connected and offers "Open Demo". Choosing it opens the demo
  device in place (QD7). The graph and the filter grid stay visible with default content.
- Settings, while nothing is connected, lists only the Application and Display categories; the System and Control
  categories (device pages) appear when a device is open.
- The demo is never chosen by a scan or a hot-plug event; a rescan keeps an open demo and reads it again (QD7, QD36).
  Leaving the demo for a real device goes through the unsaved-changes question of M42.

### 1.4 Demo device

- Banner while the demo is open: says it is a simulated DSPi and nothing reaches a real device. It has no buttons.
- In the device list the demo is its own row under USB, with a play icon and a one-line description, and a check
  mark when it is the open device.
- When the app's hub is sharing, the hub shares a separate demo of its own; the Networking page says so (QD36 F4,
  §10).

## 2. Banner stack

Today banners sit at the top of the right pane, full width, stacked. Each is a rounded card.

**Placement in the rebuild:** the plan puts the banner host (`BannerHost`) directly under the menu bar, across the
window. That placement wins over today's; only the order and the content below carry over.

Order from the top, as observed when several show at once:

1. Firmware banner: either the mismatch banner (any version other than the expected one; "Update…" and "Hide") or
   the read-only banner (§14; "Update…" for older firmware, "Details…" for newer). Warning colour with a warning icon.
2. Link banner: connection to the hub lost and reconnecting, with "Disconnect" (§6). Warning colour.
3. Demo banner (§1.4), or the nothing-connected banner with "Open Demo" (§1.3). Accent colour, or neutral.
4. Secret-store notice: the hub pairing was saved in a file only this account can read, because no secret store
   answered (§7). Neutral, with "OK" to dismiss.
5. Update banner: a newer release exists, with "Details" (opens Settings > Updates) and "Later" (§17). Accent
   colour.
6. Delay-refusal banner (§15). Its place in the stack is **not observable** here (no screenshot shows it); put it
   with the other device banners, below the link banner.

The firmware banner and the link banner precede the demo banner. When the window is short, banners do not scroll the
graph out of view; each banner is one or two lines high.

## 3. Device status line and lock notice

- Bottom of the left column: a coloured dot and the device name with its serial in parentheses, for example
  "DSPi (DSPI0001)". Green when connected, red with "No Devices" or "Disconnected" otherwise.
- Its tooltip names the connection state and serial and says that a right-click reconnects (M57). A right-click
  forces a reconnect; during a hub session it does nothing (QD36).
- During a hub session the tooltip leads with the hub's name for the device, then "via" and the hub's name, then the
  serial, and says where to choose devices (Device > Devices…). With no device name it falls back to the generic
  name; with no hub name it falls back to the plain form (UPW2).
- **Lock notice:** while another console holds the device lock on the hub, the status line itself reads that another
  console is applying changes, in orange, and the tooltip starts with the same sentence. It is shown for as long as
  the lock is held and goes when it is released (UPW2; Android shows it for 4 s after a refused write instead). The
  lock holder comes from the hub's device record compared with this client's own session id.
- Above the status line: the C0 and C1 CPU bars, each a short bar with a percentage (QD5).

## 4. Device list

### 4.1 Layout

A popover titled "Devices" over the main window, with a "Done" button at the top right. Sections:

- **USB:** one row per DSPi on USB (or a line saying there is none), then the demo row (§1.4).
- **DSPI LINK:** the Link status line for the current hub (§4.2), with its buttons on the right; under it the hub's
  devices, one row each (link icon, the device's name, then "hub name · slot · firmware"), the chosen one with a
  check mark.
- "Find hubs automatically" switch (on by default; stored, §21), a status line while browsing,
  the hubs found, a field for a typed address (placeholder: host or host:port) with "Connect", and a short help text
  saying that a DSPi shared by another console, or on a Link bridge, shows here.

It opens from Device > Devices…. Rescan and reconnect do nothing while a hub session is bound, so no local USB device
opens under it (QD36). Picking a USB device during a hub session first asks the unsaved-changes question once (QD37).

### 4.2 States of the Link part

The Link status line follows the core's `LinkPhase`, with the core's phase text (`dspi_link_phase_text`); the client
adds only the buttons. Observed states:

| State | Status line | Buttons and rows |
|---|---|---|
| idle | not connected to a hub | none; discovery rows below |
| choose | "<hub>: " + the core's text for connected, choose a DSPi | "Disconnect"; the hub's devices listed, none checked |
| PIN | the pairing prompt is open (§5) over the list | list dimmed |
| wrong PIN | the prompt shows the core's error under the field, and the same line repeats under the address field | prompt stays open |
| bound | "<hub>: Connected" | "Disconnect"; the chosen device checked; the main window shows that device |
| reconnecting | "<hub>: " + the core's reconnecting text, in orange, then a line with the attempt number and the seconds to the next try | "Disconnect" and "Retry Now"; the device row greyed |
| refused | the core's refusal heading in orange, then the hub's reason in plain text | none |
| closed | the connection has ended, then the reason (for example a protocol error) | none |

Transitions: typing an address and "Connect", or picking a found hub, dials. A hub that needs pairing goes to PIN; a
saved token skips PIN (§7). Choosing a device binds it. Losing the connection goes to reconnecting at once and shows
the link banner (§6). "Retry Now" dials immediately; "Disconnect" closes and returns to idle. A close from the hub
goes bound → closed. `rate_limited` and `wrong_hub` keep the saved token; only `token_refused` deletes it (QD8). A
reconnect passes the expected hub id, so a different hub at the same address is refused, not silently used.

### 4.3 Discovery

- The list comes from the core's browser (`dspi_link_browse_*`). The client lists, re-resolves on demand and dials;
  it never parses DNS-SD itself.
- Linux: the core's embedded mDNS. No firewall prompt exists on Linux.
- Windows: the OS's DNS-SD service is the recommended source (DISC1). With it, browsing never triggers Windows'
  firewall prompt; only turning sharing on does, which the firewall rows of §11 handle. Windows' resolve can return
  only an IPv6 link-local address for seconds; the dial goes by the `.local` host name in that case (DISC2).
- With the embedded mDNS on Windows (today's build), opening Networking or the device list with "Find hubs
  automatically" on shows Windows' firewall prompt even with sharing off. Cancelling it leaves discovery working only
  partly. That is why the source changes; the UI does not explain the prompt.
- A hub that stays listed but cannot be reached is removed when a dial to it fails (the core's verify).

## 5. Pairing prompt

- A modal card over the device list, titled "Pair with <hub name>", with a link icon.
- Body: a help line saying to enter the PIN shown on the hub, and where to find it on another console (Settings >
  Networking, Allow a New Client).
- One field for the six-digit PIN (large, spaced digits); "Cancel" and "Pair". Pair is disabled until six digits are
  entered. The prompt does not block the rest of the app: the main window keeps updating behind it.
- A wrong or expired code shows the core's error under the field in orange; the field stays for another try.
- While the hub is being asked, the body shows the phase text instead of the help line.

## 6. Lost-connection banner (Link client)

- Shown within a frame of the drop (the smoke measures about 25 ms), until the session is bound again or closed.
- Text: connection lost, reconnecting; a "Disconnect" button that aborts, also in the middle of a dial
  (`dspi_link_close` works from any thread).
- While it shows, the mirror stays readable: every page shows the last known values. A write fails at once and is
  never sent later. After the session is back the spectrum analyser restarts by itself.

## 7. Link token store

- A pairing's token is stored per hub id, so the device list can reconnect without the PIN.
- Primary store: Windows Credential Manager, or the Secret Service on Linux (macOS keychain where the app runs
  there). The entry names used today are **not observable** here (no keyring in the test environment). The desktop's
  token-store code is kept from the current tree (plan, salvage list), so the names carry over with it; the migration
  test must still read a token written by the current app.
- Fallback when no secret store answers: a JSON file `link-tokens.json` in the app's config directory (Linux:
  `$XDG_CONFIG_HOME/DSPi/DSPi Console/link-tokens.json`). It holds one object whose keys are hub ids (UUID strings)
  and whose values are the token strings. Mode 0600. A fresh file is `{}`.
- When the fallback is used, the secret-store notice (§2 item 4) shows once, with "OK".
- A damaged fallback file is reported in the log (observed) and its tokens are not used. Whether the app rewrites
  or keeps the damaged file is **not observable** here; keep it untouched until a new token must be saved.
- A token is deleted only on `token_refused`.
- The Terminal's tokens live elsewhere (`~/.config/dspi/link-tokens.toml`); the desktop app does not read or write
  that file.

## 8. Settings window

- A separate window, default 860 × 630. Left: a search field, then categories with coloured icons in groups:
  Application (About, Advanced, Networking, Updates), Display (Graphing, Spectrum Analyser), System (Overview, Inputs,
  Outputs, I2S Configuration, Global Parameters), Control (Control Surfaces, Control Interfaces, Channel Groups,
  Macros, Auxiliary Outputs). System and Control show only while a device is open.
- Title bar of the content: back and forward arrows (page history within the window), the page title, a close button.
- The last page shown is stored (§21) and opens next time.
- **Held changes and the save bar:** device settings changed on the System and Control pages are held, not sent,
  until Save. While anything is held, a bar along the bottom of the window shows "Unsaved changes" with "Revert" and
  "Save", whatever page is showing. If Save fails part way, the bar stays, with a line in orange naming what the
  device did not accept (for example the startup preset, or the latency, §16); nothing that failed reads as saved
  (QD31). Display pages (Graphing, Spectrum Analyser) are app preferences and apply at once.
- Advanced also shows device facts (platform, firmware, serial, channel counts), Reset Channel Names (takes effect
  at once), and a Core Voltage entry with the running voltage and "Open…" (§20).

## 9. Settings search

- Typing in the search field filters the category list as you type. Matching is on each page's content (its rows,
  labels and keywords), not only its title: "lat" leaves Global Parameters (latency), "pin" leaves Networking (the
  pairing code), Overview and Outputs (pins), "tray" leaves Networking.
- Group headings stay above their remaining pages; empty groups disappear.
- No match: the list is replaced by a one-line "no matching settings" message.
- A clear button (×) appears in the field while it holds text; clearing restores the full list.
- Down and Return open the first remaining page. Clicking a page opens it. The page opens at its top; rows are not
  highlighted.
- The page already showing does not change while typing.

## 10. Networking page (desktop extras)

The Mac's `NetworkingSettingsView` is the reference. Observed desktop specifics:

- First card: "Share Devices on This Network" switch; under it a description, replaced while sharing by the address
  and port and the number of connected clients. A second row says the hub shares its own demo device, separate from
  the one shown here.
- A failure to start sharing (port in use, for instance) shows one line in orange with a warning icon under that
  card, with the OS's reason.
- This Hub: Name (default: a 12-character hex id until changed; stored once set), Access (default "Require Pairing"),
  Port (default 11915; read-only while sharing, with a note to turn sharing off to change it). A footnote says the PIN
  crosses the network as plain text without encryption.
- Paired Clients: each client with its name, last seen, a role picker (Viewer, Control, Admin) and a remove button;
  "Pair a New Client" with "Pair…", disabled until sharing is on. While a code is open, a card shows a one-line
  instruction, the six digits in large spaced type, a countdown from two minutes and "Cancel".
- Other Hubs: "Find hubs automatically" (the same setting as in the device list), a status line, and "Connect by
  Address" with a field and "Connect".
- Running in the Background: "Show in the tray", "Start in the tray only" (enabled only when the first is on), "Open
  at login", "Keep awake while sharing" (§12). When no tray can be shown, a footnote says so and that closing the
  window quits.
- On Windows only, while sharing, the firewall and Public-network rows (§11).
- The page keeps its watch on the hub while open; closing Settings and the device list in either order ends it
  (QD37).

## 11. Windows Firewall and Public-network rows

Windows only. Two rows under the sharing card, shown only while sharing is on or starting (UPW2). Nothing shows while
sharing is off, on Linux, or on macOS.

### 11.1 Firewall row

- State is read on a worker thread: the enabled inbound firewall rules whose program is this executable (path compared
  without case, environment variables expanded), judged for the Private and Domain profiles (the current profiles
  minus Public; Private when the current one is Public). A Block rule wins over an Allow rule, as in the firewall.
- Result: blocked, allowed, none, or off (the firewall is off for those profiles). Only blocked and none show a row.
  A port-only rule without a program counts as none (known limit).
- Wording: blocked says Windows Firewall blocks the app; none says Windows Firewall has no rule for it. A detail line
  says that allowing it lets it in on Private networks only, and that Windows will ask permission for "Network Command
  Shell", the part of Windows that changes the rule. One button allows the app in Windows Firewall.
- The button starts `netsh.exe` from the System32 directory (full path, working directory System32), elevated
  directly with one UAC prompt; never through `cmd`. For blocked: a `set rule` that turns this program's inbound rules
  into Allow rules for Private and Domain (Windows' own Block rules become two Allow rules, TCP and UDP). For none: an
  `add rule` named after the app, inbound, allow, this program, profiles Private and Domain. The app waits for the
  exit code and re-reads.
- Declined UAC prompt: a line says Windows did not get permission and nothing changed; the rules stay as they were.
- An executable path containing `%` cannot be named in a rule; the row then says to move the app instead of offering
  the button.
- Re-read every 5 s while the page is open and sharing is on, so a rule deleted from outside shows within one read.
- The installer adds no firewall rule (§24).

### 11.2 Public-network row

- Shown when a connected network is set to Public and it is on an adapter that is up and has a default gateway. This
  leaves out host-only virtual adapters (Hyper-V, WSL, VirtualBox), and also a direct cable without a router (known
  limit).
- Text: this network is set to Public; one button that opens Windows' network status settings
  (`ms-settings:network-status`).
- Both rows go when sharing goes off.

## 12. Tray, open at login, keep awake

### 12.1 Tray (Linux and Windows)

Settings (§21): show in tray (default off), start in the tray only (default off; only meaningful with the first).

States:

| State | Window | Tray icon | Closing the window |
|---|---|---|---|
| no tray setting | shown | none | quits |
| tray on, tray available | shown | shown | hides the window; the app, the hub and polling keep running |
| hidden to tray | hidden | shown | (n/a) "Open" in the menu or activating the icon shows the window again |
| start in tray only, tray available | starts hidden | shown | as above |
| tray on but no tray host available | shown, even with start-in-tray-only set | none | quits |

- The no-tray fallback is decided at launch and whenever the setting changes: when the system has no tray, the
  window is always shown at start, closing quits, and the Networking page says so (observed under a display with no
  tray host).
- The tray itself is **not observable** here (no tray host in the test display); the rows above come from QD12, the
  Networking page and the no-tray runs. Activating the icon shows the window.
- Tray menu, in order (the Mac's menu-bar mode): a status line (sharing or not, client count), the hub address and
  the pairing code while one is open, Open, Share on This Network (toggle), Allow a New Client, Quit. Quick volume is
  to be added (plan). The two hub items stay hidden until the hub is available.
- Quit from the tray ends the app, including sharing. On Windows the installer refuses to run while the app is still
  in the tray (§24).
- The smoke driver launches the app four times with the tray settings on (hidden, hidden then quit, app quit, no
  tray) and fails with exit code 3 if a launch does not quit by itself (§23).

### 12.2 Open at login

- No setting key: the state is the presence of the OS entry.
- Linux: an XDG autostart file `$XDG_CONFIG_HOME/autostart/dspi-uconsole.desktop` (Type Application, the app's name and
  a comment, `Exec` with the executable's absolute path quoted per the desktop-entry rules, Icon `dspi-uconsole`,
  Terminal false, `X-GNOME-Autostart-enabled=true`). Paths with spaces, quotes, backslashes, `$`, backticks and `%`
  are escaped as the desktop-entry spec requires (observed with such a path).
- Windows: a value under the user's `Run` key named after the app (the current name is "DSPi Console"; inferred from
  the test fixture), holding the quoted executable path.
- An entry that belongs to another copy of the program (another path) is left alone, not overwritten; turning the
  switch off removes only this copy's entry. A symlinked or unquoted entry pointing at this executable counts as this
  copy's (QD13b, QD37).

### 12.3 Keep awake while sharing

- Setting `system/keepAwake` (§21), default off. While on and sharing, idle sleep is inhibited; the screen may still
  turn off. Linux: an inhibitor through `systemd-inhibit` (idle sleep). Windows: `SetThreadExecutionState`, so the
  system does not idle to sleep. Released when sharing stops or the setting goes off (documented, QD13).

## 13. Device access help (udev dialog, Windows advice)

Documented, not observed (needs a USB device the app cannot open).

- The core reports devices the last scan saw but could not open (`dspi_inaccessible_devices`).
- Linux: when that count is above zero, the app offers a dialog explaining that the DSPi needs permission, with a
  button that installs the udev rule through `pkexec` (one authentication prompt), then rescans. The rule file is
  `70-dspi.rules` (vendor 2e8b, the `uaccess` tag); the packages install the same rule, so the dialog matters for the
  AppImage and the tar. A declined or failed `pkexec` leaves the dialog with a short error; nothing else changes.
- The status area at the bottom of the left column is one of the places that offers it (documented: the inventory
  lists the status area and the main window).
- What a declined or failed `pkexec` shows is **not observable** here; say what failed and leave the rule
  uninstalled.
- Windows: no rule; the dialog is replaced by advice to close other programs that hold the device, and that very old
  firmware needs a WinUSB driver installed with Zadig.
- Other platforms get their own short advice that the device cannot be opened.

## 14. Read-only mode

When the firmware's wire format is older or newer than this app supports (`dspi_wire_features`,
`dspi_read_only_refusals`), the app shows everything the firmware has and allows no write. Smoke fixtures: wire V17
(1.1.5), V26 and V31 (1.1.6 betas, older) and V33 (1.1.7, newer).

### 14.1 What the user sees

- Banner (§2 item 1) in warning colour. Older firmware: the firmware is too old for this console, settings are
  read-only, features it lacks are hidden, update to the expected version; button "Update…" opens the firmware
  window. Newer firmware: the firmware is newer than this console understands, settings are read-only, some features
  may not show, update the console; button "Details…".
- Features the firmware lacks are hidden, not disabled: on V17 the psychoacoustic-bass button is gone from the quick
  strip and the limiter cell is gone from the output card. Newer firmware hides nothing it knows.
- Every value is shown as read. Pickers (Preset, Source) show their value without the chevron. Batch buttons (Enable
  All, Bypass All, Clear All) are disabled.

### 14.2 What stays enabled (the matrix)

The rule (QD6): every control that writes to the device is disabled; reads, navigation, local view settings, clip
clear and entering the bootloader stay enabled. The table applies the rule per area; which individual controls the
counts below cover is **not observable** from outside, so the rule decides.

| Area | Enabled | Disabled |
|---|---|---|
| Main window | channel selection, the quick strip (opens tool windows), the app menu, the device list, the graph's view controls, PEQ/XO tabs | every device write: preset, source, volume writes, gains, delays, mutes, invert, filters, batch buttons, rename |
| Tool windows (matrix, crossfeed, loudness, leveller, psybass, subharmonic, tube, upmixer, signal generator, AutoEQ apply) | opening, closing, view toggles | every control that writes |
| Spectrum analyser | the Curves / Bars / Both view switch and channel selection (local) | — |
| Stats | its local action | device resets |
| Interrupt monitor | its local controls (pause, copy) | — |
| Settings: Graphing, Spectrum Analyser | everything (app preferences) | — |
| Settings: Overview | navigation | — |
| Settings: device pages | reading | every row; Save |
| Always | clip-indicator clear, entering the bootloader from the firmware window, all reads and polls, the device list, Link | — |

The smoke clicks every enabled control on each window in turn and fails if any setter is called. The counts below
are from today's run of today's app, for reference only; a rebuilt window will have other totals, and the rule above,
not the counts, is the requirement. Counts per window (clicked of total): graphing 17/49, spectrum settings 17/35, overview 17/59, matrix 0/67, loudness
0/17, crossfeed 0/18, leveller 0/14, psybass 0/27, subharmonic 0/34, tube 0/42, upmixer 0/1, generator 0/1, AutoEQ 0/20,
stats 1/1, analyser 3/3, monitor 3/30, input page 2/37, output page 6/50, main window 23/30.

- Polling and notifications continue. A queued write is dropped, not sent, when the next device opens (QD6).
- During a hub session a read-only device behaves the same.

## 15. Delay-refusal banner

- Every delay write (channel delay, output delay, the matrix's delay, a paste) goes through the core's checked write.
  When the core refuses it (outside the device's delay line: 0 to 21 ms on RP2040 and unknown platforms, 0 to 42 ms
  on RP2350; NaN or infinity), nothing is sent and the control returns to the device's value.
- The refusal shows as a one-line banner (§2 item 6) with the core's sentence verbatim (for example that 47 ms is out
  of range and this device holds 0 to 42 ms), fixed height, with a dismiss button.
- It clears on the next accepted delay write, on dismiss, or on a device change.
- Documented from the delay work's merge record (DL3), not seen on screen here: the hand-driven app bounds its
  fields, so no refusal could be produced by hand.
- Sliders and fields are bounded by `dspi_max_delay_ms`, so the banner shows mainly for pastes and stale limits.
  Slider rule SL1 applies: a drag sends whole milliseconds live and on release; typed and file values are kept exactly.
- A pasted channel whose delay is past the line is held at the line.

## 16. Output latency and its "held" state

- Global Parameters has a Latency card: "Target" with a picker whose options come from the device (the default marked
  "(default)"), and a description under it.
- Changing it is held like every other device setting (§8). On Save, the app writes the latency and reads back the
  device's effective value. If the device has not applied it yet (requested and effective differ, or a change is
  pending), the save bar stays, with an orange line saying the device has not applied the latency yet. The picker keeps
  the requested value. This is the "held" state.
- On Revert it clears. Whether a later read that shows the value applied clears it by itself is **not observable**
  here; it should. A stall of the latency request hides the card (QD5).

## 17. App updates

- Settings > Updates (desktop only; off in distribution packages).
- Rows: "Check for Updates Automatically" (off until the user turns it on), the current version with the time of the
  last check (or that it has not checked yet) and "Check Now".
- Privacy line under them, always visible: what a check sends and to whom: the feed's host, the request carries only
  what any web request does (the IP address) and the app's version in the User-Agent (`DSPiConsole/<version>`);
  nothing about devices, presets or settings; with automatic checks on, a check runs at launch and once a day. The
  smoke checks that the request carries no headers beyond host, user-agent, accept, accept-language, connection and
  accept-encoding.
- A build without a feed (developer and distribution builds) says it never checks; the switch and "Check Now" are
  disabled.
- The feed is https only. A loopback http feed is allowed for tests.
- When a newer release is found: an "Available" card with the release name and version, "View Release" (opens the
  release page in the browser), the release notes shown as plain text (no markup interpreted), and "Skip This
  Version" with "Skip" (automatic checks then stay quiet about that version). A footnote may say the release brings
  matching firmware, which the app offers to install when a device is connected.
- The update banner (§2 item 5): "Details" opens this page, "Later" hides it. How long "Later" lasts is **not
  observable** here; until the next check that finds a newer version is the sensible rule. Nothing is downloaded or
  installed by the app.

## 18. Host audio format

- The channel count the host plays to the DSPi decides how many USB input strips show (M25). The app asks the OS: on
  Windows the active render endpoint's format (WASAPI), matched to the board by the USB serial found walking the
  endpoint's device-tree parents, so two boards can differ; on Linux PipeWire or ALSA. When the OS gives no answer, the
  device's own report is used.
- Observed with an eight-channel host format on the demo: eight USB inputs in the sidebar.

## 19. Firmware window (desktop details)

The Mac's `FirmwareUpdateView` is the reference. Desktop specifics observed:

- Steps Prepare, Write, Verify, Done along the top; a summary of what will be installed (version and file name) and
  the connected device's version; "Use this Console's firmware instead" when a custom file is chosen.
- The core's firmware guard can stop the install and ask for a typed confirmation sentence (core-voltage risk, wire
  mismatch). The text and the sentence come from the core; "Go Ahead" stays disabled until the sentence matches
  exactly; "Don't Install" leaves.
- When the board is already in its bootloader and nothing has been written, a panel says so and offers to install
  again or unplug.
- "Export Configuration…" before installing, as on the Mac.
- Bootloader drives are found by volume label: Linux `/dev/disk/by-label` with `udisksctl mount` when not mounted,
  Windows by `QStorageInfo` label. The UF2 file is written in binary mode and flushed to disk before the app waits
  for the board to come back.

## 20. Core voltage page

Reached from Settings > Advanced > Core Voltage "Open…"; never from elsewhere. Hidden on firmware without the
extension. All warnings, limits and states come from the core (`dspi_core_vreg_*`).

- A "Back" row returns to Advanced. A warning block (from the core) heads the page.
- On the Device: running voltage (with "(trial)" during a trial), saved voltage, build default, hangs this session,
  last failed trial.
- Calibrate: state (Not started, Ready, Soaking, and the core's other states), a description, and the actions the
  state allows: trial voltage and soak time pickers (the suggested soak marked), Start…, Close; during a soak, the
  elapsed and total soak time and "Abort" in red.
- Manual: Voltage picker, Trial…, Save…, Reset…, Automatic raising (Turn On… / Turn Off…), Clear…. Disabled while a
  calibration runs.
- Above 1.20 V a Confirm block asks for the core's word (OVERVOLT) typed exactly; "Confirm" stays disabled until it
  matches; "Cancel" leaves.
- States shown by the smoke screenshots: idle, review, confirm word, soaking, trial found, passed, committed, failed,
  verify failed, hardware problem, left running, reset unreadable, unknown, and the stock-firmware view.

## 21. Settings storage and migration

### 21.1 Where

| What | Linux | Windows |
|---|---|---|
| App settings (QSettings, INI) | `$XDG_CONFIG_HOME/DSPi/DSPi Console.conf` | `%APPDATA%\DSPi\DSPi Console.ini` (INI format forced) |
| Link token fallback (§7) | `$XDG_CONFIG_HOME/DSPi/DSPi Console/link-tokens.json` | Qt's AppConfigLocation + `link-tokens.json` (not observed) |
| Hub store (paired clients, hub id, name, access) | `$XDG_DATA_HOME/DSPi/DSPi Console/hub/hub-store.json` | Qt's AppDataLocation + `hub/hub-store.json` (not observed) |
| QML cache | `$XDG_CACHE_HOME/DSPi/DSPi Console/qmlcache` | `%LOCALAPPDATA%\DSPi\DSPi Console` |
| AutoEQ cache | Qt's AppDataLocation, `$XDG_DATA_HOME/DSPi/DSPi Console` | Qt's AppDataLocation |
| Login entry | `$XDG_CONFIG_HOME/autostart/dspi-uconsole.desktop` | HKCU `…\Run`, value per §12.2 |
| Installer record | — | HKCU `Software\DSPi\DSPi uConsole` (`InstallDir`), uninstall key `DSPiuConsole` |

macOS uses the standard preferences plist for the same keys.

The hub store is the core's file (keys `hub_id`, `name`, `access`, `next_cid`, `clients`, `device_names`); the app
gives the core the directory only. Keep the directory so paired clients survive.

### 21.2 Keys the current app writes

Union of every key found in 452 settings files written by runs of the current app (smoke runs, a desktop session run
and the hand runs), plus the keys seen only when a control was changed by hand. Types as written in the INI file.

| Key | Type | Default | Meaning |
|---|---|---|---|
| `onboarding/setupDone` | bool | false | Getting Started finished or skipped |
| `onboarding/whatsNewShown` | string | — | last app version whose What's New was shown |
| `settingsWindow/lastPage` | string | `about` | last Settings page as a lower-case id; observed values `about` and `updates` (the other ids are not observed) |
| `sidebar/showMaster` | bool | false | volume mode: false = User Volume, true = Master (M47) |
| `sidebar/width` | int | 270 | sidebar width, px |
| `dashboard/cardsPerRow` | int | 0 | overview cards per row, 0 = automatic (M59) |
| `graph/dbCenter` | number | 0 | graph centre, dB |
| `graph/dbRange` | number | 50 | graph span, dB |
| `graph/minFreq`, `graph/maxFreq` | number | 15, 20000 | frequency axis, Hz |
| `graph/height` | int | 250 | graph height in the main window, px |
| `graph/lineWidth` | number | 1.5 | curve width |
| `graph/gridOpacity` | number | 0.5 | |
| `graph/showDbGrid`, `showDbLabels`, `showFreqGrid`, `showFreqLabels` | bool | true | |
| `graph/showGlow` | bool | false | |
| `graph/showPhase`, `graph/phaseUnwrapped` | bool | false | |
| `graph/freqReadout`, `graph/levelReadout` | bool | true | hover readouts |
| `graph/popOutFollows` | bool | true | the pop-out graph follows the selected channel |
| `spectrum/fftOrder` | int | 10 | |
| `spectrum/avgMs` | int | 300 | |
| `spectrum/peakDecay` | number | 12 | |
| `spectrum/floorDb`, `spectrum/ceilingDb` | number | -90, 6 | |
| `spectrum/smoothing`, `spectrum/showPeakHold` | bool | true | |
| `spectrum/graphOpacity` | number | 1 | analyser overlay on the graph |
| `spectrum/barColumns`, `spectrum/barHeight` | int | 2, 96 | bar strip |
| `spectrum/dashboardShowGraph`, `dashboardShowBars` | bool | true, false | overview page |
| `spectrum/dashboardSource` | string | empty | e.g. `out:0,1,2,3` |
| `spectrum/dashboardOtherSide` | string | empty | |
| `spectrum/channelPagesShowSpectrum`, `channelShowGraph`, `channelShowBars` | bool | true, true, false | channel pages |
| `stereoUpmixer/advanced`, `tubeModeller/advanced` | bool | false | Basic / Advanced view of those windows |
| `inputLinks/<serial>` | int | — | per device serial: which input pairs are linked (M48). Observed value 1 for the demo's one linked pair; the encoding for several pairs is not observed |
| `hub/sharing` | bool | false | Share Devices on This Network |
| `hub/name` | string | — | written once the user names the hub |
| `hub/port` | int | 11915 | written once changed |
| `hub/lookForHubs` | bool | true | Find hubs automatically (written when changed) |
| `tray/showInTray`, `tray/startMinimised` | bool | false | §12.1 |
| `system/keepAwake` | bool | false | §12.3 |

Not written by any observed run: window geometry (main, Settings, tool windows), the update opt-in and skipped
version, the demo choice, AutoEQ favourites. See §21.3.

### 21.3 Migration for the new app (plan Q12)

- Keep organisation `DSPi` and application `DSPi Console`, so the same files are read.
- Carry over: `onboarding/setupDone`, `onboarding/whatsNewShown`, `sidebar/showMaster`, `hub/*`, `tray/*`,
  `system/keepAwake`, `inputLinks/*`. Under new names, map them on first start and leave the old keys in place, so
  going back to an old build keeps working.
- Window geometry: no key exists today, so there is nothing to migrate.
- Update opt-in and demo: **not observable** (a build with no feed disables the switch; the demo is chosen per
  launch). If the reviewer finds keys for them in a real profile, add them here.
- The login entry, the token fallback file, the keyring entries and the hub store are files or OS entries, not keys:
  keep their names and paths unchanged.
- Everything else may start fresh with the Mac's defaults.

## 22. Pseudo-locale pass

- Two extra launches on the demo device after the main smoke run: first untranslated English, then a pseudo
  locale. Each opens the main windows in turn (main window, an input page, an output page, the device list, every
  Settings page, every tool window, the graph, firmware, What's New, AutoEQ and its rebuild, Getting Started) and
  screenshots each into `pseudo-en/` and `pseudo/`.
- Pseudo strings: every translatable string gets accented letters, is wrapped in square brackets and padded with
  `~` to about a third longer. Strings from the core and from the device (names, values, units from the core) are not
  translated and stay plain, which is how untranslated strings in the UI are spotted.
- Per run the driver logs: number of texts, untranslated ones (written to `unmarked.txt`), clipped ones (a text cut
  at its `maximumLineCount`), how many of those were already clipped in English, and elided ones (a text shortened
  with an ellipsis on purpose, at any end: `Text.ElideLeft` is 0, so a check compares with `ElideNone`), then each
  clipped and elided text with its window. The English run writes `clipped.txt` as the baseline; the pseudo run
  fails only on new clipping. (QPI4)
- What it does not prove: `Text.truncated` is set only by an elide or a line count, so a text that overflows its box
  without either is not seen; untranslated strings are listed, not a gate (device words, the core's About and journal
  labels, C++ strings translated once at construction); the device list, Getting Started and the AutoEQ rebuild are
  not visited.

## 23. Smoke driver contract

The offscreen driver is kept and adapted (plan). What the CI script and reviewers rely on:

- **Launch:** `run.sh <smoke binary> <out dir>`. It sets a scratch HOME and XDG config under the out dir, writes a
  settings file with `onboarding/setupDone=true`, runs with `QT_QPA_PLATFORM=offscreen`, the software Qt Quick
  backend, `RUST_LOG=warn`, and `--demo`.
- **Environment:** `DSPI_SMOKE_OUT` (out dir), `DSPI_SMOKE_HW` (`1` real device, `dry` the hardware pass on the demo,
  `replug` waits for the owner), `DSPI_SMOKE_DEMO_ENTRY` (start without `--demo`), `DSPI_SMOKE_TRAY` (`hidden`,
  `hidden-quit`, `app-quit`, `no-tray`) with `DSPI_SMOKE_TRAY_PROBE`, `DSPI_SMOKE_PSEUDO` (`en`, `pseudo`; the
  pseudo run reads the English run's `clipped.txt` beside its own out dir), `DSPI_SMOKE_SECTION_LIMIT` (seconds per
  section, default 120), `DSPI_SMOKE_SECTION=<name>`, `DSPI_SMOKE_FAKEHUB` and `DSPI_SMOKE_TERMINAL` (helper
  binaries).
- **Log lines** (the CI greps `^smoke:` and `^FAIL`): `smoke: == <section>` at a section's start, `smoke: -- <section>:
  <s> s` at its end, `FAIL <section or check>: <what>` per failure, and the summary lines:
  - `smoke: <n> sections in <s> s; slowest: <name> <s> s; …` (five slowest);
  - `smoke: <n> checks, <f> failures (<q> QML errors, <t> thread warnings)`, with `, <k> not run` appended in the
    hardware pass. The CI fails if this line is missing. A Qt thread warning is itself a `FAIL thread:` with a
    backtrace.
  - Tray launches end with `smoke: tray launch <mode>: <n> checks, <f> failures, quitting`.
- **Exit codes:** 0 pass; non-zero on any failure; 2 when the launch's mode has no section in this build (`smoke:
  <VAR>=<value> has no section in this build`); 3 when a tray launch does not quit; 4 when the section watchdog
  fires (the log names the section). The whole run is also bounded by `timeout` (900 s).
- **Warning budget:** after the main run (the other launches get the backtrace and summary checks only), any core
  warning line (`[… WARN …]`) repeated more than five times fails the run, except the firmware guard's log of an accepted typed confirmation. In the replug mode repeats are only
  reported. dspi-capi's own warnings are rate-limited and do not count.
- **Order of runs in one CI step:** main run, demo-entry run (`demo-entry/`), four tray launches (`tray-*/`), two
  pseudo-locale runs (`pseudo-en/`, `pseudo-pseudo/`), then the hardware pass on the demo (`hw-dry/`). A failure in
  any launch fails the step. Last, `index.html` in the out dir shows every PNG (the gallery: the gallery section's
  dark and light pairs first). The K340 (BUG1) launch is not rebuilt.
- **Outputs:** one PNG per window or state in the out dir. Today's names, which the gallery and reviewers use:
  `window-<name>` for the main window and tool windows (main, main-cpu-bars, graph, matrix, crossfeed, crossfeed-k340,
  crossfeed-stock, crossfeed-editor, crossfeed-editor-short, loudness, leveller, psybass, subharm, tube, upmix, siggen,
  spectrum, spectrum-following, stats, stats-clock, stats-ring, stats-ring-067, monitor, autoeq, autoeqRebuild,
  firmware, firmware-demo, firmware-question, firmware-verified, firmware-in-bootloader, wizard,
  wizard-firmware-question, whatsNew, banner, disconnected, link-banner, link-fault-dial, devices-idle,
  devices-choose, devices-pin, devices-pin-wrong, devices-bound, devices-reconnecting, devices-refused,
  devices-closed, devices-lock-notice); `settings-<page>` and its states (about, advanced, networking,
  networking-pairing, networking-client, networking-error, updates, graphing, spectrum, overview, overview-stock,
  overview-core-voltage, overview-core-voltage-raised, inputs, outputs, i2s, global, latency, latency-held,
  names-global, names-inputs, surfaces, control, groups, macros, aux, save-refused, save-setter-refused, and the
  core-voltage states of §20); `readonly-v17|v26|v31|v33` with `-output` and `-settings`; `cs-*`, `about-*`,
  `hostaudio-8-strips`, `graph-following-empty-selection`, `update-banner`.
- **Object names:** the driver finds controls by object name. The names are not observable from outside and change
  with the rebuild; the plan's naming rule (`<Screen>.<control>`, listed in `smoke/NAMES.md`) replaces them. Each
  rebuilt screen's section must exist before its screen counts as done.

## 24. Windows installer checks

The NSIS script and its test (`apps/desktop/packaging/windows/`) are kept. Behaviour:

- Per-user install, no administrator rights: `%LOCALAPPDATA%\Programs\DSPi uConsole`, a Start menu shortcut, an entry
  in Settings > Apps (HKCU uninstall key `DSPiuConsole`: name, version, publisher, icon, location; no modify, no
  repair).
- Before installing and before uninstalling, the installer checks that the installed executable is not running, by
  trying to open it for writing (a running exe cannot be). If it is running (also when only in the tray), it asks to
  quit the app and offers Retry or Cancel. A silent run cancels with exit code 2 and changes nothing.
- An install first removes the previous version's `qml` directory and the other bundled documentation directory, so no stale files stay.
- The finish page offers to run the app, started through the shell as the Start menu would.
- Uninstall removes the program directory, the shortcut and the registry entries, and leaves the settings in
  `%APPDATA%\DSPi`.
- No firewall rule is added by the installer (§11).
- The test runs the installer and uninstaller silently under Wine, including the refusal while the app runs.

## 25. Other desktop behaviours not in the Mac app

- **Output labels:** an output whose slot name is still the default shows the derived label from the core, for
  example "Out 1 · L · Full", in the sidebar, the overview cards and the matrix; a user name wins.
- **Quick strip:** eight icon buttons (matrix, crossfeed, loudness, leveller, psychoacoustic bass, stats, settings,
  bypass). A secondary click opens the tool (M60). Buttons for features the firmware lacks are hidden (§14.1).
- **Slider reset and rename:** right-click resets a slider to its default; Alt+click on a channel row renames it
  (M51).
- **Channel row menu:** right-click on a sidebar row gives rename, copy, paste and identify (M45, M46).
- **Curve pills:** the coloured IN/OUT pill on each sidebar row toggles that channel's curve (M58).
- **Version string:** About names the platform ("for Linux", "for Windows").
- **Analyser following another console:** the analyser's status bar says it follows another console, with a "Take
  control" link (M13).
- **Settings save failures** are named per setter (§8).
- **Hidden-window shortcuts** only act while their window has focus (§1.2).

## 26. Popover anchor rule (QD30)

Applies to every popover the app opens from a button: the graph's gear (graph options), the analyser strip's gear,
the dashboard layout popup, and any later one built on the shared popover component.

- While a popover is open, the pointer over its anchor button shows that button's normal cursor and hover state at
  once. It must not stay in the popover's cursor until the mouse is wiggled (the popup's pointer grab must not hold
  the anchor's cursor).
- One click on the anchor while the popover is open closes it, once. The same click must not close it and then reopen
  it (the outside-press closing the popup and the button's own click toggling it again).
- A click elsewhere outside the popover closes it as before; Escape closes it.
- The rule holds on KDE Plasma under Wayland and under X11 (the owner's report was on KDE Wayland); Windows should
  behave the same.
- Offscreen tests cannot see the cursor. The smoke can check the click part (open, click the anchor, closed and not
  reopened); the cursor part is a hand check.

**Hand check for QP21:** on KDE Plasma Wayland, then X11: open the graph gear's popover, move the pointer onto the
gear without wiggling: the hand cursor shows at once. Click the gear once: the popover closes and stays closed.
Repeat with the analyser strip's gear. Record OS, session type and date in `docs/core-merge/HW-QT.md` with QD35's
other hand checks.

## 27. About: third-party notices list

Mechanics only; the wording and the texts are data shipped with the app, not written in the UI.

- The About page has a section listing the third-party components' notices, one row per notice (its name, and the
  component it covers). The list should be built from the notice files the packaging stages (plan section 5: the
  staging script adds each notice to this list), so it follows the packaging, never an edit of the page. The row
  content is the plan's, not observed.
- Choosing a row opens a text viewer over the Settings window: a modal card nearly the window's size, the notice's
  name as its title at the top left, a "Close" button at the top right, and the notice text below in a monospaced
  font, unwrapped as in the file, scrolling vertically.
- "Close" closes the viewer and returns to the same place in the About page. Escape should do the same, and the
  text should be selectable for copying (both **not observable** in a screenshot; recommended).
- Observed in the About screenshots of 2026-11-09 (the viewer with its title and Close). The list rows themselves are
  outside the visible part of the About page screenshot, so their exact layout is **not observable**: use the
  Settings row style (§8).

