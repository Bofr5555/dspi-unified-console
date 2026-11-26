# Words the app uses

What the words on the screens mean, in a line or two each, with where to read more. The apps (desktop, web,
Android, Terminal) use the same word for the same thing. The command palette (Ctrl+K) finds a page by these words
too: type "high-pass" and it offers the Crossover.

This page is for users. Writers of the app keep their own list of terms in [`docs/GLOSSARY.md`](../GLOSSARY.md).

## Outputs and speakers

**Output (Out 1 · L).** A DSPi output, counted from 1. Until you name it, the app calls it by its number, the channel
that feeds it and its band: "Out 1 · L · Low". A name you give it, or the speaker layout's ("Left woofer"), comes
first, the number second: "Left woofer · 1".

**Unused output ("3 · None", "N unused").** An output nothing is routed to. Pages that only show leave it out; pages
where you edit outputs list it as "3 · None". In Modern, the Crossover and Delay & Gain tables fold such outputs under
one row at the end, "3 unused", which a click opens. An output that plays is never hidden, and an output of your speaker
layout or one with a crossover or an EQ band never folds.

**PDM.** The DSPi's subwoofer output, a pulse-density (PDM) signal on one pin. It is the last output (Out 9 on the
RP2350).

**Speaker layout.** Which output drives which speaker, set under Settings › Speakers: by hand, from a detection you
confirm, or from what you told an AI agent. It is one table, edited there and in the first run's Speakers step;
Measure and Auto-tune show it in one line with **Edit in Speakers…**. The app, the measurement and the agents all go
by it. Under the installer's lock it is the installer's and cannot be changed.

**Speaker (name).** The speaker an output belongs to, named in the layout ("Left", "Centre", "Sub"). Outputs with the
same name are measured together, and the name gives the speaker its side when From Speaker Layout routes it.

**Role: woofer, mid driver, tweeter, subwoofer, full range.** What an output's driver plays. Roles come from you
(the "stated roles"), never from a guess the app keeps on its own.

**Tweeter protection (tweeter floor).** An output set up as a tweeter needs an active high-pass of at least
12 dB/octave at 1 kHz or above, a mid driver one at 80 Hz or above. Nothing is measured on an output without it, a
lower crossover is refused with the reason, and an agent asks before it lowers it. An installer's limits may raise this
floor, never lower it.

**From Speaker Layout.** Sets the crossovers (Equaliser › Crossover › From Speaker Layout…) and the routing (Routing ›
From Speaker Layout…) that your confirmed speaker layout asks for, when you press Apply, as one Undo step. Desktop
only. See [Everyday use](everyday.md#4-crossovers-and-routes-from-your-speakers-desktop).

**Solo.** Plays only one output for a while: the others are muted on the device until **End Solo**. Never saved,
never an Undo step.

## Filters

**PEQ.** The equaliser's bands on each channel. Each band has a shape, a frequency, a gain and a **Q**. The band
shape that lifts or cuts around a frequency is a **Bell** (some apps call it "peaking").

**Bell, Low Shelf, High Shelf, Low Cut, High Cut, Notch, All Pass.** The band shapes. A bell lifts or cuts around its
frequency; a shelf everything above or below it; a low cut (high-pass) removes what is below, a high cut (low-pass)
what is above.

**Q.** How narrow a band is: a high Q is narrow, a low Q wide.

**Crossover (High pass, Low pass).** The high-pass and low-pass filters that split the sound between a speaker's
drivers, set on the Equaliser's Crossover page. A high pass lets through what is above its frequency (a tweeter's), a
low pass what is below (a woofer's or a sub's).

**Slope (LR 24).** How steeply a crossover filter falls past its frequency, in dB per octave. "LR 24" is a
Linkwitz-Riley filter of 24 dB/octave, the usual choice for a two-way speaker.

**Routing (matrix, crosspoint).** Which input feeds which output. Each crosspoint of the matrix connects one input to
one output, with its own gain.

## Saving and going back

**Unsaved changes ("3 unsaved").** What you changed since the device last saved. The count on Save to Device says
how many; hover over it to see them in words ("Out 1 · L gain"). Each device tab shows a dot and "N unsaved" while
that device has some.

**Save to Device (Ctrl+S).** Writes the settings you hear now into the device's active preset, so they stay after a
power cut. Until then a change lives only while the DSPi is on (the agents' questions call the device's memory
"flash"). Its question names the device, the preset slot and its name. Classic's menu called it Commit Parameters
before.

