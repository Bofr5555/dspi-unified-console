# The Terminal: `uconsole`

The Terminal of uConsole for DSPi: cross-platform terminal control for
[DSPi](https://github.com/WeebLabs/DSPi) audio processors. It is the layout and
controls of Weeb Labs' DSPi Console in a full-screen terminal interface, a plain
line mode for screen readers and scripts, and a scriptable one-shot CLI. The
command is `uconsole` (package `dspi-term`).

It is one of the clients in this repository. It came from Weeb Labs'
DSPi-Terminal (see [`docs/CREDITS.md`](../../docs/CREDITS.md)) and uses the
repository's session crates directly (`crates/dspi-proto`, `dspi-transport`,
`dspi-session`, `dspi-cmd`, `dspi-tui`), writes through `dspi-core` (also
`uconsole flash`, the PRO commands and `uconsole measure`) and runs `dspi-hub` for
`uconsole hub`.

**Status:** release 0.16.0. The Terminal is tested against the simulated device,
mocks and a fake hub; nothing new in 0.16.0 has run on a real DSPi yet. What it
lacks against the macOS Console is in [`docs/PARITY.md`](../../docs/PARITY.md),
section 5. The plan and surveys it was built from, before the merge, are in
[`docs/terminal/plan/`](../../docs/terminal/plan/). `uconsole --help` lists every
command.

**New in 0.16.0** (since 0.14.0; 0.15.0 was not published):

