# USB-direct home theatre

For installers who feed a home theatre from one central computer over USB: the
computer plays one multichannel programme (5.1, 7.1, ...), and several DSPi units,
each on its own USB cable, drive a few speakers each. No audio link (alink) cable
is involved: every unit takes its audio from USB and runs as an **alink USB-direct
unit**, locked to the computer's USB frame clock.

| Source | What |
|---|---|
| DSPi-bofr `Documentation/Features/extension_channel_spec.md` 2.9 | the firmware's USB-direct block (normative): modes, D, lock error, status |
| DSPi-bofr `Documentation/Features/alink_usb.md` | the design behind it |
| DSPi-bofr `Documentation/Bench/usb_direct_bench.md` | the bench procedure (two units, scope) |
| [audio-link.md](audio-link.md), "USB-direct units" | what the hub does with such units: alignment, start offsets, alarms |
| `apps/terminal/src/usb_direct_gen.rs` | `uconsole usb-direct generate`, the computer's files (this page, section 6) |

**State (2026-11-14):** firmware and hub support are built and tested in
simulation; the bench with two real units (E0-E3 in the bench document) has not
been run yet. The capacity figures in section 1 are computed from the USB 2.0
specification, not measured. Every "untested" below means exactly that.

## How it works, in short

Every unit on one USB controller sees the same 1 ms Start of Frame (SOF) with the
same frame number. A unit in USB-direct mode measures that clock and follows it:

- **Mode 1, rate lock:** the unit's clock runs at the computer's USB rate. All
  units then play at the same rate, but each starts wherever its stream happened
  to start, so they are not sample-aligned.
- **Mode 2, phase lock:** rate lock, and every sample is played exactly **D**
  samples after it arrived. All units with the same D play the same sample at the
  same instant. This is the mode for a home theatre. A unit in mode 2 that cannot
  get in step is in **lock error** (section 7).

The computer still has to put the same moment of the programme into the same USB
frame on every unit. PipeWire opens the units' streams one after the other, so
they start a frame or two apart; the hub measures that (the *start offset*) and
evens it out with the units' output delays (audio-link.md, "Start offsets").

## 1. Plan the channels and the units

Each unit takes one USB stream. What one stream can carry is fixed by the
firmware's USB formats (full speed, 12 Mb/s):

| Unit | 44.1 kHz | 48 kHz | 96 kHz |
|---|---|---|---|
| RP2350 | 2 (16 or 24 bit) | 2 (16 or 24 bit), 4 (16 or 24 bit), 6 or 8 (16 bit) | 2 (16 or 24 bit) |
| RP2040 | 2 (16 or 24 bit) | 2 (16 or 24 bit) | 2 (16 or 24 bit) |

The generator always opens the 24-bit form where there is one. (An RP2350 running the *sender* firmware build, `DSPI_ROLE_SENDER`, also offers
6 channels in 24 bit at 48 kHz and 4 channels in 16 bit at 96 kHz; it can be a
USB-direct unit too, but the generator and these tables use only the formats of
the ordinary unit build.)

- **44.1 kHz cannot phase-lock** (a 1 ms frame holds no whole number of samples):
  rate lock only. Run the system at 48 kHz; the player resamples once.
- **More than two channels only at 48 kHz.** 4 channels go in 24 bit, 6 and 8 in
  16 bit.
- A unit with 3 speakers opens a 4-channel stream; the fourth channel carries
  silence. The generator does this for you.
- An RP2350 unit has 8 USB inputs and 9 outputs (four output pairs and the PDM
  sub, output 8); an RP2040 unit 2 inputs and 5 outputs (output 4 is the sub).

How many units one computer carries (computed, **not measured**):

