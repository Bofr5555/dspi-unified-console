# DSPi Terminal

Cross-platform terminal control for [DSPi](https://github.com/WeebLabs/DSPi)
audio processors: the DSPi Console's layout and controls in a full-screen
terminal interface, plus a scriptable one-shot CLI. The command is `uconsole`
(package `dspi-term`).

It is one of the clients in this repository. It came from Weeb Labs'
DSPi-Terminal (see the root [`README.md`](../../README.md), Credits) and uses the
repository's session crates directly (`crates/dspi-proto`, `dspi-transport`,
`dspi-session`, `dspi-cmd`, `dspi-tui`), with `dspi-core` for `uconsole flash` and
`dspi-hub` for `uconsole hub`.

**Status:** the terminal's work packages are done except its hardware pass on a
real board (TM13 in
[`docs/core-merge/PLAN-CLIENTS.md`](../../docs/core-merge/PLAN-CLIENTS.md)). It is
tested against the simulated device, mocks and a fake hub. What it lacks against
the macOS Console is in [`docs/PARITY.md`](../../docs/PARITY.md), section 5. The
plan and surveys it was built from, before the merge, are in
[`docs/terminal/plan/`](../../docs/terminal/plan/).

## The interface

```
 DSPi  RP2350 · fw 1.1.6 · A1B2C3D4                     ● Connected   Preset 3: Living Room *
╭────────────────────╮╭ Filter Response · FL ─────────────────────────────────────────── ⤢ g ╮
│ INPUTS             ││+20                  ┆                    ┆        ┆           ┆      │
│▍▪FL     ▓▓▓▓▓▓▏ IN1││                     ┆                    ┆        ┆           ┆      │
│▍▪FR     ▓▓▓▓▓▌▏ IN2││    ⠒⠒⠒⠒⠒⠦⠤⠤⠤⠤⠤⢤⣀⣀⣀⣀⡀┆                   ⢀⣀⣀⣀⣀⣀⡤⠤⣄⡀┆     ⣀⣀⡤⢤⣀⣀⣀⣀⣀⣀⣀⣀⣀│
│ ▪FC     ░░░░░░▏ IN3││  0 ────────────────⠉⠉⠉⠙⠒⠒⠒⠒⠒⠒⠒⠋⠉⠉⠉⠉⠉⠉⠉⠉⠉⠉────────⠙⢦⣠⠖⠋⠉⠉⠁────────────│
│ ▪LFE    ░░░░░░▏ IN4││                     ┆                    ┆        ┆           ┆      │
│ ▪BL     ░░░░░░▏ IN5││-20                  ┆                    ┆        ┆           ┆      │
│ ▪BR     ░░░░░░▏ IN6││      20      50    100   200      500   1k    2k      5k     10k  20k│
│ ▪SL     ░░░░░░▏ IN7││──────────────────────────────────────────────────────────────────────│
│ ▪SR     ░░░░░░▏ IN8││ Link 1/2   ▸Preamp +0.0 dB ━━━━━━━━━━━━━━━━━━━━━━━━●━━━━   Clear PEQ │
│                    ││──────────────────────────────────────────────────────────────────────│
│ OUTPUTS            ▼│   # TYPE                       FREQ      GAIN   WIDTH                │
│ ●Crossfeed Matrix  ││ ● 1 Low Shelf (12dB)         105 Hz   +8.8 dB   0.707                │
│ ○Loudness  Stats   ││ ● 2 Peaking                   64 Hz   -6.2 dB     0.3                │
│ ○Leveller  Settings││ ● 3 Peaking                 2856 Hz   -8.6 dB    3.58                │
│ ●Bass      ○Bypass ││ ● 4 Peaking                 1880 Hz   +3.6 dB    1.69                │
│ Preset ‹3: Livin…› ││ ● 5 Peaking                 6749 Hz   +4.0 dB    4.74                │
│ Source ‹USB›       ││   6 Off                                                              │
│ User -12.0 ━━━━━●━ ││   7 Off                                                              │
│ C0 ▓░ 31% C1 ▓░ 74%││   8 Off                                                             ▼│
╰────────────────────╯╰──────────────────────────────────────────────────────────────────────╯
 :eq in.1 3 freq 2856                                     Enable All │ Bypass All  Clear All
 ↑ ↓ Band, or the header · ← → Field · Enter Edit · Space Bypass · 1-9,0 Jump to a band ·
```

