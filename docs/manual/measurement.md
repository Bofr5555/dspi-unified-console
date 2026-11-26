# Measuring and auto-tune

*User manual of uConsole for DSPi 0.15.0: measuring in the web console and in the desktop app (both layouts). The
technical contract is [docs/integrator/measurement.md](../integrator/measurement.md).*

With a measurement microphone you can see what each output of your DSPi really does in the room or on a headphone
coupler. You can then correct it from the measured curve, or let auto-tune line up a multi-way speaker for you. Every
change is shown before it is written, and every change can be undone.

**Where to find it**

| Console | Where | What it offers |
|---|---|---|
| Web console | the **Measure** screen, with the tools (Advanced and Installer modes) | tabs Measure, Results, Live, Auto-tune, Documents; Passive RTA when you may not play |
| Desktop app, Modern layout | **Measure** in the rail on the left (Ctrl+K, "Measure", also finds it) | tabs Measure, Results, Live, Auto-tune, Documents, Several units; Passive RTA when you may not play |
| Desktop app, Classic layout | **Tools > Measure…** | the same tabs, in a window |
| Terminal (`uconsole measure`) | the command line | importing calibration and target files, listing, showing and exporting documents, and a tune from a saved measurement. It opens no microphone |

On the demo device a **Simulated microphone** stands in for a real one, so you can try every step without hardware. It
is labelled that way everywhere and never runs on a real DSPi.

## 1. How a measurement works, in short

- **The DSPi plays the test signal itself**, with its built-in signal generator (firmware v1.1.5 or later). The
  console never plays test audio. If the DSPi is unplugged, nothing can fall back to other speakers. Without a
  generator, only the Passive RTA is offered, and the screen says why.
- **The microphone is the console's audio input.** The console sends the samples to the measurement, which works out
  each output's response.
- **Outputs are measured one after the other in one run** ("walked" sweeps). Each output plays a sweep, then the next
  output plays, and the whole round repeats a few times (the **passes**). Timing between outputs is only compared
  inside one run.
- **Raw microphone audio is never saved.** A measurement keeps only the curves it derived. A WAV file is written only
  when you export one.

## 2. Placing the microphone

Do not move the microphone, or anything near it, during a run. Keep the room quiet: a measurement starts only when no
input is playing, and stops if audio starts.

### 2.1 Speakers

Pick the placement in the setup. Nothing is chosen for you: **Measure** stays off until you have picked one, because
headphones are measured much more quietly than speakers.

| Placement | Where the microphone goes | What it is good for |
|---|---|---|
| **Listening position** | where your head is when you listen, at ear height | alignment (delay, polarity), the crossover and the EQ. This is the one auto-tune needs |
| **Several positions** (3 to 9) | position 1 at the listening position, the others spread around it, over the area where people sit. You move the microphone when asked (**Next position**) | an EQ that suits more than one spot. Dips that change more than 3 dB between positions are never filled. Only position 1 is used for timing |
| **Near-field** | a few centimetres from one driver, on its axis | seeing one driver on its own. It is kept as a labelled near-field measurement and not used by auto-tune yet |

Point the microphone the way its calibration file expects:

- a **0°** calibration: point the microphone at the speakers;
- a **90°** calibration: point it at the ceiling. Use this when the speakers are left and right of the listening
  position. The console notices a 90° file from its name (`_90deg`) or its header.

Use **3 passes or more** when you want auto-tune to touch timing. A run of 2 passes is always marked "timing
unreliable" (section 6); the setup says so under **Passes** when it is 2.

### 2.2 Headphones and couplers

Headphones are measured **on a coupler, not on a head**: the console reminds you of that before each run. Choose the
coupler type in the setup:

| Coupler type | What it is |
|---|---|
| **IEC 60318-4 ear simulator** (also called a 711 coupler) | a standard ear simulator |
| **Flat-plate coupler** | a flat plate with the microphone in it |
| **HATS or another coupler with pinna** | a head and torso simulator, or another coupler with an artificial ear |