| Where the units hang | Units | Why |
|---|---|---|
| A single-TT hub, or anything that makes a hub run at full speed (an isolator or a full-speed-only extender in front of it) | **1** per hub | Every RP2350 unit reserves 788 bytes per frame whatever it plays: about 624 us of the 900 us (90 %) a full-speed frame allows for periodic transfers. An RP2040 unit (582 bytes) about 463 us: also one (formulas below). |
| A multi-TT hub | **1 per port** (7 on a 7-port hub) | each downstream port has its own transaction translator, so its own full-speed frame |
| A root port of a USB 3 (xHCI) computer, no hub | 1 per port | the port runs at full speed for that unit alone |
| All of them on one USB controller | about **30** (computed) | through the hubs each unit costs about 25 us of high-speed bus time per frame in split transactions, of the 800 us (80 %) allowed; real controllers may refuse sooner (they check bandwidth themselves) |

How the figures are computed (USB 2.0 specification; host delay left out):

- Full-speed periodic budget: 90 % of the 1 ms frame = 900 us (5.6.4).
- Full-speed isochronous transaction time (5.11.3):
  `9107 + 83.54 x floor(3.167 + 1.1667 x 8 x bytes)` ns. 788 bytes: about
  624 us; 582 bytes: about 463 us. Two units do not fit in 900 us.
- Through a transaction translator, a 788-byte packet goes as 5 start-splits of
  up to 188 bytes, each about `55 x 8 x 2.083 + 2.083 x floor(3.167 + 1.1667 x 8 x 188)`
  ns = 4.6 us of high-speed time: about 23 us, plus about 2 us for the feedback
  endpoint's splits, so about 25 us per unit per frame. The high-speed periodic
  budget is 80 % of each 125 us microframe (5.6.4), 800 us per frame: about 32
  units, "about 30" with margin.
- **Older computers with USB 2.0 only (EHCI):** a full-speed unit plugged straight
  into a root port is handed to a companion controller (UHCI or OHCI), which is
  another controller with another clock than the units behind a hub on the same
  computer. On such computers put every unit behind the one hub.

All units that must play aligned **must be on one USB controller**: units on two
controllers count different frames, follow two different clocks, and can be
neither phase-locked together nor fed from one PipeWire clock. On many computers
the front and rear ports, or two USB-C ports, belong to different controllers
(`lsusb -t`: one `Bus` number per controller and speed). Put one hub on one port
and everything behind it.

The same figures, for comparison with the audio link's line classes, are in the
firmware's `alink_protocol_spec.md` 5.7.

## 2. Cables

The units are USB 2.0 full-speed (12 Mb/s) units, so any USB 2.0 cable that
carries full speed works. Length is the limit:

- **Passive cable:** at most 5 m per segment.
- **Active USB 2.0 cable (repeater cable):** typically 5 m per repeater. Almost all
  contain a small one-port hub, which counts as a hub. USB allows at most **five
  hubs in a chain** between the computer and a unit, so with the multi-TT hub at
  the computer that leaves four active cables: about 20 m from the hub to a unit.
  The cable's data sheet says how many repeaters it holds (a 10 m cable often has
  two).
- With an active cable per unit, the cable's own hub does the translation for
  that unit, so each unit has its own full-speed frame even behind a single-TT
  hub. Use a multi-TT hub anyway: some active cables are signal boosters without a
  hub, and a unit on a short passive cable straight into the hub needs it.
- **Optical USB cables and USB-over-Cat5/fibre extenders:** only ones that carry
  full-speed units and isochronous transfers. Some optical USB 2.0 cables carry
  only high speed, and some extenders drop isochronous traffic. Untested here.
- The cable adds a fixed delay well under 1 us (the bench expects a fixed extra
  offset under 1 us per hub or active cable). That is far below one sample
  (20.8 us at 48 kHz) and the same every time, so nothing needs to be done about it.

## 3. Hubs: multi-TT

A USB 2.0 hub talks high speed to the computer and translates for full-speed
units behind it with a *transaction translator* (TT). A **single-TT** hub has one
for all its ports: one full-speed frame shared by every unit on it, and one unit
fills it (section 1). A **multi-TT** hub has one per port.