The shape is the Console's: a channel sidebar with a colour swatch, meters
and clip cells; the response graph for the selected channel, with a linked
partner's curve underneath in grey; below it the input page or the output
page. The overview is a grid of small graphs, one per group of channels
whose curves are identical, each with a one-line summary. The output page
carries the output limiter beside Mute, with its threshold, release, link
group and gain reduction. The Console's tool windows are panels that
replace the right pane: the Matrix Mixer, the DSP panels (Loudness,
Crossfeed, Volume Leveller, Psychoacoustic Bass, Stereo Upmixer,
Subharmonic Synthesizer and Tube Modeller), the Signal Generator,
the Spectrum Analyser, Stats, the Interrupt Monitor and AutoEQ. Settings
replaces the screen, with the Console's pages, including Display > Spectrum
Analyser and Control > Auxiliary Outputs for relays, LEDs and dimmers on
spare GPIOs. Colour follows attention: the selected channel's hue, red for
clip and mute, orange for warnings, greys for the rest. `--theme console` puts
the Console's colour on every channel instead.

### Keys

Uppercase letters open the Console's tools, with the Console's own
mnemonics (its Shift-Cmd-M is `M` here):

| Key | Opens |
|---|---|
| `M` | Matrix Mixer |
| `L` `X` `V` `P` `U` | Loudness, Crossfeed, Volume Leveller, Psychoacoustic Bass, Stereo Upmixer |
| `S` `D` | Subharmonic Synthesizer, Tube Modeller |
| `G` `A` | Signal Generator, Spectrum Analyser |
| `T` `I` | Stats for Nerbs, Interrupt Monitor |
| `B` | AutoEQ |
| `,` | Settings |

Everywhere:

| Key | Does |
|---|---|
| `Tab` | Next region: channels, footer, detail |
| `↑ ↓` | Select a channel; `Enter` returns to the overview |
| `Space` | Toggle: a bypass, a feature in the quick strip, mute on the volume row |
| `;` | Page commands: a small grammar scoped to the current page, with live hints (`1 2 > 3` routes on the matrix, `3 peak 1k -2` sets a band, `gain -3` trims an output) |
| `Ctrl-P` | Search every parameter, tool and action |
| `:` | Command line, the same grammar as the shell |
| `Ctrl-S` | Commit Parameters to the active preset |
| `Ctrl-Z` `Ctrl-Y` | Undo, redo |
| `= g p . + - h l` | Graph height, pop-out, phase, linked partner, zoom, cursor |
| `f t` | The live spectrum on the graph; take the analyser back from another console |
| `F` | The analyser's third-octave bars above the page |
| `b` `c` | Bypass Master EQ, clear clip latches |
| `?` | Help for whatever has focus |
| `q` | Quit, with the unsaved-changes prompt |

The echo line prints every change as the `:` command that would make it, so
anything done in the interface can be pasted into a script.

## Building

From the repository root:

```sh
cargo build --release -p dspi-term     # the binary is target/release/uconsole
cargo test -p dspi-term
```

In this repository every toolchain runs in a container: `ci/check.sh --step
terminal` builds the release binary, and `ci/check.sh --crate dspi-term` runs
its format check, clippy and tests (the root `README.md` has the rest).

Requires Rust 1.90 or newer. USB goes through `nusb`, which talks WinUSB, IOKit
and usbfs directly, so USB needs no C library. The default features add `--midi`
(through `midir`, which needs the ALSA development files on Linux) and finding
hubs on the network (mDNS); `--no-default-features` builds without both.

## One-shot commands

```sh
uconsole                              # open the interface
uconsole --demo                       # the same, on the built-in simulated device
uconsole doctor                       # why can't it see my device?
uconsole dump                         # the whole device state
uconsole watch                        # notifications as they arrive

uconsole get vol.user                 # read anything
uconsole vol.user -18                 # write anything; `set` is optional
uconsole eq in.1 3 peak 2856 3.58 -8.6
uconsole mix 0 4 on -3 inv            # a crosspoint
uconsole set cs.binding 3 type=encoder noun=user_volume action=step gpio=10,11
uconsole params                       # every parameter this build knows

uconsole export tuning.txt            # Console-compatible filter file
uconsole export room.dspipreset       # whole-device document
uconsole import tuning.txt            # apply one back
uconsole export - --json | jq .meta   # whole-device document on standard output
uconsole export f.json --format filters-json   # filters as JSON
uconsole import - --dry-run --json < room.dspipreset
uconsole cs export surfaces.json      # control-surface configuration
uconsole cs import surfaces.json --save
uconsole -f evening.dspi              # a script, one command per line

uconsole autoeq search hd600          # headphone corrections
uconsole autoeq rebuild --yes         # download the profiles from GitHub again
uconsole undo                         # put back what the last change replaced
uconsole raw 0x87 41                  # any opcode, hex-dumped
uconsole completions zsh              # shell completion
```

### Firmware

```sh
uconsole export backup.dspipreset     # first: updating can reset settings
uconsole flash DSPi.uf2               # check, reboot to bootloader, write, confirm
uconsole flash                        # only reboot into the bootloader
uconsole flash DSPi.uf2 --dry-run     # what would happen; changes nothing
```