Each type has its own default target, our own curve, labelled as ours. A compensation file from the coupler's maker is
better than the default; you can import one as a target (section 3.2).

**A home-made flat plate.** A measurement microphone squeezed through a hole in a plate of polystyrene (or other stiff
foam) is a flat-plate coupler. Choose **Flat-plate coupler**. For good results:

- push the capsule through until it is **flush with the surface**, neither sunk in nor sticking out, and make the hole
  tight so no air leaks round it;
- use a plate that is flat, wider than the ear cup, and thick enough not to flex;
- lay the cup on the plate with the capsule where your ear canal would be, roughly in the middle of the cup;
- press or weigh the headphones down evenly so the pads seal. A leak shows as a loss of bass that changes from seat to
  seat.

**Reseats.** Choose 1 to 9 reseats. Between runs you lift the headphones and put them back, and the console averages
them and keeps how much they spread. Several reseats show you how much of the curve is the seal rather than the
headphones. Left and right are measured on their own.

On a coupler the levels are lower than for speakers (section 4), and the pause between sweeps is shorter.

## 3. Calibration files and targets

### 3.1 The microphone's calibration

Import the file that came with your microphone (the calibration file field of the setup in the web console,
**Import Calibration…** in the desktop app, or `uconsole measure mic import FILE`). It becomes a `mic` document that you can use again. These formats are read:

| File | What it gives |
|---|---|
| **miniDSP UMIK-1 / UMIK-2** text, with its `Sens Factor` line | the correction curve, and a sound level scale from the sensitivity |
| **REW** calibration text (frequency, dB, optionally phase) | the correction curve. A `Sensitivity … dBFS` line also gives a sound level scale |
| **CSV** (comma, semicolon or tab) | the correction curve only |
| **Dayton Audio** (iMM-6C, UMM-6), with its `*1000Hz` sensitivity line | the correction curve, and a sound level scale from the sensitivity. A UMM-6 file with a `Sens Factor` line is read as a USB microphone, like a UMIK |

The correction curve is applied to every measurement made with that microphone.

**Levels in dB SPL are approximate.** The console cannot read your input's gain setting, so a scale from a sensitivity
is always marked **approximate**. Set the microphone input as its maker says (for a UMIK, the gain its file was made
for).

**The Dayton full-scale assumption.** A Dayton file gives the sensitivity in volts per pascal (dBV/Pa).
To turn that into a level, the console assumes that a full-scale signal on your input is 1 V RMS (0 dBV). Most
headset and interface inputs reach full scale below 1 V. There the console reads levels **too high**, so the sound
level cap stops the ramp early, which errs on the safe side. On a line input whose full scale is above 1 V it reads
levels **too low**: the sound level cap then acts later than it should, and the level shown is lower than the real
one. The generator's level ceiling still holds, as it does with no calibration file.

**A calibrator step would override the assumption.** With an acoustic calibrator (a 94 or 114 dB tone at 1 kHz), the
measured level of the tone would replace every assumption above, and the scale would no longer be approximate. The
maths for this is built, but **no screen offers the calibrator step yet**, so today's levels are always marked
approximate.

**Without a calibration file**, levels are relative (dB, not dB SPL), the response is uncorrected, and there is **no
sound level cap**: only the generator's level ceiling limits the level. The console warns you of this before a run.

### 3.2 Targets

A target is the curve the EQ aims for, drawn dashed on every graph. Built in, all by our own parameters: **flat**, a
**tilt**, a **house curve** (bass shelf plus tilt), and one default per coupler type. You can import any target or
coupler compensation as REW or CSV text (the target file field in the web console, **Import Target…** in the desktop app, or `uconsole measure target import
FILE`). A target is
lined up with your curve over a reference band (500 Hz to 2 kHz by default).

## 4. The safety rules, and what they mean for you

The measurement protects your speakers, your headphones and your ears. These rules cannot be switched off. When one
stops you, the console says which one and what to do.