- Check: `lsusb -v` on the hub shows `bDeviceProtocol 2 TT per port`
  (single-TT hubs say `1 Single TT`). Data sheets say "MTT" or "multi-TT".
- Symptom of a single-TT hub: the second unit's stream does not start (PipeWire
  logs a bandwidth error, the kernel `cannot submit urb ... err = -28`, or the
  sink stays silent), while the first unit plays.
- Use a powered (self-powered) hub, section 5.
- A USB 3 hub contains a USB 2.0 hub for USB 2.0 units: the same rule applies to
  that part. Its data sheet, not its USB 3 speed, decides.

## 4. Isolators and ground loops

Each unit is connected to the computer's ground through the USB cable's ground
and shield, and to the amplifiers, which are earthed at their own mains outlets.
That is a ground loop, heard as hum or buzz, worst with long cables across a room.

- **USB isolators** built on the ADuM3160/ADuM4160 work at full speed only, which
  suits the units (they are full speed anyway). Put one **at the unit end**:
  cable (active or not), then the isolator, then the unit. The isolator adds a
  small fixed delay (well under a microsecond): harmless (section 2).
- **Never put an isolator between the computer and a hub.** The hub then runs at
  full speed for everything behind it, and only one unit fits (section 1).
- The isolated side gets its power from the isolator's own converter, which is
  often limited (a few hundred mA). A unit that takes its power from USB may need
  more; a unit with its own supply needs only the data isolated.
- Without isolators: power the computer, the hub and every amplifier from one
  outlet or one distribution strip, keep signal and mains cables apart, and use the
  units' optical outputs where the amplifier has an optical input (light carries
  no ground). Never lift a safety earth.

## 5. Power

- USB 2.0 promises 500 mA per port. A hub that powers its ports needs its own
  supply (self-powered), sized for every unit that draws from USB plus the active
  cables, which take their own current from the 5 V line.
- Long runs lose voltage: at the end of 20 m of active cable a bus-powered unit
  may brown out under load. Power far units locally, or use active cables with a
  power input at the far end.
- A unit with its own supply still sees the USB 5 V line; nothing to do.
- When a unit loses power or USB, the others keep playing; it anchors again
  (in step, muted until then) when it is back, about 0.5 s after the USB frames
  return once it has learnt this computer's rate.

## 6. The computer

### Linux with PipeWire: `uconsole usb-direct generate`

The Terminal writes the computer's side from one installation file. It writes
files only: it opens no unit and plays nothing.

```sh
uconsole list                                   # the units' USB serials
uconsole usb-direct generate cinema.json --out usb-direct-cinema
uconsole usb-direct generate cinema.json --dry-run --json   # what it would write
```

An installation file (JSON):

```json
{
  "name": "cinema",
  "description": "Cinema 7.1",
  "rate": 48000,
  "layout": "7.1",
  "units": [
    {"id": "front", "name": "Front L/C/R and sub", "serial": "E6614C311B4A8B2F",
     "channels": [
       {"channel": "FL", "output": 0, "distanceM": 3.4},
       {"channel": "FR", "output": 1, "distanceM": 3.4},
       {"channel": "FC", "output": 2, "distanceM": 3.1},
       {"channel": "LFE", "output": 8, "distanceM": 2.6}]},
    {"id": "sides", "serial": "E6614C311B4A9C10",
     "channels": [{"channel": "SL", "output": 0}, {"channel": "SR", "output": 1}]},
    {"id": "rears", "serial": "E6614C311B4AA0D7", "onLockError": "play_unaligned",
     "channels": [{"channel": "RL", "output": 0}, {"channel": "RR", "output": 1}]}
  ]
}
```

