# Glossary

*WP P0U, 2026-11-12. The source of truth for the words every client shows.*

The parameter registry (`crates/dspi-proto/src/registry.rs`) gives each parameter a `label`, a `plain` line of help, a unit
and a range. This file explains the words in it. Clients take labels, help lines, units and ranges from the core (the registry
through `describe` on the web, `by_path(..)` in the Terminal) and follow this file where they still write their own text.
`crates/dspi-proto/tests/glossary_wp_p0u.rs` pins every label and unit, and fails when a label changes without this file.

## 1. Rules for every client

- **One word per concept.** Use the term in section 3, not a synonym. "Input trim", not "Preamp" or "Pre-gain"; "Q", not
  "Width" or "Bandwidth"; "Input source" for where audio comes from, "Input" for an input channel.
- **Never name the host device** (owner rule). No "phone", "tablet", "computer", "Mac", "PC", "laptop" and no "host" in
  text a user reads; say it with no device noun. **Device** always means the DSPi.
- **Spelling is British:** centre, leveller, modeller, analyser, equaliser, colour, synthesised in prose. The one exception is
  the firmware's feature name **Subharmonic Synthesizer**.
- **Case.** Parameter labels are sentence case ("Master volume", "Knee hardness"). Feature names (section 2) and acronyms
  (S/PDIF, I2S, ADAT, DAC, GPIO, UART, I2C, IR, LG Sound Sync) keep their capitals. A client may set labels in title case
  where its platform does that for controls (the Mac's windows do); the words stay the same.
- **No full stop** after a label or a one-line help text.
- **"Stats for nerbs"**, the Mac's menu item for System Statistics, is spelt so on purpose (owner, 2026-11-12): a joke,
  not a typo. Keep it where a client uses it.
- **Units** (section 4) are written with a space before them ("-6.0 dB", "120 Hz", "10 ms"), except "%" after a bare number
  in compact layouts.
- **Gains are signed, levels are not.** "+3.0 dB" for a boost; "-1.0 dBFS", "85 dB SPL" for a level.
- **Choice values** in the registry ("lowshelf", "0.67ms", "with-preset") are command words for the Terminal's grammar and
  files, not display text. Clients show their own display names for them (for example "Low shelf"), consistently.

## 2. Features

The DSP blocks have proper names, used as section and window titles and as the label of their on switch.

| Feature | What it does | Registry |
|---|---|---|
| **Output Limiter** | A brickwall limiter per output, with lookahead. The first one switched on delays every output by 32 samples (0.67 ms at 48 kHz) | `limit.*` |
| **Loudness** | Restores perceived bass and treble at low volume, calibrated to a reference level | `loud.*` |
| **Leveller** | Evens out loud and quiet passages | `lev.*` |
| **Crossfeed** | Blends the channels below a cutoff frequency to soften hard stereo separation on headphones | `cf.*` |
| **Psychoacoustic Bass** | Generates harmonics that imply bass a small speaker cannot reproduce | `bass.*` |
| **Upmixer** | Spreads a stereo recording across centre and surround outputs | `up.*` |
| **Subharmonic Synthesizer** | Generates a subharmonic at half the frequency of the source's bass | `sub.*` |
| **Tube Modeller** | Valve-style harmonic colour, supply sag and an output stage | `tube.*` |
| **Matrix** | Routes inputs to outputs at crosspoints, with level and polarity | `mix`, `out.*` |
| **EQ** | The filter bands on every channel (parametric bands and crossover bands) | `eq.*`, `bypass` |

## 3. Terms

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **Device** | The DSPi board. Never the host | | `dev.*` |
| **Channel** | An input or an output, in the unified list: inputs first, then outputs | | |
| **Input** | An input channel (IN1, IN2, ...) | | |
| **Output** | An output channel (OUT1, ...), which drives a pin | | `out.*` |
| **Input source** | Where the device takes audio from: USB, S/PDIF 1 to 4, I2S, ADAT, Audio link. Not "Input" alone | | `in.source` |
| **Volume** | The listening volume, shared with the operating system's volume | dB, -60 to 0 | `vol.user` |
| **Master volume** | A ceiling on the output level; -128 dB is muted | dB, -128 to 0 | `vol.master` |
| **Mute** | Silence; "Output mute" silences one output | | `vol.mute`, `out.mute` |
| **Input trim** | Level trim on an input before processing. The Mac's older word is "Preamp" | dB, -60 to +24 (RP2350), +18 (RP2040) | `pre` |
| **Output gain** | Level trim on an output | dB, -60 to +20 | `out.gain` |
| **Gain** | Boost or cut by one filter band | dB, -24 to +24 | `eq.gain` |
| **Frequency** | Centre or corner frequency of a band | Hz, 10 to 24000 | `eq.freq` |
| **Q** | How narrow a band is; higher is narrower. Never "Width" or "Bandwidth" | no unit, 0.1 to 20 | `eq.q` |
| **Target frequency**, **Target Q** | The Linkwitz Transform's target alignment | Hz; no unit | `eq.fp`, `eq.qp` |
| **Filter type** | The shape of a band: peak, low shelf, high pass, ... | | `eq.type` |
| **High-pass**, **low-pass** | Written with a hyphen in prose and labels | | |
| **Crossover** | The filters that split a signal between drivers (LR, BW, Bessel bands). Not the crossfeed's cutoff | | `eq.*` bands 20 to 23 |
| **EQ bypass**, **Band bypass** | Pass audio without the whole equaliser, or without one band | | `bypass`, `eq.bypass` |
| **Delay** | Time alignment of a channel or an output. Always in milliseconds; a client may show a distance beside it | ms, 0 to 21 (RP2040) or 42 (RP2350) | `ch.delay`, `out.delay` |
| **Latency** | Delay the processing adds, not set directly. In samples only where the firmware fixes it in samples, then with ms beside it | ms (samples) | `out.latency`, `limit.on` |
| **Output latency** | How full the output buffers are kept; lower is quicker, higher is safer | 0.67 to 8 ms | `out.latency` |
| **Threshold** | The limiter's ceiling: no sample leaves the output above it | dBFS, -30 to 0 | `limit.threshold` |
| **Release** | How fast gain recovers after a peak (limiter) or steering relaxes (upmixer) | ms | `limit.release`, `up.release` |
| **Attack** | How quickly the upmixer's steering responds | ms, 1 to 500 | `up.attack` |
| **Link group** | Outputs in one group limit together; 0 is unlinked | 0 to 4 | `limit.link` |
| **Gain reduction** | How much a limiter is turning the signal down | dB | `limit.meter` |
| **Lookahead** | Delaying the signal so a peak is seen before it arrives. One word | | `lev.lookahead`, `limit.status` |
| **Silence gate** | Below this level the leveller stops working | dBFS, -96 to 0 | `lev.gate` |
| **Sub ceiling** | A soft limit on the synthesised sub; 0 dBFS is off | dBFS, -40 to 0 | `sub.ceiling` |
| **Reference level** | The listening level the loudness compensation is calibrated to | dB SPL, 40 to 100 | `loud.ref` |
| **Intensity**, **Amount**, **Strength**, **Mix**, **Depth** | How strongly a block acts, in its own sense | %, as the registry says | |
| **Cutoff frequency** | The crossfeed's corner: below it, the channels blend | Hz, 500 to 2000 | `cf.freq` |
| **Feed level** | How much of each channel crosses to the other | dB, 0 to 15 | `cf.feed` |
| **Interaural delay** | The delay sound takes to reach the far ear (crossfeed) | on or off | `cf.itd` |
| **Speaker limit** | The lowest note the speaker can produce (Psychoacoustic Bass) | Hz, 30 to 300 | `bass.cutoff` |
| **Centre width** | How wide the upmixer's centre image is. A percentage, not Q | %, 0 to 100 | `up.width` |
| **Correlation threshold** | How alike two channels must be to count as centre | %, 0 to 95 | `up.threshold` |
| **Surround delay** | Delay on the upmixer's surround outputs | ms, 0 to 20 | `up.sur_delay` |
| **Drive** | Gain into a shaper (Psychoacoustic Bass, Tube Modeller) | dB | `bass.drive`, `tube.drive` |
| **Hold** | Burst length, or how long a note must ring to count as sustained | ms, 50 to 400 | `sub.hold` |
| **Damping factor** | How tightly the tube's output stage holds the speaker | no unit | `tube.damping` |
| **Crosspoint** | One input routed to one output in the matrix | dB, -60 to +20 | `mix` |
| **Preset** | A stored set of settings in a slot (10 slots). Save, load, delete | | `preset.*` |
| **Startup preset** | The preset that loads at power on | | `preset.startup` |
| **Live** | A setting not yet saved: lost at power off unless a preset is saved | | |
| **Snapshot** | A whole-device state kept by the app, not on the device: every live setting plus the device's name, the input sources' names, the output latency and the control surfaces. Capture, recall, rename, add a note, export, import. One word: not "scene" (the hub's, below) and not "preset" (a slot on the device) (OD-P1a, WP P1SN) | | |
| **Recall** | Putting a snapshot back on the device as it was captured. The result is live, not saved to a preset slot, and ends the undo history | | |
| **Sample rate** | The rate the device runs at when it is the clock master | Hz, 44100 to 96000 | `in.rate` |
| **Clock master**, **clock slave** | Whether the device generates the I2S clock or follows it (firmware words) | | `in.i2s.clock` |
| **Master clock** | MCLK for an external DAC, a multiple of the sample rate | 128x, 256x | `i2s.mck*` |
| **GPIO** | A pin number on the board | 0 to 29 | `*.pin` |
| **Meters** | Per-channel levels, clip indicators and processor load | dBFS | `meters` |
| **Clip indicator** | A latch set when a channel reaches full scale; cleared by the user | | `meters.clear` |
| **Factory reset** | Resets live settings; stored presets are kept | | `dev.reset` |
| **Firmware update mode** | Reboots into the bootloader; the device disconnects | | `dev.bootloader` |
| **Control surface** | Physical controls (encoders, buttons, IR remote, display) bound to parameters | | `cs.*` |

### 3.1 The audio link

*WP SW1, 2026-11-12. The multi-unit link of the SYS2 design (the fork's `extension_channel_spec.md` 2.8). The core's calls
are `alink_*`, `fetch_alink_frame_counter` and `apply_at_frame`.*

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **Audio link** | The optical or SFP cable that carries audio from a sender to devices, with one clock for all of them. Always "Audio link" in full: **DSPi Link** is the network connection to a hub, and "Link" alone means that. The input source's display name; its command word is `link` (source 7) | | `in.source` |
| **Sender** | The device that takes USB audio in and sends it on the audio links; its clock is the system's | | |
| **Receiver** | A device that plays what an audio link carries. A **Repeater** is a receiver that passes the stream on to the next one | | |
| **Audio link channel** | One audio channel in the link's stream, numbered from 0. A stream carries up to 64 | 0 to 63 | |
| **Audio link slots** | Which audio link channel feeds each of a receiver's 8 matrix inputs; "Silent" for none. Not "Mapping" or "Routing" (the Matrix routes) | | |
| **Locked**, **Searching**, **Holdover**, **Off** | The link's state. Holdover: the link was lost, the outputs are muted and the clock is held until it returns | | |
| **Line class** | The link's speed: L1 (TOSLINK parts), L2 (50 MBd parts), L3 (SFP) | L1, L2, L3 | |
| **Phase lock** | Every receiver plays an audio link frame the same fixed delay after it arrives, so all devices play in step. The other mode, **Fill servo**, is how S/PDIF and ADAT inputs follow their source | on or off | |
| **Audio link frame** | One sample period of the audio link's stream, counted by the sender (the frame counter); the audio link delay, scenes and apply at frame are given in them. Not "link frame" in text a user reads (ALINK1) | frames | |
| **Audio link delay** | The fixed delay of phase lock, in audio link frames, with ms beside it (one frame is one sample period). The plans' `D_local` | frames (ms) | |
| **System delay** | The delay D every unit's audio link delay is worked out from, so all units of one sender play an audio link frame at the same moment. The hub picks the smallest every unit takes unless an installer fixes it. The plans' `D` | frames (ms) | |
| **Audio link delay range** | The shortest and longest audio link delay a unit takes; it follows its output latency (`d_local_min`..`d_local_max`) | frames | |
| **Hop offset** | Frames added to a unit's audio link delay for the fan-outs and repeaters between it and the sender, which each pass the stream on a few frames late (`hops_offset`) | frames, -64 to 64 | |
| **Re-anchor** | Restarting the outputs in step after a loss or a new audio link delay: a short silence (2 to 6 ms). Not "Resync" in text a user reads | | |
| **Frame counter** | The link's running count of frames, the same on every device of one sender | frames | |
| **Apply at frame** | A change (mute, unmute, preset, master volume, output delay, rate, test click) that every device makes at the same audio link frame | | |
| **Scene** | A hub's audio link system change: mute, preset load, master volume or delay, made at one audio link frame across units. Not a snapshot, which is one device's whole state (OD-P1a) | | |

### 3.2 The hub's services

*WP P2DOC, 2026-11-15. PRO phase 2, the hub as a server ([`docs/integrator/hub-api.md`](integrator/hub-api.md)). The words a
client shows for what the hub keeps and does.*

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **Inventory** | Every device a hub has ever seen, online or not, kept by serial: its name, firmware, board, location, note, tags, first and last seen, last restart. Not the device list, which shows only what is attached now | at most 1024 devices | |
| **Health** | A device's state as the hub last read it: processor load, clip indicator, temperature, uptime, restarts. Read, never changed | | |
| **Identify** | Makes one device show itself: its LED blinks, and on request a quiet tone plays on one output. Not "Locate" or "Find" | 0 to 120 s | |
| **On the hub** | What the hub keeps for everyone, shown beside what this client keeps: its history, its snapshots | | |
| **Alarm** | A condition the hub raises and clears by itself (offline, restarted, clipping, too hot, ...); a person acknowledges it. Raised, acknowledged, cleared | | |
| **Alarm rule** | Whether an alarm is on and the threshold it is raised at, set by an Admin for the hub; also the optional "Mute on clip storm" | | |
| **Schedule** | A rule the hub runs by itself at a local time on given days, or once: load a preset, recall a snapshot, apply a scene, set the volume or mute. Not "Timer" or "Automation" | | |
| **API key** | A key an Admin makes on the hub for another program (a script, a venue controller, Home Assistant), with a role; shown once, revoked like a paired client. Not "Token" in text a user reads | | |
| **Notification target** | Where the hub sends alarms and schedule results: a webhook, a push address or MQTT. Off until an Admin adds one | | |
| **Snapshot** | Restated: a whole-device state kept by a client or by the hub, never on the device. A snapshot on the hub is shared; the hub's copy wins over a different one with the same id, which a client keeps as a local copy | | |
| **Scene** | Restated: a hub's audio link system change at one audio link frame across units (section 3.1). Not a snapshot and not a preset | | |

### 3.3 The fleet

*WP P3DOC, 2026-11-16. PRO phase 3, the fleet in the web console ([`docs/core-merge/PLAN-P3.md`](core-merge/PLAN-P3.md);
the contract: [`docs/integrator/hub-api.md`](integrator/hub-api.md) sections 5 and 19 to 21). The words a client shows for
many devices at once.*

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **Group** | A named set of devices kept by the hub, with a note: what a volume, a preset, a bulk apply, a schedule or a firmware rollout can be sent to at once. A device may be in many groups. A group has no role of its own and no group inside it. Not "Zone" (the audio link's) or "Tag" (an inventory label) | at most 64 groups of 64 devices | |
| **Bulk apply** | Recalling one snapshot on every device of a group, one device after another, each with its own guarded apply and one confirm or cancel for all. Not "Broadcast" or "Sync" | | |
| **Rollout** | A firmware update the hub carries out by itself on the devices on its own USB, one at a time: the settings exported first, the device checked after it restarts, an alarm if it does not come back. Started by an Admin. Not "Deploy" or "Push" | | |
| **Overview** | The web console's screen with every output of one device in one table, and every meter. Not "System" (below) | | |
| **System** | Restated: the audio link system, its units, rooms, zones and signal chain (section 3.1, the System screen). Never the many-device views: those are the hub's pages, and one device's dense table is the Overview | | |

### 3.4 Measurement and auto-tune

*WP P5MDOC, 2026-11-16. PRO phase 5 ([`docs/core-merge/PLAN-P5.md`](core-merge/PLAN-P5.md); the contract:
[`docs/integrator/measurement.md`](integrator/measurement.md)). The words a client shows for tuning with a measurement
microphone.*

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **Measurement** | A run of the generator on one or more outputs while the measurement microphone listens, and the document that keeps its results: each output's response, the chain it was measured through, the timing quality. Kept per device; never holds audio | at most 16 outputs, 9 positions | |
| **Generator** | The device's own test signal generator: sweeps, pink noise and tones on chosen outputs, through their crossover and EQ. The only source of test audio when measuring. Not "Signal source" or "Test tone player" | peak level, dBFS, -120 to 0 | `sig.*` |
| **Sweep** | A tone gliding from 20 Hz to 20 kHz on a logarithmic scale, played by the generator; repeated passes are averaged. Not "Chirp" | 1 to 15 s | |
| **Impulse response** | What one output does to a single click, worked out from the sweep the microphone heard; the response and the timing come from it. Kept only while measuring, never in a document | | |
| **Target** | The curve a tune aims for, drawn dashed on every graph: built in (flat, tilt, house curve, a coupler's default) or imported from a file. Not "Reference curve" | dB | |
| **Microphone calibration** | The measurement microphone's own correction and sensitivity, imported from its calibration file; with it the graphs read in dB SPL, marked approximate until a calibrator check. Not "Mic profile" | dB, dB SPL | |
| **Placement** | Where the measurement microphone is: the listening position, several positions averaged, near-field at one driver, or headphones on a coupler | 3 to 9 positions, 1 to 9 reseats | |
| **Coupler** | The fixture a headphone is measured on: an IEC 60318-4 ear simulator, a flat-plate coupler, or a head and torso simulator. Each has its own default target. Not "Rig" or "Dummy head" | | |
| **Way** | One driver's share of a loudspeaker on its own output: Full, Sub, Low, Mid or High. A High way is never measured without its protective high-pass | | |
| **Alignment group** | The outputs measured in one run and aligned to each other: by default one speaker's ways, or the whole system when it fits in one run. Delays are only compared within a group | | |
| **Auto-tune** | The guided tune of a speaker from its measurement: delay and polarity, crossover corner and levels, then a target EQ fit, shown as one proposal and applied only with a guarded apply between a "Before" and an "After" snapshot. Not "Room correction" | | |
| **Live response** | The measured response of one output, updated after each short sweep or as pink noise plays, shown with the target and the predicted curve of a pending change | | |
| **Microphone RTA** | The real-time spectrum of what the measurement microphone hears, with pink noise or programme audio. Always "Microphone RTA" in full: "RTA" alone is ambiguous | 1/1 to 1/48 octave | |
| **Device analyser** | The device's own spectrum of its inputs and outputs (the firmware's analyser). Not the Microphone RTA | | `rta.*` |
| **Timing quality** | Whether a measurement's timing can be trusted for delay, polarity and crossover changes: "Timing reliable" or "Timing unreliable" (then only its magnitudes are used) | at most 10 µs per cycle | |

### 3.5 Several units, the guard, and the firmware generations

*WP DOCSYNC, 2026-11-19. Measuring several units together (PLAN-AT,
[`docs/integrator/multi-unit-measurement.md`](integrator/multi-unit-measurement.md)), the hub's guard of a system apply
([`docs/integrator/hub-api.md`](integrator/hub-api.md) section 23.4) and both firmware generations (PLAN-PG,
[`docs/integrator/protocol-generations.md`](integrator/protocol-generations.md)). The words as the notices
(`assets/notices/notices.json`) and the manual use them.*

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **alink**, **dlink** | Short names in code, plans and file names only: **alink** is the audio link (section 3.1), **dlink** is DSPi Link, the network connection to a hub. Text a user reads says "Audio link" and "DSPi Link" in full. Never "link" alone for the audio link | | |
| **Time base** | The sample counter several units share and play by: the audio link's frame counter, the USB time base of USB-direct units on one USB controller, or the one a DSPi set as an S/PDIF source carries to the units that follow it. Units on one time base can be measured and aligned together; "These units do not share a clock" otherwise | samples | |
| **S/PDIF source**, **S/PDIF follower** | A DSPi that sends a DSPi time base on its S/PDIF output, and a DSPi that follows it on its S/PDIF input. A follower fed by other equipment (a TV, a receiver) finds no DSPi time base | | |
| **Settle wait** | The hub's wait, up to 40 s, before a run starts: right after the units lock, or after anything disturbed their clock, it waits until it has seen them locked for 40 s. Said in words ("waiting for the units' clocks to settle"), never as an error | up to 40 s | |
| **Anchor output** | The output every run of a large group repeats, so the runs can be joined through it; by default the first unit's first output | | |
| **Lateness** | How much later than the others a unit plays what it is sent (its output delays, a USB-direct stream's start offset). The planner spaces the sweeps and the listening for it; a unit more than a second later is refused before anything plays | ms, up to 1000 | |
| **Summed check** | Playing together the outputs where two drivers or two units meet, and comparing the sum with the prediction, before anything is applied and again in the verify. A disagreement of more than 1.5 dB withdraws the timing and crossover changes (between units: every delay between them) | dB | |
| **Guard** | The way back of a guarded apply: a countdown after the change, during which Keep keeps it; when it runs out, or the console goes away, the settings from before are put back by themselves (by the hub, for a system apply or a guarded recall over DSPi Link). The feature is called Safe apply; the hub's side of a system apply is `docs/integrator/hub-api.md` section 23.4 | s | |
| **Generation** | Which protocol a device's firmware speaks: **legacy** (1.1.6 and older, page 0, with or without this fork's extension channel) or **paged** (upstream 1.1.7 and later, commands by page and id). Shown in the device info; decided once per connection | | |
| **Extension channel** | This fork's own requests (`REQ_EXTENSION` 0x00 on legacy firmware, the tunnel on page 0xE0 on paged firmware): the crossfeed pair map, core voltage, the audio link and its time base. Absent on stock firmware, where its features are hidden | | |
| **alink sender** | The device that feeds the audio link from USB (section 3.1's Sender); it connects as its own device kind with its device info, its audio link block and the firmware update, and has no DSP screens. The core recognises it on either generation; the dual-stack sender firmware (paged) is not written yet | | |

## 4. Units

| Unit | Meaning | Used for |
|---|---|---|
| `dB` | A gain or a level difference: boost, cut, trim, attenuation. Shown signed | Volume, gains, trims, feed, drive, gain reduction |
| `dBFS` | A level against digital full scale, at most 0. Shown unsigned | Thresholds, ceilings, gates, meter levels, signal generator level |
| `dB SPL` | An acoustic sound pressure level | Loudness reference level |
| `Hz` | Frequency; `kHz` from 1000 Hz up ("2.86 kHz") | Band frequencies, cutoffs, sample rate |
| Q | No unit; shown with two decimals | Filter Q |
| `%` | A proportion of the block's full effect | Amounts, mixes, widths |
| `ms` | Time | Delays, attack, release, hold |
| samples | Only for a latency the firmware fixes in samples, with ms beside it | Limiter lookahead |

## 5. Modes

From `docs/UI-MODES.md`; the words a client shows.

| Term | Meaning |
|---|---|
| **Simple** | The front page first; the default on phones. It hides nothing permanently |
| **Advanced** | Today's screens; the default on desktops and the web |
| **Installer** | Advanced plus limits, power-on state, firmware, factory reset and core voltage; behind the PIN when one is set |
| **Locked user** | A state, not a mode: what an installer has left a user to change (volume, mute, preset) |
| **Installer PIN** | A UI lock an installer sets; not a security boundary |
| **Pro layer** | The plans' name for the new Advanced screens. Not a word in the UI |