| Rule | What you notice |
|---|---|
| **No player** | The DSPi plays the test signal, never the console. Nothing can come out of other speakers |
| **Volume** | A measurement starts only with the DSPi's volume at **-12 dB or lower**. The measurement never changes the volume, a mute or a gain itself. Turn it down before you start |
| **Start quiet, then ramp** | Each output starts very quietly: -50 dBFS for speakers, -60 dBFS on a coupler. It gets 6 dB louder a step until the microphone hears it clearly. It never goes past a ceiling (speakers -12 dBFS, coupler -30 dBFS) or, with a calibration, past 95 dB SPL for speakers or 90 dB SPL on a coupler. If the microphone does not hear the output, or stops getting louder with it, the ramp stops and asks you to check the microphone and the wiring. Never full scale |
| **Stop on clipping** | If the microphone or an output reaches full scale, or the sound gets past the level cap, the generator stops at once and the run is thrown away |
| **Stop on anything foreign** | The run is stopped and thrown away if another console changes a setting or loads a preset, the volume changes (including the volume sent over USB), the input rate changes, audio starts playing, the microphone's capture skips, the DSPi stops answering, or you press **Stop**. Nothing is retried by itself |
| **Tweeter protection** | A mid or high way is measured only through its own high-pass filter: at least 1 kHz for a tweeter and 80 Hz for a midrange, at least 12 dB/octave (an installer's limits profile may set other values; a profile's high-pass minimum also holds the output's music high-pass, so it belongs on tweeter and midrange outputs, never on a woofer's). The filter is never bypassed. Auto-tune never sets a high-pass below these values |
| **Every run ends by itself** | Sweeps repeat a set number of times, pink noise runs at most 60 s, and a run lasts at most 120 s. If the console crashes, the DSPi stops on its own |
| **Silence first** | A run starts only when no input is playing. Stop your music first |
| **Bounded changes** | A proposal never boosts the EQ by more than +3 dB (speakers) or +6 dB (headphones), never cuts more than 15 dB, and never raises an output's level. A level change is always a cut. An EQ boost is balanced by an equal cut of the output's gain, so you keep your headroom |
| **Guarded and reversible** | A tune is applied only after the checks run again, a "Before auto-tune" snapshot is saved, and the trial has started its countdown (section 7) |
| **Locks and roles** | You need a connection that may write, read-only mode off, and the Control or Admin role on a hub. Measuring must not be locked by an installer. A measurement holds the device lock, so other consoles cannot change the DSPi meanwhile |
| **Privacy** | Microphone audio stays in memory for the run only. It is never logged, uploaded, or put in a document |
| **No hidden processing** | A run is refused while crossfeed, psychoacoustic bass, the tube preamp, the subharmonic synthesizer, loudness, or a limiter that would act is on for a measured output: the measurement cannot model them. Switch them off for the run. The console names what to switch off and changes nothing itself |

## 5. Results and the live view

**Results** shows each output's curve with the target dashed. Smoothing is 1/6 octave by default, and you can change it
or hide the target. Points the measurement does not trust (too much noise, or repeats that disagree) are drawn faint
and are never used by the EQ fit. You can export an output as REW text.

**Live** repeats short sweeps on one output, or plays pink noise with the microphone RTA (fast, slow or infinite
averaging, with hold and compare). Over the curve, **Cut this peak** and **Fit this range** propose one EQ change and
show the predicted curve first. Applied, each change is one undo step.

## 6. "Timing unreliable"

Delay, polarity and crossover changes need precise timing. A run is **timing reliable** only when every sweep lands
where the clock fit expects it (within 10 µs), the repeats agree in phase through the crossovers, and the microphone's
clock stayed steady. Otherwise the result says **"timing unreliable"**, with the reason in words. Its curves are still
good, so auto-tune then proposes the **EQ only** and says why.

Common reasons:

| Reason | What to do |
|---|---|
| fewer than 3 passes | measure again with 3 passes or more |
| the microphone or something near it moved, or there was noise | keep still and quiet, then measure again |
| "the capture skipped", lost or repeated frames | close other programs that use audio. Another console, or an input without resampling, may do better |
| an arrival the analysis could not settle (often a tweeter) | more level or more passes |
| **"the capture clock wandered … µs / … ppm inside its sweep"** | see below |

**USB microphone clock wander.** A USB microphone runs on its own clock, not the DSPi's. A steady difference between the
two clocks is measured and taken out. But some USB microphones let their clock **wander** during a sweep. In our own
test, a USB measurement microphone wandered by 20 to 80 µs, 30 to 95 parts per million. No analysis can take that out.
Your magnitude curves and the EQ are unaffected, but the timing is not trusted. If you see this reason every time,
use a capture path with a steadier clock, such as an analogue measurement microphone on an audio interface.

## 7. Auto-tune

Auto-tune lines up a multi-way speaker: the delay and polarity of each way, the crossover corner and the levels, then an
EQ toward the target. Each step shows its result before the next, and you can stop after any step with nothing written.

1. **Layout.** Your speaker layout in one line: each speaker with its outputs and their roles (woofer, mid driver,
   tweeter, subwoofer), and whether you confirmed it. You correct it in one place, **Settings › Speakers** (**Edit in
   Speakers…**); outputs with the same speaker name are measured together. Untick an output to leave it out of this
   tune.
2. **Checks.** Everything that would stop a run (section 4) is listed in words.
3. **Measure.** One walked run of each speaker's ways at the listening position, three passes. Choose where it is
   measured first: in the web console's Auto-tune tab **Measured: Speakers, at the listening position** or
   **Headphones on a coupler** (it takes the setup's choice when that is one of the two; nothing is assumed). For
   headphones the run then uses the lower headphone levels of section 4. In the desktop app the tune uses the
   measurement you made on the Measure tab, with the placement you chose there.
4. **Options.** You can keep the crossover (align only), keep the delays, keep the levels, or leave the EQ out (**No EQ
   fit**).
5. **Summed check, always.** The ways of each crossover play together, and the measured sum is compared with what the
   measurement predicts. If they disagree by more than 1.5 dB, every delay, polarity and crossover change is withdrawn
   with the reason, and only the EQ remains. Timing and crossover changes are applied only after the summed check has
   passed. The web console keeps Apply disabled until it has run.
6. **Proposal.** Every change in words, for example "Output 2 (L high): crossover 2.8 kHz LR4 → 3.1 kHz LR4; delay 0 →
   0.21 ms". It also shows the curves before and predicted, what was held to a limit, and what was withdrawn and why.
   Nothing is written yet.

### 7.1 What auto-tune changes, and what it will not change

It may change, within the limits of section 4:

- each way's **delay** (the smallest delay of a speaker keeps its present value, so an existing left/right or sub
  alignment stays);
- a way's **polarity**, through the matrix's invert of the crosspoints that feed it;
- the **crossover corner**, within half an octave of the current one, keeping the filter's family and order (an LR4
  stays an LR4);
- the ways' **levels**, by cuts only;
- **free PEQ bands** (flat or bypassed ones) toward the target. For speakers this is 20 to 300 Hz by default, for
  headphones 20 Hz to 10 kHz.

It will not:

- **lower a tweeter's high-pass on the curve's level alone.** A lower corner for a tweeter is only proposed together
  with the timing that goes with it: from a timing-reliable run, applied only after the summed check passed, and never
  below the tweeter protection of section 4. With **Keep the delays** it is never lowered at all;
- **change timing it cannot settle.** On a steep crossover (LR6, LR8, BW3 or BW4 near a low corner such as 120 Hz),
  or where the best timing is half a cycle away from what was measured, the delay and polarity are **kept**. The
  proposal's note says so, for example "the timing could not be settled at this corner; the delay and polarity are
  kept". The measured arrivals are first corrected for the crossover's own delay, so most
  crossovers do get their timing. The note remains for the cases that are still unclear;
