# uConsole for DSPi 0.16.0

Release date: 2026-11-26. One version for every client but the Swift app.

0.16.0 is about everyday use. The app now builds your crossovers and routes from the speakers you confirmed, in one
step that one Undo takes back. A/B in the top bar compares what is saved on the device with what you hear now. The
Overview lists what is left to set up, every device tab says when it has unsaved changes, and Simple mode and a DSPi
locked by its installer open on **Home**, a remote with volume, mute, source and presets. Outputs are called by your
speakers' names, the demo is a tuned two-way speaker, and an agent's questions come in the words of the screen.

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

**Please read [What was tested, and where](#what-was-tested-and-where).** Nothing new in 0.16.0 has run on a real
DSPi yet: it was tested against simulated devices, a simulated home theatre and a simulated microphone.

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

## Several devices (desktop)

- With more than one device open, the dock offers **All devices**: volume, mute and preset recall then act on every
  open device, each within its own limits and keeping its own offset. Home's volume follows the link too.

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

## Web console

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

## Android

- **Mute** beside the volume on Home; the notification's steps are 1 dB.
- A tap the installer's lock refuses says why, with **Unlock…**.
- **Settings › Speakers** and a Speakers step in Getting Started: the role and side of each output, the tweeter's
  floor, Confirm. Under the lock it is read-only.
- A touch **band editor** (a sheet with − and + per value), the crossover in two rows above the bands with **Same for
  the right side**, legend pills that choose the channel.
- Notices when the USB DSPi goes away and comes back; Exit demo and Allow USB access are buttons on the bar.
- **Phones open in Advanced** (as do iPhone and iPad, whose app is not in this release).

## Terminal (`uconsole`)

- **Plain mode** (`uconsole --plain`, or `TERM=dumb`): the interface as sentences, one command per line, for screen
  readers and scripts. Piped, every line runs, nothing waits for an answer (an `ab` or a guarded recall keeps), and
  the exit code is the first failure's; `--plain` with a command on the line is refused.
- Ctrl-P search finds words ("crossover", "mute"); `uconsole` with nothing plugged in waits for a DSPi and offers the
  demo and DSPi Link; the `:` line runs `snapshot`, `ab`, `diff`, `lock`, `limits` and `status`; numbers take `k`
  (`2k`).
- Tab goes channels → page → footer; Enter on a route edits its gain and never connects; `[` `]` `{` `}` and `m` set
  the volume and mute from any page; unused outputs fold.

## One word for one thing (every client)

- **Save to Device** everywhere (Classic's menu said Commit Parameters). Filters say **Bell** (not Peaking), **Q**,
  and **Low pass** / **High pass**. The unsaved list says "Out 1 · L gain". A snapshot applied for a while is a
  **trial**. What's New opens once with the app's own news.

## Fixes

- **From the last review before this release:**
  - A/B "as saved": your edits on B were lost without a question when you switched preset, closed the tab, quit or
    recalled on all devices while listening to A; a toggle could raise the master volume in "master volume with
    preset" mode.
  - One Allow could approve an agent's change that sat below the sheet's last row; a sheet now holds only what it
    shows, and the app's server refuses an answer to a question it has not shown yet.
  - The speaker layout could be changed under the installer's lock (on every client).
  - Home's volume ignored the All-devices link, so the next linked move could raise the other device at once.
  - A solo's mutes outlived a page reload or an app crash and could then be saved.
  - "From Speaker Layout" could route a tweeter without a high-pass, and a sub or centre fed from both sides could
    come out louder than before.
  - Speakers with a confirmed role or a crossover were folded as "unused" in the Crossover and Delay & Gain tables.
  - In Modern under the lock, shortcuts, the palette and Open in Window still offered writes the lock refuses.
  - Terminal plain mode, piped: `ab` and a guarded recall read the script's next lines as answers.
  - Measure forgot the simulated microphone you chose once the demo was opened again.
- When another console or a control surface starts the **signal generator**, every output of that device is shown
  while it plays (none is folded as unused), instead of the app asking the device which outputs it plays on. Nothing
  that plays is hidden; this is a change from 0.15.0 the release keeps on purpose.

## What was tested, and where

- **Every client** is tested automatically against simulated DSPi devices, a simulated room and microphone, fakes of
  both firmware generations and a fake hub: the desktop app's screens in both layouts (the qt-offscreen suite), the
  web console in Chromium and Firefox, Android's unit tests (Robolectric), the Terminal's, and the MCP server against
  the official MCP conformance suite. The full CI ran green on the release commit.
- **The new desktop features** were walked in the real app against a **simulated home theatre** (three DSPi units, a
  room and a microphone): From Speaker Layout, A/B as saved, the setup list, Save All, Home, the installer's lock, a
  fresh install, the product demo; and the recorded agent stories (presets and snapshots with every change asked; a
  preset that would expose a tweeter) were replayed against it.
- **Real hardware: nothing new in 0.16.0 has run on a real DSPi.** Android 0.16.0 has not run on a phone (its new
  pages are tested in unit tests). The Claude Desktop extension has **not** been installed in a real Claude Desktop.
  The iPhone and iPad defaults changed in code, but the Swift app's tests have not run (it is not in this release).

## Known limits

- **Install in Claude Desktop…** writes an extension that has never been tried in a real Claude Desktop, and it is
  not signed. If Claude Desktop does not take it, use Copy JSON in Settings › AI Agents, as in 0.15.0. After an
  AppImage moves, make the extension again.
- An agent that starts the app with `--mcp` (Claude Desktop, Claude Code) handles one request at a time, so its
  questions do not come together as one sheet; agents over HTTP do.
- From Speaker Layout and the Home page are on the desktop (Home also on the web and Android); the web and Android
  have no From Speaker Layout yet.
- The Terminal's A/B is "now against a snapshot", not "as saved".
- If the desktop app is killed during a solo, the outputs it muted stay muted on the device (not saved) until you
  unmute them; a web page's solo ends on a reload only as far as the browser lets it finish. Another console on the
  same hub sees a lost solo's mutes until this one reconnects.
- Agents run only in the desktop app. Measurement and the Modern layout are not on Android; the Swift apps are still
  deferred. Not yet: Web MIDI; Apple Developer ID and Windows code signing; Apple silicon has never run the Mac app.

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