- [Plain mode](#plain-mode-lines-for-a-screen-reader-a-log-or-a-script):
  `uconsole --plain`, or `uconsole` at a terminal with `TERM=dumb`, is the
  interface as sentences, one command per line.
- With nothing plugged in, `uconsole` waits for a DSPi and offers the demo and
  DSPi Link. The demo is a tuned two-way speaker; `:reset-demo` starts it again.
- `Ctrl-P` search finds words ("crossover", "mute"); the `:` line runs `snapshot`,
  `ab`, `diff`, `lock`, `limits` and `status` on the open device; numbers take `k`
  (`2k`).
- `Tab` goes channels, page, footer; `Enter` on a route in the Matrix Mixer edits
  its gain and never connects; `[` `]` `{` `}` and `m` set the listening volume and
  mute from any page; outputs nothing plays to fold under "N unused".
- Settings › Speakers (which output drives what) is read-only under the
  installer's lock. Filters say **Bell** and **Q**, and `Ctrl-S` is **Save to
  Device**.
- Snapshots and A/B (`N`), `uconsole mcp` to relay an AI agent to the desktop
  app, unused outputs left out of the Overview, Spectrum and Stats, and
  `uconsole measure` taking its roles from the device's speaker layout.

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
│ OUTPUTS            ▼│   # TYPE                       FREQ      GAIN       Q                │
│ ●Crossfeed Matrix  ││ ● 1 Low Shelf (12dB)         105 Hz   +8.8 dB   0.707                │
│ ○Loudness  Stats   ││ ● 2 Bell                      64 Hz   -6.2 dB     0.3                │
│ ○Leveller  Settings││ ● 3 Bell                    2856 Hz   -8.6 dB    3.58                │
│ ●Bass      ○Bypass ││ ● 4 Bell                    1880 Hz   +3.6 dB    1.69                │
│ Preset ‹3: Livin…› ││ ● 5 Bell                    6749 Hz   +4.0 dB    4.74                │
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
| `N` | Snapshots: save the device as it is, Recall, and A/B (the device now against a snapshot: `a` starts it, `Space` switches, `k` keeps the side it is on). Over DSPi Link a recall counts down: `k` keeps it, `r` puts the settings back |
| `,` | Settings (Settings › Speakers: which output drives what, the roles the measurement and its safety checks take; `n` names a speaker, `g` sets a group, Confirm keeps it; read-only under the installer's lock) |

Everywhere:

| Key | Does |
|---|---|
| `Tab` | Next region: channels, then the page, then the footer (one region: `↑ ↓` move inside it); `Shift-Tab` goes back |
| `↑ ↓` | Select a channel; `Enter` or `→` goes into its page, `Esc` back to the overview. Outputs nothing is routed to are folded under "N unused ▸" (`Enter` opens it) |
| `[` `]` | The listening volume down, up by 1 dB, from any page; `{` `}` (with Shift) by 3 dB |
| `m` | Mute or unmute the listening volume, from any page |
| `Space` | Toggle: a bypass, a feature in the quick strip, mute on the volume row |
| `;` | Page commands: a small grammar scoped to the current page, with live hints (`1 2 > 3` routes on the matrix, `3 peak 1k -2` sets a band, `gain -3` trims an output) |
| `Ctrl-P` | Search: words find places, tools, actions, verbs and parameters ("crossover", "mute", "low pass", "snapshot"); `Enter` goes there, `Tab` puts a command on the `:` line |
| `:` | Command line, the shell's grammar (numbers take `k`: `2k`); `:help` is `?`. `snapshot`, `ab`, `diff`, `lock`, `limits`, `status`, `measure docs`, `show`, `export` and, on the demo, `reset-demo` run here on this device, with the terminal handed back while they run; `:snapshot recall NAME` and `:ab NAME` run in the interface and open the Snapshots panel (`:snapshot` and `:ab` alone open it) |
| `Ctrl-S` | Save to Device: the active preset |
| `Ctrl-Z` `Ctrl-Y` | Undo, redo |
| `Ctrl-D` | Choose another device |
| `= g p . + - h l` | Graph height, pop-out, phase, linked partner, zoom, cursor |
| `f t` | The live spectrum on the graph; take the analyser back from another console |
| `F` | The analyser's third-octave bars above the page |
| `b` `c` | Bypass Master EQ, clear clip latches |
| `?` | Help for whatever has focus |
| `q` | Quit, with the unsaved-changes prompt |

The echo line prints every change as the `:` command that would make it, so
anything done in the interface can be pasted into a script.

In the Matrix Mixer (`M`), `Space` connects a route (or enables or mutes an
output), `Enter` on a route edits its gain and never connects it, `i` inverts it
and `d` sets Direct 1:1.

With no DSPi attached, `uconsole` opens on a page that waits for one and opens
it the moment it is plugged in; meanwhile `d` opens the demo, `l` a DSPi Link
hub (an address, or empty to look on the network) and `?` shows what
`uconsole doctor` finds. A board in BOOTSEL shows as the drive RPI-RP2 instead
(`uconsole flash`). One-shot commands still end with exit code 2 when there is
no device.

### Plain mode: lines for a screen reader, a log or a script

`uconsole --plain` (and plain `uconsole` when `TERM=dumb`) is the interface as
lines: no full-screen drawing, no colour, no box, braille or meter
characters. It greets with the overview as sentences, then takes one command
per line and says every change back as the command that makes it:

```
uconsole 0.16.0, plain mode. Connected to the demo device.
Volume 0.0 dB, not muted.
No unsaved changes.
USB 1: 1 filter, trim 0.0 dB
…
Left woofer: 2 filters (low-pass 2.2 kHz LR4), gain 0.0 dB, not muted
Right woofer: 2 filters (low-pass 2.2 kHz LR4), gain 0.0 dB, not muted
Left tweeter: 2 filters (high-pass 2.2 kHz LR4), gain -3.0 dB, not muted, delay 0.1 ms
Right tweeter: 2 filters (high-pass 2.2 kHz LR4), gain -3.0 dB, not muted, delay 0.1 ms
5 unused outputs are not listed (overview all lists them).
Type a command, help for the list, or quit.
dspi> out.gain 0 -3
out.gain 0 -3
dspi> show out 1
Left woofer: 2 filters (low-pass 2.2 kHz LR4), gain -3.0 dB, not muted
```

Every parameter and command of the shell's grammar works, as do `overview`
(`overview all` adds the unused outputs), `show out 1`, `save` (Save to
Device), `params`, `help` and the shell's `snapshot`, `ab`, `diff`, `lock`,
`limits`, `status` and `measure docs`/`show`/`export`, on the same session.
The shell's grammar counts outputs from 0 (`out.gain 0` is Out 1); `show` and
the sentences count from 1. `quit` asks once when something is unsaved. With no
DSPi plugged in, plain mode waits for one: type `demo`, `link ADDRESS` (`link`
alone looks on the network), `doctor` or `quit`.

With standard input piped (`uconsole --plain < session.txt`) there is no prompt,
every line runs (a mistake does not stop the rest), nothing waits for an answer,
and the exit code is the first failure's. A script's lines are its commands,
never answers: `ab` keeps the side it is on (`ab NAME --keep b` ends on B), a
guarded recall is kept at once, and the next line runs.

`--plain` takes its commands from standard input only: `uconsole --plain
vol.user -20` (a command on the line as well) is refused with a message saying
so; run the command without `--plain`, or type it in plain mode. `TERM=dumb`
starts plain mode only when standard input is a terminal; piped, the input is a
script as before.

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
uconsole usb-direct generate cinema.json --out usb-direct-cinema
                                      # a USB-direct home theatre's PipeWire files,
                                      # unit scripts and hub model (files only;
                                      # docs/integrator/usb-direct-home-theater.md)

uconsole autoeq search hd600          # headphone corrections
uconsole autoeq rebuild --yes         # download the profiles from GitHub again
uconsole -f tune.dspi                 # a script; `undo` in it steps back one of its own changes
uconsole raw 0x87 41                  # any opcode, hex-dumped
uconsole completions zsh              # shell completion
```

### PRO commands, measurement and the protocol

Each one-shot command is one session of the core, so an unlock lasts for that command.
[`docs/STATUS.md`](../../docs/STATUS.md) and [`docs/FEATURES.md`](../../docs/FEATURES.md) describe the features.

```sh
uconsole open site.dspipreset         # edit a document with no device, in the interface
uconsole diff site.dspipreset         # what it would change on the device
uconsole import site.dspipreset --only eq,crossover --guard 60   # type keep, or it goes back
uconsole snapshot save "Before" --note "as found"   # also: list | recall | diff | export | import | delete
uconsole ab "Before" 2 --keep b       # compare two snapshots, end on B
uconsole limits set profile.dspipreset   # also: show | get | clear
uconsole lock status                  # also: unlock | set-pin | clear-pin
uconsole --link hub.local mode status # the modes a hub pins per role (mode pin <role> <mode|none>)
uconsole status                       # connection, device, limits, lock and audio link
uconsole autoeq preview <id>          # what applying a profile would do

uconsole measure mic import umik.txt  # a calibration file (UMIK, Dayton, REW, CSV) as a mic document
uconsole measure target import house.txt --use speakers
uconsole measure docs                 # the filed documents (over --link: the hub's too)
uconsole measure show <id>            # one document in words
uconsole measure export <id> --output 2 out2.txt   # REW text
uconsole measure tune <id> --dry-run  # the proposal in words; without --dry-run: applied,
                                      # guarded, keep or revert
uconsole --demo measure run           # the demo device with its simulated microphone

uconsole info                         # generation, protocol version, pages, extension channel, board
uconsole raw --id 0x0110 16           # one paged GET by command id (firmware 1.1.7 and later)
```

`uconsole measure` opens no microphone. Delay, polarity and crossover changes need the
summed check and a verify, which only the demo's simulated microphone can do here, so off
the demo device a tune applies its EQ only, unverified, and says so (`uconsole measure
--help`). `raw --id` refuses, with nothing sent, whatever writes flash, restarts or is a
bulk session, and any id on a 1.1.6 device
([`docs/integrator/protocol-generations.md`](../../docs/integrator/protocol-generations.md)).

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

`flash` runs the core's update check first, for both firmware
generations: a board without BOOTSEL (the DSPi-Zero) gets no image and no reboot; an
image that would leave this fork's firmware asks for the typed phrase "I ACCEPT LOSING
THE EXTENSIONS", which `--force` does not answer; and a change of generation (1.1.6 to
1.1.7 or back) is said with what works afterwards. The write itself goes through the
core's write gate, which judges the board the core read.

Every command takes `--json` for machine-readable output, `--dry-run` to change
nothing, `--quiet`, and `--device <serial>` to pick a unit. Exit codes
distinguish bad input (1) from no device (2) from a refused write (3) from a
transport failure (4). `uconsole --help` lists every command and option.

Exit code 3 means nothing (or not everything) was written, and trying again
unchanged will not help:

- the device kept another value, or refused the request;
- the write is outside this console's limits profile (`uconsole limits`), in the
  core's words;
- the installer lock refused it ("Locked by the installer: ..."); give the
  installer's PIN, below;
- a dangerous write was not confirmed (see `--yes`);
- an import, `import` of filters or `cs import` had writes refused: `--json` says
  `"ok": false`, and each refused write is named with its reason.

**Every write goes through the console core.** `set`, `eq`, `undo`/`redo`,
scripts, `raw out`/`war`, `ext xfeed-map`, `ext vreg`, the imports, `cs import`,
`autoeq apply` and the interface (the TUI) all write through it, as the other
consoles do, so the limits profile, the installer lock and the confirmation tokens
apply to them.

**Dangerous writes need `--yes` in a script.** A preset save or delete
(`preset.save`, `preset.delete`), `dev.save`, `dev.save.io`, a factory reset
(`dev.reset`) and the bootloader (`dev.bootloader`) ask y/N on a terminal. In a
script add `--yes`; without it they exit 3 and nothing is sent.

**The installer's PIN.** While this console's (or the hub's) installer lock is
on, only volume, mute, the input source and a preset recall are written. With the PIN, every
writing command goes through: `DSPI_INSTALLER_PIN=... uconsole set ...`, or
`--installer-pin -` with the PIN as a line of standard input (for a script on
standard input, its first line), or the prompt on a terminal (nothing is echoed).
A PIN is never taken on the command line. The unlock lasts for that one command.
In the interface, `:unlock` (or the prompt a locked write offers) unlocks it until
the connection ends.

**`set --json`'s `outcome`.** The object is still `{path, value, outcome}`. For a
number, `outcome` is now `Confirmed(<the value the core holds>)`, after its clamp
and its rounding (`Confirmed(Float(-2.0))` for `-2.03`), where it used to be the
device's read-back. An action is `Triggered`, a dry run `Accepted`, and a toggle
or choice that did not take is still `Rejected` with exit code 3.

**`undo` in a one-shot command.** Each `uconsole` command is its own process with
a fresh history, so `uconsole undo` on its own has nothing to undo and says so.
`undo` and `redo` in a script step back through that script's own changes; the
interface keeps the history for as long as it is open, and there a compound edit
(a linked pair, Clear All, an import) is one undo step.

#### Moving scripts to this version

- A script line that saves or deletes a preset, saves (`dev.save`,
  `dev.save.io`), resets or enters the bootloader: add `--yes` to the command.
- A device with this console's installer PIN set and a lock on: give the PIN with
  `DSPI_INSTALLER_PIN` or `--installer-pin -`, or the writes outside volume, mute,
  the input source and presets exit 3.
- A script that read `set --json`'s `outcome` as the device's read-back: it is now
  the core's value after its clamp and rounding. Read the value back with
  `uconsole get` if the device's own word is wanted.
- An import that had writes refused used to exit 0; it now exits 3 with
  `"ok": false`.
- `ext vreg set` below the build default is refused (the core's range).

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
variable gives it to a script. The connection is TLS:
pairing shows the hub key's fingerprint to compare with the hub's, the key is pinned,
and a pinned hub is never reached in plaintext. A pasted `dspi://pair?...` link from the
hub's pairing screen works as `--link` (its key is checked). An unencrypted hub (an
upstream one, or `--tls off`) shows as "(unsecured)": `uconsole link security` says what
that means ([`docs/integrator/security.md`](../../docs/integrator/security.md)).

### Sharing devices: `uconsole hub`

```sh
uconsole hub                          # share this machine's devices on the network
uconsole hub --help                   # pairing, clients and roles
```

The hub pairs clients by PIN; a PIN gives at most the control role, and no pairing gives
more. Admin is given on the hub's machine only: `uconsole hub clients` lists the paired
clients and `uconsole hub role <cid> admin` makes one an Admin (with the hub stopped), or
type `clients` and `role <cid> admin` at the running hub's prompt, where it applies at once. Example service files for systemd, launchd and the
Windows task scheduler are in [`packaging/service/`](packaging/service/), with
install notes.

An audio link installation of simulated devices, for testing:

```sh
uconsole --demo hub --alink            # a sender, a fan-out, four units, two USB-direct units and their model
  --with-plain-unit                   # test option: one more without the audio link (unit P)
  --with-usb-lock-error               # test option: USB-direct Rear L/R can't lock (host out of range)
  --with-usb                          # test option: this machine's USB DSPis too, to test a
                                      # mixed installation on hardware (add them to the model)
  --with-measure-units                # test option: units on one time base to measure together
                                      # (one re-anchors every 15 s), one on another, S/PDIF units,
                                      # and the demo hub's simulated microphone
uconsole --demo hub --rollout --with-rollout-bench
                                      # test option: a simulated DSPi as if on this machine's USB,
                                      # with a simulated bootloader and writer (nothing is written)
```

The web console (`apps/web`'s built dist; [`apps/web/DEPLOY.md`](../web/DEPLOY.md)):

```sh
uconsole hub --web --console <dist>   # / links both: the console at /console/,
                                  # the hub's own page (DSPi Link's) at /hub/
uconsole web [--demo]                 # this machine only: http://localhost:11916/console/,
                                  # WebUSB and Link in one page (uconsole hub --local)
```

The hub API ([`docs/integrator/hub-api.md`](../../docs/integrator/hub-api.md)
section 16). A hub runs every family (health, inventory, Identify, history, alarms,
schedules), each turned off with `--no-health`, `--no-inventory`, `--no-identify`,
`--no-history`, `--no-alarms` or `--no-schedule`; the REST mirror only with `--rest`:

```sh
uconsole hub --rest [--data <dir>]    # /api/v1/ on the same port, for API keys
uconsole hub api-key new <name> [viewer|control|admin]
                                      # mint a key, shown once (hub stopped; at a running
                                      # hub's prompt: api-key new|list|revoke)
uconsole --hub api inventory.list     # any hub message; the answer's fields as JSON
uconsole --hub api param.get '{"serial": "E46058388B1A2E2C", "path": "vol.master"}' --json
```

Encryption ([`docs/integrator/hub.md`](../../docs/integrator/hub.md)): the
hub serves TLS on its port, `--tls require` by default (`on` also takes unencrypted
clients, which cannot pair; `off` encrypts nothing); `--upstream-compat` is the stated
downgrade for upstream DSPi Link clients.

```sh
uconsole hub tls fingerprint          # the key's fingerprint, and curl's --pinnedpubkey form
uconsole hub tls cert                 # the certificate, PEM (curl --cacert, Home Assistant)
uconsole hub tls rotate               # a new key, which clients follow for 30 days (hub stopped)
```

The fleet (hub-api.md sections 19 to 21): groups (`--no-groups` turns them off) and
the firmware rollout (`--rollout`, off by default, served over TLS only; an Admin updates
the DSPis on this machine's USB one at a time, the settings exported first):

```sh
uconsole hub firmware add DSPi.uf2    # store an image for a rollout (hub stopped; or at its prompt)
uconsole hub firmware list
```

At the running hub's prompt: `groups`, `group set [<id>] <name> <serial>...`, `group delete
<id>`, `firmware list`, `firmware add <file.uf2>`, `rollout <serial|group:id> <image>
[--while-live]` (the device must be muted unless `--while-live`), `rollout status [<id>]`
and `rollout cancel <id>`.

At the running hub's prompt, `inventory`, `alarms` and `schedule` list those. `uconsole
api` exits by the hub's answer: 0 ok, 2 not found, 3 another refusal, 4 no hub, 5 denied,
6 offline, 7 past the limits, 8 a bad request, 9 busy (`uconsole api --help`).

## Device access

- **Windows**: nothing to install. The firmware ships MS OS 2.0 descriptors, so
  WinUSB binds automatically.
- **macOS**: nothing to install. The vendor interface is *exclusive*, so only
  one program (the desktop app, Weeb Labs' DSPi Console or this tool) can hold
  the device at a time.
- **Linux and Raspberry Pi**: `sudo uconsole --install-udev`, then
  `sudo udevadm control --reload`, then unplug and replug the device. It writes
  `/etc/udev/rules.d/70-dspi.rules` (also in the archive), the same rule the
  desktop app and its packages install; it lets every account open a DSPi
  (`MODE="0666"`, so it works over SSH and for the hub's service) and covers the
  bootloader a firmware update uses.

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