- raise any output's gain, touch the volume, overwrite a PEQ band you have set (unless you free it), change a filter
  family or order, or apply anything without your press.

### 7.2 Apply, verify and Keep

1. **Apply.** The checks run again. A "Before auto-tune" snapshot is saved (on the hub when you use one, otherwise in
   the console). The changes are then written as one undo step, applied on **trial** with a countdown. The countdown
   is long enough for the verify plus a margin, at most 590 s. If a verify would take longer, the proposal warns you.
2. **Verify.** The tuned speaker is measured again: a walked run and a summed run for each crossover. It passes when
   everything is within 1.5 dB of the prediction.
   - Web console: the verify starts by itself right after the apply.
   - Desktop app: press **Verify**.
   - Terminal: only on the demo device. Elsewhere it has no microphone, so it applies only EQ-only tunes, says they are
     unverified, and leaves timing changes to the web console or the desktop app.
3. **Keep.** The rules for Keep:
   - **While the verify runs**, Keep waits for its verdict. The web console does not offer it then, and the other
     consoles say "The verify is still running".
   - **After a verify that passed**, one press of Keep keeps the tune.
   - **After a verify that failed**, Keep takes **two steps**. The first press is refused with "The verify did not
     confirm the tune". You are asked again, and only your second press, at least a second later, keeps it. The second
     question is withdrawn if you revert, apply again, verify again, or the countdown ends. Nothing ever presses Keep
     for you.
   - **Before any verify has given a verdict:** in the desktop app and the Terminal one press of Keep keeps the tune,
     unverified (the Terminal says "Not verified"). The web console asks you once more ("The tune is not verified")
     before it keeps.

   Keep saves an "After auto-tune" snapshot, which records which measurement the tune came from.

### 7.3 How to go back

- **Revert** during the countdown puts every setting back as it was in the "Before" snapshot.
- **The countdown running out** does the same. Leaving it alone is always safe.
- **A lost connection:** through a hub that runs the trial, the hub puts the settings back. Over USB nothing can
  be written while the DSPi is gone; Revert stays offered and puts the settings back once the same DSPi is open again.
  Until then the console says that the DSPi may still hold the tune.
- **A/B** after Keep: switch between the tuned settings (A) and the "Before" snapshot (B) to listen, then keep the one
  you prefer.
- **The snapshots** "Before auto-tune" and "After auto-tune" stay in your snapshot list. Recall either at any time. The
  web console also lets you download them.
- **Undo:** the whole tune is one step in the history.
- **A revert that is not fully successful** (a setting could not be written back, or reads back differently) says so:
  "The revert was not fully successful" and "Check the settings, or restore a backup (a snapshot)." Recall the
  "Before auto-tune" snapshot, or check the settings it names.

## 8. Documents

Measurements, microphones (`mic`) and targets are documents. The **Documents** tab lists your own and, with a hub, the
hub's in one list; when both hold one, the hub's copy wins. You can rename them, add a note, export a measurement as
REW text, start a tune from one, or save one to the hub. On a hub, renaming and notes need Control or Admin, and
deleting needs Admin; a button you may not use is not offered, and the reason is given.

## 9. The Passive RTA, for Viewers and read-only use