| Field | Meaning | Default |
|---|---|---|
| `name` | short name: a-z, 0-9, `-`, `_`; names the sink, the files and the hub's room | needed |
| `description` | what people see (the sink's name) | `name` |
| `rate` | 44100, 48000 or 96000 | 48000 |
| `layout` or `channels` | the programme: `2.0`, `2.1`, `4.0`, `5.1` (FL FR FC LFE RL RR), `7.1` (adds SL SR), `7.1.4`; or a list of PipeWire channel names | needed |
| `quantum` | PipeWire's samples per cycle, a power of two 32..8192, fixed | 256 (512 at 96 kHz) |
| `mode` | `phase_lock` (mode 2) or `rate_lock` (mode 1) | `phase_lock` |
| `d` | D in samples, mode 2 only (section 7) | 192 at 48 kHz, 324 at 96 kHz |
| `keepAligned` | the hub's "Keep USB-direct units aligned" | true |
| `units[].id`, `.name` | the unit's id (a-z, 0-9, `-`, `_`) and name | name = id |
| `units[].serial` | its USB serial (`uconsole list`); how PipeWire and the hub find it | needed |
| `units[].chip` | `rp2350` or `rp2040` (what formats it has) | `rp2350` |
| `units[].onLockError` | `mute` or `play_unaligned` (section 7) | `mute` |
| `units[].channels[]` | `channel` (a programme channel), `output` (the unit's output), `input` (its USB input; default: the order given), `distanceM` (to the listening position, for the hub's distance delays; default 0) | |

It refuses what cannot work, with the reason: 44.1 kHz in mode 2, D outside the
units' range, more than two channels at 44.1 or 96 kHz, more than 8 channels (2 on
an RP2040), a serial or output used twice, a channel the programme does not have.
A programme channel that goes to no unit is a warning.

What it writes, in the folder:

| File | What | Where it goes |
|---|---|---|
| `pipewire/dspi-usb-direct-<name>.conf` | one sink with the programme's channels, split by `libpipewire-module-combine-stream` into one stream per unit (matched by serial), each channel onto the unit's USB input; the graph held at the rate (`default.clock.allowed-rates`) with a fixed quantum (min = max), no latency compensation by PipeWire | `~/.config/pipewire/pipewire.conf.d/` |
| `wireplumber/dspi-usb-direct-<name>.conf` | per unit: the pro-audio profile, the exact USB format (channels, bits, rate), one clock name and one group for all units (so no rate matching and no resampling between them), one unit preferred as the driver, never suspended | `~/.config/wireplumber/wireplumber.conf.d/` (WirePlumber 0.5) |
| `units/<id>.dspi` | a Terminal script: input source USB and the matrix crosspoints (every input of each used output, so earlier routing cannot add to it) | run with `uconsole --device <serial> -f units/<id>.dspi`, then save the unit's preset |
| `hub-system-model.json` | the hub's model: one room, the units as USB-direct with mode, D (`dFrames`), the lock-error action and the speakers | the hub (`system.set`, audio-link.md), or enter the same in the Installer |
| `SETTINGS.md` | the settings sheet: every unit's stream, inputs, outputs, mode, D, lock-error action, and the steps | for people |

Then:

1. Copy the two configuration files, `systemctl --user restart wireplumber pipewire
   pipewire-pulse`. `wpctl status` shows the combined sink and one "(DSPi
   USB-direct)" sink per unit; make the combined sink the default. `pw-top` shows
   the rate and quantum on the driver's row only (one unit; the others and the
   streams follow it and show 0); `pw-dump` shows each unit's format (for example
   `S24LE 48000 4`). No unit is resampled.
