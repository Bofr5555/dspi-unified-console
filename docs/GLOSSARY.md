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
are `audio_link_*`, `fetch_audio_link_frame_counter` and `apply_at_frame`.*

| Term | Meaning | Unit and range | Registry |
|---|---|---|---|
| **Audio link** | The optical or SFP cable that carries audio from a sender to devices, with one clock for all of them. Always "Audio link" in full: **DSPi Link** is the network connection to a hub, and "Link" alone means that. The input source's display name; its command word is `link` (source 7) | | `in.source` |
| **Sender** | The device that takes USB audio in and sends it on the audio links; its clock is the system's | | |
| **Receiver** | A device that plays what an audio link carries. A **Repeater** is a receiver that passes the stream on to the next one | | |
| **Link channel** | One audio channel in the link's stream, numbered from 0. A stream carries up to 64 | 0 to 63 | |
| **Link slots** | Which link channel feeds each of a receiver's 8 matrix inputs; "Silent" for none. Not "Mapping" or "Routing" (the Matrix routes) | | |
| **Locked**, **Searching**, **Holdover**, **Off** | The link's state. Holdover: the link was lost, the outputs are muted and the clock is held until it returns | | |
| **Line class** | The link's speed: L1 (TOSLINK parts), L2 (50 MBd parts), L3 (SFP) | L1, L2, L3 | |
| **Phase lock** | Every receiver plays a link frame the same fixed delay after it arrives, so all devices play in step. The other mode, **Fill servo**, is how S/PDIF and ADAT inputs follow their source | on or off | |
| **Link delay** | The fixed delay of phase lock, in link frames, with ms beside it (one frame is one sample period). The plans' `D_local` | frames (ms) | |
| **System delay** | The delay D every unit's link delay is worked out from, so all units of one sender play a link frame at the same moment. The hub picks the smallest every unit takes unless an installer fixes it. The plans' `D` | frames (ms) | |
| **Link delay range** | The shortest and longest link delay a unit takes; it follows its output latency (`d_local_min`..`d_local_max`) | frames | |
| **Hop offset** | Frames added to a unit's link delay for the fan-outs and repeaters between it and the sender, which each pass the stream on a few frames late (`hops_offset`) | frames, -64 to 64 | |
| **Re-anchor** | Restarting the outputs in step after a loss or a new link delay: a short silence (2 to 6 ms). Not "Resync" in text a user reads | | |
| **Frame counter** | The link's running count of frames, the same on every device of one sender | frames | |
| **Apply at frame** | A change (mute, unmute, preset, master volume, output delay, rate, test click) that every device makes at the same link frame | | |

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