If you may not play the generator (a **Viewer** on a hub, a read-only connection, or the console's read-only mode), you
get the **Passive RTA** instead of the tabs that play. It shows the microphone's spectrum of **whatever is playing
now**, corrected and in dB SPL when the microphone is calibrated, with hold, compare and the averaging choices. It
never plays anything and never writes to the DSPi. You can still open Results and Documents.

It does not run while this console is measuring, in the live view or in auto-tune, so it never shows the test signal as
if it were your music.

<!-- WP ATDOC -->
## 10. Measuring several units together

*† Needs the DSPi fork firmware ([Bofr5555/DSPi](https://github.com/Bofr5555/DSPi), branch `bofr-main`) on every unit: the units' shared time base and the generator started on it are the fork's
([`../FEATURES.md`](../FEATURES.md#firmware-stock-vs-fork)). Sections 1 to 9, one unit at a time, work on stock
firmware 1.1.5 and later.*

*This is in the web console (Measure, the "Several units" tab) and in the desktop app (Modern: Measure in the rail,
the "Several units" tab; Classic: Tools > Measure…, the "Several units" tab). It has not been tried on real units yet. The technical page is [docs/integrator/multi-unit-measurement.md](../integrator/multi-unit-measurement.md).*

When one room is played by several DSPi units (a subwoofer unit and a mains unit, or one unit per speaker), one
microphone can measure them all and line them up in time. Each unit plays its own test signal, and every start is set
on a sample counter the units share, so each unit starts exactly when it was told to.

### 10.1 What you need

- **The DSPi hub.** Measuring several units together runs through our hub. Without it, the console offers one unit at a
  time and says "Measuring several units needs the DSPi hub".
- **Units that share one clock.** These can be measured together:

  | Units | Measured together |
  |---|---|
  | the units behind one audio link sender (fan-outs and repeaters included) | yes |
  | the units fed over USB-direct from one USB controller, in phase lock at 48 or 96 kHz, with the hub running where they are plugged in | yes |
  | one DSPi set as an S/PDIF source and the DSPi units that follow it over S/PDIF | yes |
  | units on two senders, two USB controllers or two S/PDIF sources, or a mix of them | no: "These units do not share a clock" |
  | units fed over S/PDIF by a TV or a receiver, over USB at 44.1 kHz or without phase lock, over ADAT or I2S | no: one unit at a time |

  A unit set to follow S/PDIF that is fed by other equipment says "This S/PDIF source does not carry a DSPi time base".
  An S/PDIF source should not be fed over USB without phase lock: each time playback starts, its followers have to lock
  again. The outputs that carry the source's S/PDIF are never measured.
- **The same sample rate on every unit**, and every unit locked to the shared clock.
- **The Control role** to measure, and **the Admin role** to apply what it finds.

### 10.2 How a run goes

1. **Choose the units and their outputs**, with the microphone at the listening position.
2. **Each unit is checked on its own**: its volume, its tweeter protection, silent inputs, no processing the measurement
   cannot model (the rules of section 4, for every unit). The console names the unit that stops it.
3. **The clocks settle.** Right after the units lock, or after anything disturbed their clock, the hub waits until it
   has seen them locked for 40 seconds. The console says "waiting for the units' clocks to settle". This can take up to
   40 seconds; it is normal.
4. **The run.** In the first pass each unit plays its outputs in turn, then the next unit. In the passes after it
   (5 when they fit, else 3) the units start together, each a short fixed time (50 to 200 ms) after the one before, so
   each unit's first outputs play together, then their second, and so on: **overlapped starts**. That keeps a USB
   microphone's clock wander the same for the units heard together. Where the units cannot be told apart that way,
   the console measures the run again with each unit in turn (**interleaved passes**), and the result says which it
   used. The hub schedules every start a little ahead (about a quarter of a second with a K340), so each unit is ready
   on time.
5. **A large group is measured in several runs.** One run takes at most 16 outputs and 2 minutes. Each run then repeats
   one **anchor output** (the first unit's first output unless you choose another), and the runs are joined through it.
   The console says "The group needs several runs". If the anchor sounds different between runs, the runs do not join
   and nothing is proposed: "The runs do not chain". **Do not move the microphone, or anything near it, until every run
   has finished.**

Any of these stops the run at once, on every unit, and names the unit: a unit clips or reaches full scale, a limiter
acts, a unit loses its clock lock, a unit's start did not happen exactly as scheduled, someone else changes a setting,
the hub loses a unit, or you press **Stop**. The hub also stops every unit by itself if the console stops answering.

### 10.3 What you get, and its limits

The result gives each output's curve, as for one unit, and the **delay between the units** with how sure it is.

- **A delay between units is offered only when it is known to within 10 µs.** Otherwise the result says **"timing
  unreliable"** with how far the microphone's clock wandered, and nothing is proposed for those units. The curves are
  still good.
- **A USB microphone whose clock wanders** (like the one in our own test, section 6) is what overlapped starts are
  for. In simulation with that microphone's wander, overlapped runs gave a reliable delay between the units in 28 or
  29 of 30 runs, where interleaved passes read "timing unreliable" every time. The delay offered is between outputs that
  played together (each unit's first output, then its second, and so on); a delay between two outputs of one unit
  still needs a steadier clock, such as an analogue measurement microphone on an audio interface.
- **Overlapped starts measure the low frequencies less cleanly.** Below about 300 Hz a later unit's curve carries
  some of the earlier unit's room. Timing is not affected; if a unit's bass looks odd, compare a single-unit
  measurement of it.
- **Units that play later are allowed for, up to a second.** The console knows how much later each unit plays (its
  output delays, and on USB-direct each stream's start offset) and spaces the sweeps and the listening for it. A unit
  more than a second later than the others is refused before anything plays: check its alignment. An output heard more
  than about 85 ms after where the console expects it (a speaker very far away) cannot be measured in that run, and
  nothing of that group is offered.

### 10.4 Apply, verify, Keep and Revert

1. **Summed check, always.** Before anything is applied, the outputs where two units meet play together and the sum is
   compared with the prediction. Each unit is checked on its own again first, and each output is ramped again. Units
   that play together play quieter: 6 dB less each time their number doubles. If the sum disagrees by more than
   1.5 dB, every delay between the units is withdrawn: "The summed check failed". A pair that would cancel out is not
   played at all, and the console says which outputs and why.
2. **Apply needs Admin.** A Control user sees the proposal and can save the measurement, but "Applying needs Admin". The
   delays go into the hub's installation, and the hub writes the units. **A countdown starts**: the console shows how
   long you have (a few minutes, enough for the verify and some margin). If you do not press Keep in that time, the
   hub puts the installation and the units' delays back by itself. It does the same at once if this console closes or
   loses its connection. Leaving it alone is always safe.
3. **Verify.** The summed check runs again with the new delays, each unit checked again first. If it does not agree:
   "The verify did not confirm the alignment".
4. **Keep.** After a verify that passed, one press keeps the alignment. **After a verify that failed, Keep takes two
   steps**: the first press is refused with the reason, and only your second press, at least a second later, keeps it.
   Nothing presses Keep for you.
5. **Revert** puts the installation and every unit's delays back as they were, and ends the countdown. If the hub has
   already put things back when you press Keep or Verify, the console says what the hub did. If a unit's delay could
   not be put back, the console says "The revert was not fully successful" and asks you to check the settings or
   restore a backup.

An older hub has no countdown for this. The console then says before the apply that nothing reverts by itself: the
new delays stay until you Keep or Revert.

Only the delays between units are proposed. Polarity, crossovers and EQ are tuned within each unit by its own
auto-tune (section 7).

### 10.5 Not tried on real units yet

Everything in this section has run on simulated units in a simulated room only. It still has to be shown on real
boards: that each unit plays exactly by the delay it reports, that real units can be opened together, and the S/PDIF
clock between real units.
<!-- /WP ATDOC -->

## See also

- [docs/integrator/measurement.md](../integrator/measurement.md): the technical contract (signal path, rules, documents)
- [docs/integrator/hub-api.md section 22](../integrator/hub-api.md#22-measurement-documents): documents on the hub
- [docs/integrator/multi-unit-measurement.md](../integrator/multi-unit-measurement.md): measuring several units
  together, the technical page (WP ATDOC)
- [docs/GLOSSARY.md](../GLOSSARY.md): the words the consoles use