**Save All.** With several devices open, saves every one that has unsaved changes after one question (in Save to
Device's ▾ menu).

**Preset (slot).** One of the device's stored sets of settings. Recall Preset loads one; the preset that loads at
power-on is the startup preset.

**Revert to Saved.** Puts every setting back as the active preset has it: your unsaved changes go.

**Factory Reset.** Clears the settings you hear now to the firmware's defaults. Your saved presets stay. It is in the
device tab's menu and on Devices › This Computer (Classic: the Device menu), not under Save.

**Snapshot.** A copy of the settings you keep in the app, to go back to or to compare with. Auto-tune makes one
("Before auto-tune") before it changes anything.

**A/B (as saved, now).** The top bar's A | B: **A** is the preset as saved on the device, **B** is what you hear
now, with your changes. Click A or press B to switch; the volume never jumps. Your changes on B still count as
unsaved while you listen to A. Library › Snapshots compares any two snapshots the same way. (The Terminal's A/B
compares now with a snapshot.)

**Undo and History.** Every change you or an agent make can be undone (Ctrl+Z); History lists them.

**Trial (Keep or Revert).** A change the app applies on trial: auto-tune's result, a snapshot recalled with rollback,
a hub's new delays. The banner asks "Keep these settings?" and counts down. Keep makes them stay; Revert, or the
countdown running out, puts everything back as it was. Leaving it alone is always safe. A kept trial is not saved on
the device until you press Save to Device. (Some screens still say "safe apply" or "rollback".)

**Setup list ("Set up this DSPi · 2 of 5 done").** The Overview's list of what is left to set up on a device: speakers
confirmed, inputs routed, crossovers set, measured, saved to the device, each with its action. Desktop.

## Measuring

**Measurement (sweep).** The app plays a tone that rises from low to high on an output, and a microphone hears it. The
volume must be at -12 dB or lower, and the placement of the microphone chosen first.

**Walked run.** One measurement that plays each output in turn, so the app knows when each one arrives.

**Placement.** Where the microphone is: at the listening position, close to a driver, or on a headphone coupler.

**Calibration file.** The file that came with your microphone. With it, levels are in dB SPL and the response is
corrected; without it they are relative.

**Auto-tune.** Proposes crossover frequencies, delays, levels and polarity from a measurement. You see the proposal
first, it is applied on trial, measured again, and kept only if you press Keep.

**Alignment group.** The outputs that play one speaker together (its woofer and tweeter, say). Auto-tune aligns the
outputs within a group, not across groups.

**Summed check.** After a change to a crossover or a delay, the outputs of a group play together and the app checks
that their sum is what the change predicted. A change that fails it is not applied.

**dBFS, dB SPL.** dBFS is a level inside the DSPi, 0 dBFS being the loudest it can make. dB SPL is how loud it is in
the room, which only a calibrated microphone can say.

**Refused.** A measurement or a change the safety rules stop, with the output, the rule and what to change. See
[why a measurement is refused](measurement.md#4-the-safety-rules-and-what-they-mean-for-you).

## Several devices and the network

**Hub.** A program that shares the DSPis connected to one machine with other apps on the network: the desktop app
(Settings › Networking) or `uconsole hub` in a terminal.

**PIN (pairing).** The code a hub shows when you choose Allow a New Client (or type `pair` at `uconsole hub`'s
prompt). Enter it in the app that wants to connect.

**All devices.** The dock's link, with two or more devices open in the desktop app: the volume, Mute and a preset
recall then act on every open device, each keeping its own difference and limits.

**Installer lock and limits.** An installer can lock a DSPi with a PIN and a limits profile: the highest levels, the
tweeter floor, what a user may change. Under the lock you may change the volume, mute, source and presets; the speaker
layout is the installer's. A locked app says who locked it and what still works and opens on Home. In Modern the
other pages are read-only with **Go to Home**; the web shows Home only; Classic stays live and refuses after the click.

**Simple and Home.** Simple is a mode for listening (Mode… in the app menu; on the web, Mode in the status bar).
It opens on **Home** (in the desktop app Overview › Home), a remote: the device, the volume with Mute, the source, the
presets to recall, one health line and, on a hub's installation (web), each room's volume. Simple's dock has no master
volume and no Bypass EQ. Every other page stays a tab away. Advanced, the default everywhere (desktops, the web and
phones), opens where the work is.

## AI agents

**AI agent (MCP).** A program such as Claude Desktop or Claude Code that reads and changes your DSPi through the
running app, over MCP, the protocol these programs speak. Turn agents on under Settings › AI Agents. See
[AI agents](ai-agents.md).

**Pairing code.** The four digits the app shows when an agent connects for the first time; the agent's side has the
same code (in its chat or its log). Allow it only if they match.

**Question (Ask me).** Before a change you chose to be asked about, the app shows what the agent wants to do. Under
"About risky changes" (the default) that is a change that makes something louder, unmutes or switches on an output,
loads or saves a preset, lowers a tweeter's protection, or plays a test signal. A question nobody answers lapses after
2 minutes, and nothing changes.

**Test-signal grant.** Allowing an agent to play test signals lets it do so on that device for 10 minutes. Revoke ends
it at once.

**Stop and Pause.** Pause holds what an agent does until you resume; **Stop Agents** turns agents off. Changes already
made stay, and you can undo them.

**Agent Activity.** The panel that lists what agents did and do, with Undo for each change (Device › Agent Activity…,
or the agent strip).

## Getting started

**Demo device.** A simulated DSPi in the app, set up as a tuned two-way speaker, to try everything without hardware:
Open Demo. **Reset Demo** starts it again as it was (Android: Reset demo).

**BOOTSEL.** The button on the DSPi's board you hold while plugging it in, so it starts as a USB drive that takes new
firmware.