The board has to be on this machine's USB, and read-only devices can update too.
Some boards hang on firmware that runs at a lower core voltage than they need, so
`flash` reads the board and the image first. If they do not match, or the image
raises the voltage above 1.20 V, or the board is already in its bootloader and
cannot say, it stops. `--force` skips the ordinary "are you sure" and nothing
else: only `--accept-core-voltage-risk` plus typing the answer it asks for gets
past this. Success means the board came back running the image's version.

Every command takes `--json` for machine-readable output, `--dry-run` to change
nothing, `--quiet`, and `--device <serial>` to pick a unit. Exit codes
distinguish bad input (1) from no device (2) from a write the device refused
(3) from a transport failure (4). `uconsole --help` lists every command and option.

### Other ways to reach a device

Without a flag, `uconsole` uses a DSPi on this machine's USB. These choose another
link, for the interface and for every command:

```sh
uconsole --demo dump                  # the built-in simulated device
uconsole --midi list                  # the MIDI ports; --midi [PORT] connects over USB MIDI
uconsole link scan                    # the DSPi Link hubs on this network
uconsole --hub dump                   # a device behind a hub found on this network
uconsole --link host:port dump        # a device behind a hub at that address
```

A hub asks for its PIN the first time; `--pin -` or the `DSPI_PIN` environment
variable gives it to a script. Link traffic, the PIN included, is plain text, so
use it on a network you trust.

### Sharing devices: `uconsole hub`

```sh
uconsole hub                          # share this machine's devices on the network
uconsole hub --help                   # pairing, clients and roles
```

The hub pairs clients by PIN; a PIN gives at most the control role. Admin is given on
the hub's machine: `uconsole hub role <cid> admin`, or `uconsole hub --pair-admin` for
the one client that pairs in that window. Example service files for systemd, launchd and the
Windows task scheduler are in [`packaging/service/`](packaging/service/), with
install notes.

An audio link installation of simulated devices, for testing (WP SW3, HWLK1F):

```sh
uconsole --demo hub --alink            # a sender, a fan-out, four units, two USB-direct units and their model
  --with-plain-unit                   # test option: one more without the audio link (unit P)
  --with-usb-lock-error               # test option: USB-direct Rear L/R can't lock (host out of range)
  --with-usb                          # test option: this machine's USB DSPis too, to test a
                                      # mixed installation on hardware (add them to the model)
```

The web console (WP WB7, `apps/web`'s built dist; [`apps/web/DEPLOY.md`](../web/DEPLOY.md)):

```sh
uconsole hub --web --console <dist>   # / links both: the console at /console/,
                                  # the hub's own page (DSPi Link's) at /hub/
uconsole web [--demo]                 # this machine only: http://localhost:11916/console/,
                                  # WebUSB and Link in one page (uconsole hub --local)
```

## Device access

- **Windows**: nothing to install. The firmware ships MS OS 2.0 descriptors, so
  WinUSB binds automatically.
- **macOS**: nothing to install. The vendor interface is *exclusive*, so DSPi
  Console and this tool cannot hold the device at the same time.
- **Linux and Raspberry Pi**: `sudo uconsole --install-udev`, then
  `sudo udevadm control --reload`, then unplug and replug the device.

See [`docs/terminal/platforms.md`](../../docs/terminal/platforms.md) for what
has been verified where.

## Protocol source of truth

The firmware headers in `crates/dspi-proto/firmware/` are vendored from
`Bofr5555/DSPi` branch `bofr-main` (Weeb Labs firmware 1.1.6 final plus the
fork's extension channel; wire format V32). The exact commit and the file list
are in `crates/dspi-proto/firmware/PROVENANCE.toml`. The headers are parsed at
build time to generate every opcode and constant and the wire image's section
offsets. What the generator cannot lift from a `#define` (the packet
structs, which the build does not parse, and the tube type table, which
lives only in `tube.c`) is a hand table with a comment citing its source
line, and tests check it against the headers or the firmware source. A
coverage test fails if an opcode has neither a registry row nor a written
reason for leaving it out. To move to a newer firmware, follow
[`docs/terminal/firmware-bump.md`](../../docs/terminal/firmware-bump.md).

Firmware on another wire format connects read-only: the interface shows a
banner and no write leaves.

The device pushes its own changes over a notification endpoint, so a knob on
a control surface, a remote, or the operating system's volume slider shows up
in the interface without a refresh, with a note saying who changed it.

## Licence

DRAFT, pending the owner's approval: the program as a whole is distributed under
the GNU GPL version 3; Bofr5555's own additions, and DSPi-Terminal's parts, are
GPL-3.0-or-later (Bofr5555's with three section 7 terms). `NOTICE.md`, beside
this file in a release archive, is the statement; `uconsole --license` prints it
and `uconsole --third-party-licences` the Rust crates' licence texts.
