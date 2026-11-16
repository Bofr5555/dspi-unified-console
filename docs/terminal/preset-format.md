# The `.dspipreset` document

A `.dspipreset` is a whole-device configuration as JSON: schema 1 of the
Windows Console's `PresetDocument.cs`, extended additively by the macOS
Console (`PresetDocument.swift`, `PresetDocumentTransfer.swift`, DSPi Console
`9dbb07a`). The Terminal reads and writes it in
`crates/dspi-session/src/preset_file.rs`. The Linux Console
(`DSPi Console for Linux`, `fadeac4`) has no `.dspipreset` support at all: its
`core/src/preset.rs` only drives the device's own preset slots.

## Shape

Top-level keys: `schemaVersion` (1; a larger number is refused), `meta`,
`global`, `loudness`, `crossfeed`, `leveller`, `psybass`, `upmix`, `subharm`,
`tube`, `channels`, `matrix`, `io`. `psybass`, `upmix`, `subharm` and `tube`
are absent when the source device lacked the feature, which is different from
present and disabled. Values are the firmware's raw wire numbers (filter
types, masks, modes, pins), never names. Every reader is lenient about a missing
key. The Mac Console gives it the Console's default, which is the firmware's (for
example `loudness.outputMask` 0xFFFF and `crossfeed.outputPairMask` 0x01), never
zero; the Terminal and Qt leave that setting as it is (see the table at the end).

`meta` carries `name`, `savedUtc` (ISO-8601 UTC), `appVersion`, `platform`
(`RP2040` or `RP2350`), `firmwareVersion`, `wireFormatVersion`,
`inputChannelCount`, `outputChannelCount`, and the Console's additive
`masterVolumeMode` and `outputConfigMode`. Nothing in `meta` gates an import.

<!-- WP P1DF -->
`meta.kind` (additive to schema 1) says which document of the family it is:
`preset`, `snapshot`, `template` or `limits`. Absent, or a word a build does not
know, reads as `preset`, so such a document still imports; a preset export
leaves the key out. One format serves all four (PRO1 section 6).

<!-- WP P5MDOC -->
Phase 5 adds three kinds that configure no device. Each carries its own block
named as its kind instead of the device blocks; `config_import` refuses them
(P5DOCK) and their own readers (`dspi_measure::doc`) refuse every other kind.
The contract is [`docs/integrator/measurement.md`](../integrator/measurement.md)
section 7.

| `meta.kind` | Block | Holds | Largest |
|---|---|---|---|
| `measurement` | `measurement` | one run's responses per output (compact curves), the chain each output was measured through and its programme feed, the stimulus, the capture's timing quality, the microphone used, the tunes applied from it | 1 MiB |
| `mic` | `mic` | a measurement microphone's calibration on a grid, its sensitivity, the file it came from | 256 KiB |
| `target` | `target` | a target curve (built-in parameters or imported points), an optional coupler compensation, the reference band | 256 KiB |

## Channel numbering

This is the part that decides where every EQ band, gain and delay lands.

Each entry in `channels` carries up to four identities:

| Key | Meaning | Written by |
|---|---|---|
| `channelId` | The Windows Console's channel id (table below) | Windows, macOS, Terminal |
| `eqChannel` | The firmware's unified channel index (config.h:788-790) | macOS, Terminal |
| `inputIndex` | Wire input index 0..7, on input entries only | macOS, Terminal |
| `outputIndex` | Matrix output index, on output entries only | macOS, Terminal |

The Windows ids were laid down when the device had two inputs; outputs grew in
the middle and the extra inputs went above them:

| Channel | Windows id |
|---|---|
| Input 0, 1 | 0, 1 |
| Input 2..7 | 11..16 |
| Output *o* | 2 + *o* (RP2040 PDM, output 4, is id 6; RP2350 PDM, output 8, is id 10) |

The unified index is inputs from 0 and outputs from `CH_OUT_1 =
NUM_INPUT_CHANNELS`. On RP2040 (2 inputs, 5 outputs) the two numberings
coincide. On RP2350 (8 inputs, 9 outputs) they agree only for inputs 0 and 1:
Windows id 2 is output 0, while unified index 2 is input 2.

**Placing an entry** (the macOS Console's `ChannelBlock.ref(platform:)`,
PresetDocument.swift:674-684), read against the connected device:

1. An output entry with `outputIndex` goes to that output; an input entry
   with `inputIndex` goes to that input.
2. Otherwise `channelId` is resolved through the table: 0..1 and 11..16 are
   inputs, anything else from 2 up is output `id - 2`.
3. A channel the device does not have is reported as "Not present on this
   device", never moved elsewhere. A duplicated channel takes the last entry.

`eqChannel` is written but no reader places a channel by it. `isOutput` is not
consulted by the Consoles when resolving `channelId`.

The matrix (`input`, `output`) and `global.inputPreampsDb` (by wire input) use
wire indices directly and need no translation.

## Files from older Terminal builds

Terminal builds before this format fix wrote the unified index as `channelId`
and none of the index fields. On RP2040 that is the same as the Windows id, so
those files were always correct. On RP2350 they are recognised by their
content: no entry carries `eqChannel`, `inputIndex` or `outputIndex`, every
entry agrees with the unified numbering (id below `meta.inputChannelCount`
exactly when it is an input), and at least one entry contradicts the Windows
table (an input at 2..10, or an output at 11..16). No Console-written file can
pass that test, because every Console entry agrees with the table. Such a file
is read by the unified numbering in the Terminal. The Consoles do not know this
rule: they will place an older Terminal RP2350 file by the Windows table, so
inputs 2..7 land on outputs 0..5 and outputs 8..16 on outputs 6..8 or nowhere.
Re-export it from the Terminal to fix it.