2. Run each unit's routing script and save its preset.
3. Set mode, D and the lock-error action through the hub (Installer, "USB-direct
   units", Admin; or the model file), then **Align**. The hub writes them to the
   units, measures the start offsets and writes the output delays.
4. Check the lock (section 7).

Why these settings:

- **No resampling.** The units in mode 1 or 2 report exactly the nominal rate to
  the computer, so their streams all run on the USB frame clock; the generated
  rules tell PipeWire they share one clock, so it never rate-matches them. The
  graph is pinned to the units' rate, so a 44.1 kHz player is resampled once,
  into the combined sink, and nothing after it.
- **Never suspended.** PipeWire normally closes an idle sink after a few seconds.
  Every close and reopen places the stream anew and moves its start offset, which
  the hub then has to align again (audio-link.md, "Start offsets move"). With
  suspend off the units keep a stream of silence open and the offsets stay put;
  "Keep USB-direct units aligned" covers the rest. The combined sink is not
  suspended either, so the next programme starts without waiting for it.
- **Fixed quantum.** The units' D does not depend on it, but a quantum that
  changes with the player changes the computer's own latency and its timing on
  the bus; a fixed one keeps lip sync and packet timing constant. These settings
  apply to the whole PipeWire instance: use a computer dedicated to the theatre.
- WirePlumber 0.4 (Lua configuration) is not generated; translate the rules by
  hand, or update WirePlumber.

The generated configuration has been loaded in PipeWire 1.4.11 with WirePlumber
0.5.14 (review ausb3, on stand-in sinks without USB hardware): it parses, splits
the channels as listed, fixes rate and quantum, and the shared clock name turns
rate matching and resampling off. The rules' match against a real unit's card
has not been seen yet; check with `pw-dump` that each unit's node carries the
format, clock name and group above.

### macOS (untested)

Audio MIDI Setup, "+", **Create Aggregate Device**: tick every unit, choose one
of them as the clock source, and leave **Drift Correction off** for all of them
(they run on the Mac's USB frame clock already; drift correction would resample).
Set each unit to 48 kHz and its channel count in the unit's format menu, then
"Configure Speakers..." on the aggregate to put the 5.1 or 7.1 channels onto the
units' channels. On Apple silicon, each USB-C port is usually its own controller:
put all units behind one hub on one port (section 1). Set mode, D and the
lock-error action through the hub as above.

### Windows (untested)

Windows has no built-in way to combine several sound cards into one
multichannel output. A router that writes each unit in exclusive mode (WASAPI
exclusive or ASIO) at 48 kHz, with no drift correction or resampling, does the
same as the combined sink; set each unit's default format to 48000 Hz in the
Sound control panel. No particular product is tested or recommended here.

## 7. Choosing D, checking the lock, lock error

### D

D is the time from a sample's arrival at a unit to its output, in samples at the
stream's rate, and the same on every unit (the hub adds each unit's own latency
difference and distance delays on top, audio-link.md "One system delay").

| Rate | Range | Default | |
|---|---|---|---|
| 48 kHz | 170..399 | 192 (4.0 ms) | |
| 96 kHz | 324..351 | 324 (3.4 ms) | 192 is out of range here |
| 44.1 kHz | - | - | no phase lock |

- A larger D tolerates later packets: a packet that arrives more than
  `D - 49 - 16 - 48` samples after its frame (1.65 frames at D 192, 48 kHz) is
  played as a faded dip and counted as late. If a unit's gap or late counts grow
  (`ext usb-direct`, the hub's USB-direct stats), raise D, e.g. to 240 or 288, on
  every unit (one Align), after looking for the cause (section 8).
- The total audio latency is the computer's (PipeWire's quantum and USB queue)
  plus D plus the unit's processing. Lip sync is set on the hub (`lipSyncUs`) or
  in the player.
- Changing D or the output latency option re-anchors the unit (a short muted
  moment). A latency option whose limits no longer include D is a lock error
  (CONFIG) until D is set again.

### Checking the lock

- `uconsole --device <serial> ext usb-direct`: state **phase-locked**, phase error
  under half a sample, `gap_count` and `late_samples` not growing.
- The hub: "USB-direct N of N locked" in the summary, the System view and the
  Installer's "USB-direct units" and "USB-direct stats" tables.
- After power-up a unit is phase-locked about 0.5 s after the USB frames start
  once it has learnt this computer's rate, else within 1-12 s. Until then it is
  muted, and it fades in when it is in step.
- One sample apart at 48 kHz is 20.8 us; the bench expects units on one hub
  within about 1 us of each other.

### Lock error: what the user sees

A unit in mode 2 that cannot get in step is in **lock error**:

| Cause | The unit says | Fix |
|---|---|---|
| the computer's USB clock is more than 295 ppm away from the unit's crystal | state *rejected* | another USB controller (or computer); the unit plays again below 285 ppm |
| the rate or D cannot phase-lock: 44.1 kHz, or D outside the limits after a latency change | state *rate-locked*, cause CONFIG | 48 kHz; set D inside the range again |
| a phase error over 2 samples while automatic re-anchoring is off | phase *pending* | turn automatic re-anchoring on, or wait until the servo pulls it in |

What the unit does is its **lock-error action**, set per unit (the installation
file's `onLockError`, the Installer's "When it can't lock"):

- **mute** (the default): the unit stays silent, its DAC muted, until it is in
  step again. Choose it where an unaligned speaker is worse than a silent one: a
  centre channel, an array, measurements.
- **play unaligned**: it keeps playing, rate-locked or free running, possibly a
  few samples or milliseconds off. Choose it where sound matters more than
  alignment.

Either way the hub raises the alarm `usb_lock_error` ("... can't lock — muted"
or "... playing unaligned", with the cause), critical when the unit mutes, a
warning when it plays, and clears it when the unit is in step. Firmware before
WP USB3b cannot play unaligned; the hub says so (`usb_lock_error_unsupported`).

Not a lock error: the short mute while a unit anchors, and a *holdover* (up to
10 s without USB frames, the unit keeps its timeline and plays on when they
return).

## 8. Troubleshooting

| What you see | Likely cause | What to do |
|---|---|---|
| One unit plays, the next one's stream never starts | a single-TT hub, or an isolator or full-speed extender in front of the hub (section 1) | a multi-TT hub; isolators at the unit end |
| A unit is not found at all | too many hubs in the chain (active cables count), a long passive cable, voltage drop | count the hubs (`lsusb -t`), at most five; power the far end |
| Units drift apart or play with growing gaps; PipeWire xruns | units on two USB controllers (two clocks) | all units behind one hub on one port (`lsusb -t`: one bus) |
| The offset between units changes after a pause or a new track | the streams were closed and reopened (PipeWire suspend, a player reopening) | the generated WirePlumber rules (no suspend); "Keep USB-direct units aligned" on; or **Align** again |
| Short dropouts, `gap_count` or `late_samples` growing | packets late: a busy computer, USB power saving, a marginal cable | raise D; a fixed, larger quantum; turn off USB autosuspend for the units; another cable or port |
| Hum or buzz | ground loop (section 4) | an isolator at the unit end; one outlet; optical outputs |
| A unit stays silent, alarm `usb_lock_error` | lock error with the action mute (section 7) | the cause in the alarm; or set the unit to play unaligned |
| A unit stays muted at start for many seconds | first time on this computer: it learns the rate (up to about 11 s) | wait; next time it locks in about 0.5 s |
| Wrong speaker plays a channel | routing or channel map | the unit's `SETTINGS.md` table; run its `.dspi` script again; `uconsole --device <serial> get mix ...` |
| Sound at 44.1 kHz is not aligned | 44.1 kHz cannot phase-lock | run the system at 48 kHz |
| A unit is not under the driver in `pw-top`, or its `pw-dump` format is not the generated one | the rules did not match (another serial, a different node name) | `pw-dump` / `wpctl inspect` the unit's node; its name must contain the serial |
| A unit comes up with an analog profile, not Pro Audio, and its format rules do not apply | a profile once picked for that card (in a mixer) is stored, and WirePlumber restores it before it reads the generated `device.profile` | set Pro Audio once (`wpctl set-profile <unit id> <index>` or pavucontrol); WirePlumber then remembers that |
