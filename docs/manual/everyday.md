# Everyday use

How to set up a DSPi and live with it in uConsole for DSPi 0.16.0: the first run, your speakers, saving, comparing,
Home, the installer's lock, several devices, solo and the demo. The desktop app comes first, in its **Modern** layout
(what new installs start in); where the **Classic** layout (menu bar and tool windows) differs, it says so. What the
web console and Android offer follows each part, and has its own sections at the end.

To install the app, see [Installing](../INSTALL.md). Measuring and auto-tune have their own page,
[Measuring and auto-tune](measurement.md), and so do [AI agents](ai-agents.md). The words used here are explained in
[Words the app uses](words.md).

Nothing new in 0.16.0 has run on a real DSPi yet: it was tested against simulated devices, a simulated home theatre
and a simulated microphone. Android 0.16.0 has not run on a phone.

## 1. The first run

The first time the desktop app starts, **Getting Started** walks through a few steps. None of them can leave you stuck:

- **Skip Setup** is always at the bottom.
- The **Board** step installs the DSPi firmware on your Pico. Without a board in front of you, choose **My DSPi
  already runs the firmware, or I will connect it later** (Windows and Linux) to go on.
- The **Speakers** step asks which output drives what (see [section 3](#3-your-speakers-settings--speakers)). Its
  main button is **Later** until you change the table; then it is **Confirm and Continue**. Later leaves the layout as
  it is, and it waits for you in Settings › Speakers. Once the layout is confirmed, **Crossovers and Routing…** sets
  the crossovers and routes from it (see [section 4](#4-crossovers-and-routes-from-your-speakers-desktop)).
- With no DSPi connected, the **Welcome** page offers **Open the Demo** (Windows and Linux): a simulated DSPi to try
  everything on (see [section 13](#13-the-demo)).

Help › Getting Started… runs it again.

## 2. The Overview's setup list (desktop)

While something is left to do on the device shown, the **Overview** starts with a short list, **SET UP THIS DSPI**,
and how far you are: "2 of 5 done". Each row says whether it is done and has the one action that does it:

| Row | Its action |
|---|---|
| Speakers confirmed | Set Up Speakers… |
| Inputs routed | Open Routing |
| Crossovers set (only when the layout has woofers, mids, tweeters or a sub) | Open Crossover |
| Measured | Measure |
| Saved to the device | Save to Device |

The list goes when every row is done, or for good on that device when you close it (the × at its right). It is not
shown on a device this app may not change, such as one under the installer's lock.

## 3. Your speakers: Settings › Speakers

**Settings › Speakers** (Settings opens on it) has the one speaker table of the app. The first run's Speakers step
shows the same table, and Measure and Auto-tune show it in one line with **Edit in Speakers…**.

- Each output has a **ROLE**: Full range, Subwoofer, Woofer, Mid driver or Tweeter.
- **SPEAKER** is the speaker's name ("Left", "Right", "Centre", "Sub"). Outputs with the same name are one speaker:
  they are measured together and aligned to each other. The name also gives the speaker its side, which From Speaker
  Layout routes by.
- **Suggest from Routing and Filters** fills the table from what the device already does; **Detect…** finds it with
  the measurement microphone (see [measurement.md](measurement.md)).
- **Confirm** keeps it. Measuring, auto-tune, AI agents and the tweeter protection all go by the confirmed layout.
  An output with an active high-pass at 1 kHz or above is measured as a tweeter, whatever its role here.
- Once it is confirmed, **Crossovers and Routing…** opens the sheet of [section 4](#4-crossovers-and-routes-from-your-speakers-desktop).

Under the installer's lock the layout is the installer's: the page shows it and says "Locked by the installer: the
speaker layout is shown, not changed." No client can change it under the lock (Android hides the page).

The **web console** has Settings › Speakers too (Output, Role, Speaker, Measured together, Confirm, Suggest from Routing
and Filters); the browser keeps the layout for each DSPi. **Android**: see [section 17](#17-on-android).

## 4. Crossovers and routes from your speakers (desktop)

Once Settings › Speakers is confirmed, the app can set the crossovers and the routing your layout asks for in one
step. Nothing changes until you press **Apply**.

- **Equaliser › Crossover › From Speaker Layout…** opens the sheet with the crossovers ticked.
- **Routing › From Speaker Layout…** opens it with the routing ticked.
- **Settings › Speakers › Crossovers and Routing…** (and the first run's Speakers step) opens it with both. In the
  Classic layout this is the way to it.

**Crossovers.** One row per speaker, with each transition between two of its drivers ("woofer ◂▸ tweeter"; a
three-way has two; a sub alone its low pass), each with a frequency and a slope. A speaker shaped like an earlier one
starts as **Same as Left** (or whichever came first) until you untick it. The values start from what the outputs
already have. Apply writes every output's high-pass and low-pass. A full-range output, or one with no other driver in
its speaker, keeps its filters, and the sheet says so.

**Routing.** Each output is fed from its speaker's side: L, R, centre, sub and surrounds, matched to the inputs by
their names. The other outputs keep their routes, and a crosspoint that stays keeps its gain and polarity. A centre or
a sub fed from both sides gets each side 6 dB below what fed it before (never louder than −6 dB each), so it plays no
louder than it did; the sheet says "… gets both sides at … each, so it plays no louder than before." An output whose
speaker's name gives no side keeps its routes.

**The tweeter is protected on the way.** A tweeter's high-pass never goes below 1 kHz, at 12 dB/octave or steeper (or
the installer's higher limit); a mid driver's never below 80 Hz. A lower value is refused with the reason, and Apply
waits. Routing to a tweeter or mid driver that has no high-pass is refused too ("… would play full range: a tweeter or
mid driver needs its high pass first"), unless the same Apply sets its crossover.

**One Undo puts it all back.** Whatever Apply wrote is one step in the History, "From Speaker Layout".

From Speaker Layout is on the desktop only. The web console and Android do not have it yet.

## 5. Outputs named as speakers, and unused outputs

In Modern, an output is called by your speaker's name first and its number second: "Left woofer · 1". So it reads in
the Crossover and Delay & Gain tables, the matrix, the Routing inspector and the Installer page. The command palette
finds outputs by those words: type "tweeter" or "left".

Outputs nothing plays to fold under one row at the end of the Crossover and Delay & Gain tables, "3 unused"; a click
shows them, as "7 · None", and folds them again. These never fold: an output of your speaker layout, an output with a
crossover or an EQ band, and an output a test signal plays on.

## 6. Saving

A change lives on the DSPi only while it is powered, until you save it into the device's active preset.

- **Save to Device** (Ctrl+S) is in the top bar. Its count is how many changes are not saved; hover over the count to
  see them in words. In Modern its question names the device, the preset slot and its name: "Save 3 changes to Living
  Room, preset 1 “Music”?", with the list. Classic's Device › Save to Device… asks "Save current parameters to preset
  slot 1?"
- Each **device tab** shows a dot and "N unsaved" while that device has unsaved changes.
- **Save All…** (in Save to Device's ▾ menu, with two or more devices open) saves every open device with unsaved
  changes after one question that lists them.
- After an auto-tune or a trial is kept, a notice says it is "Kept, not saved on the device yet.", with **Save to
  Device…**.
- **Revert to Saved…** (Ctrl+Shift+R) puts back what the active preset has.
- **Factory Reset…** is no longer next to Save: it is in a device tab's menu and on **Devices › This Computer**
  (Classic: the Device menu). It clears the running settings; saved presets stay.

Closing a tab, quitting, or recalling another preset asks first when something is unsaved.

The **web console** has Save to Device in its top bar; it has no per-tab count and no Save All. On **Android**, Save to
Device... is under the Presets screen's Tools.

## 7. A/B: as saved against now

As soon as you change something, the top bar shows **A | B**:

- **A** is the preset **as saved on the device**; **B** is **what you hear now**, with your changes.
- Click A, or press **B**, to hear the saved settings; click B, or press B again, to come back. There is no snapshot to
  make first.
- A toggle never jumps the volume: the listening volume, Mute and the master volume stay as they are on both sides.
- While you listen to A, your changes on B still count as **unsaved**. Switching preset, closing the tab, quitting or
  recalling a preset on all devices asks first, and Save to Device saves B.
- If the saved preset leaves a tweeter without its high-pass, A holds that tweeter muted, as loading that preset
  would.

**Library › Snapshots** compares any two: under **A/B Compare** choose A (or Current settings) and B, then **Start**;
**Toggle A/B** switches, **Keep A** or **Keep B** ends the compare on one of them. The top bar then names the two.

In Classic, Device › Toggle A/B (key B) does the same. A/B is off under the installer's lock.

The **web console** has the same A | B and key B. In the **Terminal** (`uconsole`), A/B still compares now with a
snapshot. **Android** compares two snapshots on its Snapshots screen ("A/B compare").

## 8. Snapshots and the trial

A **snapshot** is a copy of the device's settings that the app keeps (not the device): **Capture…** in Library ›
Snapshots, then Recall, Compare with Current, Export… and Import…. Auto-tune makes one before it changes anything.

A **trial** is a snapshot recall or an auto-tune result applied for a while: the bar asks "Keep these settings?" and
counts down. **Keep** makes them stay, **Revert** (or the countdown running out) puts back what was there. Leaving it
alone is always safe. In Snapshots, tick **Recall with rollback** to recall on trial. A kept trial is still not saved
on the device until you press Save to Device. Auto-tune's trial is explained in
[measurement.md](measurement.md#72-apply-verify-and-keep).

On the **web console**, snapshots are kept in this browser for each DSPi and survive a reload ("Kept in this browser
for this DSPi."); Export… saves one as a file. Not every screen says "trial" yet: the web's bar is still announced as
"Safe apply", and Android's Snapshots screen says "Recall with rollback".

## 9. Home

**Home** is a remote for listening. It opens first in **Simple** mode and on a DSPi locked by its installer; in the
desktop app it is **Overview › Home**. It shows:

- the device and how it is connected;
- one big **Volume** with **Mute**;
- the **Source**;
- **PRESETS** as a list: click one to recall it (no Save, no Copy Here);
- on a hub's installation, one health line.

Simple's dock has no master volume and no **Bypass EQ**; every other page stays a tab away. Switch the mode with
**Mode…** in the app menu (Classic: Window › Mode…): Simple opens on Home, Advanced opens where the work is. Advanced
is the default everywhere. Home is a page of the Modern layout.

The **web console** has Home too (Volume, Mute, Source, Presets, "Health: No problems", and the rooms' volumes on a
hub's installation); switch with **Mode** in the status bar. **Android**'s main page is its Home: the volume with Mute
beside it, the preset, the source and the meters with a health line.

## 10. The installer's lock

An installer can lock a DSPi with a PIN. Under the lock you may change the **volume, mute, source and presets**;
nothing else. The speaker layout is the installer's too. The lock is on this app, not a security boundary: the PIN
opens it again (the lock icon in the top bar: "Locked by the installer. Unlock…").

- **Modern:** the app opens on Home. The other pages are shown read-only, with "Locked by the installer: this page is
  shown read-only. Volume, mute and presets are on Home." and **Go to Home**. Save, Undo, A | B, and the shortcuts and
  palette rows that would write are off, so nothing you press is refused.
- **Classic** stays as it was: every window is live, and what the lock covers is refused after the click, with the
  line "Locked by the installer: only volume, mute, source and presets can be changed."
- **Web console:** a locked user sees Home only, with that line, the installer's note and **Unlock…**.
- **Android:** a page or a tap the lock refuses shows that line with **Unlock…**; Settings shows only About and
  Installer.

## 11. Several devices (desktop)

In Modern each DSPi you open (on USB, through a hub, or the demo) has a tab of its own, with its own edits, pages and
Undo. With two or more open, the dock offers **All devices**. While it is on (it starts off, and lasts until you quit):

- the User volume moves every open device by as much, each keeping its own difference from the others (Home's volume
  too);
- **Mute** mutes them all;
- a preset recall asks once ("Recall preset 2 on all devices?", naming each device's preset and the unsaved changes it
  drops), then **Recall on All**.

Each device keeps its own limits, and its master volume and source stay its own. The web console and Android have no
All devices.

## 12. Solo an output (desktop and web)

**Solo** plays only one output, to hear what it does: the switch in the Routing inspector, or Solo in an output's
menu. A strip says "Solo: only … plays. The other outputs are muted until you end it; nothing is saved.", with **End
Solo**.

- A solo only mutes the other outputs on the device. It never unmutes anything, and never touches a filter, so the
  tweeter protection stays as it is.
- It never reaches a preset, a snapshot or a save: it ends before anything is saved, loaded or measured, and before
  the device is closed. It is not an Undo step.
- If the device or its DSPi Link connection is lost during a solo, the app lifts the solo's mutes when it connects to
  that DSPi again, before it looks at what is unsaved.
- If the desktop app is killed during a solo, the outputs it muted stay muted on the device (not saved) until you
  unmute them.
- **Web console:** closing or reloading the page during a solo brings the browser's own question to leave, and the
  solo ends as the page goes, as far as the browser lets it finish.

## 13. The demo

The demo is a simulated DSPi, set up as a tuned two-way speaker: a crossover at 2.2 kHz, a room-mode cut, a few EQ
bands, the tweeters a little quieter and later, outputs named "Left woofer" to "Right tweeter", and a confirmed speaker
layout. Nothing is unsaved when it opens. Open it with **Open Demo** in the strip shown with no DSPi connected.

**Reset Demo** (in the demo's strip, and Device › Reset Demo in Classic) starts it again as it was. The web console
has Reset Demo in its demo strip; Android has **Reset demo**, with **Exit demo** on the bar.

## 14. Accessibility (desktop and web)

**Settings › Appearance › Accessibility** has two choices:

- **Outline controls**: a line round every field, menu and button, for when they are hard to tell from the page.
  Both layouts.
- **Control size**: Automatic, Standard or Comfortable (larger controls and targets, in Modern). Automatic is
  Comfortable on a computer with a touch screen.

Both are off and Automatic by default: with a mouse nothing changes. The web console has the same two in Settings ›
Appearance, kept in this browser.

## 15. Keys and the command palette

**Help › Keyboard Shortcuts** (Ctrl+/) lists every key of the app (Modern). First the keys for moving around:

| Key | What it does |
|---|---|
| Ctrl+K | Search settings, channels and commands |
| F6 | Next area: top bar, sections, page, inspector, dock |
| Shift+F6 | Previous area |
| Arrow keys | Another section or tab, in the section rail or the tab row |
| Ctrl+PgDown, Ctrl+PgUp | Next and previous tab of the page |
| Ctrl+/ | This list |

Then each menu's keys, among them Save to Device… (Ctrl+S), Revert to Saved… (Ctrl+Shift+R), Undo (Ctrl+Z), Redo
(Ctrl+Shift+Z), Mute (Ctrl+M), Bypass Master EQ (Ctrl+B), Toggle A/B (B) and Preset 1 to 9 (Ctrl+1 … Ctrl+9). In
Classic, the menu bar shows each key beside its command.

The **command palette** (Ctrl+K, or the top bar's "Search settings, channels, commands…") finds a page, a setting, a command, a channel or another
open device by the words you type, also by other names: "high-pass", "slope" or "tweeter" find the Crossover, "roles"
finds Settings › Speakers, "keys" this list. Its **Help: …** entries open the manual where it answers:

- Help: Connect an AI Agent
- Help: The Questions an Agent Asks
- Help: Watching and Undoing an Agent
- Help: Why Was My Measurement Refused?
- Help: Auto-tune's Trial, Keep or Revert
- Help: USB Access on Linux
- Help: The Words the App Uses

The **web console** has the same palette (Ctrl+K, ⌘K on a Mac) with the same Help entries, and a **Keyboard
Shortcuts** list (Shift+?); it has no F6.

## 16. On the web console only

- **Devices › Fleet**: a page served by a hub starts here, with every DSPi of the hub, its state, alarms and firmware,
  **Identify** (blinks its LED) and **Open**.
- **Snapshots** are kept in this browser for each DSPi and survive a reload; Export… gives a file.
- **Settings › Graphing › Show Phase Response** draws the selected channel's phase on the EQ graph.
- **Crossfeed** has the **Outputs** picker: which two outputs each crossfeed pair processes (Default, One DAC per ear
  (K340), Custom…).
- The **Routing inspector** sets an output's **Polarity** (Normal or Inverted).
- **Add to Home Screen** (your browser's menu) installs the console as an app.

## 17. On Android

- **Home**: Mute is the speaker button beside the volume. The notification's Vol − and Vol + move the volume 1 dB a
  press.
- **Settings › Speakers** ("Which output drives what?"): each output's Role (Tweeter, Mid, Woofer, Sub, Full range,
  None) and Side (L, R, C), the high-pass it needs ("Its high-pass: at least 1000 Hz, 12 dB/octave" for a tweeter),
  **Same for the right side**, **Use the Suggestion** and **Confirm**. **Getting Started** has the same Speakers step,
  with **Later** until you confirm. Under the installer's lock neither is there.
- **The band editor**: on a phone, a band opens in a sheet with − and + beside its Freq, Gain and Q, and Done. An
  output's page has its crossover in two rows above the bands (High-pass and Low-pass), with **Same for the right
  side**. On the Response Graph, a tap on a channel's pill chooses the channel you edit.
- **The installer's lock**: a tap the lock refuses says why, with **Unlock…**.
- **Notices** when the USB DSPi goes away and comes back; **Exit demo** and **Allow USB access** are buttons on the bar.
- **Phones open in Advanced** (Mode… in the ⋮ menu switches).

Android has no Modern layout, no measurement, no From Speaker Layout, no Solo and no A/B as saved.

## 18. Not there yet

- From Speaker Layout is on the desktop only: the web console and Android do not have it.
- The setup list, the per-tab unsaved count, Save All and All devices are the desktop's.
- The Terminal's A/B compares now with a snapshot, not with what is saved.
- Not every screen says "trial" yet (the web's "Safe apply", Android's Snapshots).
- Not in this release: the desktop app's layout for low-resolution screens, Android's Modern navigation (a bottom bar
  and sections) and larger fonts on Android. The iPhone and iPad app is not part of this release.

## See also

- [Words the app uses](words.md)
- [Measuring and auto-tune](measurement.md)
- [AI agents](ai-agents.md)
- [Installing](../INSTALL.md)