## What an import applies

In the Console's order (PresetDocumentTransfer.swift:295-667): output enables
(every disable, then every enable, an enable that would collide on Core 1
reported rather than forced); each channel's name, delay, output gain, mute and
`outputDelayMs`, EQ and crossover (a non-empty bank replaces the channel's,
the rest going flat; an empty one leaves it alone) and each input's preamp;
the matrix; bypass, LG Sound Sync; loudness, crossfeed, leveller, psybass,
subharm, tube and upmix, each with its parameters and mask before its switch.
Volume levels are opt-in, and master volume is written only when the device's
master-volume mode is with-preset. Hardware I/O (pins, clocks, inputs, the
limiters inside each output entry) is opt-in. The input source goes last.
`global.inputPairLinked` is app state; the Terminal applies it to its own pair
links.

<!-- WP P1DF -->
The core's import (`config_import_confirmed`, with its token; `config_preview`) also applies by
block: with `IMPORT_BY_BLOCK` each of `IMPORT_EQ` (per channel, and the master
EQ bypass), `IMPORT_CROSSOVER`, `IMPORT_ROUTING` (matrix, input source, preamps,
input delays, input pair links, LG Sound Sync), `IMPORT_OUTPUTS` (gain, mute,
enable, delays), `IMPORT_LIMITERS`, `IMPORT_EFFECTS`, `IMPORT_VOLUMES`,
`IMPORT_HARDWARE` (the `io` block) and `IMPORT_NAMES` is chosen on its own, and
a block left out keeps the device's values. Without it, `IMPORT_VOLUMES` and
`IMPORT_HARDWARE` keep their meaning (the audio blocks always, the hardware bit
bringing the limiters). `config_preview` and `config_diff` say what would change,
one row per value: `{block, section, item, before, after}`.

<!-- WP P1AFIX -->
With `IMPORT_BY_BLOCK` and no block bit nothing is chosen: `config_import_confirmed`
refuses (`noticeKey` `config.nothing_selected`) and writes, journals and logs
nothing, so the undo history stays. Every refusal of `config_import_confirmed`,
`config_preview`, `config_diff` and `config_inspect` is
`{ok: false, error, noticeKey}`: `config.unreadable` for a text that is not a
document this build reads, `config.not_read` for a preview before the device's
settings were read, otherwise the core error's own key (`disconnected`,
`read_only`, `locked`, ...); C and the web binding pass it on. The crossfeed rows
of a diff or preview compare the stored custom pair (`crossfeed.freqHz`,
`feedDb`), which the document holds, not a named preset's values that
`get_state` shows; `crossfeed_custom_pair()` (C `dspi_get_crossfeed_custom_pair`,
web `crossfeedCustomPair`) reads that pair typed.

## Keys a document does not hold

A value in a Terminal export is one the device reported. A read that fails (the
device is gone, a timeout, a short reply) fails the export and writes no file;
files are written beside the target and renamed over it, so an old export
survives whole. A read the device refuses (a stall, or a link's `Denied`) means
it does not report that setting: on a read-only device the value comes from the
bulk image mapped onto this build's layout, if that image's wire format carries
it, and otherwise the key is left out and the export warns, naming the keys. A
block left with no keys is left out whole. An EQ or crossover bank is whole or
absent, because a shorter one flattens the bands it leaves out; list positions
in `inputPreampsDb` and the `io` lists stop at the first unreported entry.
`schemaVersion`, `meta` and `channels` are always written.

A filter export (text or JSON) follows the same rule: a failed read writes no
file, and a refused one takes the image, or else leaves that channel's preamp or
whole bank out, with a warning naming them. A filter import leaves an absent bank
alone.

On import the Terminal and Qt (`dspi_core::files`) leave an omitted key alone and
report it (`Left as they are`, and `leftAlone` under the Terminal's `--json` and in
the core's import reply). Both warn when `meta.wireFormatVersion` is below this
build's, since an export from such firmware by a build before this rule can hold
0 dB or off for settings nobody set. The Mac Console reads an omitted key
differently, and takes the same files without error:

| Omitted | Terminal and Qt | Mac Console |
|---|---|---|
| A `global` key | left alone | written as its default (0 dB, off, USB) |
| Whole `loudness`, `crossfeed`, `leveller` | left alone | written as defaults |
| A key inside one of those blocks | left alone | written as its default |
| A channel's `gainDb`, `muted`, `enabled`, `delayMs` | left alone | written as a default |
| A channel's `eq` or `crossover` | left alone | left alone |

So a file that left keys out is safest imported by the Terminal or Qt. The Mac's
reader accepts any document with a `schemaVersion` and `channels`
(PresetDocument.swift:627, 729-744), and Qt's the same. Qt's reader also takes a
value of the wrong type, which counts as omitted; the Terminal refuses that file.

Qt's export follows the same rule from its mirror: the device's wire format decides
each key (`dspi_proto::wire_compat::features`), so an older device's document has no
key its format lacks, and `crossfeed.pairMap` is written only when the device reports
a map over the extension channel. Capture asks the device and keeps what it answers,
which a mirror cannot, so the two gates are not one table: they agree on the masks
and blocks by format, and capture also needs the firmware's feature flag. Otherwise
Qt leaves `qp` out below V22 (capture writes 0.707), writes `i2sBckPinSlave` from V21
(capture never does), and goes by format alone for `spdifEnabledExt` and the ADAT
keys, so RP2040 carries the latter. With no device's state to export, it writes no file: the core
returns a typed error (`Disconnected`, `Unsupported` for an image it cannot map,
`Transfer` before the image is read). A filter import in Qt leaves a bank alone when
its section has no lines for that bank; an `OFF` line is a flat band and counts as a
line.
