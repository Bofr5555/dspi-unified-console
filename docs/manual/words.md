# Words the app uses

What the words on the screens mean, in a line or two each, with where to read more. The apps (desktop, web,
Android, Terminal) use the same word for the same thing. The command palette (Ctrl+K) finds a page by these words
too: type "high-pass" and it offers the Crossover.

This page is for users. Writers of the app keep their own list of terms in [`docs/GLOSSARY.md`](../GLOSSARY.md).

## Outputs and speakers

**Output (Out 1 · L).** A DSPi output, counted from 1. Until you name it, the app calls it by its number, the channel
that feeds it and its band: "Out 1 · L · Low". A name you give it, or the speaker layout's ("Left woofer"), comes
first, the number second: "Left woofer · 1".

**Unused output ("3 · None").** An output nothing is routed to. Pages that only show leave it out; pages where you
edit outputs list it as "3 · None". An output that plays is never hidden.

**PDM.** The DSPi's subwoofer output, a pulse-density (PDM) signal on one pin. It is the last output (Out 9 on the
RP2350).

**Speaker layout.** Which output drives which speaker, set under Settings › Speakers: by hand, from a detection you
confirm, or from what you told an AI agent. The app, the measurement and the agents all go by it.

**Role: woofer, mid driver, tweeter, subwoofer, full range.** What an output's speaker plays. Roles come from you
(the "stated roles"), never from a guess the app keeps on its own.

**Tweeter protection.** An output set up as a tweeter needs an active high-pass of at least 12 dB/octave at 1 kHz or
above, a mid driver one at 80 Hz or above. Nothing is measured on an output without it, and an agent asks before it
lowers it. An installer's limits may raise this floor ("tweeter floor"), never lower it.

## Filters

**PEQ.** The equaliser's bands on each channel. Each band has a shape, a frequency, a gain and a **Q**.

**Bell, Low Shelf, High Shelf, Low Cut, High Cut, Notch, All Pass.** The band shapes. A bell lifts or cuts around its
frequency; a shelf everything above or below it; a low cut (high-pass) removes what is below, a high cut (low-pass)
what is above.

**Q.** How narrow a band is: a high Q is narrow, a low Q wide.

**Crossover.** The high-pass and low-pass filters that split the sound between a speaker's drivers, set on the
Equaliser's Crossover page.

**Slope (LR 24).** How steeply a crossover filter falls past its frequency, in dB per octave. "LR 24" is a
Linkwitz-Riley filter of 24 dB/octave, the usual choice for a two-way speaker.

**Routing (matrix, crosspoint).** Which input feeds which output. Each crosspoint of the matrix connects one input to
one output, with its own gain.

## Saving and going back

**Unsaved changes.** What you changed since the device last saved. The count on Save to Device says how many; hover
over it, or close the device's tab, to see them in words.

**Save to Device (Ctrl+S).** Writes the settings you hear now into the device's active preset, so they stay after a
power cut. Until then a change lives only while the DSPi is on (the agents' questions call the device's memory
"flash").

**Preset (slot).** One of the device's stored sets of settings. Recall Preset loads one; the preset that loads at
power-on is the startup preset.

**Revert to Saved.** Puts every setting back as the active preset has it: your unsaved changes go.

**Factory Reset.** Clears the settings you hear now to the firmware's defaults. Your saved presets stay. It is in the
device's menu (right-click its tab), not under Save.

**Snapshot.** A copy of the settings you keep in the app, to go back to or to compare with. Auto-tune makes one
("Before auto-tune") before it changes anything.

**A/B.** Switches between two snapshots, so you can hear the difference. Press B.

**Undo and History.** Every change you or an agent make can be undone (Ctrl+Z); History lists them.

**Trial (Keep or Revert).** A change the app applies on trial: auto-tune's result, a hub's new delays. The banner asks
"Keep these settings?" and counts down. Keep makes them stay; Revert, or the countdown running out, puts everything back
as it was. Leaving it alone is always safe.

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

**Installer lock and limits.** An installer can lock a DSPi with a PIN and a limits profile: the highest levels, the
tweeter floor, what a user may change. A locked app says who locked it and what still works, opens on Home, and
shows the other pages read-only, so nothing you press is refused.

**Simple and Home.** Simple is a mode for listening (Mode… in the app menu; on the web, Mode in the status bar).
It opens on **Home**, a remote: the volume, Mute, the source, the presets to recall and, on a hub's installation,
each room's volume. Every other page stays a tab away. Advanced, the desktop's and the web's default, opens where
the work is.

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

**Stop and Pause.** Pause holds what an agent does until you resume; Stop turns agents off. Changes already made stay,
and you can undo them.

## Getting started

**Demo device.** A simulated DSPi in the app, to try everything without hardware: Open Demo.

**BOOTSEL.** The button on the DSPi's board you hold while plugging it in, so it starts as a USB drive that takes new
firmware.
